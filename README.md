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
| `scan [PATHS] [--format human\|json\|compact] [--color ...] [--no-icons]` | list directives with `file:line` (JS/TS, Python, Go, shell, Dockerfile comments; string-aware lexer) |
| `generate [PATHS] -o policy.yaml [--providers-out f.json\|f.sh] [--scaffold-profiles DIR] [--profiles-dir DIR] [--format ...]` | emit policy.yaml with a comment per entry; re-validated with the OpenShell schema crate; providers as JSON or a setup script |
| `lint [PATHS] [--policy policy.yaml] [--deny-warnings] [--format human\|json\|compact] [--color auto\|always\|never] [--no-icons]` | directive validation + tree-sitter detection (JS/TS, Python, Go, bash) of un-annotated network calls, URL literals, filesystem writes, secret reads, subprocesses; sanity-checks a policy file |
| `diff-logs LOG... [--write] [--deny-stale] [--format human\|json\|compact] [--color ...] [--no-icons]` | map sandbox allow/deny events (`openshell logs` shorthand or OCSF JSONL, `-` = stdin) to directives by binary, host, port: suggests missing directives (`--write` inserts them), flags stale ones, separates non-actionable (SSRF/metadata) and declined (`ignore kind=net host=`) denials; exit 1 when something is missing |
| `drift [PATHS] --policy policy.yaml [--providers f.json] [--strict]` | regenerate in memory and compare as typed documents; reports missing / extra / changed, tagged `startup` or `live` |

Exit codes: 0 clean, 1 findings/errors, 2 usage or I/O error.

### Lint output

`lint` prints a rustc-style report by default: a header per finding with severity and category icons, the location, the
source lines around it with the offending call underlined, and a copy-pasteable `@openshell` directive under `= help:`.
Findings are grouped by file and end with a summary by severity and category.

```text
warning[net]: unannotated network call to api.github.com:443
 --> src/server.ts:6:21
  |
5 |   const token = process.env.GITHUB_TOKEN;
6 |   const res = await fetch("https://api.github.com/repos/o/r/issues", {
  |                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ fetch() reaches api.github.com:443 ...
7 |     headers: { Authorization: `Bearer ${token}` },
  |
  = help: declare the destination on the line above:
          // @openshell egress api.github.com:443 reason="..."
```

| Category | Icon | Meaning |
|---|---|---|
| `net` | 🌐 | network call or URL literal with no matching `egress` |
| `fs` | 📁 | filesystem write outside `/tmp` and the declared `fs write` roots |
| `env` | 🔑 | secret-like environment variable with no `provider` |
| `exec` | ⚙️ | subprocess the policy does not describe |
| `directive` | 📝 | malformed `@openshell` directive |
| `policy` | 📜 | problem in the `--policy` file |

Severities are `✖ error`, `⚠ warning` and `ℹ info`. Colour is on for a terminal and off otherwise; `--color always|never` overrides
it, and `NO_COLOR` / `CLICOLOR_FORCE` are honoured in `auto` mode. Icons fall back to plain ASCII with `--no-icons` (alias
`--ascii`), a non-UTF-8 locale or `TERM=dumb`.

For scripts, CI and AI agents use a stable format: `--format compact` prints one `file:line: severity[category]: message` line per
finding, and `--format json` (alias `--json`) prints the JSON array. Both keep the old `lint: N directive(s); ...` summary on
stderr. Exit codes and `--deny-warnings` do not depend on the format.

### Readable output for `scan`, `generate` and `diff-logs`

The other commands share the lint report's look and flags (`--format human|json|compact`, `--color auto|always|never`,
`--no-icons`/`--ascii`; `NO_COLOR` and `CLICOLOR_FORCE` are honoured). `human` is the default; `--format compact` prints
the plain output of earlier versions (one line per item) and `--json` stays an alias for `--format json`. Exit codes do not
depend on the format. Category icons: 🌐 net, 📁 fs, 🔑 env, ⚙️ exec, 📝 directive, 📜 policy, 🧹 stale/drift.

`scan` groups directives by file, shows each one's `file:line` and its source line, and ends with counts by category:

```text
📄 cmd/main.go  (2 directives)

🌐 egress api.github.com:443  # release lookup
 --> cmd/main.go:3
  |
3 | // @openshell egress api.github.com:443 reason="release lookup" access=read-only
  |

🌐 egress api.github.com:443  # release lookup (second call site)
 --> cmd/main.go:4
  |
4 | // @openshell egress api.github.com:443 reason="release lookup (second call site)" access=read-only
...
✔ 8 directives in 4 files
by category: 🔑 env 1 · 📁 fs 2 · 🌐 net 5
```

`diff-logs` groups what the logs say by what to do about it: missing directives, binaries not covered, L7 rules too narrow,
connections that were allowed and then failed (`NET:FAIL` right after an ALLOWED: usually a client that does not trust the
OpenShell TLS-termination CA; fix with `tls=skip` or by trusting the CA), things to review by hand, stale directives, deliberate declines and always-blocked destinations. Each fix is previewed as an
inserted `+` line above the line `--write` would put it on; the footer counts suggestions and names the follow-up command.

```text
🔍 diff-logs  7 events: ✖ 6 denied · ✔ 1 allowed

➕ Missing directives  (1)

✖ 🌐 missing egress: api.newvendor.io:443  ×1  /usr/bin/python3.12
  = why: no directive covers this host:port
  = where: no source line mentions this host; `--write` appends it to openshell/observed-egress.sh
  = add: @openshell egress api.newvendor.io:443 reason="observed in sandbox log: /usr/bin/python3.12 (confirm it is needed)" bin=/usr/bin/python3.12

⚙️ Binary not covered  (1)

⚠ 🌐 binary mismatch: registry.npmjs.org:443  ×2  /usr/bin/curl
 --> scripts/run.sh:2
  |
1 | #!/usr/bin/env bash
+ | # @openshell egress registry.npmjs.org:443 reason="observed in sandbox log: /usr/bin/curl (confirm it is needed)" bin=/usr/bin/curl access=read-only
2 | # @openshell egress registry.npmjs.org:443 reason="npm install" bin=/usr/local/bin/node access=read-only
  |
  = why: host:port is annotated at scripts/run.sh:2 but for /usr/local/bin/node; this binary is not in
         the rule

🔒 L7 rules too narrow  (1)

⚠ 🌐 l7 allow: huggingface.co:443  ×1
...
```

`generate` never changes what it writes: the policy YAML is the only thing on stdout (or in `-o FILE`) for every format, so
`openshell-annotate generate > policy.yaml` and pipes work as before. The readable summary (rules with their source lines,
filesystem roots, providers, warnings) goes to **stderr**, and its colour follows stderr. With `--format json` the summary is
a JSON object on stderr; `--format compact` prints the previous `warning: ...` and `generated: ...` lines.

```text
✔ generated a policy from 8 directives

📜 policy     → stdout
🌐 network    4 rules · 4 endpoints · 4 binary paths
   • api_github_com
       → api.github.com:443 [protocol=rest access=read-only enforcement=enforce]
       bins: /opt/app/server
       from cmd/main.go:3, cmd/main.go:4
   • api_openai_com
       → api.openai.com:443 [protocol=rest access=read-write enforcement=enforce]
       bins: /usr/local/bin/node
       from src/llm.ts:1
...
```

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
                          # profiles outside the repo: the generated providers.sh uses ${OPENSHELL_PROFILES_DIR:-<that dir>}/<file>,
                          # so it keeps working after the repo or the OpenShell checkout moves
[providers.env]           # env var -> profile id overrides
[providers]
expect = []               # credentials read through dynamic lookups no scan can see, e.g. ["GITHUB_PERSONAL_ACCESS_TOKEN"]; each is treated
                          # like an `@openshell provider ENV` directive (mapped to a profile, in the providers output, satisfies lint)
[providers]
profiles_dirs = ["openshell/profiles"]   # extra directories of custom provider profiles (also lintable with `openshell profile lint -f`)
[lint]
engine = "treesitter"      # or "regex" (the original line heuristics)
fs_dynamic = "summary"     # fs writes with computed paths: summary (one repo-level info) | info (per file) | warn | off
window = 8                 # lines a directive covers below it
exclude = []               # lint-only extras; [scan].exclude already applies to lint. "^prefix/" anchors at the repo root;
                           # anything else is a substring of "/<relative path>" (literal, not a regex)
ignore_hosts = []          # hosts (exact or "*.example.com") never reported as unannotated destinations, e.g. doc links ("treesitter" engine)
categories = []            # restrict call-site heuristics to any of "net", "fs", "env", "exec" (empty = all)
[scan]
exclude = []               # same pattern syntax (literal, NOT a regex); hides files from scan, generate, drift and lint alike.
                           # list one entry per directory: ["^packages/cli/", "^packages/admin-mcp/"]. A pattern that matches no file warns.
[scan.comment_leaders]     # extension -> "hash" (#) | "slash" (// and /* */) | "none"; extends/overrides the built-ins
# rb = "hash"              # built in: rb rake gemspec pl pm r (hash); rs java kt kts cs php swift scala dart c h cc cpp hpp (slash)
[languages]
# rb = "/usr/bin/ruby"     # default executable for generic languages is keyed by file extension
[diff-logs]
exclude = []               # files diff-logs never picks as the place for a suggested directive (e.g. the probe script
                           # that drives the sandbox run). Same literal pattern syntax. The --observed-file is always skipped.
```

### Credentials with no provider profile

`@openshell provider ENV` needs a provider profile that declares `ENV`; the bundled OpenShell profiles cover the common
cloud APIs only. For every credential nothing declares (reported as `unmapped`), `generate --scaffold-profiles DIR` writes
`DIR/<env-lowercased-with-hyphens>.yaml`: the credential, and the endpoints, `access` and binaries taken from the `egress`
directives in the file that declares it (`tls=skip` endpoints get `allow_uninspected_credentials: true`, which OpenShell
requires). Files that already exist are kept. Review the auth style, then check and import it:

```
openshell-annotate generate -o openshell/policy.yaml --providers-out openshell/providers.sh --scaffold-profiles openshell/profiles
openshell provider profile lint -f openshell/profiles/context7-api-key.yaml
```

Add the directory to `[providers] profiles_dirs` so the next `generate` maps the credential.
The "duplicates an endpoint a provider profile adds" warning is only shown when the profile's `binaries` include a binary of
the rule; a profile bound to other binaries (the bundled `github` profile is for `gh`/`git`) does not cover your app, so the explicit rule is needed.

## Limits (be aware)

- Lint parses real syntax (tree-sitter) but still cannot see through SDKs or computed URLs: those need a directive or
  `@openshell ignore`. Filesystem writes are checked against the declared `fs write` roots and `/tmp` (the sandbox baseline);
  a write whose path is computed cannot be proven statically, so it is one repo-level info (or one warning per file when no
  root is declared at all). Writes through an already-open `FileHandle` are not seen. `fs.mkdir` etc. are only recognised
  as fs calls when imported from `fs`, `node:fs`, `fs/promises`, `fs-extra` (or called on an object named like `fs`).
- Languages: JS/TS, Python, Go and shell are first-class (string-aware lexing and lint call-site detection). Ruby, Rust,
  Java, Kotlin, C#, PHP, Swift, Scala, Dart, C/C++ and a few more are read for `@openshell` comments only (generic `#` or `//`
  lexer, no call-site lint). Map more extensions with `[scan.comment_leaders]`. Files in other known source languages (for
  example `.lua`, `.ex`, `.hs`) are counted and reported on stderr as `note: N .ext files were not scanned`. When you
  cannot or do not want to edit the source, keep the directives in a sidecar `.sh` file (comments only); it is scanned like any
  other shell file. A first-class sidecar annotations file is tracked in #20.
- Dynamic environment lookups are invisible to lint and provider detection: Ruby `ENV[key.to_s.upcase]`, Go viper
  (`SetEnvPrefix("github")` + `GetString("personal_access_token")`), a Python library such as litellm picking the variable
  by model name. Nothing static can resolve them. Declare the credential once, either with `@openshell provider ENV` next to
  the code or, when there is no good place for a comment, with `[providers] expect = ["ENV"]` in `openshell-annotate.toml`.
- URL literals are not reported (info[net]) when they sit in a docstring or bare string statement, in a `raise`/`throw`, in
  an argument of a print/log/echo/error-constructing call or `add_argument`, or in a variable or keyword named like text for
  people (`help`, `usage`, `msg`, `message`, `error`, `doc`, ...). Real calls are still flagged. A host that still shows up can be
  listed in `[lint] ignore_hosts`.
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
