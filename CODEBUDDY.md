# CODEBUDDY.md

This file provides guidance to CodeBuddy Code when working with code in this repository.

## Commands

```sh
cargo test                    # all tests (unit tests only, no integration suite)
cargo test lexer::            # one module
cargo test block_comment      # one test by name substring
cargo clippy --all-targets    # must stay warning-free
cargo build --release         # single static binary at target/release/todos
```

Manual behaviour checks (there is no integration test harness; these are the
scenarios that matter):

```sh
./target/release/todos .                              # human table, TTY only
./target/release/todos . | cat                        # piped: path:line:col tag text
./target/release/todos check --base HEAD --staged     # pre-commit gate
./target/release/todos check --base origin/main       # CI gate
```

## Architecture

A single Rust binary in five layers, each with one job. Data flows strictly
downward: discovery → L0 filter → L1 lexer → model → diff/report.

**`src/discover.rs`** — *what to read.* Inside a repo this shells out once to
`git ls-files -z --cached --others --exclude-standard` and never re-derives
gitignore semantics (negation, dir-only patterns, `.git/info/exclude`, global
excludes). Outside a repo, `Mode::Walk` does a plain recursive walk and the
caller discloses the degraded mode to the user — "not a git repository" is
never an error.

**`src/filter.rs`** — *L0, which files are worth opening.* Pure, no I/O:
extension allowlist, vendored-path rejection, minified-suffix rejection, NUL
byte detection, and a max-line-length check for minified files. Rejects
unknown extensions rather than guessing, because the lexer needs a syntax
family.

**`src/lexer.rs`** — *L1, the load-bearing layer.* This is the entire moat and
the riskiest code. It classifies bytes as code/string/comment and carries
in-comment state across lines, so block comments work. It is **not a parser**:
the only question asked is "am I inside a comment, and where does it start".
Comment regions are `(comment_start, content_start)` pairs, where
`content_start` is exactly where the comment text begins — the caller never
re-derives whether it is looking at `<!--`, `-->`, `/*`, `//`, `#`, or `--`.

Two invariants to preserve when editing:

- A block comment that continues onto a line yields a region starting at offset
  `0` for that whole line, so the star-banner style (`/*\n * TODO: x\n */`)
  reports once, on the line carrying the tag.
- A hyphen is **not** a tag boundary, so `TODO-family` and `TODO-tracking` do
  not match. A plural `TODOS` does not match either.

**`src/model.rs`** — the `Marker` type and its comparison key. The key is
`(file, tag, text)` and deliberately excludes line and column: a marker that
merely shifted is not new debt. Include the path, which is why a rename reads
as delete+add (documented limitation).

**`src/scan.rs`** — parallel driver. Threads pull from a shared atomic index
rather than taking pre-computed chunks, because file sizes vary enormously.
Results are sorted deterministically by `(file, line, col)` regardless of
thread scheduling; output stability is a testability requirement.

**`src/diff.rs`** — the gate. `new_markers` scans the baseline **only for files
that currently contain a marker**, streaming each base blob through
`git cat-file --batch` in a single process. This scoping is what makes the
pre-commit hook fast: an earlier implementation that archived and re-scanned
the whole base tree took 11–18s on a 100k-file repo; the scoped version takes
0.12s. Do not reintroduce a whole-tree baseline scan.

`staged_paths` uses `git diff --cached --name-only --diff-filter=ACMR`, so
unstaged work can never block a commit.

**`src/report.rs`** — two shapes only. `human` groups by file with a
breakdown footer; piped output is `path:line:col tag text`. `is_interactive`
is the only place terminal detection happens.

## Design constraints that must not be violated

- **The gate defaults to new markers only.** `--all` is opt-in and warns. A
  gate on the whole tree trains developers to use `--no-verify`, which
  disables every other hook with it.
- **No author attribution, ever.** Blame authors are frequently not the owner,
  and surfacing names invites blame-the-author culture.
- **No stable IDs in v0.** "New" is a set difference, not an object match. Do
  not half-implement fuzzy matching.
- **Config is never required.** Default behaviour in an unconfigured repo must
  be identical to a configured one.

## Testing notes

Unit tests live beside their modules. The lexer tests encode the specific false
positives measured against real repos — `interface Todo`, a regex containing
`todo`, i18n strings, URLs containing `TODO` — so deleting one of those tests
is how the moat silently disappears. When adding a language family, add a test
per false-positive class, not one happy path.
