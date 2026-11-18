//! Output. Two shapes only: human (a table) and machine (JSON).
//!
//! When stdout is piped we emit `path:line:col tag text` — the `file:line`
//! convention every terminal, editor and `grep -n` consumer already understands.

use crate::model::Marker;
use std::io::IsTerminal;

/// Emit markers in the human table shape.
pub fn human(markers: &[Marker], files_read: usize) {
    if markers.is_empty() {
        println!("No TODOs found in {files_read} files.");
        return;
    }

    let mut last_file: Option<&str> = None;
    for m in markers {
        if last_file != Some(m.file.as_str()) {
            if last_file.is_some() {
                println!();
            }
            println!("{}", m.file);
            last_file = Some(m.file.as_str());
        }
        println!("  {:>4}:{:<3} {:<6} {}", m.line, m.col, m.tag, m.text);
    }

    println!();
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for m in markers {
        match counts.iter_mut().find(|(t, _)| *t == m.tag.as_str()) {
            Some((_, n)) => *n += 1,
            None => counts.push((m.tag.as_str(), 1)),
        }
    }
    let breakdown = counts
        .iter()
        .map(|(t, n)| format!("{t} {n}"))
        .collect::<Vec<_>>()
        .join(" · ");
    let files = unique_files(markers);
    println!("{} TODOs in {} files ({breakdown})", markers.len(), files);
}

/// Emit one `path:line:col tag text` line per marker, for pipes.
pub fn piped(markers: &[Marker]) {
    for m in markers {
        println!("{}:{}:{} {} {}", m.file, m.line, m.col, m.tag, m.text);
    }
}

/// Emit the flat JSON array.
pub fn json(markers: &[Marker]) -> Result<(), String> {
    serde_json::to_writer(std::io::stdout(), markers).map_err(|e| format!("json: {e}"))?;
    println!();
    Ok(())
}

/// Choose the shape from the terminal, unless the caller forced one.
pub fn is_interactive() -> bool {
    std::io::stdout().is_terminal()
}

fn unique_files(markers: &[Marker]) -> usize {
    let mut files: Vec<&str> = markers.iter().map(|m| m.file.as_str()).collect();
    files.sort_unstable();
    files.dedup();
    files.len()
}
