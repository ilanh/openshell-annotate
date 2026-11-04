#!/usr/bin/env bash
# @openshell egress github.com:443 reason="clone the pinned tools repo" access=read-only bin=/usr/bin/git
git clone --depth 1 https://github.com/example/tools.git /workspace/tools

# @openshell egress github.com:443 reason="fetch the release tarball" access=read-only bin=/usr/bin/curl
curl -fsSL "https://github.com/example/tools/archive/refs/tags/v1.tar.gz" -o /tmp/tools.tar.gz

# A quoted heredoc is data, not a comment. The marker inside is not a directive.
cat <<'EOF'
# @openshell egress not-a-directive.test:443 reason="inside a heredoc"
EOF
