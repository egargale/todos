//! L0 filter: which files are worth reading at all.
//!
//! Binary, minified and oversized files are rejected before the (much more
//! expensive) lexer ever sees them. This filter is pure and cheap so it can run
//! on every path with no I/O, aside from the sniff of the head.

/// Largest file we will lex, in bytes.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// A line longer than this is treated as a sign of minification.
pub const MAX_LINE_LEN: usize = 5_000;

/// Can this path plausibly hold human-written comments?
///
/// Extension-based. Unknown extensions are rejected rather than guessed at:
/// the lexer needs a syntax family, and an unknown file has no family.
pub fn is_candidate_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();

    // Minified artifacts are not hand-written.
    for suffix in [".min.js", ".min.css", ".map", ".min.mjs"] {
        if lower.ends_with(suffix) {
            return false;
        }
    }

    // Vendored and generated trees.
    for segment in [
        "/node_modules/",
        "/vendor/",
        "/dist/",
        "/target/",
        "/.venv/",
        "/__pycache__/",
    ] {
        if lower.contains(segment) || lower.starts_with(&segment[1..]) {
            return false;
        }
    }

    let ext = match lower.rsplit_once('.') {
        Some((_, ext)) => ext,
        None => return false,
    };

    matches!(
        ext,
        "rs" | "go"
            | "c" | "h" | "cc" | "cpp" | "hpp"
            | "js" | "mjs" | "cjs"
            | "ts" | "tsx" | "jsx"
            | "java" | "kt" | "kts" | "scala" | "swift"
            | "py" | "rb" | "sh" | "bash" | "zsh" | "pl" | "r" | "ex" | "exs"
            | "css" | "scss" | "sass" | "less"
            | "toml" | "yaml" | "yml" | "ini" | "conf"
            | "sql" | "lua" | "vim"
            | "html" | "htm" | "xml" | "vue" | "svelte"
    )
}

/// Cheap content checks. Rejects binaries (NUL in the first block) and files
/// whose longest line suggests minification.
pub fn is_candidate_content(bytes: &[u8]) -> bool {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return false;
    }
    let head = &bytes[..bytes.len().min(8192)];
    if head.contains(&0) {
        return false;
    }
    let mut longest = 0usize;
    let mut current = 0usize;
    for &b in bytes {
        if b == b'\n' {
            longest = longest.max(current);
            current = 0;
            if longest > MAX_LINE_LEN {
                return false;
            }
        } else {
            current += 1;
        }
    }
    longest.max(current) <= MAX_LINE_LEN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_sources() {
        assert!(is_candidate_path("src/main.rs"));
        assert!(is_candidate_path("web/app.tsx"));
        assert!(is_candidate_path("scripts/deploy.sh"));
    }

    #[test]
    fn rejects_minified_and_vendored() {
        assert!(!is_candidate_path("dist/bundle.min.js"));
        assert!(!is_candidate_path("node_modules/left-pad/index.js"));
        assert!(!is_candidate_path("assets/app.js.map"));
        assert!(!is_candidate_path("README"));
        assert!(!is_candidate_path("image.png"));
    }

    #[test]
    fn rejects_binary_and_minified_content() {
        assert!(!is_candidate_content(b"\x00\x01\x02"));
        let long = vec![b'a'; MAX_LINE_LEN + 1];
        assert!(!is_candidate_content(&long));
        assert!(is_candidate_content(b"fn main() {}\n"));
    }
}
