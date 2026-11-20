//! Process-level tests for `todos check --json`.
//!
//! These must be process-level: the acceptance criterion is stream separation
//! (data on STDOUT, diagnosis on STDERR), which a `#[cfg(test)]` unit test
//! cannot observe — it shares the harness's own stdout/stderr.

use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_todos");

/// A throwaway git repo, removed on drop.
struct Repo {
    dir: PathBuf,
}

impl Repo {
    /// A repo with one commit (`a.rs = "fn a() {}\n"`), then `debt` appended to
    /// `a.rs` as an uncommitted change.
    fn with_added_debt(name: &str, debt: &str) -> Repo {
        let dir = std::env::temp_dir().join(format!(
            "todos-check-json-{}-{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp repo dir");

        let repo = Repo { dir };
        repo.git(&["init", "-q", "."]);
        repo.git(&["config", "user.email", "test@example.invalid"]);
        repo.git(&["config", "user.name", "todos test"]);
        std::fs::write(repo.path("a.rs"), "fn a() {}\n").expect("write a.rs");
        repo.git(&["add", "-A"]);
        repo.git(&["commit", "-qm", "base"]);

        if !debt.is_empty() {
            let mut body = std::fs::read_to_string(repo.path("a.rs")).expect("read a.rs");
            body.push_str(debt);
            std::fs::write(repo.path("a.rs"), body).expect("append debt");
        }
        repo
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }

    fn git(&self, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            .output()
            .expect(
                "the `todos` runtime requires `git`; it must be on PATH to run the test suite",
            );
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .args(args)
            .current_dir(&self.dir)
            // Isolate from the developer's environment.
            .env_remove("TERM")
            .output()
            .expect("spawn the todos binary")
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).expect("STDOUT is UTF-8")
}

fn stderr(o: &Output) -> String {
    String::from_utf8(o.stderr.clone()).expect("STDERR is UTF-8")
}

fn parse_array(s: &str) -> Vec<Value> {
    let v: Value = serde_json::from_str(s)
        .unwrap_or_else(|e| panic!("STDOUT is not valid JSON ({e}): {s:?}"));
    match v {
        Value::Array(a) => a,
        other => panic!("STDOUT is not a JSON array: {other:?}"),
    }
}

const DEBT: &str = "fn b() {}\n// TODO: new debt here\n";

#[test]
fn check_json_emits_array_on_stdout() {
    let repo = Repo::with_added_debt("flagged", DEBT);
    let out = repo.run(&["check", "--base", "HEAD", "--json"]);

    assert_eq!(out.status.code(), Some(1), "STDERR: {}", stderr(&out));

    let arr = parse_array(&stdout(&out));
    assert_eq!(arr.len(), 1, "STDOUT: {arr:?}");
    assert_eq!(
        arr[0],
        serde_json::json!({
            "file": "a.rs",
            "line": 3,
            "col": 4,
            "tag": "TODO",
            "text": "new debt here",
        })
    );

    // One array, one trailing newline.
    let s = stdout(&out);
    assert!(s.ends_with("]\n"), "STDOUT must end with a single newline: {s:?}");
    assert_eq!(s.lines().count(), 1, "STDOUT must be exactly one array: {s:?}");

    // Diagnostics are still emitted — on STDERR only.
    let err = stderr(&out);
    assert!(err.contains("a.rs:3:4 TODO new debt here"), "STDERR: {err:?}");
    assert!(err.contains("todos: 1 new marker(s)"), "STDERR: {err:?}");
    assert!(
        !s.contains("todos:"),
        "a diagnostic leaked to STDOUT: {s:?}"
    );
}

#[test]
fn check_json_clean_diff_emits_empty_array() {
    let repo = Repo::with_added_debt("clean", "");
    let out = repo.run(&["check", "--base", "HEAD", "--json"]);

    assert_eq!(out.status.code(), Some(0), "STDERR: {}", stderr(&out));
    assert_eq!(stdout(&out), "[]\n");
    assert!(stderr(&out).contains("todos: no markers found in diff"));
}

#[test]
fn check_json_matches_list_json_shape() {
    let repo = Repo::with_added_debt("parity", DEBT);

    let check = repo.run(&["check", "--base", "HEAD", "--json"]);
    let list = repo.run(&["--json"]);

    assert_eq!(check.status.code(), Some(1), "STDERR: {}", stderr(&check));
    assert_eq!(list.status.code(), Some(0), "STDERR: {}", stderr(&list));

    // Same element shape *and* same order: `check --json` and `todos --json`
    // are interchangeable for a consumer.
    assert_eq!(parse_array(&stdout(&check)), parse_array(&stdout(&list)));
}

#[test]
fn check_json_flag_before_subcommand_still_works() {
    // The legacy position keeps working; previously it parsed and did nothing.
    let repo = Repo::with_added_debt("legacy-position", DEBT);
    let out = repo.run(&["--json", "check", "--base", "HEAD"]);

    assert_eq!(out.status.code(), Some(1), "STDERR: {}", stderr(&out));
    assert_eq!(parse_array(&stdout(&out)).len(), 1);
}

#[test]
fn check_json_all_warning_stays_on_stderr() {
    let repo = Repo::with_added_debt("all", DEBT);
    let out = repo.run(&["check", "--all", "--json"]);

    assert_eq!(out.status.code(), Some(1), "STDERR: {}", stderr(&out));
    assert_eq!(parse_array(&stdout(&out)).len(), 1);

    let err = stderr(&out);
    assert!(
        err.contains("todos: --all compares the whole tree; existing debt will block."),
        "STDERR: {err:?}"
    );
    assert!(
        !stdout(&out).contains("--all"),
        "the --all warning leaked to STDOUT: {:?}",
        stdout(&out)
    );
}

#[test]
fn check_json_degraded_mode_stays_on_stderr() {
    let repo = Repo::with_added_debt("degraded", DEBT);
    let out = repo.run(&["check", "--base", "no-such-ref-xyz", "--json"]);

    assert_eq!(out.status.code(), Some(1), "STDERR: {}", stderr(&out));
    assert_eq!(parse_array(&stdout(&out)).len(), 1);

    let err = stderr(&out);
    assert!(err.contains("todos: cannot compare against no-such-ref-xyz:"), "STDERR: {err:?}");
    assert!(
        err.contains("todos: falling back to reporting all markers"),
        "STDERR: {err:?}"
    );

    let s = stdout(&out);
    assert!(!s.contains("cannot compare"), "degradation notice leaked to STDOUT: {s:?}");
    assert!(!s.contains("falling back"), "degradation notice leaked to STDOUT: {s:?}");
}

#[test]
fn check_without_json_stdout_stays_empty() {
    // AC4 guard: the human path is untouched — STDOUT empty, human output on
    // STDERR, exit 1.
    let repo = Repo::with_added_debt("human", DEBT);
    let out = repo.run(&["check", "--base", "HEAD"]);

    assert_eq!(out.status.code(), Some(1), "STDERR: {}", stderr(&out));
    assert_eq!(stdout(&out), "", "human mode must not write to STDOUT");
    assert_eq!(
        stderr(&out),
        "a.rs:3:4 TODO new debt here\n\ntodos: 1 new marker(s)\n"
    );
}

#[test]
fn list_json_is_unaffected_by_global_flag() {
    // `global = true` must not disturb the root-level list path.
    let repo = Repo::with_added_debt("list", DEBT);
    let out = repo.run(&["--json"]);

    assert_eq!(out.status.code(), Some(0), "STDERR: {}", stderr(&out));
    assert_eq!(parse_array(&stdout(&out)).len(), 1);
    assert_eq!(stderr(&out), "");
}
