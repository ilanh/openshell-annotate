# GitHub Actions

Reusable composite actions install a [release](https://github.com/ilanh/openshell-annotate/releases) of
`openshell-annotate` and run the usual CI gates (`drift`, `lint`, `generate`, `diff-logs`). They work on `ubuntu-*`
and `macos-*` runners (linux/darwin, x86_64 and arm64).

## Versioning

Pin the **same ref** for every action in a workflow, for example a release tag:

```yaml
uses: ilanh/openshell-annotate/actions/check@v0.1.0-rc.3
```

Tags track [releases](https://github.com/ilanh/openshell-annotate/releases). You may also pin a commit SHA for
maximum stability. The `setup` action’s `version` input is the **CLI** version (defaults to `v0.1.0-rc.3`; `latest` is
supported). That is independent of the `@ref` on `uses:` — keep the action ref on a tag/SHA you trust.

Do not rely on `@main` in production workflows; it moves with every merge.

## Quick start (drift + lint)

```yaml
permissions:
  contents: read

jobs:
  openshell:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v4.2.2
      - uses: ilanh/openshell-annotate/actions/check@v0.1.0-rc.3
        with:
          root: .
          policy: openshell/policy.yaml
          providers: openshell/providers.json
          failure-threshold: warning
```

## Actions

| Path | Purpose |
| --- | --- |
| [`actions/setup`](../actions/setup/action.yml) | Install CLI (release or `source`) and add to `PATH` |
| [`actions/drift`](../actions/drift/action.yml) | Fail when committed policy ≠ directives |
| [`actions/lint`](../actions/lint/action.yml) | Annotations + job summary from `lint --format json` |
| [`actions/generate`](../actions/generate/action.yml) | Regenerate policy / providers; optional artifact |
| [`actions/diff-logs`](../actions/diff-logs/action.yml) | Compare sandbox logs to directives |
| [`actions/check`](../actions/check/action.yml) | `setup` + `drift` + `lint` |

### `actions/setup`

| Input | Default | Description |
| --- | --- | --- |
| `version` | `v0.1.0-rc.3` | Release tag, `latest`, or `source` (build the action's own sources with cargo; Rust 1.94) |
| `install-dir` | `$RUNNER_TOOL_CACHE/openshell-annotate` | Install location |
| `verify-attestation` | `false` | `gh attestation verify` (needs GitHub CLI) |
| `cache` | `true` | Cache the binary between runs |

| Output | Description |
| --- | --- |
| `version` | `openshell-annotate --version` |
| `install-dir` | Directory on `PATH` |

### `actions/drift`

| Input | Required | Description |
| --- | --- | --- |
| `policy` | yes | Committed `policy.yaml` (relative to `root`) |
| `root` | no | Repository root (default `.`) |
| `providers` | no | Optional providers JSON |
| `profiles-dir` | no | `--profiles-dir` override |
| `strict` | no | `--strict` |
| `paths` | no | Space-separated scan paths |

### `actions/lint`

| Input | Default | Description |
| --- | --- | --- |
| `root` | `.` | Repository root |
| `policy` | _(empty)_ | Policy file for sanity checks |
| `paths` | _(empty)_ | Space-separated paths |
| `failure-threshold` | `error` | `error`, `warning` (`--deny-warnings`), or `never` |

Findings use GitHub workflow commands (`::error` / `::warning` / `::notice` with `file` and `line`) and a Markdown job
summary table.

### `actions/generate`

| Input | Required | Description |
| --- | --- | --- |
| `policy-out` | yes | `-o` path (relative to `root`) |
| `root` | no | Repository root |
| `providers-out` | no | `--providers-out` |
| `profiles-dir` | no | `--profiles-dir` |
| `scaffold-profiles` | no | `--scaffold-profiles` |
| `upload-artifact` | `false` | Upload policy (+ providers) |
| `artifact-name` | `openshell-policy` | Artifact name |

| Output | Description |
| --- | --- |
| `policy-path` | Absolute path to generated policy |
| `providers-path` | Absolute path to providers (if requested) |

### `actions/diff-logs`

| Input | Required | Description |
| --- | --- | --- |
| `logs` | yes | Space-separated log files |
| `root` | no | Repository root |
| `write` | no | `--write` |
| `deny-stale` | no | `--deny-stale` |

### `actions/check`

Combines `setup`, `drift`, and `lint`. Inputs: `version`, `root`, `policy`, `providers`, `profiles-dir`,
`failure-threshold`, `strict-drift`. Output: `version` (installed CLI).

The actions share their logic through scripts in [`actions/lib`](../actions/lib), called via
`github.action_path`. `check` does not nest `uses: ./actions/...` steps, because inside a composite action a
relative `uses:` resolves against the caller's workspace and breaks remote consumers (#65). The
`consumer-check` job in `.github/workflows/actions-selftest.yml` runs `check` from a subdirectory checkout to
catch that regression.

## CD: regenerate policy in CI

```yaml
      - uses: ilanh/openshell-annotate/actions/setup@v0.1.0-rc.3
      - uses: ilanh/openshell-annotate/actions/generate@v0.1.0-rc.3
        id: policy
        with:
          policy-out: openshell/policy.yaml
          providers-out: openshell/providers.json
          upload-artifact: true
```

## Example: Policy Playground (`openspell-python-tester`)

```yaml
permissions:
  contents: read

jobs:
  policy:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v4.2.2
      - uses: ilanh/openshell-annotate/actions/check@v0.1.0-rc.3
        with:
          root: .
          policy: openshell/policy.yaml
          providers: openshell/providers.json
          failure-threshold: warning
```

Split steps (same pins):

```yaml
      - uses: ilanh/openshell-annotate/actions/setup@v0.1.0-rc.3
      - uses: ilanh/openshell-annotate/actions/drift@v0.1.0-rc.3
        with:
          policy: openshell/policy.yaml
          providers: openshell/providers.json
      - uses: ilanh/openshell-annotate/actions/lint@v0.1.0-rc.3
        with:
          policy: openshell/policy.yaml
          failure-threshold: warning
```

## Permissions

These actions need `contents: read` only. No secrets. Optional attestation verify uses the GitHub CLI with the
default `GITHUB_TOKEN`.

## Self-test

The repository runs [`.github/workflows/actions-selftest.yml`](../.github/workflows/actions-selftest.yml) on changes
to `actions/` (Ubuntu and macOS, including an expected drift failure).
