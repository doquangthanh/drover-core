//! RPC methods `review.diff`, `review.hunk.*` and `review.summary` (the comment methods are in `review_comments`).

use std::sync::Arc;

use serde_json::{Value, json};

use super::review_comments;
use crate::ctx::Ctx;
use crate::method;
use crate::review::diff::{self, FileDiff};
use crate::review::store::reviewed_key;
use crate::review::{Repo, reject, safe_rel, with_status};
use crate::rpc::{E_GENERAL, MethodSpec, RpcError, RpcResult, bad, need_str, opt_i64, truthy};

const MAX_SUMMARY_FILES: usize = 300;

pub fn methods() -> Vec<MethodSpec> {
    let mut all = vec![
        method!("review.diff", "Working-tree diff of a file against the merge base, as hunks with lines, hash and reviewed flag", "cwd, base, file", review_diff),
        method!("review.hunk.accept", "Mark a hunk reviewed (accepted=false clears it)", "cwd, base, file, index, accepted?", hunk_accept),
        method!("review.hunk.reject", "Revert one hunk in the working tree; returns undo_ref, a commit to get it back from", "cwd, base, file, index", hunk_reject),
        method!("review.summary", "Per changed file: hunks, reviewed hunks, open comments; plus totals", "cwd, base", review_summary),
    ];
    all.extend(review_comments::methods());
    all
}

pub fn err(e: String) -> RpcError {
    RpcError::new(E_GENERAL, e)
}

pub async fn repo(p: &Value) -> Result<Repo, RpcError> {
    Repo::open(&need_str(p, "cwd")?).await.map_err(err)
}

/// (repo, merge base, file, its diff) from the usual `cwd, base, file` params.
async fn load(p: &Value) -> Result<(Repo, String, String, FileDiff), RpcError> {
    let repo = repo(p).await?;
    let file = safe_rel(&repo.root, &need_str(p, "file")?).map_err(bad)?;
    let mb = repo.merge_base(&need_str(p, "base")?).await.map_err(err)?;
    let d = diff::file_diff(&repo, &mb, &file).await.map_err(err)?;
    Ok((repo, mb, file, d))
}

fn index_param(p: &Value, d: &FileDiff) -> Result<usize, RpcError> {
    let i = opt_i64(p, "index", -1)?;
    usize::try_from(i).ok().filter(|i| *i < d.hunks.len()).ok_or_else(|| bad(format!("no hunk {i} in this file (it has {})", d.hunks.len())))
}

async fn review_diff(_ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let (repo, mb, file, d) = load(&p).await?;
    let seen = repo.store().read(&repo.key).reviewed;
    let hunks: Vec<Value> = d.hunks.iter().map(|h| h.to_json(seen.contains(&reviewed_key(&file, &h.hash)))).collect();
    Ok(json!({"file": file, "merge_base": mb, "binary": d.binary, "hunks": hunks}))
}

async fn hunk_accept(_ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let (repo, _, file, d) = load(&p).await?;
    let index = index_param(&p, &d)?;
    let accepted = p.get("accepted").is_none() || truthy(&p, "accepted");
    let key = reviewed_key(&file, &d.hunks[index].hash);
    repo.store().update(&repo.key, |t| {
        if accepted { t.reviewed.insert(key) } else { t.reviewed.remove(&key) };
        Ok(())
    }).map_err(err)?;
    Ok(json!({"index": index, "accepted": accepted}))
}

async fn hunk_reject(_ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let (repo, _, file, d) = load(&p).await?;
    let index = index_param(&p, &d)?;
    let undo_ref = reject::recovery_point(&repo).await;
    reject::revert_hunk(&repo, &d, index).await.map_err(err)?;
    let key = reviewed_key(&file, &d.hunks[index].hash);
    repo.store().update(&repo.key, |t| { t.reviewed.remove(&key); Ok(()) }).map_err(err)?;
    Ok(json!({"index": index, "undo_ref": undo_ref}))
}

/// Files that differ from the merge base: tracked changes plus untracked files.
async fn changed_paths(repo: &Repo, mb: &str) -> Result<Vec<String>, String> {
    let (a1, a2) = (["diff", "--name-only", "-z", "--no-renames", mb], ["ls-files", "--others", "--exclude-standard", "-z"]);
    let (tracked, new) = tokio::join!(
        crate::git::git(&repo.root, &a1, true),
        crate::git::git(&repo.root, &a2, true));
    let mut files: Vec<String> = tracked?.split('\0').chain(new?.split('\0')).filter(|f| !f.is_empty()).map(String::from).collect();
    files.sort();
    files.dedup();
    Ok(files)
}

async fn review_summary(_ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let repo = repo(&p).await?;
    let mb = repo.merge_base(&need_str(&p, "base")?).await.map_err(err)?;
    let tree = repo.store().read(&repo.key);
    let open: Vec<String> = with_status(&repo, tree.comments).await.into_iter().filter(|(_, s)| s == "open").map(|(c, _)| c.file).collect();
    let (mut files, mut hunks, mut reviewed) = (vec![], 0, 0);
    for path in changed_paths(&repo, &mb).await.map_err(err)?.into_iter().take(MAX_SUMMARY_FILES) {
        let Ok(rel) = safe_rel(&repo.root, &path) else { continue };
        let d = diff::file_diff(&repo, &mb, &rel).await.map_err(err)?;
        if d.hunks.is_empty() && !d.binary {
            continue;
        }
        let done = d.hunks.iter().filter(|h| tree.reviewed.contains(&reviewed_key(&rel, &h.hash))).count();
        let mut f = json!({"path": rel, "hunks": d.hunks.len(), "reviewed": done, "open": open.iter().filter(|c| **c == rel).count()});
        if d.binary {
            f["binary"] = json!(true);
        }
        (hunks, reviewed) = (hunks + d.hunks.len(), reviewed + done);
        files.push(f);
    }
    Ok(json!({"files": files, "hunks": hunks, "reviewed": reviewed, "open_comments": open.len()}))
}
