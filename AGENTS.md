# AGENTS.md

This repo is `openshell-annotate`: a Rust CLI that turns `@openshell` source comments into an OpenShell
`policy.yaml`.

## When a user wants a policy for *their* codebase
Follow `skills/annotate-openshell-policy/SKILL.md` (scan code for network calls, file writes, secrets and
subprocesses; add directives with reasons; run `scan`, `generate`, `lint`, `drift`; after a sandbox run feed the logs to
`diff-logs`).

## When changing this tool
- Build/test: `cargo build && cargo test` (Rust 1.94+). The schema crate is a git dependency pinned to the
  OpenShell `v0.1.6` tag (see `Cargo.toml`, `Cargo.lock` pins the commit). Update the tag deliberately and re-run
  the golden tests.
- Never emit a field the pinned `openshell-policy-schema` rejects. `generate` re-parses its own output with
  `parse_policy`; tests assert this.
- Generated YAML is written as text (comments with `file:line`) because `serialize_policy` drops comments.
- No secrets anywhere (directives, reasons, fixtures, providers output). Providers use the bare
  `--credential KEY` form so values come from the caller's environment.
- Add a test for every new directive, attribute, lint pattern or error message. Fixtures live in
  `tests/fixtures/sample`; the golden `policy.expected.yaml` is intentionally reviewed by hand when it changes.
- Lint call-site detection lives in `src/ast.rs` (tree-sitter: one handler per language) and `src/lint.rs` (coverage, fs path
  policy, summaries); the regex engine is kept behind `[lint] engine = "regex"`. New patterns need a case in
  `tests/cli.rs` and in a fixtures repo for sandbox-backed checks.
- `diff-logs` (`src/diff_logs.rs`) parses shorthand and OCSF JSON; keep both formats in `tests/fixtures/logs`.
- Human output: `src/report.rs` renders lint; `src/render.rs` renders `scan`, `diff-logs` and the `generate` summary in the same
  style (shared `Style`, icons, `--format/--color/--no-icons`). `--format compact` and `--format json` must stay byte-stable
  and `generate` must never put anything but the policy on stdout. Snapshots live in `tests/golden` (`UPDATE_GOLDEN=1 cargo test`).
