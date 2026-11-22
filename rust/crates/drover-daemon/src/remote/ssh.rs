//! SSH through the system's OpenSSH client, so `~/.ssh/config`, ProxyJump, ssh-agent and known_hosts work exactly
//! as in a terminal. Port of `remote_bootstrap.py` (ensure / install) plus the tunnel `remote_link.py` opened with
//! asyncssh: here `ssh -W 127.0.0.1:<port>` carries the websocket over the ssh process's stdin/stdout.

use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::{Child, Command};

use super::registry::Machine;

/// Run `script` in a login shell on the remote, quoted once and correctly.
fn sh(script: &str) -> String {
    format!("sh -lc {}", shlex::try_quote(script).map(|q| q.into_owned()).unwrap_or_default())
}

/// Prefer the uv-tool install, and skip any `dr` that is not drover's (another tool of that name would shadow it).
fn ensure_cmd() -> String {
    sh(r#"for c in "$HOME/.local/bin/dr" dr; do command -v "$c" >/dev/null 2>&1 && "$c" server --help >/dev/null 2>&1 && exec "$c" server ensure --json; done; exit 127"#)
}

fn ssh_options(m: &Machine, port_flag: &str) -> Vec<String> {
    let mut a: Vec<String> = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=15", "-o", "ServerAliveInterval=20"].map(String::from).into();
    // Port 22 is "not set": the command line would override a `Port` the user wrote in ~/.ssh/config.
    if m.port != 22 {
        a.extend([port_flag.to_string(), m.port.to_string()]);
    }
    if !m.key.is_empty() {
        a.extend(["-i".into(), m.key.clone()]);
    }
    if !m.known_hosts {
        let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
        a.extend(["-o".into(), "StrictHostKeyChecking=no".into(), "-o".into(), format!("UserKnownHostsFile={null}")]);
    }
    a
}

fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    cmd
}

fn check_target(m: &Machine) -> Result<(), String> {
    if m.target.is_empty() || m.target.starts_with('-') || m.target.chars().any(char::is_whitespace) {
        return Err(format!("bad target for {}: {}", m.alias, m.target));
    }
    Ok(())
}

pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run a remote command; exit code 255 means ssh itself failed (connect, auth, host key).
pub async fn run(m: &Machine, script: &str, timeout: Duration) -> Result<Output, String> {
    check_target(m)?;
    let mut cmd = command("ssh");
    cmd.args(ssh_options(m, "-p")).args(["--", &m.target, script]).stdin(Stdio::null());
    let out = tokio::time::timeout(timeout, cmd.output()).await
        .map_err(|_| format!("ssh to {} timed out", m.target))?
        .map_err(|e| format!("could not run ssh: {e}"))?;
    Ok(Output { code: out.status.code().unwrap_or(-1), stdout: String::from_utf8_lossy(&out.stdout).into(),
                stderr: String::from_utf8_lossy(&out.stderr).trim().into() })
}

fn ssh_failure(o: &Output) -> String {
    let text = if o.stderr.is_empty() { "ssh failed" } else { o.stderr.as_str() };
    text.chars().take(300).collect()
}

/// Run `dr server ensure --json` remotely. Returns {"port", "token", "version"}.
pub async fn ensure_remote_daemon(m: &Machine) -> Result<Value, String> {
    let o = run(m, &ensure_cmd(), Duration::from_secs(60)).await?;
    match o.code {
        0 => {}
        127 => return Err("dr is not installed on this machine — use 'Install dr there'".into()),
        255 => return Err(ssh_failure(&o)),
        _ => return Err((if o.stderr.is_empty() { &o.stdout } else { &o.stderr }).trim().chars().take(300).collect()),
    }
    let info = o.stdout.trim().lines().last().and_then(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|i| i["port"].as_u64().is_some() && i["token"].is_string())
        .ok_or_else(|| format!("unexpected reply from remote dr: {}", o.stdout.chars().take(200).collect::<String>()))?;
    Ok(json!({"port": info["port"], "token": info["token"], "version": info["version"].as_str().unwrap_or_default()}))
}

/// `ssh -W 127.0.0.1:<port>`: stdin/stdout of the returned process are a TCP stream to the remote daemon.
pub fn open_tunnel(m: &Machine, port: u64) -> Result<Child, String> {
    check_target(m)?;
    let mut cmd = command("ssh");
    cmd.args(ssh_options(m, "-p")).args(["-W", &format!("127.0.0.1:{port}"), "--", &m.target])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    cmd.spawn().map_err(|e| format!("could not run ssh: {e}"))
}

/// Install uv (if missing) and the drover wheel on a Linux/macOS remote. `progress` gets each step.
pub async fn install_dr(m: &Machine, wheel: Option<&std::path::Path>, progress: impl Fn(&str)) -> Result<(), String> {
    let long = Duration::from_secs(600);
    progress("Checking for uv");
    if run(m, &sh(r#"command -v uv || test -x "$HOME/.local/bin/uv""#), Duration::from_secs(60)).await?.code != 0 {
        progress("Installing uv");
        let o = run(m, &sh("curl -LsSf https://astral.sh/uv/install.sh | sh"), long).await?;
        if o.code != 0 {
            return Err(format!("could not install uv: {}", o.stderr.chars().take(200).collect::<String>()));
        }
    }
    let wheel = wheel.ok_or("no local drover wheel to upload; run `uv build --wheel` in the drover repo first")?;
    progress("Uploading drover");
    // Private, unpredictable upload dir (mktemp -d is 0700): nobody else can swap the wheel.
    let tmp = run(m, &sh("mktemp -d"), Duration::from_secs(60)).await?;
    let dir = tmp.stdout.trim().to_string();
    if tmp.code != 0 || !dir.starts_with('/') {
        return Err("could not create a temp folder on the remote machine".into());
    }
    let name = wheel.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let remote_wheel = format!("{dir}/{name}");
    let mut scp = command("scp");
    scp.args(ssh_options(m, "-P")).arg("--").arg(wheel).arg(format!("{}:{remote_wheel}", m.target)).stdin(Stdio::null());
    let up = tokio::time::timeout(long, scp.output()).await.map_err(|_| "upload timed out".to_string())?.map_err(|e| e.to_string())?;
    if !up.status.success() {
        return Err(format!("upload failed: {}", String::from_utf8_lossy(&up.stderr).trim()));
    }
    progress("Installing drover");
    let q = |s: &str| shlex::try_quote(s).map(|q| q.into_owned()).unwrap_or_default();
    let script = format!(r#"UV=$(command -v uv || echo "$HOME/.local/bin/uv"); "$UV" tool install --force {}; rc=$?; rm -rf {}; exit $rc"#,
                         q(&remote_wheel), q(&dir));
    let o = run(m, &sh(&script), long).await?;
    if o.code != 0 {
        return Err(format!("install failed: {}", (if o.stderr.is_empty() { &o.stdout } else { &o.stderr }).chars().take(300).collect::<String>()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_scripts_are_quoted_once_and_options_follow_the_machine() {
        assert_eq!(sh("mktemp -d"), "sh -lc 'mktemp -d'");
        assert!(ensure_cmd().starts_with("sh -lc 'for c in \"$HOME/.local/bin/dr\" dr;"));
        let mut m = Machine::new("lab", "bob@lab");
        m.port = 2222;
        m.key = "/k".into();
        m.known_hosts = false;
        let a = ssh_options(&m, "-p").join(" ");
        assert!(a.contains("BatchMode=yes") && a.contains("-p 2222") && a.contains("-i /k") && a.contains("StrictHostKeyChecking=no"));
        // The default port is left to OpenSSH (and its config); only another port is passed.
        let plain = ssh_options(&Machine::new("lab", "bob@lab"), "-p").join(" ");
        assert!(!plain.contains("-p "));
        assert!(check_target(&Machine::new("x", "-oProxyCommand=evil")).is_err());
    }
}
