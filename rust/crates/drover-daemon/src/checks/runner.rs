//! Runs the steps of one check run, one after another, keeping the output (capped) and the status in the store.

use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;
use tokio::sync::watch;

use drover_core::status_tracker::now_secs;

use super::CheckStore;
use super::config::Step;
use crate::event_hub::EventHub;

/// Output kept per step: the end of it (that is where a failing test says why).
pub const OUTPUT_CAP: usize = 64 * 1024;
const TRUNCATED: &str = "[earlier output cut]\n";

/// Output of a step as text, at most `OUTPUT_CAP` bytes from the end (cut on a character boundary).
pub fn cap_output(raw: &[u8]) -> String {
    if raw.len() <= OUTPUT_CAP {
        return String::from_utf8_lossy(raw).into_owned();
    }
    let tail = String::from_utf8_lossy(&raw[raw.len() - OUTPUT_CAP..]).into_owned();
    format!("{TRUNCATED}{}", tail.trim_start_matches('\u{FFFD}'))
}

fn build_command(step: &Step, cwd: &str) -> tokio::process::Command {
    let mut cmd = if step.shell {
        let line = &step.command[0];
        #[cfg(windows)]
        let mut c = tokio::process::Command::new("cmd");
        #[cfg(windows)]
        c.arg("/C").raw_arg(line);
        #[cfg(not(windows))]
        let mut c = tokio::process::Command::new("sh");
        #[cfg(not(windows))]
        c.arg("-c").arg(line);
        c
    } else {
        let program = crate::launch::which(&step.command[0]).map(|p| p.into_os_string()).unwrap_or_else(|| step.command[0].clone().into());
        let mut c = tokio::process::Command::new(program);
        c.args(&step.command[1..]);
        c
    };
    cmd.current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    #[cfg(unix)]
    cmd.process_group(0);
    cmd
}

/// Kill the program and everything it started (a shell step's children would otherwise keep running).
async fn kill_tree(pid: Option<u32>) {
    let Some(pid) = pid else { return };
    #[cfg(unix)]
    {
        // The step runs in a process group of its own (process_group(0)): kill that group directly, and only when
        // it really is its own. `kill -KILL -<pid>` through the kill program took the daemon's group with it on
        // Linux (procps reads "-<pid>" without "--" its own way).
        let pid = pid as libc::pid_t;
        // SAFETY: plain syscalls on a pid we started; getpgid guards against signalling any other group.
        unsafe {
            if pid > 1 && libc::getpgid(pid) == pid && libc::getpgid(0) != pid {
                libc::killpg(pid, libc::SIGKILL);
            }
        }
    }
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("taskkill");
        cmd.args(["/T", "/F", "/PID", &pid.to_string()]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        cmd.creation_flags(0x0800_0000);
        let _ = cmd.status().await;
    }
}

fn pump<R: tokio::io::AsyncRead + Unpin + Send + 'static>(mut r: R, buf: Arc<Mutex<Vec<u8>>>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut chunk = [0u8; 8192];
        while let Ok(n) = r.read(&mut chunk).await {
            if n == 0 {
                break;
            }
            let mut b = buf.lock().unwrap();
            b.extend_from_slice(&chunk[..n]);
            if b.len() > OUTPUT_CAP * 2 {
                let cut = b.len() - OUTPUT_CAP;
                b.drain(..cut);
            }
        }
    })
}

/// (status, exit code, output) of one step.
async fn run_step(step: &Step, cwd: &str, cancel: &mut watch::Receiver<bool>) -> (String, Option<i32>, String) {
    let mut child = match build_command(step, cwd).spawn() {
        Ok(c) => c,
        Err(e) => return ("error".into(), None, format!("cannot start {}: {e}", step.command[0])),
    };
    let pid = child.id();
    let buf = Arc::new(Mutex::new(Vec::new()));
    let readers: Vec<_> = [child.stdout.take().map(|o| pump(o, buf.clone())), child.stderr.take().map(|e| pump(e, buf.clone()))].into_iter().flatten().collect();
    let outcome = tokio::select! {
        r = child.wait() => match r {
            Ok(s) => (if s.success() { "passed" } else { "failed" }, s.code()),
            Err(_) => ("error", None),
        },
        _ = tokio::time::sleep(Duration::from_secs(step.timeout_s)) => ("timeout", None),
        _ = cancel.wait_for(|c| *c) => ("cancelled", None),
    };
    if outcome.0 == "timeout" || outcome.0 == "cancelled" {
        kill_tree(pid).await;
        let _ = child.kill().await;
    }
    for r in readers {
        let _ = tokio::time::timeout(Duration::from_secs(2), r).await; // a stray grandchild must not hold the run
    }
    let output = cap_output(&buf.lock().unwrap());
    (outcome.0.into(), outcome.1, output)
}

/// Run every step in `cwd`; the run (already registered under `key`) moves queued -> running -> its final status.
pub async fn execute(checks: Arc<CheckStore>, hub: Arc<EventHub>, key: String, cwd: String, steps: Vec<Step>, mut cancel: watch::Receiver<bool>) {
    let set_run = |status: &str, finished: bool| {
        let status = status.to_string();
        checks.update(&hub, &key, move |r| {
            r.status = status;
            if finished {
                r.finished_at = Some(now_secs());
            }
        });
    };
    set_run("running", false);
    let mut cancelled = false;
    for (i, step) in steps.iter().enumerate() {
        if cancelled || *cancel.borrow() {
            cancelled = true;
            checks.update(&hub, &key, |r| r.results[i].status = "cancelled".into());
            continue;
        }
        checks.update(&hub, &key, |r| r.results[i].status = "running".into());
        let started = Instant::now();
        let (status, exit, output) = run_step(step, &cwd, &mut cancel).await;
        cancelled = status == "cancelled";
        let ms = started.elapsed().as_millis() as u64;
        checks.update(&hub, &key, |r| {
            let res = &mut r.results[i];
            (res.status, res.exit, res.ms, res.output) = (status, exit, ms, output);
        });
    }
    let failed = checks.full(&key).is_some_and(|v| v["failed"].as_u64().unwrap_or(0) > 0);
    set_run(if cancelled { "cancelled" } else if failed { "failed" } else { "passed" }, true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_capped_at_the_end() {
        let big = vec![b'a'; OUTPUT_CAP * 3];
        let s = cap_output(&big);
        assert!(s.starts_with(TRUNCATED) && s.len() <= OUTPUT_CAP + TRUNCATED.len());
        assert_eq!(cap_output(b"ok"), "ok");
    }

    #[tokio::test]
    async fn runs_a_step_and_reports_exit_and_timeout() {
        let (_tx, mut rx) = watch::channel(false);
        let ok = Step { name: "a".into(), command: vec!["echo hi".into()], shell: true, timeout_s: 20 };
        let (status, exit, out) = run_step(&ok, ".", &mut rx).await;
        assert_eq!((status.as_str(), exit), ("passed", Some(0)));
        assert!(out.contains("hi"));
        let bad = Step { name: "b".into(), command: vec!["exit 3".into()], shell: true, timeout_s: 20 };
        assert_eq!(run_step(&bad, ".", &mut rx).await.0, "failed");
        let missing = Step { name: "c".into(), command: vec!["definitely-not-a-program-xyz".into()], shell: false, timeout_s: 20 };
        assert_eq!(run_step(&missing, ".", &mut rx).await.0, "error");
    }

    #[tokio::test]
    async fn cancel_stops_a_slow_step() {
        let (tx, mut rx) = watch::channel(false);
        let cmd = if cfg!(windows) { "ping -n 30 127.0.0.1" } else { "sleep 30" };
        let slow = Step { name: "s".into(), command: vec![cmd.into()], shell: true, timeout_s: 60 };
        let t = Instant::now();
        let h = tokio::spawn(async move { run_step(&slow, ".", &mut rx).await.0 });
        tokio::time::sleep(Duration::from_millis(300)).await;
        tx.send(true).unwrap();
        assert_eq!(h.await.unwrap(), "cancelled");
        assert!(t.elapsed() < Duration::from_secs(15));
    }

    fn setup(steps: &[Step]) -> (Arc<CheckStore>, Arc<EventHub>, watch::Receiver<bool>, Arc<Mutex<Vec<String>>>) {
        let (store, hub) = (Arc::new(CheckStore::default()), Arc::new(EventHub::default()));
        let seen = Arc::new(Mutex::new(vec![]));
        let sink = seen.clone();
        hub.subscribe(move |name, data| if name == "check" { sink.lock().unwrap().push(data["status"].as_str().unwrap_or("").to_string()) });
        let (tx, rx) = watch::channel(false);
        let results = steps.iter().map(|s| super::super::StepResult::pending(&s.name, &s.command)).collect();
        let run = super::super::Run { cwd: "k".into(), pane: None, status: "queued".into(), source: "repo".into(), started_at: 1.0, finished_at: None, results, cancel: tx };
        store.start("k", run).unwrap();
        (store, hub, rx, seen)
    }

    #[tokio::test]
    async fn a_run_goes_through_its_statuses_and_keeps_going_after_a_failure() {
        let steps = vec![
            Step { name: "bad".into(), command: vec!["exit 1".into()], shell: true, timeout_s: 20 },
            Step { name: "good".into(), command: vec!["echo fine".into()], shell: true, timeout_s: 20 },
        ];
        let (store, hub, rx, seen) = setup(&steps);
        execute(store.clone(), hub, "k".into(), ".".into(), steps, rx).await;
        let run = store.full("k").unwrap();
        assert_eq!(run["status"], "failed");
        assert_eq!((run["passed"].as_u64(), run["failed"].as_u64()), (Some(1), Some(1)));
        assert!(run["results"][1]["output"].as_str().unwrap().contains("fine"));
        assert!(run["finished_at"].as_f64().is_some());
        let seen = seen.lock().unwrap();
        assert_eq!((seen.first().map(String::as_str), seen.last().map(String::as_str)), (Some("running"), Some("failed")));
    }

    #[tokio::test]
    async fn cancelling_a_run_marks_the_rest_cancelled() {
        let slow = if cfg!(windows) { "ping -n 30 127.0.0.1" } else { "sleep 30" };
        let steps = vec![
            Step { name: "slow".into(), command: vec![slow.into()], shell: true, timeout_s: 60 },
            Step { name: "after".into(), command: vec!["echo x".into()], shell: true, timeout_s: 20 },
        ];
        let (store, hub, rx, _) = setup(&steps);
        let h = tokio::spawn(execute(store.clone(), hub, "k".into(), ".".into(), steps, rx));
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(store.cancel("k"));
        h.await.unwrap();
        let run = store.full("k").unwrap();
        assert_eq!(run["status"], "cancelled");
        assert_eq!((run["results"][0]["status"].as_str(), run["results"][1]["status"].as_str()), (Some("cancelled"), Some("cancelled")));
        assert!(!store.cancel("k"), "nothing left to cancel");
    }
}
