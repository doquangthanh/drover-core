//! Starting panes and reacting to their exit (second half of `session_manager.py`).

use std::path::Path;
use std::sync::Arc;

use serde_json::json;

use drover_core::launch_command::{match_agent_kind, split_command};
use drover_core::layout_tree::{self as layout, Dir};
use drover_core::pane_session::{PaneMeta, PaneSession};
use drover_core::status_tracker::now_secs;
use drover_core::workspace_models::new_id;

use crate::launch::{default_shell, in_login_shell, pane_env, resolve_argv, with_subagent_hooks};
use crate::manager::Manager;
use crate::messages_log::Policy;
use crate::rpc::{E_BAD_PARAMS, E_GENERAL, E_POLICY, RpcError};

#[derive(Default)]
pub struct NewPane {
    pub command: String,
    pub cwd: String,
    pub name: String,
    pub tab_id: Option<String>,
    pub split_from: Option<String>,
    pub direction: Option<Dir>,
    pub parent: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub new_space: bool,
    pub outlive_parent: bool,
}

/// Why a pane could not start. `rpc(policy_code)`: pane.create reports a policy refusal as code 4; other callers
/// (space.create, session.resume) let it surface as a general error, like the Python daemon.
pub enum SpawnError {
    Policy(String),
    Bad(String),
    Failed(String),
}

impl SpawnError {
    pub fn rpc(self, policy_code: i64) -> RpcError {
        match self {
            SpawnError::Policy(m) => RpcError::new(policy_code, m),
            SpawnError::Bad(m) => RpcError::new(E_BAD_PARAMS, m),
            SpawnError::Failed(m) => RpcError::new(E_GENERAL, m),
        }
    }

    pub fn policy_rpc(self) -> RpcError {
        self.rpc(E_POLICY)
    }
}

fn home() -> String {
    dirs::home_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into())
}

impl Manager {
    pub fn create_pane(&mut self, req: NewPane) -> Result<Arc<PaneSession>, SpawnError> {
        let (orchestration, shell, scrollback, hooks) = {
            let s = self.settings.lock().unwrap();
            (s.value("orchestration"), s.str("shell", ""), s.int("scrollback", 5000).max(0) as usize,
             s.bool("claude_subagent_hooks", true))
        };
        let live = self.panes.values().filter(|p| p.alive()).count();
        let parent_depth = req.parent.as_ref().and_then(|p| self.depth_of(Some(p.clone())));
        Policy::from_settings(&orchestration).check_spawn(live, parent_depth).map_err(SpawnError::Policy)?;
        let cwd = if !req.cwd.is_empty() {
            req.cwd.clone()
        } else if let Some(parent) = req.parent.as_ref().and_then(|p| self.panes.get(p)) {
            parent.meta.lock().unwrap().cwd.clone()
        } else {
            home()
        };
        if !Path::new(&cwd).is_dir() {
            return Err(SpawnError::Bad(format!("folder does not exist: {cwd}")));
        }
        let command = req.command.trim().to_string();
        let argv = if command.is_empty() {
            default_shell(&shell)
        } else {
            split_command(&req.command).ok_or_else(|| SpawnError::Bad("No closing quotation".into()))?
        };
        if crate::launch::which(&argv[0]).is_none() {
            // Same message as the Python daemon (ptyprocess), which scripts and tests match on.
            return Err(SpawnError::Failed(format!("The command was not found or was not executable: {}.", argv[0])));
        }
        let kind = if command.is_empty() { "shell".to_string() } else { match_agent_kind(&req.command, &self.rule_sets) };
        let name = if req.name.is_empty() { self.default_name(&kind) } else { req.name.clone() };
        let pane_id = new_id("%");
        let meta = PaneMeta {
            id: pane_id.clone(), name: name.clone(), kind: kind.clone(),
            command: if req.command.is_empty() { argv.join(" ") } else { req.command.clone() },
            argv: argv.clone(), cwd: cwd.clone(), parent: req.parent.clone(), created_at: now_secs(),
            resume: self.rule_set(&kind).map(|r| r.resume.clone()).unwrap_or_default(),
            outlive_parent: req.outlive_parent,
            ..Default::default()
        };
        let pane = PaneSession::new(meta, req.cols.unwrap_or(120), req.rows.unwrap_or(32), scrollback);
        pane.state.lock().unwrap().shell_launched = kind == "shell";
        let me = self.me.clone();
        pane.set_exit_hook(move |p| {
            if let Some(m) = me.upgrade() {
                m.lock().unwrap().on_pane_exit(p);
            }
        });
        // claude panes report their sub-agents (Task/Agent tool) to the Flow board through per-pane hooks.
        let launch = if hooks { with_subagent_hooks(argv) } else { argv };
        // Linux / macOS: a command (agent, `npm test`…) runs inside the login shell, like typing it in a terminal.
        // Windows keeps the direct start (ConPTY + resolve_argv); a plain shell pane is that shell already.
        let launch = if cfg!(windows) || command.is_empty() { launch } else { in_login_shell(&default_shell(&shell)[0], launch) };
        // May fail: create no space/tab before this succeeds.
        pane.start(&resolve_argv(launch), &pane_env(&pane_id)).map_err(SpawnError::Failed)?;
        let tab_id = match req.tab_id.filter(|t| self.tabs.contains_key(t)) {
            Some(t) => t,
            None => {
                let space = if req.new_space { self.add_space(&cwd) } else { self.ensure_space(&cwd) };
                self.create_tab(&space, &name)
            }
        };
        let tab = self.tabs.get_mut(&tab_id).expect("tab exists");
        {
            let mut m = pane.meta.lock().unwrap();
            m.space_id = tab.space_id.clone();
            m.tab_id = tab_id.clone();
        }
        let target = req.split_from.or(tab.focused.clone());
        tab.layout = Some(layout::insert(tab.layout.as_ref(), target.as_deref(), &pane_id, req.direction.unwrap_or(Dir::Row)));
        tab.focused = Some(pane_id.clone());
        self.panes.insert(pane_id.clone(), pane.clone());
        // parent None = started by you (UI / CLI), so every agent gets a lane on the Flow board.
        let command = pane.meta.lock().unwrap().command.clone();
        self.messages.add("spawn", req.parent.as_deref(), Some(&pane_id), &command);
        self.changed();
        Ok(pane)
    }

    pub fn on_pane_exit(&mut self, pane: &Arc<PaneSession>) {
        let (id, parent) = { let m = pane.meta.lock().unwrap(); (m.id.clone(), m.parent.clone()) };
        let code = pane.state.lock().unwrap().exit_code;
        let code_text = code.map(|c| c.to_string()).unwrap_or_else(|| "None".into());
        // A one-shot helper (`dr run claude -p ...`) exits instead of going "done": still report it.
        self.messages.add_state(&id, parent.as_deref(), "exited", &format!("exit code {code_text}"));
        self.report_to_parent(pane, "exited", &format!("exit code {code_text}"));
        let data = pane.to_json();
        self.hub.emit("pane", data.clone());
        self.hub.emit("status", json!({"pane": data, "old": "working", "new": "exited"}));
        let (focused, title) = { let st = pane.state.lock().unwrap(); (st.focused, st.screen.title()) };
        if !focused {
            let (name, kind) = { let m = pane.meta.lock().unwrap(); (m.name.clone(), m.kind.clone()) };
            crate::notify::notify_state(&self.settings.lock().unwrap(), &name, &kind, "exited", &title);
        }
        let kill_children = self.settings.lock().unwrap().bool("orchestration.kill_children_on_parent_exit", true);
        if kill_children {
            for child in self.children_of(&id) {
                if let Some(c) = self.panes.get(&child).filter(|c| !c.meta.lock().unwrap().outlive_parent) {
                    c.kill(false);
                }
            }
        }
        self.changed();
    }
}
