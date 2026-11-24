//! RPC methods `review.list`, `review.comment.*` and `review.send`.

use std::sync::Arc;

use serde_json::{Value, json};

use super::agent_input::submit;
use super::panes::mark_busy;
use super::review::{err, repo};
use crate::ctx::Ctx;
use crate::method;
use crate::review::message::{Item, PaneInfo, format_message, select_pane};
use crate::review::store::{Comment, MAX_COMMENT_CHARS};
use crate::review::{Repo, safe_rel, side_lines, with_status};
use crate::rpc::{E_BLOCKED, E_GENERAL, MethodSpec, RpcError, RpcResult, bad, need_str, not_found, opt_i64, opt_str, truthy};
use drover_core::status_tracker::now_secs;

pub fn methods() -> Vec<MethodSpec> {
    vec![
        method!("review.list", "Review comments of a worktree (optionally of one file) with their effective status", "cwd, file?", comment_list),
        method!("review.comment.add", "Comment on a line of a file's diff (side old|new)", "cwd, base, file, side, line, text", comment_add),
        method!("review.comment.edit", "Change a comment's text (a sent comment becomes open again)", "cwd, id, text", comment_edit),
        method!("review.comment.resolve", "Resolve a comment (resolved=false reopens it)", "cwd, id, resolved?", comment_resolve),
        method!("review.comment.delete", "Delete a comment", "cwd, id", comment_delete),
        method!("review.send", "Type every open comment into the agent working in the folder (pane when several)", "cwd, pane?", review_send),
    ]
}

fn comment_text(p: &Value) -> Result<String, RpcError> {
    let text = need_str(p, "text")?.trim().to_string();
    if text.is_empty() || text.chars().count() > MAX_COMMENT_CHARS {
        return Err(bad(format!("text must be 1-{MAX_COMMENT_CHARS} characters")));
    }
    Ok(text)
}

async fn comment_list(_ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let repo = repo(&p).await?;
    let file = opt_str(&p, "file");
    let mut all = repo.store().read(&repo.key).comments;
    if !file.is_empty() {
        all.retain(|c| c.file == file);
    }
    let comments: Vec<Value> = with_status(&repo, all).await.iter().map(|(c, s)| c.to_json(s)).collect();
    Ok(json!({"comments": comments}))
}

async fn comment_add(_ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let repo = repo(&p).await?;
    let file = safe_rel(&repo.root, &need_str(&p, "file")?).map_err(bad)?;
    let side = need_str(&p, "side")?;
    if side != "old" && side != "new" {
        return Err(bad("side must be old or new"));
    }
    let line = u32::try_from(opt_i64(&p, "line", 0)?).ok().filter(|l| *l > 0).ok_or_else(|| bad("line must be 1 or more"))?;
    let text = comment_text(&p)?;
    let mb = repo.merge_base(&need_str(&p, "base")?).await.map_err(err)?;
    let anchor = side_lines(&repo, &side, &mb, &file).await.and_then(|l| l.get(line as usize - 1).cloned())
        .ok_or_else(|| bad(format!("{file} has no {side} line {line}")))?;
    let c = Comment::new(&file, &side, line, &anchor, &text, &mb);
    let out = c.to_json("open");
    repo.store().update(&repo.key, |t| { t.comments.push(c); Ok(()) }).map_err(err)?;
    Ok(out)
}

/// Change one comment by id; the closure returns the wire status to report.
fn change(repo: &Repo, id: &str, f: impl FnOnce(&mut Comment)) -> RpcResult {
    repo.store().update(&repo.key, |t| {
        let c = t.comments.iter_mut().find(|c| c.id == id).ok_or_else(|| format!("not found: comment {id}"))?;
        f(c);
        Ok(c.to_json(&c.status))
    }).map_err(|e| if e.starts_with("not found") { not_found(format!("comment {id}")) } else { err(e) })
}

async fn comment_edit(_ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let (repo, id, text) = (repo(&p).await?, need_str(&p, "id")?, comment_text(&p)?);
    change(&repo, &id, |c| {
        c.text = text;
        if c.status == "sent" {
            (c.status, c.sent_at) = ("open".into(), None);
        }
    })
}

async fn comment_resolve(_ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let (repo, id) = (repo(&p).await?, need_str(&p, "id")?);
    let resolved = p.get("resolved").is_none() || truthy(&p, "resolved");
    change(&repo, &id, |c| {
        if resolved { c.status = "resolved".into() } else { (c.status, c.sent_at) = ("open".into(), None) }
    })
}

async fn comment_delete(_ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let (repo, id) = (repo(&p).await?, need_str(&p, "id")?);
    repo.store().update(&repo.key, |t| {
        let before = t.comments.len();
        t.comments.retain(|c| c.id != id);
        if t.comments.len() == before { Err(format!("not found: comment {id}")) } else { Ok(()) }
    }).map_err(|e| if e.starts_with("not found") { not_found(format!("comment {id}")) } else { err(e) })?;
    Ok(json!({"deleted": true}))
}

fn agent_panes(ctx: &Ctx) -> Vec<PaneInfo> {
    ctx.manager.lock().unwrap().panes.values().map(|p| {
        let m = p.meta.lock().unwrap();
        PaneInfo { id: m.id.clone(), name: m.name.clone(), cwd: m.cwd.clone(), kind: m.kind.clone(), state: p.state_name() }
    }).collect()
}

async fn review_send(ctx: Arc<Ctx>, p: Value) -> RpcResult {
    let repo = repo(&p).await?;
    let pane_id = select_pane(&agent_panes(&ctx), &repo.key, &opt_str(&p, "pane")).map_err(|e| RpcError::new(E_GENERAL, e))?;
    let open: Vec<Comment> = with_status(&repo, repo.store().read(&repo.key).comments).await.into_iter().filter(|(_, s)| s == "open").map(|(c, _)| c).collect();
    if open.is_empty() {
        return Err(RpcError::new(E_GENERAL, "no open comments to send"));
    }
    let pane = ctx.manager.lock().unwrap().get(&pane_id)?;
    let multiline = pane.state.lock().unwrap().screen.modes().bracketed_paste;
    let items: Vec<Item> = open.iter().map(|c| Item { file: &c.file, side: &c.side, line: c.line, anchor: &c.anchor, text: &c.text }).collect();
    let message = format_message(&items, multiline);
    {
        let lock = ctx.input_lock(&pane_id);
        let _guard = lock.lock().await; // keeps our text and Enter together
        match pane.state_name().as_str() {
            "blocked" => return Err(RpcError::new(E_BLOCKED, "agent_blocked: the agent is waiting on a question or permission prompt; nothing was sent")),
            s @ ("exited" | "stopped") => return Err(RpcError::new(E_GENERAL, format!("agent is {s}"))),
            _ => {}
        }
        submit(&pane, &message).await?;
        mark_busy(&ctx.manager.lock().unwrap(), &pane);
    }
    let ids: Vec<String> = open.into_iter().map(|c| c.id).collect();
    let now = now_secs();
    repo.store().update(&repo.key, |t| {
        for c in t.comments.iter_mut().filter(|c| ids.contains(&c.id)) {
            (c.status, c.sent_at) = ("sent".into(), Some(now));
        }
        Ok(())
    }).map_err(err)?;
    Ok(json!({"sent": ids.len(), "pane": pane_id}))
}
