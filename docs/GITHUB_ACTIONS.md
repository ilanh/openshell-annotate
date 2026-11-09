# GitHub Actions

Composite actions under [`actions/`](../actions) install `openshell-annotate` and run the usual policy gates in another
repository's CI: `drift` (the committed policy still matches the `@openshell` directives), `lint` (no unannotated calls),
`generate` (rebuild the policy, for example for an image build) and `diff-logs` (compare a sandbox run's logs with the
directives). This page covers setup, choosing a version, every action's inputs and outputs, common recipes and
troubleshooting.

## Prerequisites

- A repository with `@openshell` directives and a committed policy, usually `openshell/policy.yaml` (plus
  `openshell/providers.json` if you use providers). Create them locally first:
  `openshell-annotate generate -o openshell/policy.yaml --providers-out openshell/providers.json`.
  See the [README](../README.md) and [DIRECTIVES.md](DIRECTIVES.md).
- A GitHub-hosted `ubuntu-*` or `macos-*` runner (linux or darwin, x86_64 or arm64). Self-hosted runners need `bash`,
  `curl` and `jq`, and `cargo` 1.94+ (or `rustup`) for `version: source`.
- Workflow permissions `contents: read`. The actions need no secrets.
- Optional: the GitHub CLI on the runner for `verify-attestation: "true"` (preinstalled on GitHub-hosted runners).

## Quick start

Add `.github/workflows/policy.yml`:

```yaml
name: OpenShell policy

on:
  pull_request:
  push:
    branches: [main]

permissions:
  contents: read

jobs:
  policy:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      - uses: ilanh/openshell-annotate/actions/setup@v0.1.0-rc.3
      - uses: ilanh/openshell-annotate/actions/drift@v0.1.0-rc.3
        with:
          policy: openshell/policy.yaml
          providers: openshell/providers.json   # drop if you have no providers file
      - uses: ilanh/openshell-annotate/actions/lint@v0.1.0-rc.3
        with:
          policy: openshell/policy.yaml
          failure-threshold: warning
```

The job fails when the committed policy has drifted from the directives or when lint finds a warning or error. Findings
appear as annotations on the pull request and as tables in the job summary.

On refs that include [#66](https://github.com/ilanh/openshell-annotate/pull/66) (`main` today, and the next release
after `v0.1.0-rc.3`), the single `check` action does all three steps:

```yaml
      - uses: ilanh/openshell-annotate/actions/check@main   # or a release tag newer than v0.1.0-rc.3
        with:
          version: source
          policy: openshell/policy.yaml
          providers: openshell/providers.json
          failure-threshold: warning
```

Do not use `actions/check@v0.1.0-rc.3` from another repository: that release still has the relative `uses:` bug
([#65](https://github.com/ilanh/openshell-annotate/issues/65)). Use `setup` + `drift` + `lint` at that tag instead.

## Choosing a version

Two settings decide what runs:

- **The `@ref` on `uses:`** picks the action code (the YAML and the scripts in `actions/lib`). Use the same ref on every
  `uses:` line in a workflow.
- **The `version` input** of `setup` and `check` picks the CLI binary. It defaults to the release the actions were cut
  with (`v0.1.0-rc.3`).

| Goal | `uses:` ref | `version` | Notes |
| --- | --- | --- | --- |
| Reproducible CI (recommended) | a release tag, e.g. `@v0.1.0-rc.3`, or that tag's commit SHA | default, or the same tag | Nothing changes until you bump the ref. |
| Track development | `@main` | `source` | Builds the CLI from the same commit as the actions. Adds about 2–3 minutes per job (no cargo cache) and breaks when `main` breaks, which is the point for an early-warning repo. |
| Newest stable release | a release tag | `latest` | `latest` resolves the newest **stable** release only. Every release so far is a `-rc` prerelease, so `latest` currently fails; pin a tag. |

`version: source` builds the openshell-annotate sources that ship with the action at its `@ref`, never the caller's
checkout. A prerelease tag (`v0.1.0-rc.3`) must be given explicitly; see
[RELEASING.md](RELEASING.md#installing) for how releases and `latest` work.

### Pinning and permissions

- Set `permissions: contents: read` at the workflow or job level. The actions only read the repository and download
  public release assets. `verify-attestation` uses the default `GITHUB_TOKEN` through the GitHub CLI.
- Pin third-party actions to a full commit SHA with the version as a comment, as the examples here do
  (`actions/checkout@3d3c42e… # v7.0.1`). For the strictest setup, pin `ilanh/openshell-annotate/actions/*` to a commit
  SHA too; Dependabot's `github-actions` ecosystem keeps SHA pins and their comments up to date.
- The actions pin their own dependencies (`actions/cache`, `actions/upload-artifact`) to SHAs.
- Avoid `@main` in production gates; it moves with every merge.

## Actions

| Action | Purpose |
| --- | --- |
| [`setup`](../actions/setup/action.yml) | Install the CLI (release download or `source` build) and add it to `PATH` |
| [`check`](../actions/check/action.yml) | `setup` + `drift` + `lint` in one step (the common PR gate) |
| [`drift`](../actions/drift/action.yml) | Fail when the committed policy no longer matches the directives |
| [`lint`](../actions/lint/action.yml) | Lint directives and call sites; annotations plus a job summary |
| [`generate`](../actions/generate/action.yml) | Regenerate the policy (and providers); optionally upload them as an artifact |
| [`diff-logs`](../actions/diff-logs/action.yml) | Compare sandbox logs with the directives and report gaps |

`drift`, `lint`, `generate` and `diff-logs` expect `openshell-annotate` on `PATH`, so run `setup` first in the same job.
All paths (`policy`, `providers`, `policy-out`, `logs`, …) are relative to `root`, because the steps run with `root` as
the working directory.

The tables below are generated from each `action.yml` by `scripts/gen-actions-docs.sh`; do not edit them by hand.

### `setup`

Downloads a release with [`install.sh`](../install.sh) (checksum-verified, retried on network errors) and caches it, or
builds from source.

<!-- BEGIN actions-docs setup -->

| Input | Required | Default | Description |
| --- | --- | --- | --- |
| `version` | no | `v0.1.0-rc.3` | Release tag to install (e.g. v0.1.0-rc.3), `latest` (newest stable release only), or `source` to build the openshell-annotate sources at the action's own ref with cargo (Rust 1.94+). |
| `install-dir` | no | _(empty)_ | Directory for the openshell-annotate binary (added to PATH); empty means `$RUNNER_TOOL_CACHE/openshell-annotate`. |
| `verify-attestation` | no | `false` | Run `gh attestation verify` after download (needs GitHub CLI + contents:read). |
| `cache` | no | `true` | Cache the installed binary between runs (keyed on OS, arch and `version`; not used for `source`). |

| Output | Description |
| --- | --- |
| `version` | Installed openshell-annotate version string (from `openshell-annotate --version`). |
| `install-dir` | Directory containing the `openshell-annotate` binary. |

<!-- END actions-docs setup -->

### `check`

Runs `setup`, then `drift`, then `lint`, with the same behaviour as the separate actions. It always caches release
downloads and has no `paths` input; use the separate actions when you need those.

<!-- BEGIN actions-docs check -->

| Input | Required | Default | Description |
| --- | --- | --- | --- |
| `version` | no | `v0.1.0-rc.3` | CLI version, as for `setup` (a release tag, `latest` for the newest stable release, or `source` to build the action's own ref). |
| `root` | no | `.` | Repository root for drift and lint. |
| `policy` | yes | — | Committed policy.yaml. |
| `providers` | no | _(empty)_ | Optional committed providers JSON for drift. |
| `profiles-dir` | no | _(empty)_ | Provider profiles directory for drift (`--profiles-dir`). |
| `failure-threshold` | no | `error` | Lint failure threshold (`error`, `warning`, or `never`). |
| `strict-drift` | no | `false` | Pass `--strict` to drift. |

| Output | Description |
| --- | --- |
| `version` | Installed openshell-annotate version. |

<!-- END actions-docs check -->

### `drift`

Runs `openshell-annotate drift --json` and writes the findings (change, section, item, detail) to the job summary. Exits
non-zero on drift; fix it by regenerating the policy and committing it.

<!-- BEGIN actions-docs drift -->

| Input | Required | Default | Description |
| --- | --- | --- | --- |
| `root` | no | `.` | Repository root passed to openshell-annotate (`--root`). |
| `policy` | yes | — | Committed policy.yaml (`--policy`). |
| `providers` | no | _(empty)_ | Optional committed providers JSON (`--providers`). |
| `profiles-dir` | no | _(empty)_ | Provider profiles directory (`--profiles-dir`), when not set in config. |
| `strict` | no | `false` | Treat unmanaged committed sections as drift (`--strict`). |
| `paths` | no | _(empty)_ | Space-separated paths to scan (default entire root). |

<!-- END actions-docs drift -->

### `lint`

Runs `openshell-annotate lint --format json` and turns every finding into a `::error`, `::warning` or `::notice`
annotation with its file and line, plus a summary table by severity.

| `failure-threshold` | The step fails on |
| --- | --- |
| `error` (default) | errors |
| `warning` | errors or warnings (passes `--deny-warnings`) |
| `never` | nothing; findings are still annotated |

<!-- BEGIN actions-docs lint -->

| Input | Required | Default | Description |
| --- | --- | --- | --- |
| `root` | no | `.` | Repository root (`--root`). |
| `policy` | no | _(empty)_ | Policy file to sanity-check (`--policy`); leave empty to skip policy checks. |
| `paths` | no | _(empty)_ | Space-separated paths to lint (default entire root). |
| `failure-threshold` | no | `error` | When to fail the step — `error` (default), `warning` (also fail on warnings), or `never`. |

<!-- END actions-docs lint -->

### `generate`

Runs `openshell-annotate generate`, creating output directories as needed.

<!-- BEGIN actions-docs generate -->

| Input | Required | Default | Description |
| --- | --- | --- | --- |
| `root` | no | `.` | Repository root (`--root`). |
| `policy-out` | yes | — | Output path for policy.yaml (`-o`). |
| `providers-out` | no | _(empty)_ | Output path for providers JSON or shell script (`--providers-out`). |
| `profiles-dir` | no | _(empty)_ | Provider profiles directory (`--profiles-dir`). |
| `scaffold-profiles` | no | _(empty)_ | Directory for `--scaffold-profiles` (creates missing provider profile stubs). |
| `upload-artifact` | no | `false` | Upload policy and providers outputs as a workflow artifact. |
| `artifact-name` | no | `openshell-policy` | Artifact name when upload-artifact is true. |

| Output | Description |
| --- | --- |
| `policy-path` | Absolute path to the generated policy file. |
| `providers-path` | Absolute path to the generated providers file, if any. |

<!-- END actions-docs generate -->

### `diff-logs`

Runs `openshell-annotate diff-logs` on one or more logs (the shorthand from `openshell logs <sandbox> --source sandbox`,
or OCSF JSONL) and copies the report to the job summary. It exits 1 while denied connections have no directive.

<!-- BEGIN actions-docs diff-logs -->

| Input | Required | Default | Description |
| --- | --- | --- | --- |
| `root` | no | `.` | Repository root (`--root`). |
| `logs` | yes | — | Space-separated log file paths (shorthand or OCSF JSONL). |
| `write` | no | `false` | Insert suggested directives (`--write`). |
| `deny-stale` | no | `false` | Fail when logs show no traffic for a declared egress (`--deny-stale`). |

<!-- END actions-docs diff-logs -->

## Recipes

### PR gate with strict drift and providers

```yaml
      - uses: ilanh/openshell-annotate/actions/setup@v0.1.0-rc.3
      - uses: ilanh/openshell-annotate/actions/drift@v0.1.0-rc.3
        with:
          policy: openshell/policy.yaml
          providers: openshell/providers.json
          profiles-dir: openshell/profiles   # only if not set in openshell-annotate.toml
          strict: "true"                     # unmanaged sections in the committed policy count as drift
      - uses: ilanh/openshell-annotate/actions/lint@v0.1.0-rc.3
        with:
          policy: openshell/policy.yaml
          failure-threshold: error           # `warning` to also fail on warnings, `never` to only annotate
```

For a monorepo, run one job per service with `root: services/api` and the paths relative to it, or limit scanning with
`paths: "src lib"` (`drift` and `lint` only).

### Regenerate the policy and keep it as an artifact

```yaml
      - uses: ilanh/openshell-annotate/actions/setup@v0.1.0-rc.3
      - uses: ilanh/openshell-annotate/actions/generate@v0.1.0-rc.3
        id: policy
        with:
          policy-out: openshell/policy.yaml
          providers-out: openshell/providers.json
          upload-artifact: "true"
          artifact-name: openshell-policy
      - run: echo "policy at ${{ steps.policy.outputs.policy-path }}"
```

### Build an image with the generated policy

Regenerate right before `docker build` so the image always ships the policy that matches the code, and gate the build on
`drift` so a stale committed policy is caught in review:

```yaml
jobs:
  check:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      - uses: ilanh/openshell-annotate/actions/check@main
        with:
          version: source
          policy: openshell/policy.yaml
  image:
    needs: check
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      - uses: ilanh/openshell-annotate/actions/setup@main
        with:
          version: source
      - uses: ilanh/openshell-annotate/actions/generate@main
        with:
          policy-out: openshell/policy.yaml
          upload-artifact: "true"
      - run: docker build -t my-sandbox:ci .
```

### Check a sandbox run's logs

Capture the logs where the sandbox runs (`openshell logs <sandbox> --source sandbox > sandbox.log`, or the OCSF JSONL),
commit them or pass them in as an artifact, then:

```yaml
      - uses: ilanh/openshell-annotate/actions/setup@v0.1.0-rc.3
      - uses: ilanh/openshell-annotate/actions/diff-logs@v0.1.0-rc.3
        with:
          logs: logs/sandbox.log logs/sandbox.ocsf.jsonl
          deny-stale: "true"   # also fail on directives the logs never exercised
```

Add `continue-on-error: true` to report gaps without failing the job. `write: "true"` inserts suggested directives into
the runner's checkout only; it is meant for local runs, since the workflow does not commit anything.

## Real example

[ilanh/openspell-python-tester](https://github.com/ilanh/openspell-python-tester) (the Policy Playground) runs
[`.github/workflows/policy.yml`](https://github.com/ilanh/openspell-python-tester/blob/main/.github/workflows/policy.yml):
`check@main` with `version: source` and `failure-threshold: error` on every push and pull request (its deny examples
are unannotated on purpose, so lint warns but only errors fail), then a job that regenerates the policy with `generate`,
uploads it as `playground-policy` and builds the sandbox image.

## Troubleshooting

| Symptom | Cause and fix |
| --- | --- |
| `Can't find 'action.yml', 'action.yaml' or 'Dockerfile' under '…/actions/setup'` when using `check` | `check@v0.1.0-rc.3` and older nest `uses: ./actions/...`, which resolves against your repository ([#65](https://github.com/ilanh/openshell-annotate/issues/65)). Use `setup` + `drift` + `lint` at that tag, or a ref that includes [#66](https://github.com/ilanh/openshell-annotate/pull/66). |
| `could not resolve the latest release (is there a stable release yet? use --version)` | `version: latest` only finds stable releases and there are none yet. Pin a tag such as `v0.1.0-rc.3`. |
| `latest` keeps installing an old release | The cache key contains the literal `version`, so `latest` reuses the first cached binary. Set `cache: "false"` on `setup` (`check` always caches), or pin a tag. |
| Source build fails with a Rust version or edition error | `version: source` uses the runner's `cargo` when present and needs Rust 1.94+; otherwise it installs 1.94 with `rustup`. Update the toolchain on self-hosted runners. |
| `install.sh failed (attempt N/5)` or download errors | Release downloads retry with backoff (and `curl` retries each request). If all attempts fail, GitHub was unavailable; re-run the failed jobs. |
| `lint-report.sh requires jq` or an empty drift summary | Install `jq` on self-hosted runners. |
| `Policy drift detected` | Regenerate locally (`openshell-annotate generate -o openshell/policy.yaml`, plus `--providers-out` if you use providers), review and commit. |
| Lint passes locally but fails in CI | CI uses `failure-threshold: warning` → `--deny-warnings`; run `openshell-annotate lint --deny-warnings`. Also check that the CI `version` matches your local CLI. |

## How the actions are built and tested

The actions share their logic through scripts in [`actions/lib`](../actions/lib), called via `github.action_path`, so
they work from any repository and checkout path. `check` does not nest `uses: ./actions/...` steps, because inside a
composite action a relative `uses:` resolves against the caller's workspace.

[`.github/workflows/actions-selftest.yml`](../.github/workflows/actions-selftest.yml) runs on changes to `actions/`,
`install.sh` and these docs: a release install, `check`/`generate`/`diff-logs` on Ubuntu and macOS from source, an
expected drift failure, and a `consumer-check` job that runs `check` the way another repository does (from a
subdirectory checkout, for `source` and a release) to catch #65 regressions.

### Changing an action

1. Edit `actions/<name>/action.yml` (and `actions/lib/*.sh`).
2. Run `scripts/gen-actions-docs.sh` to refresh the tables above, and update the prose and recipes on this page.
   CI runs `scripts/gen-actions-docs.sh --check` and fails when an input or output is missing, renamed or stale here,
   or when an action has no section.
3. Note user-visible changes under `## [Unreleased]` in [CHANGELOG.md](../CHANGELOG.md).
4. A new release moves the default `version` and the tags in these docs (see [RELEASING.md](RELEASING.md)).
