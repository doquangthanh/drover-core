//! Desktop notifications (notify-send on Linux, osascript on macOS, a PowerShell toast on Windows) with quiet
//! hours. Port of `desktop_notifier_service.py`; uses the system's own tools so no D-Bus/WinRT code is built in.

use chrono::NaiveTime;
use serde_json::Value;

use crate::settings::Settings;

/// How a kind of agent is called in messages and menus ("claude" -> "Claude Code"); unknown kinds as they are.
pub fn agent_label(kind: &str) -> &str {
    match kind {
        "claude" => "Claude Code", "codex" => "Codex", "gemini" => "Gemini", "opencode" => "OpenCode", "copilot" => "Copilot", "shell" => "Shell",
        other => other,
    }
}

/// Title and body for a state change: what happened, then what the agent was doing.
pub fn build_message(name: &str, kind: &str, state: &str, detail: &str) -> Option<(String, String)> {
    let (title, hint) = match state {
        "blocked" => (format!("{name} needs you"), "{agent} is waiting for your answer or approval."),
        "done" => (format!("{name} finished"), "{agent} finished its work. Review the result when you are ready."),
        "exited" => (format!("{name} stopped"), "The {agent} process has ended."),
        _ => return None,
    };
    let agent = if kind.is_empty() { "The agent" } else { agent_label(kind) };
    let mut body = hint.replace("{agent}", agent);
    let detail: String = detail.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(140).collect();
    if !detail.is_empty() {
        body = format!("{body}\n{detail}");
    }
    Some((title, body))
}

/// `spec` like "23:00-07:00" (may wrap midnight). Empty or unreadable = never quiet.
pub fn in_quiet_hours(spec: &str, now: NaiveTime) -> bool {
    let Some((start, end)) = spec.split_once('-') else { return false };
    let parse = |t: &str| NaiveTime::parse_from_str(t.trim(), "%H:%M").or_else(|_| NaiveTime::parse_from_str(t.trim(), "%H:%M:%S"));
    let (Ok(start), Ok(end)) = (parse(start), parse(end)) else { return false };
    if start <= end { start <= now && now < end } else { now >= start || now < end }
}

/// Notify about an unfocused pane's state change when settings ask for it (`notify.<state>` has "desktop").
pub fn notify_state(settings: &Settings, name: &str, kind: &str, state: &str, detail: &str) {
    let channels: Vec<&str> = settings.get(&format!("notify.{state}")).and_then(Value::as_array).into_iter().flatten()
        .filter_map(Value::as_str).collect();
    if !channels.contains(&"desktop") || in_quiet_hours(&settings.str("notify.quiet_hours", ""), chrono::Local::now().time()) {
        return;
    }
    let Some((title, body)) = build_message(name, kind, state, detail) else { return };
    // blocked is the only state that must not be missed, so it asks the OS for a higher urgency.
    let urgency = match state { "blocked" => "critical", "done" => "normal", _ => "low" };
    send(&title, &body, channels.contains(&"sound"), urgency);
}

/// Fire and forget; a missing notifier never breaks the daemon.
pub fn send(title: &str, body: &str, sound: bool, urgency: &str) {
    let mut cmd = command(title, body, sound, urgency);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    match cmd.spawn() {
        // Reap it off the daemon's threads so no zombie is left behind.
        Ok(mut child) => { std::thread::spawn(move || child.wait()); }
        Err(e) => crate::log::info(&format!("desktop notifications unavailable: {e}")),
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn command(title: &str, body: &str, sound: bool, urgency: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new("notify-send");
    cmd.args(["--app-name=drover", &format!("--urgency={urgency}")]);
    if sound {
        cmd.arg("--hint=string:sound-name:message-new-instant");
    }
    cmd.args(["--", title, body]);
    cmd
}

#[cfg(target_os = "macos")]
fn command(title: &str, body: &str, sound: bool, _urgency: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new("osascript");
    let show = if sound { "display notification (item 2 of argv) with title (item 1 of argv) sound name \"default\"" }
               else { "display notification (item 2 of argv) with title (item 1 of argv)" };
    cmd.args(["-e", "on run argv", "-e", show, "-e", "end run", title, body]);
    cmd
}

#[cfg(windows)]
fn command(title: &str, body: &str, sound: bool, _urgency: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    // Text goes in through environment variables and is XML-escaped in PowerShell: never part of the script.
    const SCRIPT: &str = "$e=[Security.SecurityElement];\
        [Windows.UI.Notifications.ToastNotificationManager,Windows.UI.Notifications,ContentType=WindowsRuntime]>$null;\
        $x=[Windows.Data.Xml.Dom.XmlDocument,Windows.Data.Xml.Dom.XmlDocument,ContentType=WindowsRuntime]::new();\
        $a=if($env:DR_SOUND){''}else{'<audio silent=\"true\"/>'};\
        $x.LoadXml('<toast><visual><binding template=\"ToastGeneric\"><text>'+$e::Escape($env:DR_TITLE)+'</text><text>'+$e::Escape($env:DR_BODY)+'</text></binding></visual>'+$a+'</toast>');\
        [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('drover').Show([Windows.UI.Notifications.ToastNotification]::new($x))";
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT]).env("DR_TITLE", title).env("DR_BODY", body)
        .env("DR_SOUND", if sound { "1" } else { "" }).creation_flags(0x0800_0000);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> NaiveTime {
        NaiveTime::parse_from_str(s, "%H:%M").unwrap()
    }

    #[test]
    fn quiet_hours_wrap_midnight_and_bad_specs_are_never_quiet() {
        assert!(in_quiet_hours("23:00-07:00", t("23:30")) && in_quiet_hours("23:00-07:00", t("06:59")));
        assert!(!in_quiet_hours("23:00-07:00", t("07:00")) && !in_quiet_hours("23:00-07:00", t("12:00")));
        assert!(in_quiet_hours("09:00-17:00", t("09:00")) && !in_quiet_hours("09:00-17:00", t("17:00")));
        assert!(!in_quiet_hours("", t("01:00")) && !in_quiet_hours("bad", t("01:00")) && !in_quiet_hours("1-x", t("01:00")));
    }

    #[test]
    fn message_names_the_agent_and_keeps_detail_short() {
        let (title, body) = build_message("auth", "claude", "blocked", "  Edit   file\n x ").unwrap();
        assert_eq!(title, "auth needs you");
        assert_eq!(body, "Claude Code is waiting for your answer or approval.\nEdit file x");
        assert!(build_message("a", "x", "working", "").is_none());
        assert_eq!(build_message("a", "x", "done", &"y".repeat(300)).unwrap().1.lines().nth(1).unwrap().len(), 140);
    }
}
