//! A marker found inside a comment.

use serde::Serialize;

/// A single `TODO`-family marker, located and annotated.
///
/// `text` is the remainder of the comment after the marker, trimmed. It is the
/// only field derived from the source beyond position, and it is what makes the
/// set-diff meaningful: editing the text is intentionally a different marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Marker {
    /// Repository-relative path, forward slashes.
    pub file: String,
    /// 1-based line number.
    pub line: u32,
    /// 1-based column of the marker's first character.
    pub col: u32,
    /// The matched tag, e.g. `TODO`.
    pub tag: String,
    /// Comment text after the tag, collapsed to one line and trimmed.
    pub text: String,
}

impl Marker {
    /// Stable comparison key used by the base-ref diff.
    ///
    /// Deliberately excludes line and column: a marker that merely moved down
    /// because code was inserted above it is not new debt. Deliberately
    /// includes the path, so a rename reads as delete+add — accepted in v0,
    /// and the reason renames must not be counted against a branch.
    pub fn key(&self) -> (&str, &str, &str) {
        (self.file.as_str(), self.tag.as_str(), self.text.as_str())
    }
}
