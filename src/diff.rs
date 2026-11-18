//! Diff against a git base ref: which markers are new.
//!
//! TODOs have no IDs, so "new" is a set difference, not an object match. The key
//! is (path, tag, text) — see `Marker::key`. Line and column are excluded so a
//! marker that merely shifted is not counted as new debt.
//!
//! Known and accepted v0 weakness: a rename reads as delete+add, so every marker
//! in a moved file looks new. Git rename detection is the fix; until then, do not
//! gate on renames.

use crate::model::Marker;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

/// Markers present now that were not present at `base_ref`.
///
/// The baseline is scanned **only for the files that currently contain a
/// marker**. That scoping is what makes the pre-commit hook usable: a hook
/// touching one file compares one file against its base version, not the whole
/// tree. A marker whose text changed is new+gone (expected); a marker that
/// merely shifted lines is neither.
pub fn new_markers(
    root: &Path,
    base_ref: &str,
    current: &[Marker],
    tags: &[String],
) -> Result<Vec<Marker>, String> {
    if current.is_empty() {
        return Ok(Vec::new());
    }

    let rev = resolve(root, base_ref)?;

    // Only the files that have markers today can contribute "new" markers, so
    // only those need a baseline. Deduplicate: one blob per file, however many
    // markers it holds.
    let mut files: Vec<&str> = current.iter().map(|m| m.file.as_str()).collect();
    files.sort_unstable();
    files.dedup();

    let base_set = scan_base_files(root, &rev, &files, tags)?;

    Ok(current
        .iter()
        .filter(|m| {
            let (f, t, x) = m.key();
            !base_set.contains(&(f.to_string(), t.to_string(), x.to_string()))
        })
        .cloned()
        .collect())
}

/// Scan specific files as they exist at `rev`.
///
/// `git cat-file --batch` streams each blob in one process, so N files cost one
/// process launch plus N small reads — not N `git show` calls. Blobs are decoded
/// with the same lexer as the working tree, so the comparison is apples to
/// apples. A file absent at `rev` contributes nothing, which makes every marker
/// in a newly added file correctly "new".
fn scan_base_files(
    root: &Path,
    rev: &str,
    files: &[&str],
    tags: &[String],
) -> Result<HashSet<(String, String, String)>, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let mut child = Command::new("git")
        .current_dir(root)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("git cat-file failed to start: {e}"))?;

    {
        let stdin = child.stdin.as_mut().ok_or("no stdin on git cat-file")?;
        for f in files {
            writeln!(stdin, "{rev}:{f}").map_err(|e| format!("cannot write to git cat-file: {e}"))?;
        }
    }
    // Dropping stdin closes it, so cat-file knows the request list is complete.
    drop(child.stdin.take());

    let stdout = child.stdout.take().ok_or("no stdout on git cat-file")?;
    let mut reader = BufReader::new(stdout);

    let mut out = HashSet::new();
    // Responses are read in request order, so index into `files` to recover the
    // path — cat-file echoes the object name, but the path is what we key on.
    let mut idx = 0usize;

    while idx < files.len() {
        let mut header = String::new();
        if reader.read_line(&mut header).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let header = header.trim_end();

        // "<oid> <type> <size>" for a hit; "<name> missing" for a miss.
        let parts: Vec<&str> = header.split(' ').collect();
        if parts.len() == 3 && parts[1] == "blob" {
            let size: usize = parts[2].parse().map_err(|_| format!("bad blob size in '{header}'"))?;
            let mut buf = vec![0u8; size];
            std::io::Read::read_exact(&mut reader, &mut buf).map_err(|e| e.to_string())?;
            // Consume the trailing newline cat-file writes after each payload.
            let mut nl = [0u8; 1];
            let _ = std::io::Read::read_exact(&mut reader, &mut nl);

            let path = files[idx];
            for m in crate::lexer::scan_file(path, &buf, tags) {
                out.insert((m.file, m.tag, m.text));
            }
        }
        // A "missing" header has no payload; just advance.
        idx += 1;
    }

    let _ = child.wait();
    Ok(out)
}

/// Resolve a user-supplied ref to a commit `cat-file` accepts.
///
/// `origin/main` often exists only as a remote-tracking ref; the merge-base is
/// the honest comparison point for a branch.
///
/// A repo with no commits has no base at all. That is not an error — it is the
/// state of every new repository, and every marker in it is genuinely new. The
/// caller checks `has_no_commits` and skips the baseline entirely.
fn resolve(root: &Path, base_ref: &str) -> Result<String, String> {
    let out = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "--verify", &format!("{base_ref}^{{commit}}")])
        .output()
        .map_err(|e| format!("git not available: {e}"))?;

    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }

    // Not directly resolvable: try merge-base against HEAD.
    let mb = Command::new("git")
        .current_dir(root)
        .args(["merge-base", base_ref, "HEAD"])
        .output()
        .map_err(|e| format!("git not available: {e}"))?;
    if mb.status.success() {
        return Ok(String::from_utf8_lossy(&mb.stdout).trim().to_string());
    }

    Err(format!("cannot resolve base ref '{base_ref}'"))
}

/// Is this a repo with no commits yet?
///
/// Every marker in a brand-new repo is new, and `HEAD` does not exist, so the
/// gate must not fail there — that would break the first commit of every
/// project that installs the hook.
pub fn has_no_commits(root: &Path) -> bool {
    match Command::new("git").current_dir(root).args(["rev-parse", "--verify", "HEAD"]).output() {
        Ok(o) => !o.status.success(),
        Err(_) => false,
    }
}

/// Paths with staged changes, for the pre-commit hook.
///
/// Only staged content is visible to the hook: unstaged work must never block a
/// commit. Returns repo-relative paths.
pub fn staged_paths(root: &Path) -> Vec<String> {
    let out = match Command::new("git")
        .current_dir(root)
        .args(["diff", "--cached", "--name-only", "-z", "--diff-filter=ACMR"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(file: &str, line: u32, text: &str) -> Marker {
        Marker { file: file.into(), line, col: 1, tag: "TODO".into(), text: text.into() }
    }

    #[test]
    fn key_ignores_position() {
        let a = m("a.rs", 10, "x");
        let b = m("a.rs", 99, "x");
        assert_eq!(a.key(), b.key());
    }

    #[test]
    fn key_includes_text() {
        assert_ne!(m("a.rs", 1, "x").key(), m("a.rs", 1, "y").key());
    }
}
