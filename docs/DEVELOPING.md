# Developing and testing openshell-annotate

How to build, test, try out and change the tool from a fresh clone. Every command below was run from a clean clone
(Linux x86_64, Ubuntu 24.04, Rust 1.94 and 1.95); timings are from a 16-core machine with a warm cargo cache, so a first
build that downloads crates takes longer.

## 1. Prerequisites

| Needed for | What |
|---|---|
| everything | Git, a C compiler (`build-essential` / Xcode command line tools: the tree-sitter grammars are C), network access to crates.io and github.com (the OpenShell schema crate is a git dependency pinned to the `v0.1.6` tag) |
| build and test | Rust **1.94** or newer through [rustup](https://rustup.rs). CI pins 1.94 |
| Docker image (optional) | Docker with BuildKit |
| sandbox flow (optional) | the `openshell` CLI and `openshell-prover` at 0.1.6, a running OpenShell gateway, and Docker (see section 7) |

```
rustup toolchain install 1.94 --profile minimal --component rustfmt,clippy
. "$HOME/.cargo/env"          # puts cargo on PATH in this shell (non-login shells such as ssh commands do not load it)
```

## 2. Get the code and build

```
git clone https://github.com/<owner>/openshell-annotate && cd openshell-annotate
cargo +1.94 build --release --locked        # about 20 s warm; binary: target/release/openshell-annotate
target/release/openshell-annotate --version  # openshell-annotate 0.1.0
cargo +1.94 install --path . --locked        # optional: installs into ~/.cargo/bin (use --root DIR to install elsewhere)
```

`--locked` makes cargo use the committed `Cargo.lock`, which is what CI does.

## 3. Test

```
cargo test --locked
```

Expected: four result lines, all `ok` (26 unit tests, 30 CLI tests in `tests/cli.rs`, 3 docs tests in `tests/docs.rs`), about
10 s including the test build.

- CLI tests run the real binary against `tests/fixtures/` (`sample/` has the sources, config, provider profile and the
  golden `policy.expected.yaml` and `providers.expected.json`; `logs/` has one `openshell logs` shorthand file and one OCSF JSONL
  file for `diff-logs`).
- When you change generated output on purpose, edit the golden file by hand and review the diff. Nothing rewrites goldens.
- Add a test for every new directive, attribute, lint pattern or error message.

## 4. Run what CI runs

CI (`.github/workflows/ci.yml`) runs `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`,
`cargo test --locked` and `scripts/gen-docs.sh --check`, plus `cargo audit`. All but the audit are one command:

```
TOOLCHAIN=1.94 scripts/ci-local.sh     # pinned like CI; omit TOOLCHAIN to use your default toolchain
```

Expected last line: `ci-local: all checks passed`. Optional audit: `cargo install cargo-audit --locked && cargo audit`.

Clippy gains new lints with every Rust release. If it fails on a newer toolchain but passes on 1.94, fix the lint in a
separate commit (CI will keep using 1.94 until the pin is bumped on purpose in `ci.yml`, `Dockerfile` and `Cargo.toml`).

`docs/CLI.md` and `docs/DIRECTIVES.md` are generated from the code. After changing a flag, directive or attribute run
`scripts/gen-docs.sh` and commit the result; `scripts/gen-docs.sh --check` (and `cargo test`) fail when they are stale.

## 5. Try it on a tiny project

```
export PATH="$PWD/target/release:$PATH"
mkdir -p /tmp/oa-demo/src && cd /tmp/oa-demo
cat > openshell-annotate.toml <<'CFG'
[languages]                          # the real executable path of each language inside your sandbox image
javascript = "/usr/local/bin/node"
python = "/usr/bin/python3"
CFG
cat > src/app.js <<'JS'
// @openshell egress api.example.com:443 reason="fetch weather" access=read-only
async function weather() { return fetch("https://api.example.com/v1/weather"); }
JS
cat > src/telemetry.js <<'JS'
async function telemetry() { return fetch("https://telemetry.example.net/ping", { method: "POST" }); }
JS
cat > src/cache.py <<'PY'
# @openshell fs write /workspace/cache reason="result cache"
def save(p, b):
    open("/workspace/cache/" + p, "wb").write(b)
PY

openshell-annotate scan             # 2 directive(s) (a file without directives is counted too)
openshell-annotate lint             # warning[net] src/telemetry.js:1: fetch() without an @openshell directive
openshell-annotate lint --deny-warnings; echo $?        # 1 (a warning now fails)
sed -i '1i // @openshell egress telemetry.example.net:443 reason="usage ping" allow="POST:/ping"' src/telemetry.js
openshell-annotate lint --deny-warnings; echo $?        # 0
mkdir -p openshell
openshell-annotate generate -o openshell/policy.yaml    # 3 directive(s) -> 2 network rule(s) ... fs read 0 / write 1
openshell-annotate drift --policy openshell/policy.yaml # no drift
sed -i 's/read-only/read-write/' src/app.js
openshell-annotate drift --policy openshell/policy.yaml; echo $?   # changed [live] network_policies api_example_com ... ; 1
```

`policy.yaml` has one comment per entry with its reason and `file:line`, and is re-validated with OpenShell's own schema
parser before it is written.

`diff-logs` closes the loop after a sandbox run. Feed it `openshell logs <sandbox> --source sandbox` output or the sandbox's OCSF
JSONL; it suggests a directive for every denied connection no directive covers, and lists directives that never matched:

```
openshell-annotate diff-logs path/to/sandbox.log        # exit 1 while directives are missing; --deny-stale also fails on stale ones
openshell-annotate diff-logs path/to/sandbox.log --write  # inserts directives at the offending source line when it can find one,
                                                          # otherwise writes openshell/observed-egress.sh (scanned like any source: review or delete it)
```

Sample logs for both formats are in `tests/fixtures/logs/`. Filesystem denials are not logged by OpenShell 0.1.6, so
`diff-logs` only sees network and L7 events.

## 6. Docker image

```
scripts/docker-build.sh                     # openshell-annotate:dev, about 35 MB; ~40 s without cache, a few seconds cached
docker run --rm openshell-annotate:dev --version
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD":/src openshell-annotate:dev lint --deny-warnings
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD":/src openshell-annotate:dev generate -o openshell/policy.yaml
```

The image has no shell; its entrypoint is the binary and its workdir is `/src`. Output matches the host binary byte for byte.
`-u` keeps files you generate owned by you. Set `IMAGE=name:tag` to change the tag; extra arguments go to `docker build`.

## 7. Full flow with a real sandbox (optional)

`scripts/flow.sh` runs scan, generate, lint, drift, a schema check, optionally a real OpenShell sandbox, and `diff-logs`, and
writes `openshell/flow-report.md` in the project:

```
scripts/flow.sh /path/to/project                                  # static checks only
scripts/flow.sh /path/to/project --diff-logs sandbox.log          # + diff-logs on an existing log
scripts/flow.sh /path/to/project --image my-image:tag --run-seconds 30 --exercise exercise.js -- sleep 600
```

It needs `openshell` and `openshell-prover` 0.1.6 on PATH and a connected gateway (`openshell status` shows `Connected`), and
`OPENSHELL_ANNOTATE=/path/to/binary` if the binary is not on PATH. The image's interpreter paths must be real executables
(not symlinks) and match `[languages]` in `openshell-annotate.toml`, because OpenShell matches the resolved binary path.
A bare `sleep` workload produces no traffic and so no events: drive the sandbox with `--exercise FILE.js` (run with `node -e`),
or run your own commands with `openshell sandbox exec -n <name> -- ... </dev/null` while it is up, then run `diff-logs`.
Use dummy credentials only. Exit status: 0 clean, 1 a check failed, 2 usage error.

Maintainers also keep a separate, optional fixtures suite with one case per policy variation that runs against real sandboxes.
It is not needed to contribute; `cargo test` covers the generator.

## 8. Branch workflow

```
git switch main && git pull --ff-only
git switch -c feat/short-name                  # or fix/..., docs/..., chore/...
# edit, add a test, run: TOOLCHAIN=1.94 scripts/ci-local.sh
git commit -s -m "feat: what and why"          # -s adds the DCO Signed-off-by line (required, see CONTRIBUTING.md)
git push -u origin feat/short-name             # CI runs on every push to a PR and on main
```

Open a pull request into `main` once CI is green. Missing sign-off: `git commit --amend -s` (last commit) or
`git rebase --signoff main`. Keep commits small and use conventional prefixes (`feat:`, `fix:`, `docs:`, `chore:`, `test:`).
Make sure `git config user.name` and `user.email` are the identity you want public; GitHub's noreply address works.

## 9. Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `cargo: command not found` over ssh or in cron | rustup's `~/.cargo/bin` is only on PATH in login shells: run `. "$HOME/.cargo/env"` first |
| `error: failed to select a version` / git fetch of `OpenShell` fails | the schema crate comes from github.com; check network or proxy; `--locked` needs the tag commit recorded in `Cargo.lock` |
| clippy errors that CI does not show | newer toolchain; run with `TOOLCHAIN=1.94` or fix the new lint |
| `docs are stale` | run `scripts/gen-docs.sh` and commit `docs/` |
| sandbox probes get `403` from `api.github.com` | unauthenticated GitHub API rate limit (60 requests per hour per IP), not a policy decision; wait for the reset or use another host |
| `openshell-prover: not found` in `flow.sh` | install the OpenShell 0.1.6 tools; the static commands (`scan`, `generate`, `lint`, `drift`, `diff-logs`) do not need them |
