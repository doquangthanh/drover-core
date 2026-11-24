//! The RPC method table (shared by the websocket UI and the HTTP CLI) and target resolution.

pub mod agent_input;
pub mod chat;
pub mod checks;
pub mod files_git;
pub mod layout;
pub mod machines;
pub mod panes;
pub mod review;
pub mod review_comments;
pub mod subagents;
pub mod system;
pub mod wait_children;
pub mod workspace;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use serde_json::Value;

use crate::ctx::Ctx;
use crate::manager::Manager;
use crate::rpc::{E_BAD_PARAMS, E_NO_METHOD, E_NOT_FOUND, MethodSpec, RpcError, RpcResult};

static TABLE: LazyLock<HashMap<&'static str, MethodSpec>> = LazyLock::new(|| {
    [system::methods(), workspace::methods(), panes::methods(), layout::methods(), agent_input::methods(), files_git::methods(), subagents::methods(), chat::methods(), machines::methods(), wait_children::methods(), checks::methods(), review::methods()]
        .into_iter()
        .flatten()
        .map(|m| (m.name, m))
        .collect()
});

pub fn table() -> &'static HashMap<&'static str, MethodSpec> {
    &TABLE
}

pub async fn call(ctx: Arc<Ctx>, name: &str, params: Value) -> RpcResult {
    let Some(spec) = TABLE.get(name) else {
        return Err(RpcError::new(E_NO_METHOD, format!("unknown method {name}")));
    };
    (spec.handler)(ctx, params).await
}

/// Resolve CLI targets ("%3", "auth", "self", "parent", "shop-api/auth") to a local pane id. Port of `target_resolver.py`.
pub fn resolve_pane(m: &Manager, target: &str, caller: Option<&str>) -> Result<String, RpcError> {
    if target.is_empty() {
        return Err(RpcError::new(E_BAD_PARAMS, "missing target"));
    }
    let parent_id;
    let target = if target == "self" {
        caller.filter(|c| !c.is_empty()).ok_or_else(|| RpcError::new(E_BAD_PARAMS, "'self' only works inside a drover pane"))?
    } else if target == "parent" {
        let parent = caller.and_then(|c| m.panes.get(c)).and_then(|p| p.meta.lock().unwrap().parent.clone())
            .ok_or_else(|| RpcError::new(E_BAD_PARAMS, "'parent' only works in a pane another agent started (dr run)"))?;
        if !m.panes.contains_key(&parent) {
            return Err(RpcError::new(E_NOT_FOUND, format!("the agent that started this one ({parent}) is gone")));
        }
        parent_id = parent;
        parent_id.as_str()
    } else {
        target
    };
    let target = target.strip_prefix("local:").unwrap_or(target);
    if m.panes.contains_key(target) {
        return Ok(target.to_string());
    }
    let (space_name, name) = target.rsplit_once('/').unwrap_or(("", target));
    let matches: Vec<String> = m.panes.values().filter_map(|p| {
        let meta = p.meta.lock().unwrap();
        if meta.name != name {
            return None;
        }
        if !space_name.is_empty() && m.spaces.get(&meta.space_id).is_none_or(|s| s.name != space_name) {
            return None;
        }
        Some(meta.id.clone())
    }).collect();
    match matches.len() {
        0 => Err(RpcError::new(E_NOT_FOUND, format!("no agent named '{target}'"))),
        1 => Ok(matches[0].clone()),
        _ => Err(RpcError::new(E_BAD_PARAMS, format!("'{target}' is ambiguous ({}); use space/name or id", matches.join(", ")))),
    }
}

/// `params[key]` as a pane target, resolved with the caller (`_pane` in Python).
pub fn pane_param(m: &Manager, p: &Value, key: &str) -> Result<String, RpcError> {
    let target = crate::rpc::opt_str(p, key);
    resolve_pane(m, &target, p.get("caller").and_then(Value::as_str))
}

/// The caller when it is a live pane here (agents pass their own pane id).
pub fn caller_pane(m: &Manager, p: &Value) -> Option<String> {
    p.get("caller").and_then(Value::as_str).filter(|c| m.panes.contains_key(*c)).map(String::from)
}
