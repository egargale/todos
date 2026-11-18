//! L1: comment-aware lexing.
//!
//! This is the layer that makes the tool worth existing. A line regex matches
//! `const t: Todo[]`, `"/...todo/"` and i18n strings; measured on a real repo,
//! 56% of naive hits were not comments. Here every byte is classified as code,
//! string, or comment before any tag is looked for, and the in-comment state
//! carries across lines so block comments work.
//!
//! It is not a parser: we never build an AST. The only question is "am I inside
//! a comment right now, and where did it start". That is a small state machine
//! per syntax family, not a grammar.

use crate::model::Marker;

/// Comment syntax family. Chosen by extension; an unknown extension has no
/// family and is filtered out before we get here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// `//` and `/* */`, with `'` `"` and backtick strings.
    CStyle,
    /// `#` to end of line, with `'` `"` strings.
    Hash,
    /// `--` to end of line (and `--[[ ]]` treated as line comments).
    Sql,
    /// `<!-- -->` block only.
    Xml,
}

impl Family {
    /// Map a path to its family, or `None` if we do not know the syntax.
    pub fn of(path: &str) -> Option<Family> {
        let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
        Some(match ext.as_str() {
            "rs" | "go" | "c" | "h" | "cc" | "cpp" | "hpp" | "js" | "mjs" | "cjs" | "ts" | "tsx"
            | "jsx" | "java" | "kt" | "kts" | "scala" | "swift" | "css" | "scss" | "sass"
            | "less" => Family::CStyle,
            "py" | "rb" | "sh" | "bash" | "zsh" | "pl" | "r" | "ex" | "exs" | "toml" | "yaml"
            | "yml" | "ini" | "conf" => Family::Hash,
            "sql" | "lua" | "vim" => Family::Sql,
            "html" | "htm" | "xml" | "vue" | "svelte" => Family::Xml,
            _ => return None,
        })
    }
}

/// Where the lexer is between lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Code,
    /// Inside a block comment opened on an earlier line. A marker here belongs
    /// to the opening line, which already reported it; continuation lines are
    /// suppressed until the comment closes.
    BlockComment,
}

/// Extract markers from one file's bytes.
///
/// `tags` is the active tag list, matched case-sensitively. Markers are
/// returned in source order.
pub fn scan_file(path: &str, bytes: &[u8], tags: &[String]) -> Vec<Marker> {
    let family = match Family::of(path) {
        Some(f) => f,
        None => return Vec::new(),
    };
    // Not UTF-8 => not source we can reason about. Cheaper than lossy decoding
    // and avoids every byte/char index mismatch below.
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    let mut state = State::Code;

    for (idx, raw_line) in text.split('\n').enumerate() {
        let line_no = idx as u32 + 1;
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);

        // Every comment region on this line, as (comment_start, content_start).
        let regions = comment_regions(line, family, &mut state);

        for region in regions {
            if let Some(marker) = match_tag(path, line_no, region, line, tags) {
                out.push(marker);
            }
        }
    }

    out
}

/// A region is `(comment_start, content_start)` — the offset of the comment
/// introducer and the offset where the comment's text begins.
///
/// A block comment that continues onto this line yields a region starting at 0:
/// the whole line is comment text, and `match_tag` searches it for a tag. That
/// is what makes the star-banner style work (`/*\n * TODO: x\n */`).
fn comment_regions(line: &str, family: Family, state: &mut State) -> Vec<(usize, usize)> {
    match family {
        Family::Xml => regions_xml(line, state),
        _ => regions_line_based(line, family, state),
    }
}

/// `<!-- -->`: a comment may open and close many times, and a line may end
/// inside one.
///
/// A region is `(start, content_start)` — the offset of the comment introducer
/// and the offset where the comment's text begins, so `match_tag` never has to
/// guess which introducer it is looking at.
fn regions_xml(line: &str, state: &mut State) -> Vec<(usize, usize)> {
    const OPEN: &str = "<!--";
    const CLOSE: &str = "-->";
    let mut regions = Vec::new();

    // Inside an inherited block comment. The marker belongs to whichever line
    // carries it, so this line is scanned as comment text from offset 0; only
    // the close terminator ends the comment.
    if let State::BlockComment = state {
        if let Some(close) = line.find(CLOSE) {
            *state = State::Code;
            regions.push((0, 0));
            // Anything after the close is code; a new comment can start there.
            if let Some(open) = line[close + CLOSE.len()..].find(OPEN) {
                let open_at = close + CLOSE.len() + open;
                regions.push((open_at, open_at + OPEN.len()));
            }
        } else {
            regions.push((0, 0));
        }
        return regions;
    }

    let mut cursor = 0usize;
    while cursor < line.len() {
        match line[cursor..].find(OPEN) {
            Some(rel) => {
                let open_at = cursor + rel;
                let after = open_at + OPEN.len();
                match line[after..].find(CLOSE) {
                    Some(rel_close) => {
                        regions.push((open_at, after));
                        cursor = after + rel_close + CLOSE.len();
                    }
                    None => {
                        // Unclosed: this line still gets scanned, and the next
                        // lines are suppressed until `-->` appears.
                        regions.push((open_at, after));
                        *state = State::BlockComment;
                        return regions;
                    }
                }
            }
            None => break,
        }
    }
    regions
}

/// Line-comment and `/* */` styles, with string tracking so `"//"` is not
/// mistaken for a comment.
fn regions_line_based(line: &str, family: Family, state: &mut State) -> Vec<(usize, usize)> {
    let (line_marker, has_block, quote_chars): (&str, bool, &[u8]) = match family {
        Family::CStyle => ("//", true, b"'\"`"),
        // Python/Shell: `#`; `'` and `"` strings. Tripled strings are a v1 gap,
        // and the reason L2 tree-sitter exists for the 2-3 worst languages.
        Family::Hash => ("#", false, b"'\""),
        Family::Sql => ("--", false, b"'\""),
        Family::Xml => unreachable!("handled by regions_xml"),
    };

    let bytes = line.as_bytes();
    let mut regions = Vec::new();
    let mut i = 0usize;
    let mut in_string: Option<u8> = None;
    let mut escaped = false;

    // A block comment inherited from a previous line makes the entire line
    // comment text, so it gets a region from offset 0. Only the close ends it.
    if let State::BlockComment = state {
        if has_block {
            match line.find("*/") {
                Some(close) => {
                    *state = State::Code;
                    regions.push((0, 0));
                    // Resume scanning after the close, in what may be code.
                    if let Some(first) = scan_from(line, close + 2, line_marker, has_block) {
                        regions.push((first, first + line_marker.len()));
                    }
                }
                None => regions.push((0, 0)),
            }
        }
        return regions;
    }

    while i < bytes.len() {
        let b = bytes[i];

        if let Some(q) = in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' && q != b'`' {
                escaped = true;
            } else if b == q {
                in_string = None;
            }
            i += 1;
            continue;
        }

        if quote_chars.contains(&b) {
            in_string = Some(b);
            i += 1;
            continue;
        }

        if has_block && line[i..].starts_with("/*") {
            regions.push((i, i + 2));
            match line[i + 2..].find("*/") {
                Some(rel) => {
                    i = i + 2 + rel + 2;
                    continue;
                }
                None => {
                    *state = State::BlockComment;
                    return regions;
                }
            }
        }

        if line[i..].starts_with(line_marker) {
            regions.push((i, i + line_marker.len()));
            return regions;
        }

        i += 1;
    }

    regions
}

/// Minimal follow-on scan used only to find a `//` comment that begins after a
/// block comment closed earlier on the same line.
fn scan_from(line: &str, from: usize, line_marker: &str, has_block: bool) -> Option<usize> {
    let mut i = from;
    while i < line.len() {
        if has_block && line[i..].starts_with("/*") {
            match line[i + 2..].find("*/") {
                Some(rel) => {
                    i = i + 2 + rel + 2;
                    continue;
                }
                None => return None,
            }
        }
        if line[i..].starts_with(line_marker) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Match a tag anywhere inside a comment region.
///
/// A tag does not have to be the first thing in the comment: `/* TODO x */`,
/// `/*\n * TODO y\n */`, and `// note — TODO z` are all real. The comment body
/// is walked, skipping leading decoration (`*`, `-`, whitespace) so the
/// conventional block-comment star does not hide the tag.
///
/// The tag must be followed by a boundary (end of comment text, whitespace, or
/// `:`/`(`), so `TODOS` and `TODO_LIST` do not match `TODO`.
fn match_tag(path: &str, line_no: u32, region: (usize, usize), line: &str, tags: &[String]) -> Option<Marker> {
    let (_comment_start, content_start) = region;
    if content_start > line.len() {
        return None;
    }
    // The comment's text ends at the terminator, if there is one on this line.
    let raw = &line[content_start..];
    let body_end = ["*/", "-->"]
        .iter()
        .filter_map(|term| raw.find(term))
        .min()
        .unwrap_or(raw.len());
    let body = &raw[..body_end];

    // Fast path: the tag is the first real thing in the comment, which is the
    // overwhelmingly common style (`// TODO: x`, `# FIXME: y`).
    let rest = body.trim_start();
    let leading = body.len() - rest.len();
    for tag in tags {
        if let Some(after) = rest.strip_prefix(tag.as_str()) {
            if is_boundary(after) {
                return Some(make_marker(path, line_no, content_start + leading, tag, after));
            }
        }
    }

    // Slow path: the tag appears later in the comment. Scan word starts only —
    // a tag is never a substring of a word, so `TODOS` and `TODO_LIST` do not
    // match, and this also keeps us from matching inside `http://x/TODO-y`.
    let mut at_word_start = false;
    for (idx, c) in body.char_indices() {
        if c.is_whitespace() || c == '*' || c == '-' {
            at_word_start = true;
            continue;
        }
        if !at_word_start {
            continue;
        }
        at_word_start = false;

        let from = &body[idx..];
        for tag in tags {
            if let Some(after) = from.strip_prefix(tag.as_str()) {
                if is_boundary(after) {
                    return Some(make_marker(path, line_no, content_start + idx, tag, after));
                }
            }
        }
    }

    None
}

/// A tag must end at a word boundary, not be a prefix of a longer word.
///
/// A hyphen is deliberately NOT a boundary: `TODO-family` and `TODO-tracking`
/// are prose adjectives, not markers. The cost is a real `TODO - later` written
/// with spaces, which is rare next to that false-positive class.
fn is_boundary(after: &str) -> bool {
    after.is_empty()
        || after.starts_with(|c: char| {
            c.is_whitespace() || c == ':' || c == '(' || c == '[' || c == '!'
        })
}

fn make_marker(path: &str, line_no: u32, tag_at: usize, tag: &str, after: &str) -> Marker {
    let text = after.trim_start_matches([':', ' ']).trim();
    Marker {
        file: path.to_string(),
        line: line_no,
        col: (tag_at + 1) as u32,
        tag: tag.to_string(),
        text: collapse(text),
    }
}

/// Comment text is single-line for diff purposes: internal whitespace runs
/// collapse, tabs become spaces.
fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for c in s.trim().chars() {
        if c.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags() -> Vec<String> {
        vec!["TODO".into(), "FIXME".into(), "HACK".into()]
    }

    fn scan(path: &str, src: &str) -> Vec<Marker> {
        scan_file(path, src.as_bytes(), &tags())
    }

    #[test]
    fn finds_line_comment_marker() {
        let m = scan("a.rs", "fn x() {} // TODO: rotate token\n");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].tag, "TODO");
        assert_eq!(m[0].text, "rotate token");
        assert_eq!(m[0].line, 1);
    }

    #[test]
    fn ignores_markers_in_strings() {
        // The classic false positives from the devil's advocate's measurement.
        let src = r#"
const msg = "TODO: fix this";
const re = /class\s+(\w+)\s+(done|doing|todo)/;
interface Todo { id: string }
const t: Todo[] = [];
"#;
        let m = scan("a.ts", src);
        assert!(m.is_empty(), "expected no hits, got {:?}", m);
    }

    #[test]
    fn ignores_marker_in_url() {
        let m = scan("a.css", "/* see https://example.com/page#TODO-anchor */");
        assert!(m.is_empty(), "got {:?}", m);
    }

    #[test]
    fn block_comment_marker_not_repeated() {
        // The star-banner style: the marker lives on the continuation line, and
        // must be reported once, not on every line of the block.
        let src = "/*\n * TODO: rewrite\n * more text\n */\nfn x() {}\n";
        let m = scan("a.rs", src);
        assert_eq!(m.len(), 1, "got {:?}", m);
        assert_eq!(m[0].line, 2);
        assert_eq!(m[0].text, "rewrite");
    }

    #[test]
    fn code_after_block_comment_still_scanned() {
        let src = "/* note */ let x = 1; // TODO: after block\n";
        let m = scan("a.rs", src);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].text, "after block");
    }

    #[test]
    fn hash_family() {
        let m = scan("a.py", "x = 1  # FIXME: nope\n");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].tag, "FIXME");
    }

    #[test]
    fn python_shebang_is_not_a_tag() {
        let m = scan("a.sh", "#!/bin/bash\n");
        assert!(m.is_empty());
    }

    #[test]
    fn xml_comment() {
        let m = scan("a.html", "<!-- TODO: markup -->\n");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].text, "markup");
    }

    #[test]
    fn tag_boundary_respected() {
        assert!(scan("a.rs", "// TODOS are fine\n").is_empty());
        let m = scan("a.rs", "// TODO(ABC-1): later\n");
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn hyphenated_prose_is_not_a_marker() {
        // `TODO-family` / `TODO-tracking` are adjectives, not markers.
        let m = scan("a.rs", "//! Find TODO-family markers\n");
        assert!(m.is_empty(), "got {:?}", m);
    }

    #[test]
    fn todos_plural_does_not_match() {
        let m = scan("a.rs", "// open TODOS list\n");
        assert!(m.is_empty(), "got {:?}", m);
    }

    #[test]
    fn tag_need_not_open_the_comment() {
        // A word before the tag is fine — the tag is still a tag.
        let m = scan("a.rs", "// the TODO is elsewhere\n");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].text, "is elsewhere");
    }

    #[test]
    fn unknown_extension_ignored() {
        assert!(scan("notes.txt", "# TODO: hi\n").is_empty());
    }

    #[test]
    fn non_utf8_skipped() {
        let bytes = [0xffu8, 0xfe, b'/', b'/'];
        let m = scan_file("a.rs", &bytes, &tags());
        assert!(m.is_empty());
    }

    #[test]
    fn col_points_at_tag() {
        let m = scan("a.rs", "    // TODO x\n");
        // 4 spaces + "// " = 7 bytes before the tag, so 1-based col 8.
        assert_eq!(m[0].col, 8);
    }
}
