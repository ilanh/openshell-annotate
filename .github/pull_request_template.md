## What and why

<!-- One or two sentences. Use a conventional-commit style title: feat:, fix:, docs:, chore:, test: -->

Closes #

<!-- One `Closes #N` line per issue so GitHub closes them on merge. Use `Refs #N` if the issue stays open. -->

## Checklist

- [ ] Branch is named `feat/`, `fix/`, `docs/`, `chore/` or `test/` and commits are signed off (`git commit -s`)
- [ ] `scripts/ci-local.sh` passes (`TOOLCHAIN=1.94 scripts/ci-local.sh`)
- [ ] Tests added or updated (`tests/cli.rs`, fixtures, golden files)
- [ ] Docs regenerated if directives or flags changed (`scripts/gen-docs.sh`)
- [ ] No secrets in code, fixtures or output
