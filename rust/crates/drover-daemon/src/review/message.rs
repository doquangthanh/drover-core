//! What `review.send` types into the agent, and which agent gets it.

use crate::checks::folder_key;

pub struct Item<'a> {
    pub file: &'a str,
    pub side: &'a str,
    pub line: u32,
    pub anchor: &'a str,
    pub text: &'a str,
}

/// One message listing every comment. `multiline` is false when the agent's terminal has no bracketed paste (a
/// newline would submit), so the same content goes out as a single line.
pub fn format_message(items: &[Item], multiline: bool) -> String {
    let mut out = String::from("Review comments on your changes. Please address each one:\n");
    for (i, it) in items.iter().enumerate() {
        let removed = if it.side == "old" { " (removed line)" } else { "" };
        out.push_str(&format!("\n{}. {}:{}{}\n   > {}\n   {}\n", i + 1, it.file, it.line, removed, it.anchor.trim(), it.text.trim().replace('\n', "\n   ")));
    }
    if multiline { out.trim_end().to_string() } else { out.split('\n').map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" | ") }
}

pub struct PaneInfo {
    pub id: String,
    pub name: String,
    pub cwd: String,
    pub kind: String,
    pub state: String,
}

fn in_folder(cwd: &str, key: &str) -> bool {
    let c = folder_key(cwd);
    c == key || c.strip_prefix(key).is_some_and(|rest| rest.starts_with('/'))
}

/// The agent to type into: `requested` (id or name) when given, else the only live agent working in the folder.
pub fn select_pane(panes: &[PaneInfo], key: &str, requested: &str) -> Result<String, String> {
    let live = |p: &&PaneInfo| p.kind != "shell" && p.state != "exited" && p.state != "stopped";
    let agents: Vec<&PaneInfo> = panes.iter().filter(live).collect();
    let requested = requested.trim().strip_prefix("local:").unwrap_or_else(|| requested.trim());
    if !requested.is_empty() {
        let named: Vec<&&PaneInfo> = agents.iter().filter(|p| p.id == requested || p.name == requested).collect();
        let near: Vec<&&&PaneInfo> = named.iter().filter(|p| in_folder(&p.cwd, key)).collect();
        return match (near.len(), named.len()) {
            (1, _) => Ok(near[0].id.clone()),
            (0, 1) => Ok(named[0].id.clone()),
            (0, 0) => Err(format!("no agent '{requested}' is running")),
            _ => Err(format!("'{requested}' matches several agents; use the pane id")),
        };
    }
    let here: Vec<&&PaneInfo> = agents.iter().filter(|p| in_folder(&p.cwd, key)).collect();
    match here.len() {
        0 => Err("no agent is working in this folder; start one or name the pane".into()),
        1 => Ok(here[0].id.clone()),
        _ => Err(format!("several agents work in this folder ({}); say which one", here.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str, name: &str, cwd: &str, kind: &str) -> PaneInfo {
        PaneInfo { id: id.into(), name: name.into(), cwd: cwd.into(), kind: kind.into(), state: "idle".into() }
    }

    #[test]
    fn formats_comments() {
        let items = [Item { file: "src/a.rs", side: "new", line: 12, anchor: "  let x = 1;", text: "rename x" },
            Item { file: "b.rs", side: "old", line: 3, anchor: "old()", text: "keep it\nplease" }];
        let m = format_message(&items, true);
        assert!(m.starts_with("Review comments on your changes."));
        assert!(m.contains("1. src/a.rs:12\n   > let x = 1;\n   rename x"));
        assert!(m.contains("2. b.rs:3 (removed line)\n   > old()\n   keep it\n   please"));
        let one = format_message(&items, false);
        assert!(!one.contains('\n') && one.contains("1. src/a.rs:12 | > let x = 1; | rename x"));
    }

    #[test]
    fn picks_the_only_agent_in_the_folder() {
        let panes = [pane("%1", "auth", "C:\\Work\\app", "claude"), pane("%2", "sh", "C:\\Work\\app", "shell"), pane("%3", "other", "C:\\Work\\zzz", "codex")];
        assert_eq!(select_pane(&panes, "c:/work/app", "").unwrap(), "%1");
        assert_eq!(select_pane(&panes, "c:/work/app", "other").unwrap(), "%3");
        assert_eq!(select_pane(&panes, "c:/work/app", "%1").unwrap(), "%1");
        assert!(select_pane(&panes, "c:/work/none", "").unwrap_err().contains("no agent"));
        assert!(select_pane(&panes, "c:/work/app", "nobody").is_err());
    }

    #[test]
    fn several_agents_ask_which_one() {
        let panes = [pane("%1", "a", "/w/app", "claude"), pane("%2", "b", "/w/app/sub", "codex")];
        let e = select_pane(&panes, "/w/app", "").unwrap_err();
        assert!(e.contains("say which one") && e.contains("a, b"));
        assert_eq!(select_pane(&panes, "/w/app", "b").unwrap(), "%2");
        let mut gone = pane("%4", "c", "/w/app", "claude");
        gone.state = "exited".into();
        assert_eq!(select_pane(&[panes.into_iter().next().unwrap(), gone], "/w/app", "").unwrap(), "%1");
    }
}
