//! Throw away one hunk of the working tree: save a recovery point, then reverse-apply just that hunk.


use super::Repo;
use super::diff::FileDiff;
use crate::git;

/// A commit holding the current working tree (`git stash create`), so a rejected hunk can be fetched back with
/// `git checkout <sha> -- <file>`. None when git cannot make one (nothing tracked changed, no identity...).
pub async fn recovery_point(repo: &Repo) -> Option<String> {
    let out = git::git(&repo.root, &["stash", "create"], false).await.ok()?;
    let sha = out.trim();
    (sha.len() >= 7 && sha.bytes().all(|b| b.is_ascii_hexdigit())).then(|| sha.to_string())
}

/// The patch that holds only hunk `index`: the file header plus that hunk.
pub fn single_hunk_patch(diff: &FileDiff, index: usize) -> Result<String, String> {
    let hunk = diff.hunks.get(index).ok_or_else(|| format!("no hunk {index} in this file (it has {})", diff.hunks.len()))?;
    Ok(format!("{}{}", diff.head, hunk.raw))
}

/// Revert hunk `index` in the working tree, leaving every other hunk (and the index) alone.
pub async fn revert_hunk(repo: &Repo, diff: &FileDiff, index: usize) -> Result<(), String> {
    if diff.binary {
        return Err("binary files cannot be rejected hunk by hunk".into());
    }
    let patch = single_hunk_patch(diff, index)?;
    let dir = repo.common.join("drover");
    tokio::fs::create_dir_all(&dir).await.map_err(|e| format!("cannot write a temporary patch: {e}"))?;
    let file = dir.join(format!("reject-{}-{}.patch", std::process::id(), (drover_core::status_tracker::now_secs() * 1e6) as u64));
    tokio::fs::write(&file, patch).await.map_err(|e| format!("cannot write a temporary patch: {e}"))?;
    let result = git::git(&repo.root, &["apply", "-R", "--whitespace=nowarn", &file.to_string_lossy()], true).await;
    let _ = tokio::fs::remove_file(&file).await;
    result.map(|_| ()).map_err(|e| format!("could not reject the hunk (did the file change since it was shown?): {e}"))
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::diff::file_diff;
    use std::path::PathBuf;
    use std::process::Command;

    fn git_sync(dir: &PathBuf, args: &[&str]) {
        let ok = Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]).args(args).current_dir(dir).status().unwrap().success();
        assert!(ok, "git {args:?}");
    }

    fn temp_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("drover-reject-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git_sync(&dir, &["init", "-q"]);
        dir
    }

    #[tokio::test]
    async fn rejects_one_hunk_and_keeps_the_other() {
        let dir = temp_repo("one");
        let original: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
        std::fs::write(dir.join("f.txt"), original.join("\n") + "\n").unwrap();
        git_sync(&dir, &["add", "."]);
        git_sync(&dir, &["commit", "-qm", "init"]);
        let mut changed = original.clone();
        changed[1] = "CHANGED 2".into();
        changed[27] = "CHANGED 28".into();
        std::fs::write(dir.join("f.txt"), changed.join("\n") + "\n").unwrap();

        let repo = Repo::open(&dir.to_string_lossy()).await.unwrap();
        let mb = repo.merge_base("HEAD").await.unwrap();
        let d = file_diff(&repo, &mb, "f.txt").await.unwrap();
        assert_eq!(d.hunks.len(), 2);
        let undo = recovery_point(&repo).await;
        assert!(undo.is_some(), "stash create gives a recovery commit");
        revert_hunk(&repo, &d, 0).await.unwrap();

        let now = std::fs::read_to_string(dir.join("f.txt")).unwrap().replace("\r\n", "\n");
        assert!(now.contains("line 2\n") && !now.contains("CHANGED 2\n"), "{now}");
        assert!(now.contains("CHANGED 28"));
        assert_eq!(file_diff(&repo, &mb, "f.txt").await.unwrap().hunks.len(), 1);
        assert!(revert_hunk(&repo, &d, 5).await.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn rejecting_an_untracked_file_removes_it() {
        let dir = temp_repo("new");
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        git_sync(&dir, &["add", "."]);
        git_sync(&dir, &["commit", "-qm", "init"]);
        std::fs::write(dir.join("n.txt"), "x\ny\n").unwrap();
        let repo = Repo::open(&dir.to_string_lossy()).await.unwrap();
        let mb = repo.merge_base("HEAD").await.unwrap();
        let d = file_diff(&repo, &mb, "n.txt").await.unwrap();
        assert_eq!(d.hunks.len(), 1);
        revert_hunk(&repo, &d, 0).await.unwrap();
        assert!(!dir.join("n.txt").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
