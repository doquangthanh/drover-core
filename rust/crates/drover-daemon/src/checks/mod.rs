//! Post-run checks: the commands a folder wants run after an agent finishes (`.drover/checks.toml`), their latest
//! result per folder, and the folder key clients match on. The web client's `checks-logic.ts` is the contract.

pub mod config;
pub mod runner;

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{Value, json};
use tokio::sync::watch;

use crate::event_hub::EventHub;

/// A folder as clients key it: forward slashes, no trailing slash, no `//?/` prefix; Windows drive paths lowercase.
pub fn folder_key(path: &str) -> String {
    let p = path.replace('\\', "/");
    let p = p.strip_prefix("//?/").unwrap_or(&p).trim_end_matches('/').to_string();
    let drive = p.len() >= 2 && p.as_bytes()[0].is_ascii_alphabetic() && p.as_bytes()[1] == b':';
    if drive { p.to_lowercase() } else { p }
}

#[derive(Clone)]
pub struct StepResult {
    pub name: String,
    pub command: Vec<String>,
    /// pending | running | passed | failed | timeout | error | cancelled
    pub status: String,
    pub exit: Option<i32>,
    pub ms: u64,
    pub output: String,
}

impl StepResult {
    pub fn pending(name: &str, command: &[String]) -> Self {
        Self { name: name.into(), command: command.to_vec(), status: "pending".into(), exit: None, ms: 0, output: String::new() }
    }

    fn to_json(&self) -> Value {
        json!({"name": self.name, "command": self.command, "status": self.status, "exit": self.exit, "ms": self.ms, "output": self.output})
    }
}

pub struct Run {
    pub cwd: String,
    pub pane: Option<String>,
    /// queued | running | passed | failed | cancelled
    pub status: String,
    pub source: String,
    pub started_at: f64,
    pub finished_at: Option<f64>,
    pub results: Vec<StepResult>,
    pub cancel: watch::Sender<bool>,
}

impl Run {
    pub fn busy(&self) -> bool {
        self.status == "queued" || self.status == "running"
    }

    pub fn summary(&self) -> Value {
        let passed = self.results.iter().filter(|r| r.status == "passed").count();
        let failed = self.results.iter().filter(|r| matches!(r.status.as_str(), "failed" | "timeout" | "error")).count();
        json!({
            "cwd": self.cwd, "pane": self.pane, "status": self.status, "source": self.source,
            "started_at": self.started_at, "finished_at": self.finished_at,
            "total": self.results.len(), "passed": passed, "failed": failed,
            "steps": self.results.iter().map(|r| json!({"name": r.name, "status": r.status})).collect::<Vec<_>>(),
        })
    }

    /// `check.list` shape: the summary plus every step's full result.
    pub fn full(&self) -> Value {
        let mut v = self.summary();
        v["results"] = json!(self.results.iter().map(StepResult::to_json).collect::<Vec<_>>());
        v
    }
}

/// What `check.list` returns for a folder that never ran checks (the client always reads `results`).
pub fn empty_run(key: &str) -> Value {
    json!({"cwd": key, "pane": null, "status": "none", "source": "none", "started_at": 0, "finished_at": null,
        "total": 0, "passed": 0, "failed": 0, "steps": [], "results": []})
}

/// Latest run per folder key, in memory only.
#[derive(Default)]
pub struct CheckStore {
    runs: Mutex<HashMap<String, Run>>,
}

impl CheckStore {
    /// Register a new run unless one is still going in this folder.
    pub fn start(&self, key: &str, run: Run) -> Result<(), String> {
        let mut runs = self.runs.lock().unwrap();
        if runs.get(key).is_some_and(Run::busy) {
            return Err("checks are already running in this folder".into());
        }
        runs.insert(key.to_string(), run);
        Ok(())
    }

    /// Change the run and broadcast its new summary as a `check` event.
    pub fn update(&self, hub: &EventHub, key: &str, f: impl FnOnce(&mut Run)) {
        let summary = {
            let mut runs = self.runs.lock().unwrap();
            let Some(run) = runs.get_mut(key) else { return };
            f(run);
            run.summary()
        };
        hub.emit("check", summary);
    }

    pub fn full(&self, key: &str) -> Option<Value> {
        self.runs.lock().unwrap().get(key).map(Run::full)
    }

    /// Stop the running run; false when nothing was running.
    pub fn cancel(&self, key: &str) -> bool {
        self.runs.lock().unwrap().get(key).filter(|r| r.busy()).is_some_and(|r| r.cancel.send(true).is_ok())
    }

    /// `checks` map of the state snapshot.
    pub fn snapshot(&self) -> Value {
        Value::Object(self.runs.lock().unwrap().iter().map(|(k, r)| (k.clone(), r.summary())).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_key_matches_the_client() {
        assert_eq!(folder_key(r"C:\Users\Me\Proj\"), "c:/users/me/proj");
        assert_eq!(folder_key(r"\\?\D:\Work"), "d:/work");
        assert_eq!(folder_key("/home/Me/proj//"), "/home/Me/proj");
    }

    #[test]
    fn summary_counts_and_full_has_results() {
        let (tx, _rx) = watch::channel(false);
        let mut r = StepResult::pending("a", &["x".into()]);
        r.status = "passed".into();
        let mut f = StepResult::pending("b", &["y".into()]);
        f.status = "timeout".into();
        let run = Run { cwd: "/p".into(), pane: None, status: "failed".into(), source: "repo".into(), started_at: 1.0,
            finished_at: Some(2.0), results: vec![r, f, StepResult::pending("c", &[])], cancel: tx };
        let s = run.summary();
        assert_eq!((s["total"].as_i64(), s["passed"].as_i64(), s["failed"].as_i64()), (Some(3), Some(1), Some(1)));
        assert_eq!(run.full()["results"].as_array().unwrap().len(), 3);
        assert_eq!(empty_run("/p")["results"].as_array().unwrap().len(), 0);
    }
}
