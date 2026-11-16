#!/usr/bin/env bash
# Regression test for #77: `actions/lib/setup.sh resolve` must turn `latest` into the concrete release tag that
# GitHub redirects /releases/latest to, so the setup/check cache key changes when a new stable release ships.
# Runs a local HTTP server that imitates the redirect; needs bash, curl and python3. Used by actions-selftest.yml.
#   tests/actions/setup-resolve.sh
set -euo pipefail
cd "$(dirname "$0")/../.."

tmp="$(mktemp -d)"
server_pid=""
cleanup() {
  if [ -n "$server_pid" ]; then kill "$server_pid" 2>/dev/null || true; wait "$server_pid" 2>/dev/null || true; fi
  rm -rf "$tmp"
}
trap cleanup EXIT

# /<case>/latest redirects like GitHub: to /<case>/tag/<tag>, or to /<case> when there is no stable release.
cat >"$tmp/server.py" <<'PY'
import http.server
TAGS = {"stable1": "v0.1.0", "stable2": "v0.2.0", "none": None}
class H(http.server.BaseHTTPRequestHandler):
    def do_HEAD(self):
        parts = self.path.strip("/").split("/")
        if len(parts) == 2 and parts[1] == "latest" and parts[0] in TAGS:
            tag = TAGS[parts[0]]
            self.send_response(302)
            self.send_header("Location", f"/{parts[0]}/tag/{tag}" if tag else f"/{parts[0]}")
        else:
            self.send_response(200)
        self.send_header("Content-Length", "0")
        self.end_headers()
    do_GET = do_HEAD
    def log_message(self, *a): pass
s = http.server.HTTPServer(("127.0.0.1", 0), H)
print(s.server_port, flush=True)
s.serve_forever()
PY
python3 "$tmp/server.py" >"$tmp/port" &
server_pid=$!
for _ in $(seq 50); do [ -s "$tmp/port" ] && break; sleep 0.1; done
port="$(cat "$tmp/port")"
[ -n "$port" ] || { echo "server did not start" >&2; exit 1; }

fail=0
# expect <case> <input version> <want version> <want cacheable>
expect() {
  local out="$tmp/out" got_v got_c
  : >"$out"
  GITHUB_OUTPUT="$out" OPENSHELL_ANNOTATE_BASE_URL="http://127.0.0.1:$port/$1" OPENSHELL_ANNOTATE_VERSION="$2" \
    bash actions/lib/setup.sh resolve >/dev/null
  got_v="$(sed -n 's/^version=//p' "$out")"
  got_c="$(sed -n 's/^cacheable=//p' "$out")"
  if [ "$got_v" = "$3" ] && [ "$got_c" = "$4" ]; then
    echo "ok   $1 version=$2 -> $got_v cacheable=$got_c"
  else
    echo "FAIL $1 version=$2 -> $got_v cacheable=$got_c (want $3 cacheable=$4)"
    fail=1
  fi
}

expect stable1 latest v0.1.0 true
expect stable2 latest v0.2.0 true          # a newer stable release gives a new cache key
expect stable1 "" v0.1.0 true              # empty means latest
expect none latest latest false            # no stable release yet: skip the cache, install.sh reports the error
expect stable1 v0.1.0-rc.3 v0.1.0-rc.3 true
expect stable1 0.1.0-rc.3 v0.1.0-rc.3 true
expect stable1 source source false
exit "$fail"
