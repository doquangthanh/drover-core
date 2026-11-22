//! One running program (agent or shell) in a pseudo-terminal, plus its derived state.
//! Port of `drover/core/pane_session.py`. Runtime-agnostic: the PTY reader thread calls in, the daemon reads state
//! under the mutex. The emulator is fast enough to run on every chunk, so there is no 0.15 s screen batching.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use serde_json::{Value, json};

use crate::osc7501::{parse_osc7501, report_summary};
pub use crate::pane_state::{PaneMeta, PaneState, REPLAY_LIMIT};
use crate::pty::{Pty, SpawnSpec};
use crate::status_tracker::now_secs;

type OutputListener = Arc<dyn Fn(&str, &str) + Send + Sync>; // (pane id, text)
type ExitHook = Box<dyn Fn(&Arc<PaneSession>) + Send + Sync>;

pub struct PaneSession {
    pub meta: Mutex<PaneMeta>,
    pub state: Mutex<PaneState>,
    pty: OnceLock<Arc<Pty>>,
    listeners: Mutex<Vec<(u64, OutputListener)>>,
    next_listener: AtomicU64,
    on_exit: Mutex<Option<ExitHook>>,
    me: Weak<PaneSession>,
}

impl PaneSession {
    pub fn new(meta: PaneMeta, cols: u16, rows: u16, history: usize) -> Arc<Self> {
        let state = PaneState::new(cols, rows, history);
        Arc::new_cyclic(|me| PaneSession {
            meta: Mutex::new(meta),
            state: Mutex::new(state),
            pty: OnceLock::new(),
            listeners: Mutex::new(Vec::new()),
            next_listener: AtomicU64::new(1),
            on_exit: Mutex::new(None),
            me: me.clone(),
        })
    }

    pub fn id(&self) -> String {
        self.meta.lock().unwrap().id.clone()
    }

    // -- lifecycle -------------------------------------------------------------------------------
    pub fn start(&self, argv: &[String], env: &[(String, String)]) -> Result<(), String> {
        // Lock order everywhere: meta before state, never both from here.
        let cwd = self.meta.lock().unwrap().cwd.clone();
        let (cols, rows) = {
            let st = self.state.lock().unwrap();
            (st.cols, st.rows)
        };
        let (out_me, exit_me) = (self.me.clone(), self.me.clone());
        let spec = SpawnSpec { argv, cwd: &cwd, env, cols, rows };
        let pty = Pty::spawn(
            spec,
            move |text| {
                if let Some(p) = out_me.upgrade() {
                    p.on_output(&text);
                }
            },
            move |code| {
                if let Some(p) = exit_me.upgrade() {
                    p.on_exit_code(code);
                }
            },
        )?;
        let _ = self.pty.set(pty);
        Ok(())
    }

    pub fn set_exit_hook(&self, hook: impl Fn(&Arc<PaneSession>) + Send + Sync + 'static) {
        *self.on_exit.lock().unwrap() = Some(Box::new(hook));
    }

    /// Output that did not come from the program in the pane (e.g. `dr show`): shown to viewers and kept in the replay.
    pub fn inject_output(&self, data: &str) {
        self.on_output(data);
    }

    fn on_output(&self, data: &str) {
        // Hot path: hand text to viewers first so typing feels instant.
        let id = self.id();
        // The listener lock is held until the chunk is in the replay buffer, so `attach` sees every chunk exactly
        // once: either in the replay it returns or through the listener it adds. Listeners only queue work.
        let listeners = self.listeners.lock().unwrap();
        for (_, l) in listeners.iter() {
            l(&id, data);
        }
        let now = now_secs();
        let mut st = self.state.lock().unwrap();
        st.last_output_at = now;
        st.replay.push_back(data.to_string());
        st.replay_size += data.len();
        while st.replay_size > REPLAY_LIMIT && st.replay.len() > 1 {
            let dropped = st.replay.pop_front().map(|s| s.len()).unwrap_or(0);
            st.replay_size -= dropped;
            st.replay_truncated = true;
        }
        if data.contains("\x1b]7501;")
            && let Some(report) = parse_osc7501(data)
        {
            if let Some(state) = report.get("state") {
                st.tracker.report_at(state, now);
            }
            if let Some(summary) = report_summary(&report)
                && summary != st.summary
            {
                st.summary = summary;
                st.summary_changed = true; // the status monitor publishes the pane
            }
        }
        drop(listeners);
        if st.screen.feed(data.as_bytes()) {
            st.screen_changed_at = now;
        }
        st.dirty = true;
    }

    fn on_exit_code(&self, code: Option<i32>) {
        {
            let mut st = self.state.lock().unwrap();
            st.exit_code = code;
            st.tracker.mark_exited();
        }
        let hook = self.on_exit.lock().unwrap();
        if let (Some(hook), Some(me)) = (hook.as_ref(), self.me.upgrade()) {
            hook(&me);
        }
    }

    // -- io --------------------------------------------------------------------------------------
    pub fn write(&self, data: &str) {
        if let Some(pty) = self.pty.get() {
            pty.write(data);
        }
    }

    /// True when the size changed (viewers need a `state` event to learn it).
    pub fn resize(&self, cols: u16, rows: u16) -> bool {
        let (cols, rows) = (cols.clamp(10, 500), rows.clamp(3, 300));
        let mut st = self.state.lock().unwrap();
        if (cols, rows) == (st.cols, st.rows) {
            return false;
        }
        st.cols = cols;
        st.rows = rows;
        st.screen.resize(cols as usize, rows as usize);
        drop(st);
        if let Some(pty) = self.pty.get() {
            pty.resize(cols, rows);
        }
        true
    }

    /// Terminate the process. Off the caller's thread by default (ConPTY teardown is slow); `wait` for shutdown.
    pub fn kill(&self, wait: bool) {
        let Some(pty) = self.pty.get().cloned() else { return };
        if wait {
            pty.kill();
        } else {
            std::thread::spawn(move || pty.kill());
        }
    }

    /// Hand the replay text to `on_replay`, then add `listener`, atomically: every chunk reaches the viewer exactly
    /// once and after the replay. `on_replay` runs under the listener lock, so it must only queue work.
    /// The replay is the raw output while it is complete, else a snapshot rebuilt from the emulator (full history).
    pub fn attach(&self, on_replay: impl FnOnce(&str), listener: impl Fn(&str, &str) + Send + Sync + 'static) -> u64 {
        let mut listeners = self.listeners.lock().unwrap();
        let replay: String = {
            let st = self.state.lock().unwrap();
            if st.replay_truncated { st.screen.snapshot() } else { st.replay.iter().map(String::as_str).collect() }
        };
        on_replay(&replay);
        let id = self.next_listener.fetch_add(1, Ordering::Relaxed);
        listeners.push((id, Arc::new(listener)));
        id
    }

    pub fn remove_output_listener(&self, id: u64) {
        self.listeners.lock().unwrap().retain(|(i, _)| *i != id);
    }

    pub fn alive(&self) -> bool {
        self.pty.get().is_some_and(|p| p.is_alive())
    }

    pub fn pid(&self) -> Option<u32> {
        self.pty.get().and_then(|p| p.pid)
    }

    pub fn foreground_pgid(&self) -> Option<i32> {
        self.pty.get().and_then(|p| p.foreground_pgid())
    }

    pub fn state_name(&self) -> String {
        self.state.lock().unwrap().tracker.state.clone()
    }

    /// Same keys as the Python `PaneSession.to_dict`.
    pub fn to_json(&self) -> Value {
        let m = self.meta.lock().unwrap();
        let st = self.state.lock().unwrap();
        json!({
            "id": m.id, "name": m.name, "kind": m.kind, "command": m.command, "cwd": m.cwd,
            "space_id": m.space_id, "tab_id": m.tab_id, "parent": m.parent,
            "created_at": m.created_at, "state": st.tracker.state, "since": st.tracker.since,
            "unread": st.unread, "exit_code": st.exit_code, "title": st.screen.title(),
            "machine": "local", "cols": st.cols, "rows": st.rows, "summary": st.summary,
        })
    }
}

#[cfg(all(test, unix))]
#[path = "pane_session_tests.rs"]
mod tests;
