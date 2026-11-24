//! Drover daemon: owns the panes, serves the UI, `/ws` and `/rpc`. Port of `drover/daemon`.

pub mod chat;
pub mod checks;
pub mod claude_session;
pub mod codex_session;
pub mod ctx;
pub mod event_hub;
pub mod fonts;
pub mod fs_access;
pub mod git;
pub mod http;
pub mod launch;
pub mod log;
pub mod manager;
pub mod manager_panes;
pub mod messages_log;
pub mod methods;
pub mod notify;
pub mod parent_notes;
pub mod paths;
pub mod persistence;
pub mod remote;
pub mod review;
pub mod rpc;
pub mod self_update;
pub mod self_update_apply;
#[cfg(windows)]
pub mod self_update_windows;
#[cfg(windows)]
mod legacy_runtime_migration;
pub mod settings;
pub mod state_store;
pub mod status_monitor;
pub mod ws_session;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU16};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};
use tokio::sync::Notify;

use drover_core::detection_rules::load_rule_sets;
use drover_core::status_tracker::now_secs;

use crate::ctx::Ctx;
use crate::event_hub::EventHub;
use crate::manager::Manager;
use crate::settings::Settings;
use crate::state_store::StateStore;

pub fn read_info() -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(paths::daemon_info_file()).ok()?).ok()
}

/// A daemon answers /health on the port in `info`.
pub fn is_alive(info: Option<&Value>) -> bool {
    let Some(port) = info.and_then(|i| i["port"].as_u64()) else { return false };
    let addr = SocketAddr::from(([127, 0, 0, 1], port as u16));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(1500)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_millis(1500)));
    if s.write_all(b"GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").is_err() {
        return false;
    }
    let mut body = String::new();
    let _ = s.read_to_string(&mut body);
    body.contains("\"app\":\"drover\"") || body.contains("\"app\": \"drover\"")
}

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("os random");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Create daemon.json owner-only from the first byte, then rename into place.
fn write_info(info: &Value) -> std::io::Result<()> {
    let path = paths::daemon_info_file();
    let tmp = path.with_extension("tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(&tmp)?.write_all(info.to_string().as_bytes())?;
    paths::restrict_to_owner(&tmp);
    std::fs::rename(&tmp, &path)
}

pub fn build_context(token: String) -> Arc<Ctx> {
    let settings = Arc::new(Mutex::new(Settings::load(paths::config_dir())));
    let hub = Arc::new(EventHub::default());
    let (rule_sets, rule_errors) = load_rule_sets(Some(&paths::user_rules_dir()));
    let save = Arc::new(Notify::new());
    let manager = Manager::new(settings.clone(), hub.clone(), rule_sets, save);
    Arc::new(Ctx {
        chats: crate::chat::manager::ChatManager::new(paths::state_dir().join("chats"), hub.clone()),
        remotes: Arc::new(crate::remote::hub::RemoteHub::new(hub.clone(), settings.clone())),
        settings, hub, manager, store: StateStore { path: paths::session_file() },
        rule_errors: Mutex::new(rule_errors), token, port: AtomicU16::new(0), started_at: now_secs(),
        clients: Mutex::new(Default::default()), pending_answers: Mutex::new(Default::default()),
        stop: Notify::new(), stopping: AtomicBool::new(false), restart: AtomicBool::new(false),
        input_locks: Default::default(), read_locks: Default::default(), checks: Default::default(),
    })
}

/// When `serve` began, for the start-up time in the log.
static SERVE_START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// `dr server run`: blocks until the daemon stops.
pub fn serve(host: &str, port: u16, foreground: bool) -> std::io::Result<()> {
    SERVE_START.get_or_init(std::time::Instant::now);
    // A self-update restart keeps the old port and token, so open browser tabs reconnect on their own. Taken out
    // of the environment so panes (agents) never inherit the token; done before any thread starts.
    let keep = (std::env::var("DROVER_KEEP_TOKEN").unwrap_or_default(),
                std::env::var("DROVER_KEEP_PORT").ok().and_then(|p| p.parse::<u16>().ok()).unwrap_or(0));
    // SAFETY: still single-threaded here (no runtime, no logger thread yet).
    unsafe {
        std::env::remove_var("DROVER_KEEP_TOKEN");
        std::env::remove_var("DROVER_KEEP_PORT");
    }
    log::init(foreground);
    #[cfg(windows)]
    if legacy_runtime_migration::hand_over(host, foreground, &keep) {
        return Ok(());
    }
    if is_alive(read_info().as_ref()) {
        log::info("another drover daemon is already running; exiting");
        return Ok(());
    }
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(serve_async(host.to_string(), port, keep))
}

async fn serve_async(host: String, port: u16, (keep_token, keep_port): (String, u16)) -> std::io::Result<()> {
    let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;
    let t = std::time::Instant::now();
    let ctx = build_context(if keep_token.is_empty() { new_token() } else { keep_token });
    let context_ms = ms(t);
    let t = std::time::Instant::now();
    persistence::load_previous(&mut ctx.manager.lock().unwrap(), &ctx.store);
    if ctx.settings.lock().unwrap().bool("session.resume_on_start", true) {
        persistence::resume_all(&mut ctx.manager.lock().unwrap());
    }
    let session_ms = ms(t);
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", if keep_port > 0 { keep_port } else { port })).await {
        Ok(l) => l,
        Err(_) => tokio::net::TcpListener::bind(("127.0.0.1", port)).await?, // the kept port was taken meanwhile
    };
    let bound = listener.local_addr()?.port();
    ctx.port.store(bound, std::sync::atomic::Ordering::SeqCst);
    let app = http::router(ctx.clone());
    if !matches!(host.as_str(), "127.0.0.1" | "localhost" | "") {
        // Extra, opt-in address (e.g. a Tailscale IP) so a phone can open the UI. Token still required.
        let extra = tokio::net::TcpListener::bind((host.as_str(), bound)).await?;
        let (app2, c2) = (app.clone(), ctx.clone());
        tokio::spawn(async move {
            let _ = axum::serve(extra, app2).with_graceful_shutdown(async move { c2.stop.notified().await }).await;
        });
    }
    let info = json!({"port": bound, "token": ctx.token, "pid": std::process::id(), "version": env!("DROVER_VERSION"), "host": host});
    write_info(&info)?;
    log::info(&format!("drover daemon {} (rust) listening on {host}:{bound}", env!("DROVER_VERSION")));
    log::info(&format!("start-up: context {context_ms:.1} ms (settings, rules, managers), session {session_ms:.1} ms, \
                        ready {:.1} ms after `server run` began", ms(*SERVE_START.get_or_init(std::time::Instant::now))));

    ctx.remotes.sync_from_file(); // links to machines.toml start now
    let tasks = [
        tokio::spawn(status_monitor::run(ctx.clone())),
        tokio::spawn(save_loop(ctx.clone())),
        tokio::spawn(housekeeping(ctx.clone())),
        tokio::spawn(stop_on_signal(ctx.clone())),
        tokio::spawn(methods::files_git::git_watcher(ctx.clone())),
    ];
    let c = ctx.clone();
    let _ = axum::serve(listener, app).with_graceful_shutdown(async move {
        while !c.stopping.load(std::sync::atomic::Ordering::SeqCst) {
            c.stop.notified().await;
        }
    }).await;
    ctx.save_now();
    ctx.chats.shutdown().await; // kill the claude processes of chats
    ctx.remotes.stop_all();
    for t in tasks {
        t.abort();
    }
    let panes: Vec<_> = ctx.manager.lock().unwrap().panes.values().cloned().collect();
    for pane in panes {
        pane.kill(true);
    }
    if read_info().and_then(|i| i["pid"].as_u64()) == Some(std::process::id() as u64) {
        let _ = std::fs::remove_file(paths::daemon_info_file());
    }
    if ctx.restart.load(std::sync::atomic::Ordering::SeqCst) {
        restart_after_update(&ctx);
    }
    Ok(())
}

/// Start the (just replaced) daemon, keeping port and token so open windows reconnect: the bundle's `dr` at the
/// path it had before the swap (the running exe has moved). Old Windows runtime: through the new drover.exe.
fn restart_after_update(ctx: &Ctx) {
    let exe = self_update_apply::restart_command();
    #[cfg(windows)]
    let exe = exe.or_else(self_update_windows::restart_command);
    let Some(exe) = exe.or_else(|| std::env::current_exe().ok()) else { return };
    let mut cmd = std::process::Command::new(exe);
    #[cfg(windows)]
    self_update_windows::strip_onefile_env(&mut cmd);
    cmd.args(["server", "run", "--background"]).env("DROVER_KEEP_TOKEN", &ctx.token).env("DROVER_KEEP_PORT", ctx.port().to_string())
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000 | 0x0000_0200);
    if let Err(e) = cmd.spawn() {
        log::warn(&format!("restart after update failed: {e}"));
    }
}

/// Debounced save: one write ~1 s after the last structure change.
async fn save_loop(ctx: Arc<Ctx>) {
    let save = ctx.manager.lock().unwrap().save.clone();
    loop {
        save.notified().await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        ctx.save_now();
    }
}

async fn housekeeping(ctx: Arc<Ctx>) {
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        ctx.remotes.sync_from_file();
        ctx.save_now();
    }
}

async fn stop_on_signal(ctx: Arc<Ctx>) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else { return };
        tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
    }
    #[cfg(windows)]
    {
        use tokio::signal::windows::{ctrl_break, ctrl_c, ctrl_close, ctrl_shutdown};
        let (Ok(mut c), Ok(mut b), Ok(mut close), Ok(mut shutdown)) = (ctrl_c(), ctrl_break(), ctrl_close(), ctrl_shutdown()) else { return };
        tokio::select! { _ = c.recv() => {}, _ = b.recv() => {}, _ = close.recv() => {}, _ = shutdown.recv() => {} }
    }
    ctx.request_stop();
}
