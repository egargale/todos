//! Parallel scan driver: discovery + L0 filter + L1 lexer in one pass.

use crate::filter::{is_candidate_content, is_candidate_path};
use crate::lexer::scan_file;
use crate::model::Marker;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Result of scanning a tree.
#[derive(Debug, Default)]
pub struct ScanResult {
    pub markers: Vec<Marker>,
    /// Files actually read (after the L0 filter).
    pub files_read: usize,
    /// Files skipped by the L0 filter.
    pub files_skipped: usize,
}

/// Scan every candidate path under `root`.
///
/// Threads pull from a shared index instead of pre-computing chunks: file sizes
/// vary by orders of magnitude and a static split would starve.
pub fn scan_all(root: &Path, paths: &[String], tags: &[String]) -> ScanResult {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let next = AtomicUsize::new(0);
    let collected: Mutex<Vec<Marker>> = Mutex::new(Vec::new());
    let read = AtomicUsize::new(0);
    let skipped = AtomicUsize::new(0);

    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let mut local = Vec::new();
                let mut local_read = 0usize;
                let mut local_skipped = 0usize;

                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= paths.len() {
                        break;
                    }
                    let rel = &paths[i];

                    if !is_candidate_path(rel) {
                        local_skipped += 1;
                        continue;
                    }
                    let Ok(bytes) = std::fs::read(root.join(rel)) else {
                        local_skipped += 1;
                        continue;
                    };
                    if !is_candidate_content(&bytes) {
                        local_skipped += 1;
                        continue;
                    }
                    local_read += 1;
                    local.extend(scan_file(rel, &bytes, tags));
                }

                read.fetch_add(local_read, Ordering::Relaxed);
                skipped.fetch_add(local_skipped, Ordering::Relaxed);
                if !local.is_empty() {
                    collected.lock().unwrap().extend(local);
                }
            });
        }
    });

    let mut markers = collected.into_inner().unwrap();
    sort_markers(&mut markers);

    ScanResult { markers, files_read: read.load(Ordering::Relaxed), files_skipped: skipped.load(Ordering::Relaxed) }
}

/// Deterministic order regardless of thread scheduling: path, then line, then
/// column. Output stability is a testability requirement, not a nicety.
pub fn sort_markers(markers: &mut [Marker]) {
    markers.sort_by(|a, b| {
        a.file.cmp(&b.file).then(a.line.cmp(&b.line)).then(a.col.cmp(&b.col))
    });
}

/// Scan an explicit list of paths (the pre-commit path).
///
/// The hook knows exactly which files changed, so there is nothing to discover
/// and no reason to walk the tree.
pub fn scan_paths(root: &Path, rel_paths: &[String], tags: &[String]) -> ScanResult {
    scan_all(root, rel_paths, tags)
}
