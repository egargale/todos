//! File discovery: what to read.
//!
//! Gitignore semantics are subtle — negation, dir-only patterns, `**`, layered
//! precedence, `.git/info/exclude`, global excludes. Reimplementing that is a
//! permanent bug source, so inside a repo we ask git once and never re-derive.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Why discovery produced what it did. The distinction matters to the UX: "not
/// a git repo" degrades features, it is not an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Paths came from `git ls-files`.
    Git,
    /// No repo (or no git binary): plain recursive walk.
    Walk,
}

/// Repository-relative paths, to be read relative to `root`.
pub struct Discovery {
    pub root: PathBuf,
    pub paths: Vec<String>,
    pub mode: Mode,
}

/// Discover candidate files under `root`.
///
/// In git mode: tracked plus untracked-not-ignored, which is exactly the file
/// set a developer thinks of as "my project". In walk mode: everything not
/// obviously vendored, with the L0 path filter applied later.
pub fn discover(root: &Path) -> Discovery {
    if let Some(paths) = git_ls_files(root) {
        return Discovery { root: root.to_path_buf(), paths, mode: Mode::Git };
    }
    Discovery { root: root.to_path_buf(), paths: walk(root), mode: Mode::Walk }
}

/// `git ls-files -z --cached --others --exclude-standard`.
///
/// Returns `None` when this is not a repo or git is missing, so the caller can
/// degrade instead of failing.
fn git_ls_files(root: &Path) -> Option<Vec<String>> {
    let out = Command::new("git")
        .current_dir(root)
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .output()
        .ok()?;

    if !out.status.success() {
        return None;
    }

    let raw = String::from_utf8(out.stdout).ok()?;
    Some(raw.split('\0').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect())
}

/// Plain recursive walk, used outside a repo.
///
/// Deliberately simple: no ignore files. The L0 filter removes binaries and
/// vendored paths, and outside a repo there is no .gitignore contract to honour.
fn walk(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();

            if name.starts_with('.') && name != ".github" {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };

            if meta.is_dir() {
                // Skip the usual offenders early; the L0 filter is a backstop.
                if matches!(name.as_ref(), "node_modules" | "target" | "dist" | "vendor") {
                    continue;
                }
                stack.push(path);
            } else if meta.is_file() {
                if let Ok(rel) = path.strip_prefix(root) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }

    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_repo_falls_back_to_walk() {
        let tmp = std::env::temp_dir().join(format!("todos-discover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.rs"), "// TODO x\n").unwrap();
        std::fs::create_dir_all(tmp.join("node_modules")).unwrap();
        std::fs::write(tmp.join("node_modules/b.js"), "// TODO y\n").unwrap();

        let d = discover(&tmp);
        assert_eq!(d.mode, Mode::Walk);
        assert!(d.paths.contains(&"a.rs".to_string()));
        assert!(!d.paths.iter().any(|p| p.contains("node_modules")));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
