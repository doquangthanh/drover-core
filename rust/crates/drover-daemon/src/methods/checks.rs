//! RPC methods `check.*`: post-run checks of a folder (`.drover/checks.toml`). Commands run only on `check.run`.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::watch;

use drover_core::status_tracker::now_secs;

use crate::checks::config::{self, Step};
use crate::checks::{Run, StepResult, empty_run, folder_key, runner};
use crate::ctx::Ctx;
use crate::fs_access;
use crate::git;
use crate::method;
use crate::rpc::{E_BAD_PARAMS, E_GENERAL, MethodSpec, RpcError, RpcResult, need_str, opt_str};

pub fn methods() -> Vec<MethodSpec> {
    vec![
        method!("check.config", "The post-run check steps configured for a folder (.drover/checks.toml)", "cwd", check_config),
        method!("check.run", "Run the folder's checks now, one after another (one run per folder at a time)", "cwd, pane?, caller?", check_run),
        method!("check.list", "The latest check run of a folder with every step's output (status none if never run)", "cwd", check_list),
        method!("check.cancel", "Stop the folder's running checks", "cwd", check_cancel),
    ]
}

/// The folder as a resolved path plus the key clients use for it.
fn folder(p: &Value) -> Result<(String, String), RpcError> {
    let path = fs_access::resolve(std::path::Path::new(&need_str(p, "cwd")?));
    if !path.is_dir() {
        return Err(RpcError::new(E_BAD_PARAMS, format!("not a folder: {}", path.display())));
    }
    let s = path.to_string_lossy().into_owned();
    let key = folder_key(&s);
    Ok((s, key))
}

/// Steps and where they came from: the repo's `.drover/checks.toml` (git root first, then the folder itself).
async fn load_steps(cwd: &str) -> Result<(Vec<Step>, &'static str), RpcError> {
    let root = git::git(cwd, &["rev-parse", "--show-toplevel"], true).await.ok().map(|s| PathBuf::from(s.trim()));
    let dirs = root.into_iter().chain(std::iter::once(PathBuf::from(cwd)));
    for dir in dirs {
        if let Some(steps) = config::load(&dir).map_err(|e| RpcError::new(E_BAD_PARAMS, e))? {
            return Ok((steps, "repo"));
        }
    }
    Ok((vec![], "none"))
}

async fn check_config(_ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let (cwd, _) = folder(&p)?;
    let (steps, source) = load_steps(&cwd).await?;
    let steps: Vec<Value> = steps.iter().map(|s| json!({"name": s.name, "command": s.command})).collect();
    Ok(json!({"steps": steps, "source": source}))
}

async fn check_run(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let (cwd, key) = folder(&p)?;
    let (steps, source) = load_steps(&cwd).await?;
    if steps.is_empty() {
        return Err(RpcError::new(E_GENERAL, "no checks configured: add steps to .drover/checks.toml"));
    }
    let pane = ["pane", "caller"].iter().map(|k| opt_str(&p, k)).find(|s| !s.is_empty());
    let (tx, rx) = watch::channel(false);
    let run = Run {
        cwd: key.clone(), pane, status: "queued".into(), source: source.into(), started_at: now_secs(), finished_at: None,
        results: steps.iter().map(|s| StepResult::pending(&s.name, &s.command)).collect(), cancel: tx,
    };
    let summary = run.summary();
    ctx.checks.start(&key, run).map_err(|e| RpcError::new(E_GENERAL, e))?;
    ctx.hub.emit("check", summary.clone());
    tokio::spawn(runner::execute(ctx.checks.clone(), ctx.hub.clone(), key, cwd, steps, rx));
    Ok(summary)
}

async fn check_list(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let (_, key) = folder(&p)?;
    Ok(ctx.checks.full(&key).unwrap_or_else(|| empty_run(&key)))
}

async fn check_cancel(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let (_, key) = folder(&p)?;
    Ok(json!({"cancelled": ctx.checks.cancel(&key)}))
}
