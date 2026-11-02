# openshell-annotate roadmap

Order matters: each milestone builds on the previous one. Nothing is published (no releases, no registry images) until the maintainer decides to.

| # | Milestone | Status |
|---|---|---|
| 1 | Auto-generated docs | done 2026-11-01 |
| 2 | Docker image | image builds from the public repo (openshell-annotate:dev, ~35 MB); registry publish pending |
| 3 | GitHub Action + GitLab CI template | next |
| 4 | AI agent skill hardening and testing | blocked on logins |
| 5 | VS Code extension | last |

## 1. Auto-generated docs
- **Scope:** reference docs derived from the code, never hand-copied.
- **Deliverables:** `openshell-annotate docs [--out DIR] [--check]`; `docs/CLI.md` (clap help of every subcommand); `docs/DIRECTIVES.md` (from `src/spec.rs`, the table the parser also uses); `scripts/gen-docs.sh`; test `tests/docs.rs` fails when the committed docs are stale.
- **Depends on:** nothing. Every later milestone links to these docs.

## 2. Docker image (publish)
- **Scope:** small runtime image with the binary (tree-sitter grammars compiled in; distroless/cc base, 34.9 MB), entrypoint `openshell-annotate`, workdir `/src`.
- **Deliverables:** multi-stage `Dockerfile`, `scripts/docker-build.sh` (local tag `openshell-annotate:dev`, no OpenShell checkout needed), then, after approval, a publish workflow to a registry (ghcr.io) with version tags.
- **Depends on:** a public home for the repo (the `openshell-policy-schema` dependency now resolves from the public OpenShell repo at tag v0.1.6, so the build is portable). Publishing to a registry needs the maintainer's approval.

## 3. GitHub Action + GitLab CI template
- **Scope:** run `lint --deny-warnings` and `drift --policy openshell/policy.yaml` on every pull/merge request; fail with annotations.
- **Deliverables:** composite GitHub Action (`action.yml`, uses the Docker image or a release binary), `.gitlab-ci.yml` include template, an example workflow, docs page, a fixture repo run in CI as self-test. Optional: sticky PR comment with the drift diff and `diff-logs` suggestions.
- **Depends on:** (1) docs to link from, (2) the Docker image.

## 4. AI agent skill hardening and testing
- **Scope:** make `skills/annotate-openshell-policy/SKILL.md` reliable for Claude Code and Cursor: scan code, add directives, loop on `lint` until clean.
- **Deliverables:** skill reads the generated `docs/DIRECTIVES.md` instead of duplicating grammar; an eval script that runs the agent on a demo repo and the fixtures and scores `lint`/`drift` results; install instructions for each agent (`AGENTS.md`, Claude skill dir, Cursor rules).
- **Depends on:** (1) docs. **Blockers:** needs working `claude` and `cursor-agent` logins for the agent evals.

## 5. VS Code extension
- **Scope:** editor support for directives.
- **Deliverables:** TextMate grammar highlighting `@openshell` comments, diagnostics from `lint --json`, quick-fix to insert a suggested directive, command to preview the generated `policy.yaml`. Thin client over the CLI (binary path setting or the Docker image).
- **Depends on:** stable `--json` output of `lint`/`scan`/`generate` (add a schema doc), (2) for distribution. Marketplace publishing needs approval.

## Ongoing: upstream compatibility
- **Scope:** know early when OpenShell changes its policy schema or syntax.
- **Done:** `scripts/upstream-compat.sh` and the weekly `upstream-compat` workflow build and test against OpenShell's newest release and `main` (informational; failures open an `upstream-compat` issue).
- **Later:** bump the pinned tag when a new release is compatible; optionally add a prerelease target.
