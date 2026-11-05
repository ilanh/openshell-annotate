---
name: annotate-openshell-policy
description: Annotate a codebase with @openshell policy directives so openshell-annotate can generate an OpenShell sandbox policy.yaml. Use when asked to "write/derive/update the OpenShell policy", "annotate network calls / secrets / file writes", or when a policy drift or lint check fails.
---

# Annotate code for an OpenShell policy

Goal: every place the code reaches the network, writes the filesystem, reads a credential or spawns a
program that matters at runtime is described by a one-line `@openshell` comment, **with a reason**.
`openshell-annotate` turns those comments into `policy.yaml` (+ a providers file) whose entries carry the
reason and `file:line`. You never hand-edit the generated policy.

## 0. Know the target

- Only code that runs **inside the sandbox** needs directives. Build tooling, tests, docs, dev scripts and
  code that runs on the host or in a different service do not. Exclude them in `openshell-annotate.toml`
  (`[scan] exclude`, `[lint] exclude`) or mark a call `@openshell ignore ... reason="..."`.
- Policy facts that drive the choices (OpenShell policy schema v1):
  - Network rules are *binaries x endpoints*. The binary is the **real path of the executable** that opens the
    connection (`/usr/local/bin/node`, `/usr/bin/python3.12`, `/usr/bin/git`), never a script, never a symlink.
    An empty binary list allows nothing.
  - Filesystem paths are absolute in-sandbox paths, no `..`, never `/` for write. Filesystem, process and landlock
    settings are startup-only (changing them needs a sandbox recreate); network rules are live.
  - `enforcement` defaults to `audit` in OpenShell (logs, does not block); this tool emits `enforce` unless told otherwise.
  - Loopback, link-local and metadata addresses can never be allowed; do not annotate them (sandbox-local
    services need no rule).
  - There is no `providers` section in policy.yaml. `@openshell provider` produces a separate setup file.
- Read `openshell-annotate.toml` first. `[languages]` must hold the real binary path per language for the image
  (ask the user or read the Dockerfile; do not guess). `[providers].profiles_dir` points at provider profiles.

## 1. Inventory (find the candidates)

Run the linter first; it lists un-annotated call sites it can recognise:

```
openshell-annotate lint --format compact   # one line per finding (the default `human` report is longer; use `--format compact` or `--json` when parsing). tree-sitter: warnings = unannotated net / fs write outside declared roots / secret reads; info = subprocesses, URL literals, computed-path fs writes
```

Then search for what a regex linter cannot see. Look for, at least:

| Category | Look for |
|---|---|
| Network | `fetch(`, `axios`, `http(s).request`, `WebSocket`, `requests.`, `httpx`, `urllib`, `http.Get/NewRequest`, `net.Dial`, SDK clients that hide HTTP (OpenAI/Anthropic/AWS/GitHub SDKs), `curl`/`wget`/`git clone`/`npm install`/`pip install` in scripts and Dockerfiles, URL/host constants and `*_URL`/`*_BASE_URL`/`*_ENDPOINT` env vars |
| Filesystem | `writeFile`, `mkdir`, `createWriteStream`, `rename`, `open(..., "w")`, `os.WriteFile`, redirects (`>`/`>>`), temp dirs, data/cache/log/state directories, config `HOME`-relative paths; reads of paths outside the workdir (certs, `/etc`, models) |
| Secrets | `process.env.X_API_KEY/TOKEN/SECRET`, `os.environ["..."]`, `os.Getenv`, `.env` loading, key files |
| Subprocesses | `spawn`, `execFile`, `child_process`, `subprocess`, `exec.Command`: the child binary, not the parent, opens its own connections |

For each candidate decide: runs in the sandbox? talks to which host:port (read the code, config defaults, docs)?
which executable makes the connection? what is the narrowest access (methods/paths)?

## 2. Write directives

Put the directive in a comment on the line(s) directly above the call (within 8 lines), same indentation.
Comment leader is whatever the language uses (`//`, `#`, `/* */`). A directive must be the **first thing in the
comment**. Directives inside strings or Python docstrings are not seen; use real comments.

```
@openshell egress <host>:<port>[,<port>] reason="..." [access=read-only|read-write|full]
           [allow="METHOD:/path"]... [deny="METHOD:/path"]... [protocol=rest|websocket|graphql|mcp|json-rpc|tcp]
           [bin=/real/path] [rule=name] [enforce=enforce|audit] [tls=skip] [scope=file]
@openshell provider <ENV_VAR> [profile=<id>] [name=<instance>]
@openshell fs read|write <absolute-path> reason="..."
@openshell bin <absolute-path>            # default executable for egress directives below it in this file
@openshell process user=<sandbox|uid> group=<sandbox|gid>
@openshell ignore kind=net|fs|env|exec|all reason="..." [scope=file]
```

Rules of thumb (least privilege):

1. **Always a reason**, one phrase saying *why the code needs it* ("LLM completions", "downloads model weights").
   Never put secrets, tokens or customer data in a reason or any directive.
2. Literal host and port, e.g. `api.openai.com:443`. Add `access=read-only` for read-only APIs, or
   `allow="POST:/v1/chat/completions"` rules when the API surface is small. Use `access=full` only when
   needed and say why. For `protocol=mcp`, `access=` is allow-all: the policy gets
   `mcp.allow_all_known_mcp_methods: true` and no `access:` preset. Use `allow=` rules to list requests instead.
   For `protocol=graphql`, `allow` and `deny` are `query`, `mutation`, or `subscription` (optionally
   `:operationName`), not HTTP methods. `protocol=rest`, `websocket`, or `graphql` also needs `access=`
   or `allow=`; the protocol alone is rejected.
   Avoid wildcards; if unavoidable use `*.example.com` (>= 3 labels, never `*.com`).
   One host per directive: `host:443` or `host:443,8443` (ports only). There is no comma-separated host list, so
   `pypi.org:443,files.pythonhosted.org:443` is an error: write one `egress` line per host.
3. Binary: rely on `[languages]` for the usual interpreter. Use `bin=` when a subprocess makes the call
   (`bin=/usr/bin/git`), or a `@openshell bin` line for a whole file. Never invent a path: confirm it in the image.
4. User-configurable or dynamic endpoints (`baseUrl` from settings): annotate the **default/documented**
   host and say in the reason that others need a policy change; use `@openshell ignore kind=net reason="..."`
   only when the target is truly operator-supplied, and list it in your report as a gap.
5. One directive per distinct (host, port, binary) is enough; repeated call sites in the same file can share
   `scope=file` or a directive near the first call. Same host in several files is fine: sources are merged.
6. Secrets: add `@openshell provider ENV` once, next to the code that reads the variable. If the tool can't
   map it to a profile (`unmapped`), report it; do not invent a profile id.
7. Filesystem: write the **in-sandbox absolute path** (`/workspace/data`, `/tmp`, `/home/sandbox/.cache`), not a
   path relative to the repo. Prefer the narrowest directory. `fs write` needs a reason; `fs read` is for paths
   outside the workdir (the workdir is already included when `include_workdir` is true).
8. Do not annotate to silence the linter. If something does not apply (tests, host-only code) exclude the
   path in config or use `ignore` with a truthful reason.

Examples (same grammar in every language):

```ts
// @openshell egress api.anthropic.com:443 reason="agent LLM calls" allow="POST:/v1/messages"
// @openshell provider ANTHROPIC_API_KEY
const res = await fetch("https://api.anthropic.com/v1/messages", { ... });
```
```python
# @openshell egress pypi.org:443 reason="pip install at start" access=read-only
# @openshell egress files.pythonhosted.org:443 reason="pip install at start (wheel downloads)" access=read-only
# @openshell fs write /workspace/cache reason="model cache"
```
```go
// @openshell egress api.github.com:443 reason="release lookup" access=read-only
// @openshell bin /opt/app/server
resp, err := http.Get("https://api.github.com/repos/o/r/releases/latest")
```
```sh
# @openshell egress github.com:443 reason="git clone" access=read-only bin=/usr/bin/git
git clone https://github.com/o/r.git /workspace/r
```

## 3. Check your work (run all, in order, fix until clean)

```
openshell-annotate scan --format compact                  # every directive with file:line; parse errors fail here (`human` is the default and longer; parse compact or --json)
openshell-annotate generate -o openshell/policy.yaml --providers-out openshell/providers.sh
openshell-annotate lint --format compact --policy openshell/policy.yaml    # directive errors + remaining unannotated calls + policy sanity
openshell-annotate drift --policy openshell/policy.yaml   # must say "no drift" after generate
```

Optional deeper checks when the OpenShell CLI is available:

```
openshell-prover check --boundary openshell/policy.yaml openshell/policy.yaml   # parses + checks the file offline (workaround until `openshell policy lint`)
openshell sandbox create --policy openshell/policy.yaml ...                      # the gateway performs full validation
openshell logs <sandbox> --source sandbox -n 5000 > logs.txt                     # wait ~5 s after the workload: delivery is batched
openshell-annotate diff-logs logs.txt --format compact                           # denials -> missing directives (+ stale ones); --write applies them
```

Fix-up table:

| Message | Fix |
|---|---|
| `egress requires reason` | add `reason="..."` |
| `a port is required` | write `host:port` |
| `no executable known for this egress` | set `[languages]` in the config, or add `bin=` / `@openshell bin` |
| `conflicting settings for host:port` | make directives agree or give them different `rule=` |
| `... all map to rule key` | add `rule=<name>` to separate hosts whose names collapse to the same key |
| `must be an absolute path` | use the in-sandbox absolute path |
| `unmapped provider` | write/choose a provider profile; set `[providers.env]`; or `profile=` |
| lint `warning[net]` | add an egress directive near the call, or `ignore` with a reason |
| `diff-logs` MISSING_EGRESS / BINARY_MISMATCH | a real run was denied: review, then `diff-logs LOG --write` (or add the directive by hand), regenerate; if it must stay blocked add `@openshell ignore kind=net host=H reason="..."` |
| `diff-logs` STALE | no event matched: advisory only (logs are lossy); remove the directive only after a run that exercised that feature |
| drift `missing` | regenerate and commit policy.yaml |
| drift `extra` | a policy grant has no directive: add the directive or remove the grant |

## 4. Report

Finish with: number of directives by kind, the generated rule list (host:port, access), providers and any
`unmapped`, remaining lint warnings and why they are not annotated, and **gaps** (dynamic endpoints, SDK-hidden
calls, subprocess binaries, startup-only items the user must recreate the sandbox for). Commit
`policy.yaml`, `providers.sh`/`providers.json` together with the source change. Never commit credentials.
