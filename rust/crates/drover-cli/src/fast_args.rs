//! Strict argv parser for the commands the fast path answers itself. It mirrors `cli_main.build_parser` but only
//! for exact, valid forms: `--help`, `--`, abbreviated or unknown flags, bad numbers or choices, positionals that
//! start with `-` all return None, and the Python CLI (argparse) handles them, including its usage errors.

use std::collections::HashMap;

pub const STATES: [&str; 5] = ["done", "idle", "blocked", "working", "exited"];

/// `--timeout`: argparse leaves the default as the int 600 (type=float only converts given strings).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Timeout {
    Default,
    Given(f64),
}

impl Timeout {
    pub fn seconds(self) -> f64 {
        match self {
            Timeout::Default => 600.0,
            Timeout::Given(s) => s,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum FastCommand {
    Ls { json: bool, names: bool },
    Send { target: String, text: String, enter: bool },
    Read { target: String, lines: i64, json: bool },
    Wait { target: String, children: Option<String>, states: Vec<String>, timeout: Timeout },
    Prompt { target: String, text: String, wait: bool, states: Vec<String>, timeout: Timeout },
    Keys { target: String, keys: Vec<String> },
    WaitOutput { target: String, pattern: String, regex: bool, lines: i64, timeout: Timeout, json: bool },
    Rename { target: String, name: String },
    Show { path: String, target: String },
    Kill { target: String, tree: bool },
    Run(crate::fast_run_command::RunArgs),
    HookClaude,
}

struct Parsed {
    flags: HashMap<&'static str, Vec<String>>,
    positionals: Vec<String>,
}

impl Parsed {
    fn on(&self, flag: &str) -> bool {
        self.flags.contains_key(flag)
    }
    fn last(&self, flag: &str) -> Option<&str> {
        self.flags.get(flag).and_then(|v| v.last()).map(String::as_str)
    }
    fn all(&self, flag: &str) -> Vec<String> {
        self.flags.get(flag).cloned().unwrap_or_default()
    }
}

/// Split `words` into known flags and positionals. `bools` take no value, `values` take exactly one.
fn parse(words: &[String], bools: &[&'static str], values: &[&'static str]) -> Option<Parsed> {
    let mut parsed = Parsed { flags: HashMap::new(), positionals: Vec::new() };
    let mut it = words.iter();
    while let Some(word) = it.next() {
        if word == "-" || !word.starts_with('-') {
            parsed.positionals.push(word.clone());
            continue;
        }
        let (name, inline) = match word.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (word.as_str(), None),
        };
        if let Some(flag) = bools.iter().find(|f| **f == name) {
            if inline.is_some() {
                return None; // argparse: "ignored explicit argument"
            }
            parsed.flags.entry(flag).or_default().push(String::new());
        } else if let Some(flag) = values.iter().find(|f| **f == name) {
            let value = match inline {
                Some(v) => v,
                None => {
                    let next = it.next()?;
                    if next.starts_with('-') && next != "-" {
                        return None; // argparse: "expected one argument"
                    }
                    next.clone()
                }
            };
            parsed.flags.entry(flag).or_default().push(value);
        } else {
            return None; // --help, --, abbreviations, unknown flags, negative numbers
        }
    }
    Some(parsed)
}

/// ASCII integer like `12` or `-3` (argparse's int() accepts more; those go to Python).
fn int(s: &str) -> Option<i64> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Plain non-negative decimal like `30` or `2.5`, at most ~31 years (larger values go to Python).
fn float(s: &str) -> Option<f64> {
    let (whole, frac) = s.split_once('.').unwrap_or((s, "0"));
    let ok = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    if !ok(whole) || !ok(frac) {
        return None;
    }
    s.parse().ok().filter(|f: &f64| *f <= 1e9)
}

fn timeout(p: &Parsed) -> Option<Timeout> {
    match p.last("--timeout") {
        None => Some(Timeout::Default),
        Some(s) => float(s).map(Timeout::Given),
    }
}

fn states(p: &Parsed) -> Option<Vec<String>> {
    let states = p.all("--state");
    states.iter().all(|s| STATES.contains(&s.as_str())).then_some(states)
}

fn exactly<const N: usize>(p: &Parsed) -> Option<[String; N]> {
    p.positionals.clone().try_into().ok()
}

/// The fast-path command for this argv (without the program name), or None to let Python run it.
pub fn parse_fast(args: &[String]) -> Option<FastCommand> {
    let (cmd, rest) = args.split_first()?;
    Some(match cmd.as_str() {
        "ls" => {
            let p = parse(rest, &["--json", "--names"], &[])?;
            exactly::<0>(&p)?;
            FastCommand::Ls { json: p.on("--json"), names: p.on("--names") }
        }
        "send" => {
            let p = parse(rest, &["--no-enter"], &[])?;
            let [target, text] = exactly(&p)?;
            FastCommand::Send { target, text, enter: !p.on("--no-enter") }
        }
        "read" => {
            let p = parse(rest, &["--json"], &["--lines"])?;
            let [target] = exactly(&p)?;
            let lines = p.last("--lines").map(int).unwrap_or(Some(0))?;
            FastCommand::Read { target, lines, json: p.on("--json") }
        }
        "wait" => {
            let p = parse(rest, &[], &["--children", "--state", "--timeout"])?;
            let target = match p.positionals.as_slice() {
                [] => String::new(),
                [t] => t.clone(),
                _ => return None,
            };
            let children = p.last("--children").map(str::to_string);
            if children.as_deref().is_some_and(|c| c != "all" && c != "any") {
                return None;
            }
            FastCommand::Wait { target, children, states: states(&p)?, timeout: timeout(&p)? }
        }
        "prompt" => {
            let p = parse(rest, &["--wait"], &["--state", "--timeout"])?;
            let [target, text] = exactly(&p)?;
            FastCommand::Prompt { target, text, wait: p.on("--wait"), states: states(&p)?, timeout: timeout(&p)? }
        }
        "keys" => {
            let p = parse(rest, &[], &[])?;
            let (target, keys) = p.positionals.split_first()?;
            if keys.is_empty() {
                return None;
            }
            FastCommand::Keys { target: target.clone(), keys: keys.to_vec() }
        }
        "wait-output" => {
            let p = parse(rest, &["--regex", "--json"], &["--lines", "--timeout"])?;
            let [target, pattern] = exactly(&p)?;
            let lines = p.last("--lines").map(int).unwrap_or(Some(80))?;
            FastCommand::WaitOutput { target, pattern, regex: p.on("--regex"), lines, timeout: timeout(&p)?, json: p.on("--json") }
        }
        "show" => {
            let p = parse(rest, &[], &[])?;
            match p.positionals.as_slice() {
                [path] => FastCommand::Show { path: path.clone(), target: "self".into() },
                [path, target] => FastCommand::Show { path: path.clone(), target: target.clone() },
                _ => return None,
            }
        }
        "rename" => {
            let p = parse(rest, &[], &[])?;
            let [target, name] = exactly(&p)?;
            FastCommand::Rename { target, name }
        }
        "kill" => {
            let p = parse(rest, &["--tree"], &[])?;
            let [target] = exactly(&p)?;
            FastCommand::Kill { target, tree: p.on("--tree") }
        }
        "run" => FastCommand::Run(crate::fast_run_command::parse(rest)?),
        "hook" if rest == ["claude"] => FastCommand::HookClaude,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast(line: &[&str]) -> Option<FastCommand> {
        parse_fast(&line.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn parses_the_common_forms() {
        assert_eq!(fast(&["ls", "--json"]), Some(FastCommand::Ls { json: true, names: false }));
        assert_eq!(fast(&["send", "--no-enter", "%3", "hi there"]),
                   Some(FastCommand::Send { target: "%3".into(), text: "hi there".into(), enter: false }));
        assert_eq!(fast(&["read", "w", "--lines=20"]), Some(FastCommand::Read { target: "w".into(), lines: 20, json: false }));
        assert_eq!(fast(&["wait", "--children", "all", "--timeout", "30"]),
                   Some(FastCommand::Wait { target: "".into(), children: Some("all".into()), states: vec![], timeout: Timeout::Given(30.0) }));
        assert_eq!(fast(&["prompt", "w", "-", "--wait", "--state", "done", "--state", "idle"]),
                   Some(FastCommand::Prompt { target: "w".into(), text: "-".into(), wait: true,
                                              states: vec!["done".into(), "idle".into()], timeout: Timeout::Default }));
        assert_eq!(fast(&["keys", "w", "esc", "ctrl+c"]), Some(FastCommand::Keys { target: "w".into(), keys: vec!["esc".into(), "ctrl+c".into()] }));
        assert_eq!(fast(&["hook", "claude"]), Some(FastCommand::HookClaude));
        assert_eq!(fast(&["show", "a.png"]), Some(FastCommand::Show { path: "a.png".into(), target: "self".into() }));
        assert_eq!(fast(&["show", "a.png", "w"]), Some(FastCommand::Show { path: "a.png".into(), target: "w".into() }));
    }

    #[test]
    fn anything_unsure_goes_to_python() {
        for line in [
            &["ls", "--help"][..], &["ls", "-h"], &["ls", "--js"], &["ls", "extra"], &["send", "w"], &["send", "w", "-x"],
            &["send", "w", "a", "b"], &["send", "--", "w", "a"], &["read", "w", "--lines", "abc"], &["read", "w", "--lines"],
            &["wait", "--children", "some"], &["wait", "w", "--state", "bogus"], &["wait", "w", "--timeout", "1e3"], &["wait", "w", "--timeout", "99999999999999999999"],
            &["keys", "w"], &["run", "-x"], &["run", "--bg=1"], &["run", "--name"], &["status"], &["tui"], &[], &["ls", "--json=1"], &["hook", "codex"],
        ] {
            assert_eq!(fast(line), None, "{line:?}");
        }
    }
}
