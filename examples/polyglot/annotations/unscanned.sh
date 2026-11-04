#!/bin/sh
# Languages whose comments are neither `#` nor `//` are not scanned.
# Their sources stay in the tree so `scan` reports them, and the directives live here.
# Each `bin=` is the executable that would open the connection, not this shell.

# src/lua/client.lua uses `--`
# @openshell egress lua.example.com:443 reason="lua client (src/lua/client.lua)" bin=/usr/bin/lua access=read-only

# src/erlang/client.erl uses `%`
# @openshell egress erlang.org:443 reason="erlang client (src/erlang/client.erl)" bin=/usr/bin/escript access=read-only

# src/haskell/Client.hs uses `--`
# @openshell egress hackage.haskell.org:443 reason="hackage (src/haskell/Client.hs)" bin=/opt/desk/haskell-tool access=read-only

# src/ocaml/client.ml uses (* *)
# @openshell egress opam.ocaml.org:443 reason="opam (src/ocaml/client.ml)" bin=/usr/bin/ocaml access=read-only

# src/clojure/client.clj uses `;`
# @openshell egress repo.clojars.org:443 reason="clojars (src/clojure/client.clj)" bin=/usr/bin/java access=read-only

# src/vb/Client.vb uses '
# @openshell egress www.nuget.org:443 reason="NuGet from VB (src/vb/Client.vb)" bin=/usr/bin/dotnet access=read-only
