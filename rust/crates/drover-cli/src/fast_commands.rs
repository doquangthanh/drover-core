//! The fast-path commands: same RPCs, output and exit codes as `cli_commands_agents.py` /
//! `cli_commands_agent_input.py`. When the daemon can't be reached they return `Delegate`, and the Python CLI
//! runs the whole command again (it can start the daemon; Python itself retries the same way).

use std::time::Duration;

use serde_json::{Value, json};

use crate::daemon_rpc::{self, RpcError};
use crate::fast_args::{FastCommand, Timeout};
use crate::fast_ls_command::{ls, print_names, state_label};
use crate::fast_run_command::RunResult;
use crate::python_json::{dumps, py_str};
use crate::python_output::{ljust, print_err, print_out, read_stdin, truthy};

pub enum Outcome {
    Exit(i32),
    /// Run the same argv in the Python CLI.
    Delegate,
    /// Run this argv in the Python CLI instead (`dr run` without `--bg` -> `attach <id>`).
    DelegateWith(Vec<String>),
}

pub enum Failure {
    Unavailable,
    Lost(String),
    Rpc { code: i64, message: String },
    Unexpected,
}

pub const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(30);

pub fn run(cmd: FastCommand) -> Outcome {
    let result = match cmd {
        FastCommand::Ls { names: true, .. } => return Outcome::Exit(print_names()),
        FastCommand::Ls { json, .. } => ls(json),
        FastCommand::Send { target, text, enter } => {
            rpc("pane.send", json!({"pane": target, "text": text, "enter": enter}), DEFAULT_RPC_TIMEOUT)
                .and_then(|r| Ok(format!("sent to {}", field(&r, "pane")?)))
        }
        FastCommand::Read { target, lines, json } => {
            let lines = if lines == 0 { Value::Null } else { json!(lines) }; // `args.lines or None`
            rpc("pane.read", json!({"pane": target, "lines": lines}), DEFAULT_RPC_TIMEOUT)
                .and_then(|r| if json { Ok(dumps(&r)) } else { field(&r, "text") })
        }
        FastCommand::Wait { children: Some(mode), timeout, .. } => wait_children(&mode, timeout),
        FastCommand::Wait { target, states, timeout, .. } => {
            if target.is_empty() {
                return Outcome::Exit(print_err("error: give an agent to wait for, or --children all|any", 5));
            }
            let states = if states.is_empty() { vec!["done".into(), "idle".into(), "blocked".into()] } else { states };
            rpc("pane.wait", json!({"pane": target, "states": states, "timeout": timeout_value(timeout)}), rpc_timeout(timeout))
                .and_then(|r| field(&r, "state"))
        }
        FastCommand::Prompt { target, text, wait, states, timeout } => {
            let text = if text == "-" { read_stdin() } else { text };
            let mut params = json!({"pane": target, "text": text, "wait": wait, "timeout": timeout_value(timeout)});
            if !states.is_empty() {
                params["states"] = json!(states);
            }
            rpc("pane.prompt", params, rpc_timeout(timeout)).and_then(|r| field(&r, "state"))
        }
        FastCommand::Keys { target, keys } => {
            rpc("pane.keys", json!({"pane": target, "keys": keys}), DEFAULT_RPC_TIMEOUT).and_then(|r| {
                let pressed = r.get("keys").and_then(Value::as_array).ok_or(Failure::Unexpected)?;
                let pressed: Vec<String> = pressed.iter().map(py_str).collect();
                Ok(format!("pressed {} in {}", pressed.join(" "), field(&r, "pane")?))
            })
        }
        FastCommand::WaitOutput { target, pattern, regex, lines, timeout, json } => {
            let params = json!({"pane": target, "pattern": pattern, "regex": regex, "lines": lines,
                                "timeout": timeout_value(timeout)});
            rpc("pane.wait_output", params, rpc_timeout(timeout))
                .and_then(|r| if json { Ok(dumps(&r)) } else { field(&r, "matched_line") })
        }
        FastCommand::Show { path, target } => show(&path, &target),
        FastCommand::Rename { target, name } => {
            rpc("pane.rename", json!({"pane": target, "name": name}), DEFAULT_RPC_TIMEOUT)
                .and_then(|r| Ok(format!("{} is now '{}'", field(&r, "pane")?, field(&r, "name")?)))
        }
        FastCommand::Kill { target, tree } => match rpc("pane.close", json!({"pane": target, "subtree": tree}), DEFAULT_RPC_TIMEOUT) {
            Ok(_) => return Outcome::Exit(0), // prints nothing
            Err(e) => Err(e),
        },
        FastCommand::Run(args) => match crate::fast_run_command::run(args) {
            Ok(RunResult::Printed(id)) => Ok(id),
            Ok(RunResult::Delegate) => return Outcome::Delegate,
            Ok(RunResult::Attach(id)) => return Outcome::DelegateWith(vec!["attach".into(), id]),
            Err(e) => Err(e),
        },
        FastCommand::HookClaude => return Outcome::Exit(crate::hook_claude::run()),
    };
    match result {
        Ok(text) => Outcome::Exit(if print_out(&text) { 0 } else { 1 }), // broken pipe: Python fails too
        Err(Failure::Unavailable) => Outcome::Delegate,
        Err(Failure::Lost(why)) => Outcome::Exit(print_err(&format!("error: {why}"), 10)),
        Err(Failure::Rpc { code, message }) => {
            Outcome::Exit(print_err(&format!("error: {message}"), if code > 0 { code as i32 } else { 1 }))
        }
        Err(Failure::Unexpected) => Outcome::Exit(print_err("error: unexpected reply from drover daemon", 1)),
    }
}

pub fn rpc(method: &str, params: Value, timeout: Duration) -> Result<Value, Failure> {
    let Value::Object(params) = params else { return Err(Failure::Unexpected) };
    daemon_rpc::call(method, params, timeout).map_err(|e| match e {
        RpcError::Unavailable => Failure::Unavailable,
        RpcError::Lost(why) => Failure::Lost(why),
        RpcError::Failed { code, message } => Failure::Rpc { code, message },
    })
}

/// `str(v[key])`; a missing key is an unexpected reply (Python would raise KeyError).
pub fn field(v: &Value, key: &str) -> Result<String, Failure> {
    v.get(key).map(py_str).ok_or(Failure::Unexpected)
}

/// argparse keeps the default 600 as an int; a given --timeout is a float (`30` -> `30.0` on the wire).
fn timeout_value(t: Timeout) -> Value {
    match t {
        Timeout::Default => json!(600),
        Timeout::Given(s) => json!(s),
    }
}

fn rpc_timeout(t: Timeout) -> Duration {
    Duration::from_secs_f64(t.seconds() + 10.0)
}

fn wait_children(mode: &str, timeout: Timeout) -> Result<String, Failure> {
    let r = rpc("pane.wait_children", json!({"mode": mode, "timeout": timeout_value(timeout)}), rpc_timeout(timeout))?;
    let children = r.get("children").and_then(Value::as_array).ok_or(Failure::Unexpected)?;
    let mut lines = Vec::new();
    match r.get("pane").filter(|p| truthy(p)) {
        Some(pane) if mode == "any" => {
            let child = children.iter().find(|c| c.get("pane") == Some(pane));
            let name = child.and_then(|c| c.get("name")).unwrap_or(pane);
            lines.push(format!("{} {}", py_str(name), field(&r, "state")?));
        }
        _ => lines.push(field(&r, "reason")?),
    }
    for c in children {
        let label = state_label(&field(c, "state")?);
        lines.push(format!("  {} {} {label}", ljust(&field(c, "name")?, 16), ljust(&field(c, "pane")?, 6)));
    }
    Ok(lines.join("\n"))
}

/// `dr show <png> [pane]`: send the image to the daemon, which draws it in the pane (works where escape sequences
/// from a program cannot get through, like the Windows console).
fn show(path: &str, target: &str) -> Result<String, Failure> {
    use base64::Engine;
    const MAX: u64 = 6 * 1024 * 1024;
    let fail = |m: String| Failure::Lost(m);
    let meta = std::fs::metadata(path).map_err(|e| fail(format!("cannot read {path}: {e}")))?;
    if meta.len() > MAX {
        return Err(fail(format!("{path} is larger than 6 MB")));
    }
    let bytes = std::fs::read(path).map_err(|e| fail(format!("cannot read {path}: {e}")))?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(fail(format!("{path} is not a PNG (only PNG is supported)")));
    }
    let png = base64::engine::general_purpose::STANDARD.encode(bytes);
    rpc("pane.show", json!({"pane": target, "png": png}), DEFAULT_RPC_TIMEOUT).map(|r| format!("shown in {}", r["pane"].as_str().unwrap_or(target)))
}
