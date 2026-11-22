//! How panes are launched: default shell, argv resolution on Windows, PATH with `dr` first, and the per-pane
//! Claude Code hooks that report sub-agents. Ports of `pty_backend_factory.py`, `self_command.pane_path` and
//! `claude_subagent_hooks.py`.

use std::path::{Path, PathBuf};

use drover_core::launch_command::split_command;

pub fn default_shell(configured: &str) -> Vec<String> {
    if !configured.trim().is_empty() {
        return split_command(configured).unwrap_or_else(|| vec![configured.to_string()]);
    }
    if cfg!(windows) {
        for name in ["pwsh.exe", "powershell.exe"] {
            if let Some(path) = which(name) {
                return vec![path.to_string_lossy().into_owned(), "-NoLogo".into()];
            }
        }
        return vec![std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())];
    }
    vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into()), "-l".into()]
}

/// Find a program on PATH (with PATHEXT on Windows).
pub fn which(name: &str) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let names = candidate_names(name, cfg!(windows), &pathext);
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| names.iter().map(|n| dir.join(n)).find(|p| p.is_file()))
}

/// File names `which` looks for. On Windows a bare name only gets PATHEXT extensions: npm puts an extensionless sh
/// shim `claude` next to `claude.cmd`, and CreateProcess cannot start it ("not a valid Win32 application").
fn candidate_names(name: &str, windows: bool, pathext: &str) -> Vec<String> {
    if !windows {
        return vec![name.to_string()];
    }
    let with_exts = pathext.split(';').filter(|e| !e.is_empty()).map(|e| format!("{name}{}", e.to_lowercase()));
    if Path::new(name).extension().is_some() {
        std::iter::once(name.to_string()).chain(with_exts).collect()
    } else {
        with_exts.collect()
    }
}

/// On Windows resolve `claude` to `claude.cmd`/`.exe` so ConPTY can start it; elsewhere unchanged.
pub fn resolve_argv(argv: Vec<String>) -> Vec<String> {
    if !cfg!(windows) || argv.is_empty() {
        return argv;
    }
    let Some(found) = which(&argv[0]) else { return argv };
    let found = found.to_string_lossy().into_owned();
    let lower = found.to_lowercase();
    if lower.ends_with(".cmd") || lower.ends_with(".bat") {
        let comspec = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
        return [comspec, "/c".into(), found].into_iter().chain(argv.into_iter().skip(1)).collect();
    }
    std::iter::once(found).chain(argv.into_iter().skip(1)).collect()
}

/// Folder of the running `dr` binary, so agents in panes can call `dr`.
pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf))
}

/// PATH for a pane: our folder first (`dr`), then on Windows `<state>/shims` (`hd.cmd`, the old name, written by
/// drover.exe) when it exists. Folders already on PATH are not added twice.
pub fn pane_path(current: &str) -> String {
    let sep = if cfg!(windows) { ";" } else { ":" };
    let mut front: Vec<PathBuf> = exe_dir().into_iter().collect();
    let shims = crate::paths::state_dir().join("shims");
    if cfg!(windows) && shims.is_dir() {
        front.push(shims);
    }
    let parts: Vec<String> = front.iter().map(|d| d.to_string_lossy().into_owned())
        .filter(|d| !current.split(sep).any(|p| p == d)).collect();
    if parts.is_empty() {
        return current.to_string();
    }
    let joined = parts.join(sep);
    if current.is_empty() { joined } else { format!("{joined}{sep}{current}") }
}

/// Variables that belong to one running Claude Code session. A daemon started from inside an agent (e.g. `dr`
/// autostarting it) would otherwise hand them to every pane, and claude there believes it is a child session
/// (transcripts off, hooks talking to the other session's socket). User config such as CLAUDE_CONFIG_DIR stays.
pub(crate) fn is_session_var(key: &str) -> bool {
    key == "CLAUDECODE" || key == "CLAUDE_PID" || key == "CLAUDE_EFFORT" || key.starts_with("CLAUDE_CODE_")
}

/// Environment of a new pane: ours (minus another agent session's markers) plus terminal type, pane id and PATH.
pub fn pane_env(pane_id: &str) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars()
        // DROVER_AGENT (wrapper hint) of a daemon started from an agent pane would mark every new pane as that agent.
        .filter(|(k, _)| !matches!(k.as_str(), "TERM" | "COLORTERM" | "DR_PANE_ID" | "DROVER" | "DROVER_AGENT" | "PATH")
            && !is_session_var(k))
        .collect();
    env.push(("TERM".into(), "xterm-256color".into()));
    env.push(("COLORTERM".into(), "truecolor".into()));
    env.push(("DR_PANE_ID".into(), pane_id.into()));
    env.push(("DROVER".into(), "1".into()));
    env.push(("PATH".into(), pane_path(&std::env::var("PATH").unwrap_or_default())));
    env
}

/// Shell command line for the Claude hook (forward slashes + double quotes work in Git Bash, cmd and sh).
fn hook_command() -> String {
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| "dr".into());
    format!("\"{exe}\" hook claude")
}

/// Write (only when it changed) and return the settings file holding drover's sub-agent hooks.
fn hooks_settings_file() -> std::io::Result<PathBuf> {
    let hook = serde_json::json!([{"hooks": [{"type": "command", "command": hook_command(), "timeout": 10}]}]);
    let body = serde_json::to_string_pretty(&serde_json::json!({"hooks": {"SubagentStart": hook, "SubagentStop": hook}}))
        .map_err(std::io::Error::other)?;
    let path = crate::paths::state_dir().join("claude-subagent-hooks.json");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(body.as_str()) {
        std::fs::write(&path, body)?;
    }
    Ok(path)
}

/// argv of a `claude` pane with `--settings <file>` right after the program; anything else unchanged.
/// `argv` run the way a terminal runs it: inside the user's login shell (`zsh -l -i -c 'exec claude …'`), so the
/// shell's startup files set PATH, aliases and variables (oh-my-zsh, nvm…) that programs like Claude Code expect, as
/// in Herdr or gnome-terminal. `exec` keeps one process: the pane still ends when the program does.
pub fn in_login_shell(shell: &str, argv: Vec<String>) -> Vec<String> {
    if shell.is_empty() || argv.is_empty() {
        return argv;
    }
    let line: Vec<String> = argv.iter().map(|a| shlex::try_quote(a).map(|q| q.into_owned()).unwrap_or_else(|_| a.clone())).collect();
    vec![shell.to_string(), "-l".into(), "-i".into(), "-c".into(), format!("exec {}", line.join(" "))]
}

pub fn with_subagent_hooks(argv: Vec<String>) -> Vec<String> {
    let is_claude = argv
        .first()
        .and_then(|a| Path::new(a).file_stem().and_then(|s| s.to_str()).map(str::to_lowercase))
        .is_some_and(|s| s == "claude");
    if !is_claude || argv.iter().any(|a| a == "--settings") {
        return argv;
    }
    match hooks_settings_file() {
        Ok(file) => {
            let mut out = vec![argv[0].clone(), "--settings".into(), file.to_string_lossy().into_owned()];
            out.extend(argv.into_iter().skip(1));
            out
        }
        Err(_) => argv, // state dir not writable: run claude without the Flow lanes rather than not at all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_runs_through_the_login_shell_with_its_words_quoted() {
        let argv = vec!["claude".into(), "--settings".into(), "/tmp/a b/hooks.json".into(), "fix it's bug".into()];
        assert_eq!(in_login_shell("/usr/bin/zsh", argv),
                   ["/usr/bin/zsh", "-l", "-i", "-c", r#"exec claude --settings '/tmp/a b/hooks.json' "fix it's bug""#]);
        assert_eq!(in_login_shell("", vec!["claude".into()]), ["claude"]);
    }

    #[cfg(unix)]
    #[test]
    fn the_shell_hands_each_word_through_unchanged() {
        let argv = in_login_shell("/bin/sh", vec!["printf".into(), "%s|".into(), "a b".into(), "it's".into(), "$HOME".into()]);
        let out = std::process::Command::new(&argv[0]).args(&argv[1..]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "a b|it's|$HOME|");
    }

    #[test]
    fn windows_bare_names_only_match_runnable_extensions() {
        let pathext = ".COM;.EXE;.BAT;.CMD";
        // npm's extensionless `claude` sh shim must not win over claude.cmd.
        assert_eq!(candidate_names("claude", true, pathext), ["claude.com", "claude.exe", "claude.bat", "claude.cmd"]);
        assert_eq!(candidate_names("pwsh.exe", true, pathext)[0], "pwsh.exe");
        assert_eq!(candidate_names("claude", false, pathext), ["claude"]);
    }

    #[test]
    fn claude_session_markers_are_dropped_but_user_config_stays() {
        for k in ["CLAUDECODE", "CLAUDE_CODE_CHILD_SESSION", "CLAUDE_CODE_MESSAGING_SOCKET", "CLAUDE_PID"] {
            assert!(is_session_var(k), "{k}");
        }
        for k in ["CLAUDE_CONFIG_DIR", "ANTHROPIC_API_KEY", "HOME"] {
            assert!(!is_session_var(k), "{k}");
        }
    }
}
