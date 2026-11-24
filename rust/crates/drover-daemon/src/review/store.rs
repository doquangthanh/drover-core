//! One JSON file per repo (`<git-common-dir>/drover/review.json`): comments and reviewed hunks of each worktree.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

use drover_core::status_tracker::now_secs;

pub const MAX_COMMENT_CHARS: usize = 4000;

/// One daemon, many requests: the read-modify-write of the file must not interleave.
static FILE_LOCK: Mutex<()> = Mutex::new(());
static SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    pub id: String,
    pub file: String,
    /// old | new
    pub side: String,
    pub line: u32,
    /// The text of the line when the comment was added.
    pub anchor: String,
    pub text: String,
    /// Stored: open | sent | resolved. (`outdated` is worked out on read, see `effective_status`.)
    pub status: String,
    pub created_at: f64,
    pub sent_at: Option<f64>,
    /// Merge base the `old` side line numbers refer to.
    pub merge_base: String,
}

impl Comment {
    pub fn new(file: &str, side: &str, line: u32, anchor: &str, text: &str, merge_base: &str) -> Self {
        let id = format!("rc{:x}{:x}", (now_secs() * 1e6) as u64, SEQ.fetch_add(1, Ordering::Relaxed));
        Self { id, file: file.into(), side: side.into(), line, anchor: anchor.into(), text: text.into(), status: "open".into(),
            created_at: now_secs(), sent_at: None, merge_base: merge_base.into() }
    }

    fn from_json(v: &Value) -> Option<Self> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
        Some(Self { id: s("id")?, file: s("file")?, side: s("side")?, line: v.get("line")?.as_u64()? as u32, anchor: s("anchor").unwrap_or_default(),
            text: s("text")?, status: s("status").unwrap_or_else(|| "open".into()), created_at: v.get("created_at").and_then(Value::as_f64).unwrap_or(0.0),
            sent_at: v.get("sent_at").and_then(Value::as_f64), merge_base: s("merge_base").unwrap_or_default() })
    }

    /// Wire form for clients; `status` is the effective one.
    pub fn to_json(&self, status: &str) -> Value {
        json!({"id": self.id, "file": self.file, "side": self.side, "line": self.line, "anchor": self.anchor, "text": self.text,
            "status": status, "created_at": self.created_at, "sent_at": self.sent_at})
    }

    fn to_stored(&self) -> Value {
        let mut v = self.to_json(&self.status);
        v["merge_base"] = json!(self.merge_base);
        v
    }
}

/// `outdated` when the line no longer holds what was commented on; resolved comments stay resolved.
pub fn effective_status(c: &Comment, current_line: Option<&str>) -> String {
    if c.status == "resolved" {
        return "resolved".into();
    }
    match current_line {
        Some(t) if t.trim_end() == c.anchor.trim_end() => c.status.clone(),
        _ => "outdated".into(),
    }
}

/// Everything stored for one worktree.
#[derive(Default)]
pub struct Tree {
    pub comments: Vec<Comment>,
    /// `<file>:<hunk hash>` of hunks marked reviewed.
    pub reviewed: BTreeSet<String>,
}

pub fn reviewed_key(file: &str, hash: &str) -> String {
    format!("{file}:{hash}")
}

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn load(&self) -> Value {
        std::fs::read_to_string(&self.path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({"worktrees": {}}))
    }

    pub fn read(&self, key: &str) -> Tree {
        let _g = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        parse_tree(&self.load(), key)
    }

    /// Change one worktree's data and save the file; the closure's error leaves the file untouched.
    pub fn update<R>(&self, key: &str, f: impl FnOnce(&mut Tree) -> Result<R, String>) -> Result<R, String> {
        let _g = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut doc = self.load();
        let mut tree = parse_tree(&doc, key);
        let out = f(&mut tree)?;
        if !doc.get("worktrees").is_some_and(Value::is_object) {
            doc = json!({"worktrees": {}});
        }
        doc["worktrees"][key] = json!({"comments": tree.comments.iter().map(Comment::to_stored).collect::<Vec<_>>(), "reviewed": tree.reviewed});
        let dir = self.path.parent().ok_or("bad review path")?;
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot save review: {e}"))?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, doc.to_string()).and_then(|_| std::fs::rename(&tmp, &self.path)).map_err(|e| format!("cannot save review: {e}"))?;
        Ok(out)
    }
}

fn parse_tree(doc: &Value, key: &str) -> Tree {
    let Some(w) = doc.get("worktrees").and_then(|w| w.get(key)) else { return Tree::default() };
    Tree {
        comments: w.get("comments").and_then(Value::as_array).map(|a| a.iter().filter_map(Comment::from_json).collect()).unwrap_or_default(),
        reviewed: w.get("reviewed").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment() -> Comment {
        Comment::new("a.rs", "new", 3, "let x = 1;", "why?", "abc")
    }

    #[test]
    fn outdated_when_the_line_changed() {
        let c = comment();
        assert_eq!(effective_status(&c, Some("let x = 1;  ")), "open");
        assert_eq!(effective_status(&c, Some("let x = 2;")), "outdated");
        assert_eq!(effective_status(&c, None), "outdated");
        let mut sent = c.clone();
        sent.status = "sent".into();
        assert_eq!(effective_status(&sent, Some("let x = 1;")), "sent");
        assert_eq!(effective_status(&sent, Some("other")), "outdated");
        let mut done = c;
        done.status = "resolved".into();
        assert_eq!(effective_status(&done, Some("other")), "resolved");
    }

    #[test]
    fn saves_and_reloads_per_worktree() {
        let dir = std::env::temp_dir().join(format!("drover-review-store-{}", std::process::id()));
        let store = Store::new(dir.join("drover").join("review.json"));
        let c = comment();
        let id = c.id.clone();
        store.update("/w1", |t| { t.comments.push(c); t.reviewed.insert(reviewed_key("a.rs", "h1")); Ok(()) }).unwrap();
        assert!(store.update("/w1", |_| Err::<(), _>("nope".into())).is_err());
        let t = store.read("/w1");
        assert_eq!((t.comments.len(), t.comments[0].id.as_str(), t.comments[0].merge_base.as_str()), (1, id.as_str(), "abc"));
        assert!(t.reviewed.contains("a.rs:h1"));
        assert!(store.read("/w2").comments.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ids_are_unique() {
        assert_ne!(comment().id, comment().id);
    }
}
