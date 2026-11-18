# todos

> [!WARNING]
> **Public sandbox — do not push sensitive data.**
> This repository exists only to exercise a development loop (LoopForge)
> against a minimal app. Everything committed here is world-readable and
> stays in git history forever.
>
> No API keys, tokens, passwords, private keys, `.env` files, customer data,
> or internal hostnames — not in a fixture, not in a "temporary" commit, not
> in a deleted-and-force-pushed one. Anything committed here is public
> permanently; deleting it later does not undo the exposure.
>
> Treat every secret as already leaked the moment it reaches a commit.

Find `TODO`-family markers in source comments — and gate on the ones your diff
adds.

Two things make this worth a binary rather than `rg TODO`:

1. **Comment-awareness.** A marker inside a string literal is not a marker. Every
   byte is classified as code, string, or comment before any tag is looked for,
   so `interface Todo`, `/...todo/`, and i18n strings do not match.
2. **Diff-scoped gating.** `todos check` asks only that the lines you just wrote
   don't add debt. Existing debt is invisible. That is the only rule a developer
   accepts on their first commit — blocking on a mature repo's backlog is a
   one-command uninstall and a permanent `--no-verify` habit.

## Usage

```sh
todos                  # list markers in the tree, grouped by file
todos . --json         # machine-readable array
todos stats            # counts by tag, plus files scanned
todos check --staged   # pre-commit: fail only on markers this commit adds
todos check --base origin/main   # CI: fail only on markers this branch adds
todos check --all      # opt-in: block on every marker (warns; existing debt blocks)
```

Output is `path:line:col tag text` when piped, so it feeds `grep -n` consumers,
`vim -q`, and editor problem matchers.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | clean, or nothing new |
| 1 | markers found that policy flags |
| 2 | usage or configuration error |

## Configuration

Defaults are `TODO`, `FIXME`, `HACK`, `XXX`, matched case-sensitively, inside
known comment syntax. No configuration is required — the tool behaves
identically in a configured and an unconfigured repo.

```sh
todos --tag SECURITY --tag TODO   # replace the tag set for one run
```

## Known limitations

These are deliberate v0 trade-offs, not oversights:

- **A rename reads as delete+add**, so every marker in a moved file looks new.
  Git rename detection is the fix. Do not gate on renames until then.
- **Tripled Python strings** and JS/TS template literals are local approximations,
  not exact. The lexer tracks quote state but does not understand the full
  grammar; tree-sitter is the answer for the two or three worst languages.
- **Doc comments that quote the syntax itself** are reported. A `///` line
  containing `TODO: x` as an example is genuinely comment text, so it matches.
- **Markdown and fixture files** are not scanned; `.md` is not in the comment
  family table by design.
- **No stable IDs.** "New" is a set difference over `(path, tag, text)`, so a
  marker that merely shifted lines is not new, but one whose text was edited is
  new-plus-gone.

## Development

```sh
cargo test                       # unit tests
cargo test lexer::               # one module
cargo test tag_boundary          # one test
cargo clippy --all-targets       # must be warning-free
cargo build --release            # single static binary
```

Requires a Rust toolchain (edition 2021). Runtime requires `git`; outside a repo
the tool degrades to a plain walk and says so.

## Contributing to this sandbox

`.gitignore` already drops `target/` and common secret files (`.env`, keys,
certs). It is a safety net, not a permission slip — check `git diff --cached`
before every push, especially when a script or agent generated the changes.
