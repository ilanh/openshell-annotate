# Contributing

Thanks for helping. openshell-annotate is an independent community project (not affiliated with NVIDIA).

## Workflow: branches and pull requests only

`main` is protected. **Never commit or push to `main` directly**; every change, including docs and one-line fixes,
goes through a branch and a pull request.

1. Branch from an up-to-date `main`, named by type: `feat/<name>`, `fix/<name>`, `docs/<name>`, `chore/<name>`
   (`test/<name>` for test-only work).
2. Commit with a DCO sign-off: `git commit -s -m "feat: what and why"` (conventional prefixes, small commits).
3. Run `scripts/ci-local.sh` before pushing (`TOOLCHAIN=1.94 scripts/ci-local.sh` pins the toolchain CI uses).
4. `git push -u origin <branch>` and open a pull request into `main`.
5. CI must pass (the `check` and `audit` jobs), the branch must be up to date with `main`, and all review
   conversations must be resolved.
6. Merge with **squash merge** (the only merge method enabled; the PR title becomes the commit message, so use a
   conventional title). The head branch is deleted automatically after the merge.

The rules are defined in `.github/rulesets/protect-main.json` and applied with `scripts/protect-main.sh`: pull request
required (0 approvals, conversations resolved), status checks `check` and `audit` required and up to date, linear
history, no force pushes, no deletion, and no bypass for admins. GitHub only enforces rulesets on private repositories
with a paid plan; until then the rules are followed by convention.

## Before you open a pull request

- Open an issue first for anything beyond a small fix, so we agree on the direction.
- Add a test for every new directive, attribute, lint pattern or error message (`tests/cli.rs`; fixtures in
  `tests/fixtures/`). If generated output changes, review `tests/fixtures/sample/policy.expected.yaml` by hand.
- Never emit a field that the pinned `openshell-policy-schema` rejects.
- No secrets in code, fixtures or test output.

## Checks (CI runs the same)

Run them all with `scripts/ci-local.sh`, or one by one:

```
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
scripts/gen-docs.sh --check      # regenerate with scripts/gen-docs.sh when directives or flags change
```

## Developer Certificate of Origin (DCO)

Every commit must be signed off, certifying that you wrote the change or have the right to submit it under the
project's Apache-2.0 license (https://developercertificate.org/):

```
git commit -s -m "your message"      # adds: Signed-off-by: Your Name <you@example.com>
```

To fix a missing sign-off: `git commit --amend -s` (last commit) or `git rebase --signoff main`.
The sign-off name and email must match your commit author.

## License

By contributing you agree that your contributions are licensed under the Apache License 2.0 (see `LICENSE`).
