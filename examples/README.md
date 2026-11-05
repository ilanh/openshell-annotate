# Examples

## polyglot

[`polyglot/`](polyglot/) is a pretend release-watcher called desk. Its files are stubs that do not run. Each one carries `@openshell` comments in that language's own comment style, and those comments are the whole policy. `openshell-annotate generate` turns them into the committed [`openshell/policy.yaml`](polyglot/openshell/policy.yaml) and [`openshell/providers.json`](polyglot/openshell/providers.json), and `tests/cli.rs` checks that they stay in sync.

- Sources: [`src/`](polyglot/src/), plus [`scripts/fetch.sh`](polyglot/scripts/fetch.sh), [`Dockerfile`](polyglot/Dockerfile), [`deploy/network.tf`](polyglot/deploy/network.tf), [`include/`](polyglot/include/), [`lib/Desk.pm`](polyglot/lib/Desk.pm), [`tasks/deps.rake`](polyglot/tasks/deps.rake) and [`desk.gemspec`](polyglot/desk.gemspec)
- Config: [`openshell-annotate.toml`](polyglot/openshell-annotate.toml), with provider profiles in [`profiles/`](polyglot/profiles/)
- Walkthrough, commands and what each directive shows: [`polyglot/README.md`](polyglot/README.md)

From the repository root:

```
openshell-annotate scan --root examples/polyglot
```

### Languages

**First-class** (directives and call-site lint): JavaScript, TypeScript (incl. TSX), Python, Go, shell (incl. `Dockerfile`).

**Built-in comment leaders** (directives only): Ruby, Perl, R, Rust, Java, Kotlin, C#, PHP, Swift, Scala, Dart, C, C++.

**Turned on in `[scan.comment_leaders]`** ([config](polyglot/openshell-annotate.toml)): Elixir, Julia, Nim, Crystal, Zig, Groovy, F#, V, Solidity, Terraform.

**Not scanned**: Lua, Erlang, Haskell, OCaml, Clojure, Visual Basic. Their sources are in the tree, and their directives live in [`annotations/unscanned.sh`](polyglot/annotations/unscanned.sh).
