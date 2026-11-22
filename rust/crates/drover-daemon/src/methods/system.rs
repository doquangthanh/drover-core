//! RPC methods for the server itself, detection rules and config files. Port of `methods_system.py`
//! update.* / notify.test / skill.text: phase 05.

use std::sync::Arc;

use serde_json::{Value, json};

use drover_core::detection_rules::BUNDLED_RULES;
use drover_core::status_detector::evaluate;
use drover_core::status_tracker::now_secs;

use crate::ctx::Ctx;
use crate::method;
use crate::rpc::{MethodSpec, RpcResult, bad, need_str, opt_str};

const CONFIG_FILES: [&str; 4] = ["settings.toml", "keys.toml", "machines.toml", "AGENTS.md"];

pub fn methods() -> Vec<MethodSpec> {
    vec![
        method!("server.info", "Version, pid, port, uptime, clients", "", server_info),
        method!("server.stop", "Save state and stop the daemon", "", server_stop),
        method!("fonts.list", "Installed monospace font families (for the terminal font picker)", "", fonts_list),
        method!("schema", "This API as OpenRPC-style JSON", "", schema),
        method!("rules.list", "Loaded detection rule sets", "", rules_list),
        method!("rules.test", "Run detection rules against a pane and explain the result", "pane, kind?", rules_test),
        method!("settings.files", "Config files with their contents", "", settings_files),
        method!("settings.write", "Save a config file (validated TOML)", "name, text", settings_write),
        method!("update.check", "Is a newer release on GitHub?", "", update_check),
        method!("update.apply", "Download a release, replace this install and restart the daemon", "tag", update_apply),
        method!("skill.text", "Instructions that teach an agent to use dr", "", skill_text),
        method!("notify.test", "Send a test desktop notification", "", notify_test),
        method!("agent.kinds", "Agents from [agents] in settings, and whether each is installed here", "", agent_kinds),
    ]
}

/// `[agents]` in settings order: kind, label, command, and whether its program is on PATH of this machine.
async fn agent_kinds(ctx: Arc<Ctx>, _p: Value) -> RpcResult {
    let agents = ctx.settings.lock().unwrap().value("agents");
    let list: Vec<Value> = agents.as_object().into_iter().flatten().map(|(kind, a)| {
        let command = a["command"].as_str().unwrap_or(kind).to_string();
        let program = drover_core::launch_command::split_command(&command).and_then(|argv| argv.into_iter().next()).unwrap_or_default();
        let installed = !program.is_empty() && crate::launch::which(&program).is_some();
        json!({"kind": kind, "label": crate::notify::agent_label(kind), "command": command, "installed": installed})
    }).collect();
    Ok(json!(list))
}

async fn server_info(ctx: Arc<Ctx>, _p: Value) -> RpcResult {
    let mut counts = serde_json::Map::new();
    for pane in ctx.manager.lock().unwrap().panes.values() {
        let state = pane.state_name();
        let n = counts.get(&state).and_then(Value::as_i64).unwrap_or(0);
        counts.insert(state, json!(n + 1));
    }
    let platform = if cfg!(windows) { "win32" } else if cfg!(target_os = "macos") { "darwin" } else { "linux" };
    let clients: Vec<Value> = ctx.clients.lock().unwrap().values().cloned().collect();
    let settings_errors = ctx.settings.lock().unwrap().errors.clone();
    Ok(json!({
        "version": env!("DROVER_VERSION"), "pid": std::process::id(), "port": ctx.port(), "platform": platform,
        "started_at": ctx.started_at, "uptime": ctx.uptime(), "clients": clients, "counts": counts,
        "config_dir": crate::paths::config_dir().to_string_lossy(), "session_saved": ctx.store.path.exists(),
        "rule_errors": *ctx.rule_errors.lock().unwrap(), "settings_errors": settings_errors,
        "engine": "rust",
    }))
}

async fn update_check(_ctx: Arc<Ctx>, _p: Value) -> RpcResult {
    crate::self_update::check().await.map_err(bad)
}

async fn update_apply(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    crate::self_update::apply(&need_str(&p, "tag")?).await.map_err(bad)?;
    ctx.restart.store(true, std::sync::atomic::Ordering::SeqCst); // open windows follow the new daemon
    ctx.save_now();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        ctx.request_stop();
    });
    Ok(Value::Null)
}

async fn skill_text(_ctx: Arc<Ctx>, _p: Value) -> RpcResult {
    Ok(json!({"text": include_str!(concat!(env!("OUT_DIR"), "/agent_skill.md"))}))
}

async fn notify_test(_ctx: Arc<Ctx>, _p: Value) -> RpcResult {
    crate::notify::send("Drover notifications are on", "You'll see one like this when an agent needs you or finishes.", true, "normal");
    Ok(Value::Null)
}

async fn server_stop(ctx: Arc<Ctx>, _p: Value) -> RpcResult {
    ctx.save_now();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        ctx.request_stop();
    });
    Ok(Value::Null)
}

async fn fonts_list(_ctx: Arc<Ctx>, _p: Value) -> RpcResult {
    let fonts = tokio::task::spawn_blocking(crate::fonts::installed_monospace_fonts).await.unwrap_or_default();
    Ok(json!(fonts))
}

async fn schema(_ctx: Arc<Ctx>, _p: Value) -> RpcResult {
    let mut specs: Vec<&MethodSpec> = super::table().values().collect();
    specs.sort_by_key(|s| s.name);
    let methods: Vec<Value> = specs.iter().map(|s| json!({"name": s.name, "summary": s.doc, "params": s.params})).collect();
    Ok(json!({"openrpc": "1.2.6", "info": {"title": "drover daemon", "version": "1"}, "methods": methods}))
}

async fn rules_list(ctx: Arc<Ctx>, _p: Value) -> RpcResult {
    let m = ctx.manager.lock().unwrap();
    Ok(json!(m.rule_sets.iter().map(|r| json!({"id": r.id, "aliases": r.aliases, "source": r.source, "path": r.path,
        "resume": r.resume, "rules": r.rules.len()})).collect::<Vec<_>>()))
}

async fn rules_test(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let m = ctx.manager.lock().unwrap();
    let pane = m.get(&super::pane_param(&m, &p, "pane")?)?;
    let kind = match opt_str(&p, "kind") { k if !k.is_empty() => k, _ => pane.meta.lock().unwrap().kind.clone() };
    let st = pane.state.lock().unwrap();
    let mut lines = st.screen.visible_lines();
    let title = st.screen.title();
    let set = m.rule_set(&kind);
    let (winner, results) = evaluate(set, &title, &lines);
    let matched = results.iter().find(|r| r.matched).map(|r| r.rule_id.as_str());
    let reason = st.tracker.reason(matched, set.is_some(), st.last_output_at, now_secs());
    // A user file replacing a bundled set of the same id: tell, so a stale override is easy to spot.
    let overrides_bundled = set.is_some_and(|s| s.source == "user" && BUNDLED_RULES.iter().any(|(n, _)| *n == format!("{}.toml", s.id)));
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let tail = lines[lines.len().saturating_sub(20)..].to_vec();
    Ok(json!({"pane": pane.id(), "kind": kind, "title": title, "state": st.tracker.state, "detected": winner,
        "reason": reason, "rule_source": set.map(|s| s.source.as_str()), "rule_path": set.map(|s| s.path.as_str()),
        "overrides_bundled": overrides_bundled, "screen": tail,
        "results": results.iter().map(|r| json!({"rule_id": r.rule_id, "priority": r.priority, "region": r.region,
            "state": r.state, "matched": r.matched})).collect::<Vec<_>>(),
        "osc7501": st.tracker.reported}))
}

fn read_capped(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).map(|t| t.chars().take(50_000).collect()).unwrap_or_default()
}

fn rule_file_names(base: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(base.join("rules")).into_iter().flatten().flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".toml")).collect();
    names.sort();
    names
}

async fn settings_files(ctx: Arc<Ctx>, _p: Value) -> RpcResult {
    let base = crate::paths::config_dir();
    let mut files: Vec<Value> = CONFIG_FILES.iter().map(|name| {
        let path = base.join(name);
        json!({"name": name, "path": path.to_string_lossy(), "exists": path.exists(), "text": read_capped(&path)})
    }).collect();
    for name in rule_file_names(&base) {
        let path = base.join("rules").join(&name);
        files.push(json!({"name": format!("rules/{name}"), "path": path.to_string_lossy(), "exists": true, "text": read_capped(&path)}));
    }
    let mut errors = ctx.settings.lock().unwrap().errors.clone();
    errors.extend(ctx.rule_errors.lock().unwrap().iter().cloned());
    Ok(json!({"dir": base.to_string_lossy(), "files": files, "errors": errors}))
}

async fn settings_write(_ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let name = need_str(&p, "name")?;
    let text = opt_str(&p, "text");
    let base = crate::paths::config_dir();
    let known = CONFIG_FILES.contains(&name.as_str()) || rule_file_names(&base).iter().any(|n| format!("rules/{n}") == name);
    let new_rule = name.strip_prefix("rules/").is_some_and(|rest| name.ends_with(".toml") && !rest.contains('/') && !name.contains(".."));
    if !known && !new_rule {
        return Err(bad("unknown config file"));
    }
    if name.ends_with(".toml") && let Err(e) = text.parse::<toml::Table>() {
        return Err(bad(format!("invalid TOML: {e}")));
    }
    let path = base.join(&name);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&path, text).map_err(|e| crate::rpc::RpcError::new(crate::rpc::E_GENERAL, e.to_string()))?;
    Ok(Value::Null)
}
