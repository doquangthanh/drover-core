//! Review loop: comments on diff lines, hunks marked reviewed or rejected, and sending open comments to an agent.
//! Contract: the web client's `review-logic.ts`. State lives in the repo's git common dir, so nothing is committed.

pub mod diff;
pub mod message;
pub mod reject;
pub mod store;

use std::path::{Component, Path, PathBuf};

use crate::checks::folder_key;
use crate::git;

/// The working tree a review belongs to.
pub struct Repo {
    pub root: String,
    /// Key of the worktree in the store (same normalisation as the checks' folder key).
    pub key: String,
    pub common: PathBuf,
}

impl Repo {
    pub async fn open(cwd: &str) -> Result<Repo, String> {
        let root = git::git(cwd, &["rev-parse", "--show-toplevel"], true).await.map_err(|_| format!("{cwd} is not in a git repository"))?;
        let root = root.trim().to_string();
        let common = git::git(&root, &["rev-parse", "--path-format=absolute", "--git-common-dir"], true).await?;
        Ok(Repo { key: folder_key(&root), root, common: PathBuf::from(common.trim()) })
    }

    pub fn store(&self) -> store::Store {
        store::Store::new(self.common.join("drover").join("review.json"))
    }

    /// Where git started from when the branch was cut: `merge-base(base, HEAD)`.
    pub async fn merge_base(&self, base: &str) -> Result<String, String> {
        if base.is_empty() || base.starts_with('-') {
            return Err("base must be a branch, tag or commit".into());
        }
        let out = git::git(&self.root, &["merge-base", base, "HEAD"], true).await.map_err(|e| format!("no common ancestor with {base}: {e}"))?;
        Ok(out.trim().to_string())
    }
}

fn split_lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes).split('\n').map(|l| l.trim_end_matches('\r').to_string()).collect()
}

/// The lines of `file` as comments see them: `new` is the working tree, `old` is the merge base. None if unreadable.
pub async fn side_lines(repo: &Repo, side: &str, merge_base: &str, file: &str) -> Option<Vec<String>> {
    if side == "old" {
        git::git(&repo.root, &["show", &format!("{merge_base}:{file}")], true).await.ok().map(|t| split_lines(t.as_bytes()))
    } else {
        tokio::fs::read(Path::new(&repo.root).join(file)).await.ok().map(|b| split_lines(&b))
    }
}

/// Each comment with its effective status (open | sent | resolved | outdated).
pub async fn with_status(repo: &Repo, comments: Vec<store::Comment>) -> Vec<(store::Comment, String)> {
    let mut cache: std::collections::HashMap<(String, String, String), Option<Vec<String>>> = Default::default();
    let mut out = Vec::with_capacity(comments.len());
    for c in comments {
        let mb = if c.side == "old" { c.merge_base.clone() } else { String::new() };
        let key = (c.file.clone(), c.side.clone(), mb);
        if !cache.contains_key(&key) && c.status != "resolved" {
            let lines = side_lines(repo, &c.side, &key.2, &c.file).await;
            cache.insert(key.clone(), lines);
        }
        let current = cache.get(&key).and_then(|l| l.as_ref()).and_then(|l| l.get((c.line as usize).wrapping_sub(1))).cloned();
        let status = store::effective_status(&c, current.as_deref());
        out.push((c, status));
    }
    out
}

/// A file path from a client: relative, no `..`, nothing that leaves the working tree (also through a symlink).
pub fn safe_rel(root: &str, file: &str) -> Result<String, String> {
    // Clients on any OS send these: read `\` as a separator and refuse drive paths here too (on Linux "C:/x" and
    // "a\..\x" would otherwise look like plain names).
    let slashed = file.replace('\\', "/");
    let p = Path::new(&slashed);
    let drive = file.as_bytes().get(1) == Some(&b':');
    let plain = !file.is_empty() && !file.contains('\0') && !file.starts_with('-') && !drive
        && p.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if !plain || p.components().next().is_some_and(|c| c.as_os_str() == ".git") {
        return Err(format!("file must be a path inside the folder: {file}"));
    }
    let full = Path::new(root).join(p);
    if let (Ok(real), Ok(base)) = (full.canonicalize(), Path::new(root).canonicalize()) {
        if !real.starts_with(&base) {
            return Err(format!("{file} points outside the folder"));
        }
    }
    Ok(file.replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_escapes() {
        let root = std::env::temp_dir().to_string_lossy().into_owned();
        for bad in ["", "../x", "a/../../x", "/etc/passwd", "C:\\x", "C:/x", "-rf", ".git/config", "a\0b"] {
            assert!(safe_rel(&root, bad).is_err(), "{bad}");
        }
        assert_eq!(safe_rel(&root, "src/a.rs").unwrap(), "src/a.rs");
        assert_eq!(safe_rel(&root, "./a.rs").unwrap(), "./a.rs");
    }
}
