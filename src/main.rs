//! `todos` — find TODO-family markers in source comments.
//!
//! Two things make this worth a binary rather than `rg TODO`:
//!   1. comment-awareness — a marker in a string literal is not a marker;
//!   2. diff-scoped gating — the gate asks the line you just wrote not to add
//!      debt, which is a rule a developer accepts on their first commit.
//!
//! Exit codes: 0 clean, 1 markers flagged, 2 usage/config error.

mod diff;
mod discover;
mod filter;
mod lexer;
mod model;
mod report;
mod scan;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

const DEFAULT_TAGS: &[&str] = &["TODO", "FIXME", "HACK", "XXX"];

#[derive(Parser)]
#[command(name = "todos", version, about = "Find TODO-family markers in source comments")]
struct Cli {
    /// Directory to scan (defaults to the current directory).
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Tag to look for, repeatable. Defaults to TODO/FIXME/HACK/XXX.
    #[arg(long = "tag", value_name = "TAG")]
    tags: Vec<String>,

    /// Emit JSON.
    #[arg(long)]
    json: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Gate on markers added relative to a base ref. Exit 1 if any are new.
    Check {
        /// Base ref to diff against.
        #[arg(long, default_value = "HEAD")]
        base: String,
        /// Only consider staged files (pre-commit hook).
        #[arg(long)]
        staged: bool,
        /// Compare the whole tree instead of the diff. Warns: existing debt will block.
        #[arg(long)]
        all: bool,
    },
    /// Counts by tag and by file.
    Stats,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let tags: Vec<String> = if cli.tags.is_empty() {
        DEFAULT_TAGS.iter().map(|s| s.to_string()).collect()
    } else {
        cli.tags.clone()
    };

    match cli.command {
        None => list(&cli.path, &tags, cli.json),
        Some(Command::Stats) => stats(&cli.path, &tags),
        Some(Command::Check { base, staged, all }) => check(&cli.path, &tags, &base, staged, all),
    }
}

fn list(root: &std::path::Path, tags: &[String], json: bool) -> ExitCode {
    let disc = discover::discover(root);
    let result = scan::scan_all(&disc.root, &disc.paths, tags);

    if json {
        if let Err(e) = report::json(&result.markers) {
            eprintln!("todos: {e}");
            return ExitCode::from(2);
        }
    } else if report::is_interactive() {
        report::human(&result.markers, result.files_read);
        if disc.mode == discover::Mode::Walk {
            println!("(not a git repository — diff-scoped gating unavailable)");
        }
    } else {
        report::piped(&result.markers);
    }
    ExitCode::SUCCESS
}

fn stats(root: &std::path::Path, tags: &[String]) -> ExitCode {
    let disc = discover::discover(root);
    let result = scan::scan_all(&disc.root, &disc.paths, tags);

    println!("scanned {} files ({} skipped)", result.files_read, result.files_skipped);
    println!("{} markers\n", result.markers.len());

    let mut by_tag: Vec<(&str, usize)> = Vec::new();
    for m in &result.markers {
        match by_tag.iter_mut().find(|(t, _)| *t == m.tag.as_str()) {
            Some((_, n)) => *n += 1,
            None => by_tag.push((m.tag.as_str(), 1)),
        }
    }
    by_tag.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (tag, n) in by_tag {
        println!("{tag:<6} {n}");
    }
    ExitCode::SUCCESS
}

fn check(root: &std::path::Path, tags: &[String], base: &str, staged: bool, all: bool) -> ExitCode {
    let disc = discover::discover(root);

    let current = if all {
        eprintln!("todos: --all compares the whole tree; existing debt will block.");
        scan::scan_all(&disc.root, &disc.paths, tags).markers
    } else if staged {
        let paths = diff::staged_paths(&disc.root);
        scan::scan_paths(&disc.root, &paths, tags).markers
    } else {
        scan::scan_all(&disc.root, &disc.paths, tags).markers
    };

    // `compared` tracks whether the result is a diff or a whole-tree report, so
    // the summary line cannot claim "no new markers" when we fell back to
    // reporting everything.
    let (flagged, compared) = if all {
        (current, false)
    } else if diff::has_no_commits(&disc.root) {
        // First commit of a new repo: no base exists, so everything is new.
        // Say it plainly rather than succeeding silently.
        eprintln!("todos: no commits yet — every marker is new");
        (current, false)
    } else {
        match diff::new_markers(&disc.root, base, &current, tags) {
            Ok(v) => (v, true),
            Err(e) => {
                // A degraded mode, not a failure: say so, then report everything
                // found rather than silently passing.
                eprintln!("todos: cannot compare against {base}: {e}");
                eprintln!("todos: falling back to reporting all markers");
                (current, false)
            }
        }
    };

    if flagged.is_empty() {
        let scope = if compared { "diff" } else { "tree" };
        eprintln!("todos: no markers found in {scope}");
        return ExitCode::SUCCESS;
    }

    for m in &flagged {
        eprintln!("{}:{}:{} {} {}", m.file, m.line, m.col, m.tag, m.text);
    }
    let noun = if compared { "new marker(s)" } else { "marker(s)" };
    eprintln!("\ntodos: {} {noun}", flagged.len());
    ExitCode::from(1)
}
