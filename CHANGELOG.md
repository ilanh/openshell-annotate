# Changelog

All notable changes to openshell-annotate. `scripts/release.sh <version>` moves the notes under "Unreleased" into a new
version section and adds the squash-merged PR titles since the previous tag. Versions follow [SemVer](https://semver.org).

## [Unreleased]

- ci: the `openshell-next` workflow also runs at 07:17 UTC as a backup for the 04:17 UTC schedule, because GitHub may delay or drop a scheduled run; both slots share one concurrency group and no-op when the pin is unchanged
- ci: `openshell-next` branch automation: a daily workflow merges main into `openshell-next`, moves its OpenShell pin to the newest upstream tag (prereleases included), runs the tests and reports on the tracking issue; `scripts/openshell-tags.sh` and `scripts/openshell-next.sh` do the same by hand
- fix(ci): CI runs dispatched by the `openshell-next` workflow publish `CI (dispatched) / check|audit` commit statuses, so the tracking PR shows their result while its own `pull_request` runs wait for approval (GitHub's rule for `GITHUB_TOKEN` pushes)
- fix(ci): the `openshell-next` workflow runs main's copy of its scripts against the branch (`OPENSHELL_ANNOTATE_ROOT`), runs steps with `pipefail` so merge/bump/test failures fail the run, and its issue comment shows the pin and every failed step
- fix(ci): `scripts/upstream-compat.sh` repoints both OpenShell crates (it left `openshell-policy` on the old tag) and runs on macOS; `upstream-compat.yml` also runs, non-blocking, on every push to main
- chore(audit): ignore RUSTSEC-2026-0459 (`libsqlite3-sys` 0.37.0, pinned by OpenShell v0.1.7 through `sqlx` 0.9.0) in `.cargo/audit.toml` until OpenShell upgrades it (#76)
- chore(deps): build against OpenShell v0.1.7 (`openshell-policy-schema`, `openshell-policy`); docs and generated policy headers now name v0.1.7
- docs(actions): `docs/GITHUB_ACTIONS.md` is now a setup and usage guide (versions, recipes, troubleshooting); its inputs/outputs tables are generated from `action.yml` by `scripts/gen-actions-docs.sh` and checked in CI
- fix(diff-logs): ignore OCSF Network Activity Refuse records (policy DNS `policy_dns_ineligible`) that have no port, so OCSF JSON and shorthand logs report the same denials

## [0.1.0-rc.3] - 2026-11-09

- fix(generate): MCP endpoints emit `mcp.allow_all_known_mcp_methods` instead of an access preset
- fix(generate): emit `operation_type` only on GraphQL rules, and only as query, mutation, or subscription
- fix(generate): reject `rest`, `websocket`, and `graphql` directives that set neither rules nor an access preset
- fix(lint): check `lint --policy`, `drift`, and generated policies with the OpenShell gateway validator

Merged pull requests since v0.1.0-rc.2:

- feat(actions): shared GitHub Actions for CI/CD (setup, drift, lint, generate, diff-logs, check) (#62)
- fix(lint): check policies with the OpenShell gateway validator (#59)
- fix(generate): reject L7 protocols that name neither rules nor access (#58)
- fix(generate): emit GraphQL operation_type only for GraphQL rules (#60)
- fix(generate): emit MCP allow-all instead of an access preset (#56)
- docs: add examples/README.md for the polyglot example (#51)

## [0.1.0-rc.2] - 2026-11-04

Merged pull requests since v0.1.0-rc.1:

- fix(drift): label strict unmanaged sections by section, network_middlewares is live (#48)
- docs: rustdoc for modules, functions and types in src/ (#46)
- fix(generate): join endpoints of one host whose port lists overlap (#42)
- fix(scan): match the not-scanned note to the language comment style (#44)
- fix(scan): show process user/group and ignore kind= after the verb (#43)
- docs(examples): add polyglot sample project (#41)
- fix(providers): name the source of a missing profiles directory and hint how to fix it (#36)

## [0.1.0-rc.1] - 2026-11-03

Merged pull requests:

- test: keep the golden policy independent of the crate version (#35)
- chore(release): tag-driven release workflow, packaging and installer (#33)
- feat(generate): propose fs read for image directories outside the baseline (--image) (#32)
- feat(lint): stop reporting URL literals in docstrings, messages and help text (#31)
- feat(generate): scaffold provider profile skeletons for unmapped credentials (#30)
- fix(providers): relocatable profile paths in providers.sh; note prover limit for tls=skip (#29)
- feat(providers): [providers] expect for dynamically looked-up credentials (#28)
- fix(generate): wrap long reasons instead of truncating them (#27)
- fix(diff-logs): find the repo root from subdirectories and keep probe scripts out of placement (#26)
- fix(scan): exclude patterns work with ./ paths, dead and regex patterns are reported (#25)
- feat(diff-logs): report connections that are allowed and then fail as TLS trust problems (#24)
- fix(lint): suggest one egress directive per host; fix invalid skill example (#22)
- feat(scan): read @openshell comments in Ruby, Rust, Java and more; report unscanned languages (#21)
- chore: issue templates, PR template and labels script (#6)
- chore(ci): test against newest OpenShell prerelease (#5)
- chore(ci): weekly upstream-compat build against OpenShell latest release and main (#4)
- feat: readable output for scan, diff-logs and generate (#3)
- feat(lint): readable multi-line output with context and category icons (#2)
- docs: developer guide, contributing workflow, clippy 1.95 fixes (#1)
