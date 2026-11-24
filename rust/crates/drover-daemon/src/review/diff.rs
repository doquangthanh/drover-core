//! The diff of one file's working tree against a merge base, parsed into hunks with a stable hash each.

use std::path::Path;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::Repo;
use crate::git;

const MAX_UNTRACKED_BYTES: usize = 2_000_000;

pub struct Line {
    pub k: char,
    pub o: Option<u32>,
    pub n: Option<u32>,
    pub t: String,
}

pub struct Hunk {
    pub index: usize,
    pub header: String,
    pub hash: String,
    pub lines: Vec<Line>,
    /// The hunk exactly as git printed it (header line, body, "\ No newline" markers), for `git apply`.
    pub raw: String,
}

#[derive(Default)]
pub struct FileDiff {
    /// Everything before the first hunk (`diff --git`, `index`, `---`, `+++`...).
    pub head: String,
    pub hunks: Vec<Hunk>,
    pub binary: bool,
}

impl Hunk {
    pub fn to_json(&self, reviewed: bool) -> Value {
        json!({"index": self.index, "header": self.header, "hash": self.hash, "reviewed": reviewed,
            "lines": self.lines.iter().map(|l| json!({"k": l.k.to_string(), "o": l.o, "n": l.n, "t": l.t})).collect::<Vec<_>>()})
    }
}

/// Stable id of a hunk: changes only when its lines change, not when the hunk moves in the file.
pub fn hunk_hash(lines: &[Line]) -> String {
    let mut h = Sha256::new();
    for l in lines {
        h.update(format!("{}{}\n", l.k, l.t));
    }
    h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn header_starts(header: &str) -> Option<(u32, u32)> {
    let re = regex::Regex::new(r"^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@").ok()?;
    let c = re.captures(header)?;
    Some((c[1].parse().ok()?, c[2].parse().ok()?))
}

/// Parse `git diff` output of a single file.
pub fn parse(diff: &str) -> FileDiff {
    let mut out = FileDiff::default();
    if diff.lines().any(|l| l.starts_with("Binary files ") || l.starts_with("GIT binary patch")) {
        out.binary = true;
        return out;
    }
    let mut all: Vec<&str> = diff.split('\n').collect();
    if all.last() == Some(&"") {
        all.pop();
    }
    let mut current: Option<(String, u32, u32, Vec<Line>, String)> = None; // header, old no, new no, lines, raw
    let finish = |cur: Option<(String, u32, u32, Vec<Line>, String)>, hunks: &mut Vec<Hunk>| {
        if let Some((header, _, _, lines, raw)) = cur {
            hunks.push(Hunk { index: hunks.len(), header, hash: hunk_hash(&lines), lines, raw });
        }
    };
    for line in all {
        if line.starts_with("@@") {
            finish(current.take(), &mut out.hunks);
            let Some((o, n)) = header_starts(line) else { continue };
            let header = line.trim_end_matches('\r').split(" @@").next().map(|h| format!("{h} @@")).unwrap_or_default();
            current = Some((header, o, n, vec![], format!("{line}\n")));
            continue;
        }
        let Some((_, o, n, lines, raw)) = current.as_mut() else {
            out.head.push_str(line);
            out.head.push('\n');
            continue;
        };
        raw.push_str(line);
        raw.push('\n');
        let text = |s: &str| s.trim_end_matches('\r').to_string();
        match line.chars().next() {
            Some('+') => { lines.push(Line { k: '+', o: None, n: Some(*n), t: text(&line[1..]) }); *n += 1; }
            Some('-') => { lines.push(Line { k: '-', o: Some(*o), n: None, t: text(&line[1..]) }); *o += 1; }
            Some('\\') => {}
            _ => {
                lines.push(Line { k: ' ', o: Some(*o), n: Some(*n), t: text(line.get(1..).unwrap_or("")) });
                *o += 1;
                *n += 1;
            }
        }
    }
    finish(current.take(), &mut out.hunks);
    out
}

/// A new file shown as one all-added hunk, in the same text form git would print.
pub fn untracked_diff_text(file: &str, content: &str) -> String {
    let mut text = format!("diff --git a/{file} b/{file}\nnew file mode 100644\n--- /dev/null\n+++ b/{file}\n");
    let lines: Vec<&str> = content.split('\n').collect();
    let (body, ends_nl) = if lines.last() == Some(&"") { (&lines[..lines.len() - 1], true) } else { (&lines[..], false) };
    if body.is_empty() {
        return text;
    }
    text.push_str(&format!("@@ -0,0 +1,{} @@\n", body.len()));
    for l in body {
        text.push_str(&format!("+{l}\n"));
    }
    if !ends_nl {
        text.push_str("\\ No newline at end of file\n");
    }
    text
}

/// Diff of `file` (working tree incl. staged) against `merge_base`; untracked files are all added.
pub async fn file_diff(repo: &Repo, merge_base: &str, file: &str) -> Result<FileDiff, String> {
    let args = ["--literal-pathspecs", "diff", "--no-color", "--no-ext-diff", "--no-renames", "-U3", merge_base, "--", file];
    let text = git::git(&repo.root, &args, true).await?;
    if !text.trim().is_empty() {
        return Ok(parse(&text));
    }
    let untracked = git::git(&repo.root, &["--literal-pathspecs", "ls-files", "--others", "--exclude-standard", "--", file], false).await?;
    if untracked.trim().is_empty() {
        return Ok(FileDiff::default());
    }
    let bytes = tokio::fs::read(Path::new(&repo.root).join(file)).await.map_err(|e| format!("cannot read {file}: {e}"))?;
    if bytes.len() > MAX_UNTRACKED_BYTES || bytes[..bytes.len().min(8000)].contains(&0) {
        return Ok(FileDiff { binary: true, ..Default::default() });
    }
    Ok(parse(&untracked_diff_text(file, &String::from_utf8_lossy(&bytes))))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/f.txt b/f.txt\nindex 1..2 100644\n--- a/f.txt\n+++ b/f.txt\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n@@ -10,2 +10,3 @@ fn x\n x\n+y\n z\n\\ No newline at end of file\n";

    #[test]
    fn parses_hunks_lines_and_numbers() {
        let d = parse(DIFF);
        assert!(!d.binary && d.head.starts_with("diff --git") && d.hunks.len() == 2);
        let h = &d.hunks[0];
        assert_eq!((h.index, h.header.as_str()), (0, "@@ -1,3 +1,3 @@"));
        let shape: Vec<(char, Option<u32>, Option<u32>)> = h.lines.iter().map(|l| (l.k, l.o, l.n)).collect();
        assert_eq!(shape, vec![(' ', Some(1), Some(1)), ('-', Some(2), None), ('+', None, Some(2)), (' ', Some(3), Some(3))]);
        assert_eq!(d.hunks[1].header, "@@ -10,2 +10,3 @@");
        assert_eq!(d.hunks[1].lines[1].n, Some(11));
        assert!(d.hunks[1].raw.ends_with("\\ No newline at end of file\n"));
    }

    #[test]
    fn hash_ignores_position_but_not_content() {
        let moved = DIFF.replace("@@ -1,3 +1,3 @@", "@@ -5,3 +5,3 @@");
        assert_eq!(parse(DIFF).hunks[0].hash, parse(&moved).hunks[0].hash);
        assert_ne!(parse(DIFF).hunks[0].hash, parse(DIFF).hunks[1].hash);
        assert_eq!(parse(DIFF).hunks[0].hash.len(), 16);
        assert!(parse("Binary files a/x and b/x differ\n").binary);
    }

    #[test]
    fn untracked_file_is_one_added_hunk() {
        let d = parse(&untracked_diff_text("n.txt", "one\ntwo\n"));
        assert_eq!(d.hunks.len(), 1);
        assert_eq!(d.hunks[0].header, "@@ -0,0 +1,2 @@");
        assert!(d.hunks[0].lines.iter().all(|l| l.k == '+'));
        assert!(parse(&untracked_diff_text("e.txt", "")).hunks.is_empty());
        assert!(untracked_diff_text("n", "a").ends_with("\\ No newline at end of file\n"));
    }
}
