//! RPC methods that act on panes: create, input, read, wait, close. Port of `methods_panes.py`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use drover_core::layout_tree::Dir;
use drover_core::pane_session::PaneSession;
use drover_core::status_tracker::now_secs;

use super::{caller_pane, pane_param};
use crate::ctx::Ctx;
use crate::manager::Manager;
use crate::manager_panes::NewPane;
use crate::messages_log::Policy;
use crate::method;
use crate::parent_notes::WaitGuard;
use crate::rpc::{E_BAD_PARAMS, E_POLICY, E_TIMEOUT, MethodSpec, RpcError, RpcResult, bad, flag, need_str, not_found, opt_f64, opt_i64, opt_str, truthy};

static ANSWER_IDS: AtomicU64 = AtomicU64::new(1);

pub fn methods() -> Vec<MethodSpec> {
    vec![
        method!("pane.create", "Start an agent or shell", "command?, cwd?, name?, tab_id?, split_from?, direction?, caller?, worktree?: {task, base?}, cols?, rows?, outlive_parent?", pane_create),
        method!("pane.write", "Raw keystrokes from the UI", "pane, data", pane_write),
        method!("pane.send", "Type text into an agent (as `dr send`)", "pane, text, enter?, caller?", pane_send),
        method!("policy.decide", "Allow or deny an agent's answer to a prompt (user only)", "id, allow", policy_decide),
        method!("pane.resize", "Resize the pseudo-terminal", "pane, cols, rows", pane_resize),
        method!("pane.show", "Show a PNG in a pane (Kitty graphics, bypasses the terminal)", "pane, png (base64)", pane_show),
        method!("pane.read", "Plain text of a pane (scrollback + screen)", "pane, lines?", pane_read),
        method!("pane.wait", "Block until a pane reaches a state", "pane, states?, timeout?", pane_wait),
        method!("pane.close", "Stop a pane and remove it from its tab", "pane, subtree?", pane_close),
        method!("pane.restart", "Restart a pane (optionally with its resume command)", "pane, resume?", pane_restart),
        method!("pane.rename", "Rename a pane", "pane, name", pane_rename),
        method!("pane.focus", "Mark a pane focused and read", "pane", pane_focus),
        method!("pane.list", "List panes with state", "", pane_list),
    ]
}

fn truthy_value(v: &Value) -> bool {
    truthy(&json!({"v": v}), "v")
}

pub fn direction_of(p: &Value, key: &str) -> Option<Dir> {
    match p.get(key).and_then(Value::as_str) {
        Some("col") | Some("down") => Some(Dir::Col),
        _ => Some(Dir::Row),
    }
}

async fn pane_create(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let mut cwd = opt_str(&p, "cwd");
    if let Some(wt) = p.get("worktree").filter(|w| truthy_value(w)) {
        let repo = if cwd.is_empty() { ".".to_string() } else { cwd.clone() };
        let info = crate::git::create_worktree(&repo, &opt_str(wt, "task"), &opt_str(wt, "base")).await
            .map_err(|e| RpcError::new(crate::rpc::E_GENERAL, e))?;
        cwd = info["path"].as_str().unwrap_or("").to_string();
    }
    let mut m = ctx.manager.lock().unwrap();
    let cols = opt_i64(&p, "cols", 120)?.clamp(1, u16::MAX as i64) as u16;
    let rows = opt_i64(&p, "rows", 32)?.clamp(1, u16::MAX as i64) as u16;
    let req = NewPane {
        command: opt_str(&p, "command"), cwd, name: opt_str(&p, "name"),
        tab_id: p.get("tab_id").and_then(Value::as_str).map(String::from),
        split_from: p.get("split_from").and_then(Value::as_str).map(String::from),
        direction: direction_of(&p, "direction"), parent: caller_pane(&m, &p), cols: Some(cols), rows: Some(rows),
        new_space: false, outlive_parent: truthy(&p, "outlive_parent"),
    };
    Ok(m.create_pane(req).map_err(|e| e.policy_rpc())?.to_json())
}

async fn pane_write(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    if truthy(&p, "caller") {
        return Err(RpcError::new(E_POLICY, "agents must use pane.send (dr send), not raw pane.write"));
    }
    let pane = ctx.manager.lock().unwrap().get(&need_str(&p, "pane")?)?;
    let data = opt_str(&p, "data");
    pane.state.lock().unwrap().draft.feed(&data, drover_core::status_tracker::now_secs());
    pane.write(&data);
    Ok(Value::Null)
}

/// New input = new turn. Leave done/idle at once so a following `dr wait` waits for *this* turn.
pub fn mark_busy(m: &Manager, pane: &PaneSession) {
    let old = {
        let mut st = pane.state.lock().unwrap();
        let old = st.tracker.state.clone();
        if old != "done" && old != "idle" {
            return;
        }
        st.tracker.force_working(now_secs(), 3.0);
        st.unread = false;
        old
    };
    let data = pane.to_json();
    m.hub.emit("pane", data.clone());
    m.hub.emit("status", json!({"pane": data, "old": old, "new": "working"}));
}

/// An agent typing into a pane stuck on a permission prompt: refuse, or forward the answer to the user.
pub fn gate_agent_input(ctx: &Ctx, m: &mut Manager, pane_id: &str, pane: &PaneSession, data: &str, label: &str, caller: Option<&str>) -> Result<(), RpcError> {
    let Some(caller) = caller.filter(|c| !c.is_empty() && *c != pane_id) else { return Ok(()) };
    let policy = Policy::from_settings(&ctx.settings.lock().unwrap().value("orchestration"));
    if policy.may_agent_answer(&pane.state_name()) {
        return Ok(());
    }
    if policy.answer_prompts == "never" {
        return Err(RpcError::new(E_POLICY, "agents may not answer permission prompts (policy: never)"));
    }
    let answer_id = format!("a{}", ANSWER_IDS.fetch_add(1, Ordering::Relaxed));
    ctx.pending_answers.lock().unwrap().insert(answer_id.clone(), (pane_id.into(), data.into(), caller.into()));
    m.messages.add("denied", Some(caller), Some(pane_id), label);
    m.hub.emit("policy", json!({"id": answer_id, "source": caller, "target": pane_id, "text": label}));
    m.hub.emit("state", Value::Null);
    Err(RpcError::new(E_POLICY, "target is waiting on a permission prompt; forwarded to the user"))
}

async fn pane_send(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let mut m = ctx.manager.lock().unwrap();
    let pane_id = pane_param(&m, &p, "pane")?;
    let pane = m.get(&pane_id)?;
    let mut text = opt_str(&p, "text");
    let enter = flag(&p, "enter", true);
    let caller = p.get("caller").and_then(Value::as_str).filter(|c| !c.is_empty());
    let helper = caller.and_then(|c| m.panes.get(c)).map(|s| s.meta.lock().unwrap().clone())
        .filter(|s| s.parent.as_deref() == Some(pane_id.as_str()));
    if let Some(s) = helper {
        text = format!("<dr from=\"{} {}\">{text}</dr>", s.id, s.name); // a helper reporting back: say who it is
    }
    let data = if enter { format!("{text}\r") } else { text.clone() };
    gate_agent_input(&ctx, &mut m, &pane_id, &pane, &data, &text, caller)?;
    pane.write(&data);
    if enter {
        mark_busy(&m, &pane);
    }
    if let Some(c) = caller {
        m.messages.add("send", Some(c), Some(&pane_id), &text);
    }
    Ok(json!({"pane": pane_id}))
}

async fn policy_decide(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    if truthy(&p, "caller") {
        return Err(RpcError::new(E_POLICY, "only the user can decide; agents may not approve answers"));
    }
    let id = need_str(&p, "id")?;
    let (pane_id, data, caller) = ctx.pending_answers.lock().unwrap().shift_remove(&id).ok_or_else(|| not_found(&id))?;
    let mut m = ctx.manager.lock().unwrap();
    if truthy(&p, "allow") && let Some(pane) = m.panes.get(&pane_id).cloned() {
        pane.write(&data);
        m.messages.add("send", Some(&caller), Some(&pane_id), data.trim());
    }
    m.hub.emit("state", Value::Null);
    Ok(Value::Null)
}

async fn pane_resize(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let (Some(cols), Some(rows)) = (p.get("cols").and_then(Value::as_f64), p.get("rows").and_then(Value::as_f64)) else {
        return Err(RpcError::new(E_BAD_PARAMS, "cols and rows must be numbers"));
    };
    let pane = ctx.manager.lock().unwrap().get(&need_str(&p, "pane")?)?;
    // Viewers size their screens from `state`: without an event they keep the old size until something else changes.
    if pane.resize(cols.clamp(0.0, 65535.0) as u16, rows.clamp(0.0, 65535.0) as u16) {
        ctx.manager.lock().unwrap().changed();
    }
    Ok(Value::Null)
}

async fn pane_read(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let (pane_id, pane, lines) = {
        let mut m = ctx.manager.lock().unwrap();
        let pane_id = pane_param(&m, &p, "pane")?;
        let lines = if truthy(&p, "lines") { Some(opt_i64(&p, "lines", 0)?.max(0) as usize) } else { None };
        if let Some(caller) = p.get("caller").and_then(Value::as_str).filter(|c| !c.is_empty()) {
            let what = lines.map(|n| n.to_string()).unwrap_or_else(|| "all".into());
            m.messages.add("read", Some(caller), Some(&pane_id), &format!("{what} lines"));
        }
        let pane = m.get(&pane_id)?;
        (pane_id, pane, lines)
    };
    if let Some(n) = lines
        && let Some(text) = super::agent_input::read_full_screen_agent(&ctx, &pane, n).await
    {
        return Ok(json!({"pane": pane_id, "text": text}));
    }
    let text = pane.state.lock().unwrap().screen.read_text(lines);
    Ok(json!({"pane": pane_id, "text": text}))
}

/// Resolve once the pane is in `wanted` (now, if `accept_current`, or on the next transition).
pub async fn wait_for_state(ctx: &Ctx, pane: &PaneSession, wanted: HashSet<String>, timeout: f64, accept_current: bool) -> Result<String, RpcError> {
    let pane_id = pane.id();
    let current = pane.state_name();
    if accept_current && wanted.contains(&current) {
        return Ok(current);
    }
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx = Mutex::new(Some(tx));
    let wanted_l = wanted.clone();
    let sub = ctx.hub.subscribe(move |name, data| {
        if name != "status" || data["pane"]["id"].as_str() != Some(pane_id.as_str()) {
            return;
        }
        let new = data["new"].as_str().unwrap_or("");
        // A pane someone is looking at goes working -> idle, never "done": that is still its turn finishing.
        let finished = wanted_l.contains("done") && data["old"] == "working" && new == "idle";
        if (wanted_l.contains(new) || finished) && let Some(tx) = tx.lock().unwrap().take() {
            let _ = tx.send(new.to_string());
        }
    });
    let result = tokio::time::timeout(Duration::from_secs_f64(timeout.max(0.0)), rx).await;
    ctx.hub.unsubscribe(sub);
    match result {
        Ok(Ok(state)) => Ok(state),
        _ => Err(RpcError::new(E_TIMEOUT, format!("timed out after {timeout:.0}s; state is {}", pane.state_name()))),
    }
}

async fn pane_wait(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let (pane_id, pane, wanted, timeout) = {
        let mut m = ctx.manager.lock().unwrap();
        let pane_id = pane_param(&m, &p, "pane")?;
        let mut wanted: HashSet<String> = p.get("states").and_then(Value::as_array).filter(|a| !a.is_empty())
            .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
            .unwrap_or_else(|| ["done", "idle", "blocked"].map(String::from).into());
        let mut shown: Vec<&String> = wanted.iter().collect();
        shown.sort();
        let label = shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(",");
        wanted.insert("exited".into());
        if let Some(caller) = p.get("caller").and_then(Value::as_str).filter(|c| !c.is_empty()) {
            m.messages.add("wait", Some(caller), Some(&pane_id), &label);
        }
        let pane = m.get(&pane_id)?;
        (pane_id, pane, wanted, opt_f64(&p, "timeout", 600.0)?)
    };
    // "idle" is the starting state of every pane, so only a *transition* into idle counts.
    let accept_current = pane.state_name() != "idle";
    let caller = p.get("caller").and_then(Value::as_str).filter(|c| !c.is_empty());
    let guard = WaitGuard::new(&ctx.manager, caller);
    let state = wait_for_state(&ctx, &pane, wanted, timeout, accept_current).await;
    drop(guard);
    let state = state?;
    if let Some(c) = caller {
        ctx.manager.lock().unwrap().notes.drop_children(c, std::slice::from_ref(&pane_id)); // this reply is the news
    }
    Ok(json!({"pane": pane_id, "state": state}))
}

fn close_tree(m: &mut Manager, pane_id: &str, subtree: bool) -> Result<(), RpcError> {
    if subtree {
        for child in m.children_of(pane_id) {
            close_tree(m, &child, true)?;
        }
    }
    m.close_pane(pane_id, true)
}

async fn pane_close(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let mut m = ctx.manager.lock().unwrap();
    let pane_id = pane_param(&m, &p, "pane")?;
    close_tree(&mut m, &pane_id, truthy(&p, "subtree"))?;
    Ok(Value::Null)
}

async fn pane_restart(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let mut m = ctx.manager.lock().unwrap();
    let pane = m.get(&pane_param(&m, &p, "pane")?)?;
    let meta = pane.meta.lock().unwrap().clone();
    let resume = if flag(&p, "resume", true) { meta.resume.clone() } else { String::new() };
    let command = if resume.is_empty() { meta.command.clone() } else { resume };
    let new = m.create_pane(NewPane {
        command: if meta.kind != "shell" { command } else { String::new() }, cwd: meta.cwd.clone(), name: meta.name.clone(),
        tab_id: Some(meta.tab_id.clone()), split_from: Some(meta.id.clone()), parent: meta.parent.clone(), ..Default::default()
    }).map_err(|e| e.rpc(crate::rpc::E_GENERAL))?;
    m.close_pane(&meta.id, true)?;
    Ok(new.to_json())
}

async fn pane_rename(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let m = ctx.manager.lock().unwrap();
    let pane_id = pane_param(&m, &p, "pane")?;
    let name: String = need_str(&p, "name")?.trim().chars().take(60).collect();
    // The resolver treats "self", "parent", "local" and "%..." specially, so such names could never be used as targets.
    if name.is_empty() || name.contains('/') || name.contains(':') || ["self", "parent", "local"].contains(&name.as_str()) || name.starts_with('%') {
        return Err(bad("name must be non-empty, contain no '/' or ':', not start with '%' and not be 'self', 'parent' or 'local'"));
    }
    let clash = m.panes.values().find(|o| { let x = o.meta.lock().unwrap(); x.name == name && x.id != pane_id }).map(|o| o.id());
    if let Some(other) = clash {
        return Err(bad(format!("name '{name}' is already used by {other}")));
    }
    m.get(&pane_id)?.meta.lock().unwrap().name = name.clone();
    m.changed();
    Ok(json!({"pane": pane_id, "name": name}))
}

async fn pane_focus(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let mut m = ctx.manager.lock().unwrap();
    let id = pane_param(&m, &p, "pane")?;
    m.focus(&id)?;
    Ok(Value::Null)
}

async fn pane_list(ctx: Arc<Ctx>, _p: Value) -> RpcResult {
    Ok(json!(ctx.manager.lock().unwrap().panes.values().map(|p| p.to_json()).collect::<Vec<_>>()))
}

/// Largest PNG `pane.show` takes (base64 characters).
const SHOW_LIMIT: usize = 8 * 1024 * 1024;

/// Put a PNG in a pane as Kitty graphics. The daemon writes it into the pane's output itself, so it reaches viewers
/// even where the OS terminal layer (Windows ConPTY) would drop the escape sequence.
async fn pane_show(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let png = opt_str(&p, "png");
    if !png.starts_with("iVBORw0KGgo") || png.len() > SHOW_LIMIT || !png.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=')) {
        return Err(bad("png must be base64 PNG data of at most 6 MB"));
    }
    let (pane_id, pane) = {
        let m = ctx.manager.lock().unwrap();
        let id = pane_param(&m, &p, "pane")?;
        (id.clone(), m.get(&id)?)
    };
    pane.inject_output(&kitty_png(&png));
    Ok(json!({"pane": pane_id}))
}

/// `ESC _G f=100,a=T,q=2 ; <base64> ESC \` in chunks of 4096, on its own line (q=2: the viewer sends no reply).
fn kitty_png(b64: &str) -> String {
    let chunks: Vec<&str> = b64.as_bytes().chunks(4096).map(|c| std::str::from_utf8(c).unwrap_or("")).collect();
    let mut out = String::from("\r");
    for (i, c) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        let head = if i == 0 { format!("f=100,a=T,q=2,m={more}") } else { format!("q=2,m={more}") };
        out.push_str(&format!("\x1b_G{head};{c}\x1b\\"));
    }
    out.push_str("\r\n");
    out
}

#[cfg(test)]
mod show_tests {
    use super::kitty_png;

    #[test]
    fn splits_into_chunks_and_marks_the_last() {
        let s = kitty_png(&"A".repeat(4096 + 10));
        assert_eq!(s.matches("\x1b_G").count(), 2);
        assert!(s.contains("f=100,a=T,q=2,m=1;") && s.contains("q=2,m=0;AAAAAAAAAA\x1b\\"));
        assert!(s.starts_with('\r') && s.ends_with("\r\n"));
    }
}
