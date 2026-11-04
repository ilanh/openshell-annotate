# Polyglot sample

A pretend release-watcher called desk. The files are stubs: they are not wired together and most of them do not run. Each one exists so `openshell-annotate` has an `@openshell` comment to read, in as many languages as the scanner knows.

The comments are the policy. `openshell/policy.yaml` and `openshell/providers.json` are generated from them.

## Run

From this directory (the config in `openshell-annotate.toml` is picked up automatically):

```
openshell-annotate scan
openshell-annotate generate -o openshell/policy.yaml --providers-out openshell/providers.json
openshell-annotate lint --policy openshell/policy.yaml --deny-warnings
openshell-annotate drift --policy openshell/policy.yaml --providers openshell/providers.json
```

From the repository root, pass `--root examples/polyglot`. `scan` prints six notes for Lua, Erlang, Haskell, OCaml, Clojure and Visual Basic. Those files are present on purpose. Their directives live in `annotations/unscanned.sh`.

`lint --deny-warnings` is clean. Call-site checks run only for JavaScript, TypeScript, Python, Go and shell. The other files are read for directives only.

## What the directives show

| Kind | Where |
|---|---|
| `egress` with `allow` | `src/ts/chat.ts`, `src/tsx/panel.tsx` |
| `egress` with `access` and `scope=file` | `src/js/github.js`, `src/py/wheels.py` |
| `egress` with `allow` and `deny` | `src/kotlin/Gradle.kt` |
| `egress` with `enforce=audit` | `src/java/Maven.java` (a `/* */` comment) |
| `egress` with two ports | `src/csharp/NuGet.cs` (`443,8443`) |
| `egress` with `protocol` | graphql in PHP, json-rpc in Swift and Solidity, mcp in Scala, websocket in Dart, tcp in C |
| `egress` with `tls=skip` | `src/c/db.c` (tcp only; it cannot be combined with inspection) |
| `egress` with a wildcard and `rule=` | `src/rust/crates.rs` (`*.crates.io`, rule `crates`) |
| `bin=` on one host for two executables | `scripts/fetch.sh` (`git clone` and curl both talk to `github.com:443`; for git the connecting process is `/usr/lib/git-core/git-remote-https`) |
| `@openshell bin` for the rest of the file | `src/go/main.go` |
| `provider` | OpenAI, GitHub and Anthropic, mapped by `profiles/` |
| `fs read` / `fs write` | Python, Go, C++ and the C and C++ headers |
| `process` | `src/go/main.go` (`user=sandbox group=sandbox`) |
| `ignore` | `src/js/github.js` (a runtime webhook, and a local `tar`), `Dockerfile` (`apt-get` runs at build time, not in the sandbox) |

The same host from several files becomes one rule. `rubygems.org` is declared from `.rb`, `.rake` and `.gemspec`, and the generated rule lists all three lines. `api.nuget.org` is declared from C# and from F#.

Paths in `[languages]` are the executables inside the sandbox image, not paths on this machine. OpenShell does not follow symlinks. A first-class language uses the family name (`javascript`, `python`, `go`, `shell`). Every other language uses the file extension (`rb`, `rs`, `tf`, ...).

## Which comments are read

A directive is a comment whose text starts with `@openshell`. The same grammar is used in every language. The comment leader is the language's own.

**First-class** (string-aware lexer and call-site lint): JavaScript and TypeScript (`.js` `.jsx` `.mjs` `.cjs` `.ts` `.tsx` `.mts` `.cts`), Python, Go, and shell (`.sh` `.bash` `.zsh` and `Dockerfile`).

**Built-in leaders**, directives only:

| `#` | `//` and `/* */` |
|---|---|
| Ruby (`.rb` `.rake` `.gemspec`), Perl (`.pl` `.pm`), R | Rust, Java, Kotlin (`.kt` `.kts`), C#, PHP, Swift, Scala, Dart, C (`.c` `.h` `.cc`), C++ (`.cpp` `.hpp`) |

**Turned on in this config** (`[scan.comment_leaders]`), because the comments really are `#` or `//`: Elixir (`.ex` `.exs`), Julia, Nim, Crystal, Zig, Groovy, F#, V, Solidity, Terraform.

**Not scanned.** Lua (`--`), Erlang (`%`), Haskell (`--`), OCaml (`(* *)`), Clojure (`;`) and Visual Basic (`'`) do not use those leaders. Setting `comment_leaders` would not make `--` or `%` visible. The sources are still in the tree, and the directives are in `annotations/unscanned.sh`, which is scanned like any other shell file. `scan` says so on stderr.

These look like directives and are ignored, so they do not appear in the policy:

- a marker inside a JavaScript string (`src/js/github.js`), a Rust string and a C string
- a marker inside a Python docstring (`src/py/wheels.py`)
- a marker inside a quoted shell heredoc (`scripts/fetch.sh`)
- a PHP `#` comment (`src/php/Packagist.php`); PHP is read as `//` and `/* */`

## Generated files

`openshell/policy.yaml` and `openshell/providers.json` were produced by the commands above. The policy header says `@VERSION@` instead of a tool version, so the file does not change on every release; `generate` writes the real version, and `drift` ignores the header. `tests/cli.rs` regenerates both files and checks they match. Edit the comments, then run `generate` again. The profiles under `profiles/` only map credential names to env vars. They are not imported into OpenShell from here.
