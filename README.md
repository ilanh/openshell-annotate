# openshell-annotate

Policy-as-code for [NVIDIA OpenShell](https://github.com/NVIDIA/OpenShell) sandboxes. Developers write
structured comments next to the code that needs access; the CLI scans the repo and generates `policy.yaml`
(every entry commented with its reason and `file:line`) plus a separate providers file, lints for
un-annotated calls, and gates CI on drift.

> **Independent community project.** openshell-annotate is not affiliated with, endorsed by, or sponsored by
> NVIDIA. It uses NVIDIA OpenShell's public policy schema crate (Apache-2.0) to validate what it generates.

Status: early prototype (version 0.1.0, no releases yet). Targets OpenShell policy schema v1
(`openshell-policy-schema` @ v0.1.6). Licensed under Apache-2.0.

## Install

Prebuilt binaries are not published yet. Build from source (Rust 1.94+ and a C compiler, because the tree-sitter
grammars are C code; the OpenShell schema crate is fetched from GitHub by cargo):

```
git clone https://github.com/ilanh/openshell-annotate && cd openshell-annotate
cargo build --release --locked        # binary: target/release/openshell-annotate
```

```ts
// @openshell egress api.openai.com:443 reason="LLM calls" access=read-write
// @openshell provider OPENAI_API_KEY
const r = await fetch("https://api.openai.com/v1/chat/completions", { ... });
```
```python
# @openshell fs write /workspace/data reason="result cache"
```

## Commands

| Command | What it does |
|---|---|
| `scan [PATHS] [--json]` | list directives with `file:line` (JS/TS, Python, Go, shell, Dockerfile comments; string-aware lexer) |
| `generate [PATHS] -o policy.yaml [--providers-out f.json\|f.sh] [--profiles-dir DIR]` | emit policy.yaml with a comment per entry; re-validated with the OpenShell schema crate; providers as JSON or a setup script |
| `lint [PATHS] [--policy policy.yaml] [--deny-warnings]` | directive validation + tree-sitter detection (JS/TS, Python, Go, bash) of un-annotated network calls, URL literals, filesystem writes, secret reads, subprocesses; sanity-checks a policy file |
| `diff-logs LOG... [--write] [--deny-stale] [--json]` | map sandbox allow/deny events (`openshell logs` shorthand or OCSF JSONL, `-` = stdin) to directives by binary, host, port: suggests missing directives (`--write` inserts them), flags stale ones, separates non-actionable (SSRF/metadata) and declined (`ignore kind=net host=`) denials; exit 1 when something is missing |
| `drift [PATHS] --policy policy.yaml [--providers f.json] [--strict]` | regenerate in memory and compare as typed documents; reports missing / extra / changed, tagged `startup` or `live` |

Exit codes: 0 clean, 1 findings/errors, 2 usage or I/O error.
`--root DIR` sets the repository root (paths in output are relative to it), `--config FILE` overrides
`<root>/openshell-annotate.toml`.

## Directive grammar

Full reference, generated from the code: [docs/DIRECTIVES.md](docs/DIRECTIVES.md) (directives and attributes) and [docs/CLI.md](docs/CLI.md) (every subcommand and flag). Regenerate with `scripts/gen-docs.sh`; `cargo test` fails when they are stale. Summary:

```ebnf
directive = "@openshell" , ws , body ;                 (* first thing in a comment line *)
body      = egress | provider | fs | bin | process | ignore ;
egress    = "egress" , ws , host , ":" , port , { "," , port } , { ws , attr } ;
provider  = "provider" , ws , ENV_NAME , { ws , attr } ;
fs        = "fs" , ws , ( "read" | "write" ) , ws , abs_path , { ws , attr } ;
bin       = "bin" , ws , abs_path ;
process   = "process" , { ws , attr } ;
ignore    = "ignore" , { ws , attr } ;
attr      = key , "=" , ( bare | '"' , text , '"' ) ;
```
Attributes: `reason` (required for egress, fs write, ignore), `access`, `protocol`, `allow`/`deny`
(`METHOD:/path`, repeatable), `enforce`, `tls=skip`, `bin`, `rule`, `scope=file`, provider `profile`/`name`,
process `user`/`group`, ignore `kind`. Unknown directives/attributes are errors, mirroring OpenShell's strict schema.

## Mapping to the OpenShell schema

| Directive | Output |
|---|---|
| `egress host:port` | `network_policies.<rule>.endpoints[]` (`host`, `port`/`ports`, `protocol`, `enforcement`, `access` or `rules`/`deny_rules`) + `binaries[]`; rule key = host slug unless `rule=` |
| binary | `bin=` > nearest `@openshell bin` in the file > `[languages]` in `openshell-annotate.toml`; missing = error (empty `binaries` allows nothing) |
| `fs read|write` | `filesystem_policy.read_only|read_write` (+ `include_workdir`, default true, because a present section defaults it to false) |
| `process` | `process.run_as_user|run_as_group` |
| `provider ENV` | **not** in policy.yaml: provider profile resolved from `credentials[].env_vars` in profile YAML (`--profiles-dir`), emitted as `providers.json` / `providers.sh` (`openshell provider create ... --credential ENV`) |
| `reason`, location | YAML comment only (OpenShell drops comments when it stores/returns a policy) |

## Config (`openshell-annotate.toml`, strict)

```toml
[languages]            # real executable path per language family inside the sandbox image
javascript = "/usr/local/bin/node"
python = "/usr/bin/python3.12"
go = "/opt/app/server"
shell = "/bin/bash"
[policy]
include_workdir = true
enforcement = "enforce"   # OpenShell's own default for inspected endpoints is "audit"
system_paths = "auto"     # auto: add /usr /lib /etc /proc /dev/urandom (ro) and /tmp /dev/null (rw) when there are
                          # no egress rules (the supervisor only adds them with network rules; without them nothing can exec);
                          # always | never
[providers]
profiles_dir = "/path/to/OpenShell/providers"
[providers.env]           # env var -> profile id overrides
[providers]
profiles_dirs = ["openshell/profiles"]   # extra directories of custom provider profiles (also lintable with `openshell profile lint -f`)
[lint]
engine = "treesitter"      # or "regex" (the original line heuristics)
fs_dynamic = "summary"     # fs writes with computed paths: summary (one repo-level info) | info (per file) | warn | off
window = 8                 # lines a directive covers below it
exclude = []               # "^prefix/" anchors at the repo root; anything else is a substring of "/<relative path>"
categories = []            # restrict call-site heuristics to any of "net", "fs", "env", "exec" (empty = all)
[scan]
exclude = []               # same pattern syntax as [lint].exclude
```

## Limits (be aware)

- Lint parses real syntax (tree-sitter) but still cannot see through SDKs or computed URLs: those need a directive or
  `@openshell ignore`. Filesystem writes are checked against the declared `fs write` roots and `/tmp` (the sandbox baseline);
  a write whose path is computed cannot be proven statically, so it is one repo-level info (or one warning per file when no
  root is declared at all). Writes through an already-open `FileHandle` are not seen. `fs.mkdir` etc. are only recognised
  as fs calls when imported from `fs`, `node:fs`, `fs/promises`, `fs-extra` (or called on an object named like `fs`).
- JS regex literals are not parsed; Python docstrings are strings (directives must be `#` comments).
- Filesystem/process/landlock are startup-only in OpenShell; drift labels them `startup`.
- `diff-logs` only sees network and L7 events: OpenShell 0.1.6 does not log Landlock/filesystem denials (upstream #4270), log
  delivery is batched and best-effort (wait a few seconds after the workload; absence of an event is not proof), and `NET:REFUSE` lines carry no binary.
  A helper spawned by an allowed binary (git -> git-remote-http) is matched through its parent.
- Semantic checks mirror `openshell-policy::validate_sandbox_policy` (documented rules) but the authoritative
  validation is the gateway (`openshell sandbox create --policy`) and, soon, `openshell policy lint` (#4035).

## Develop

```
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test
scripts/gen-docs.sh --check                  # docs/CLI.md and docs/DIRECTIVES.md are generated from the code
```

Full guide (prerequisites, a hands-on quick start, Docker, the sandbox flow, branch workflow): [docs/DEVELOPING.md](docs/DEVELOPING.md).

Contributions need a DCO sign-off, see [CONTRIBUTING.md](CONTRIBUTING.md). Security reports: [SECURITY.md](SECURITY.md).

## Docker (local build)

```
scripts/docker-build.sh                      # builds openshell-annotate:dev (~35 MB, distroless/cc; not published)
docker run --rm -u $(id -u):$(id -g) -v "$PWD":/src openshell-annotate:dev lint
docker run --rm -v "$PWD":/src openshell-annotate:dev scan
```
Entrypoint is `openshell-annotate`, workdir `/src`. The build needs network access to fetch crates and the OpenShell schema crate from GitHub.
