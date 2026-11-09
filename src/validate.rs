//! Validation that mirrors OpenShell's authored-policy rules.
//!
//! Authoritative checks live in `openshell-policy-schema` (parsing, unknown fields) and in
//! [`gateway_violations`], which calls `openshell_policy::validate_and_canonicalize_sandbox_policy`
//! (OpenShell v0.1.7, the same tag as the schema crate). This module also re-implements the
//! file-level semantic rules documented in `docs/how-it-works/policies/schema.mdx` so errors carry
//! directive locations. `generate` round-trips its output through `parse_policy` and the gateway
//! validator.

use crate::directive::{Directive, Kind};
use openshell_policy_schema::PolicyDocument;

/// Longest filesystem path OpenShell accepts, in bytes.
pub const MAX_PATH_LEN: usize = 4096;
/// Most paths a `filesystem_policy` may list (`read_only` and `read_write` together).
pub const MAX_FS_PATHS: usize = 256;
/// Values of `egress protocol=`.
pub const PROTOCOLS: &[&str] = &["rest", "websocket", "graphql", "mcp", "json-rpc", "tcp"];

/// Checks that `host` is a destination OpenShell can authorize.
///
/// Brackets around an IPv6 literal are ignored. Wildcards need at least three labels; `**` may only be the whole
/// first label, and `*` may appear inside the first label or as a whole later label.
///
/// # Errors
///
/// Returns a message when the host is empty, loopback, link-local or unspecified, contains characters other than
/// ASCII letters, digits, `.`, `-`, `_`, `*` and `:`, or breaks the wildcard rules.
pub fn check_host(host: &str) -> Result<(), String> {
    let h = host.trim_matches(|c| c == '[' || c == ']');
    if h.is_empty() {
        return Err("empty host".into());
    }
    if h == "localhost"
        || h.starts_with("127.")
        || h == "0.0.0.0"
        || h.starts_with("169.254.")
        || h == "::1"
        || h == "::"
        || h.starts_with("fe80:")
    {
        return Err(format!("`{host}` is loopback, link-local or unspecified; OpenShell never authorizes these destinations (a sandbox-local service needs no network rule)"));
    }
    if h.chars()
        .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '*' | ':')))
    {
        return Err(format!(
            "`{host}` contains characters that are not valid in a host name or wildcard"
        ));
    }
    if h.contains('*') {
        let labels: Vec<&str> = h.split('.').collect();
        if labels.len() < 3 {
            return Err(format!("wildcard host `{host}` must have at least three DNS labels (TLD wildcards such as `*.com` are rejected)"));
        }
        for (i, l) in labels.iter().enumerate() {
            if l.contains("**") && !(i == 0 && *l == "**") {
                return Err(format!(
                    "`**` is allowed only as the whole first label of `{host}`"
                ));
            }
            if l.contains('*') && i > 0 && *l != "*" {
                return Err(format!(
                    "`*` in `{host}` may appear inside the first label or as a whole later label"
                ));
            }
        }
    }
    Ok(())
}

/// Checks a `filesystem_policy` path; `write` is true for `read_write` entries.
///
/// # Errors
///
/// Returns a message when the path is not absolute, contains a `..` component, is longer than [`MAX_PATH_LEN`], or
/// is the root `/` in a write entry.
pub fn check_fs_path(path: &str, write: bool) -> Result<(), String> {
    if !path.starts_with('/') {
        return Err(format!("`{path}` must be an absolute path (OpenShell rejects relative filesystem paths; write the in-sandbox path)"));
    }
    if path.split('/').any(|c| c == "..") {
        return Err(format!("`{path}` must not contain `..`"));
    }
    if path.len() > MAX_PATH_LEN {
        return Err(format!("path exceeds {MAX_PATH_LEN} bytes"));
    }
    if write && path.trim_end_matches('/').is_empty() {
        return Err("read_write cannot contain `/`".into());
    }
    Ok(())
}

/// Checks a `run_as_user` / `run_as_group` value.
///
/// # Errors
///
/// Returns a message unless `v` is `sandbox` or a numeric id from 1 to 4294967294 (so root, `0`, is rejected).
pub fn check_identity(v: &str) -> Result<(), String> {
    if v == "sandbox" {
        return Ok(());
    }
    match v.parse::<u64>() {
        Ok(n) if (1..=4_294_967_294).contains(&n) => Ok(()),
        _ => Err(format!(
            "`{v}` must be `sandbox` or a numeric id from 1 to 4294967294 (root is rejected)"
        )),
    }
}

/// Checks an `allow=` / `deny=` value (`what` names the attribute in the message).
///
/// # Errors
///
/// Returns a message unless `s` is `METHOD:path` with an upper-case (or `_`) method or `*`, and a path starting with
/// `/` or `*`.
fn check_allow(s: &str, what: &str) -> Result<(), String> {
    let Some((m, p)) = s.split_once(':') else {
        return Err(format!("{what}=\"{s}\": expected METHOD:/path"));
    };
    let ok_m = m == "*" || (!m.is_empty() && m.chars().all(|c| c.is_ascii_uppercase() || c == '_'));
    if !ok_m {
        return Err(format!(
            "{what}=\"{s}\": method must be upper-case (GET, POST, ...) or `*`"
        ));
    }
    if !(p.starts_with('/') || p.starts_with('*')) {
        return Err(format!("{what}=\"{s}\": path must start with `/` or `*`"));
    }
    Ok(())
}

/// Checks a GraphQL `allow=` / `deny=` value: `query`, `mutation`, or `subscription`, optionally
/// `:operationName`. `operation_type` is GraphQL-only, so other protocols keep [`check_allow`].
fn check_graphql_op(s: &str, what: &str) -> Result<(), String> {
    let (op, name) = match s.split_once(':') {
        Some((op, name)) => (op, name),
        None => (s, ""),
    };
    if !matches!(op, "query" | "mutation" | "subscription") {
        return Err(format!(
            "{what}=\"{s}\": graphql operation_type must be query, mutation, or subscription"
        ));
    }
    if s.contains(':') && !graphql_name(name) {
        return Err(format!(
            "{what}=\"{s}\": graphql operation name must match [_A-Za-z][_0-9A-Za-z]*"
        ));
    }
    Ok(())
}

fn graphql_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// Directive-level checks (location is prepended by callers).
pub fn check_directive(d: &Directive) -> Vec<String> {
    let mut e = Vec::new();
    let reason_ok = d.reason().map(|r| !r.trim().is_empty()).unwrap_or(false);
    match &d.kind {
        Kind::Egress { host, .. } => {
            if !reason_ok {
                e.push(
                    "egress requires reason=\"...\" (it becomes the comment in policy.yaml)".into(),
                );
            }
            if let Err(m) = check_host(host) {
                e.push(m);
            }
            if let Some(a) = d.attr("access") {
                if !matches!(a, "read-only" | "read-write" | "full") {
                    e.push(format!(
                        "access must be read-only, read-write or full, got `{a}`"
                    ));
                }
            }
            if let Some(p) = d.attr("protocol") {
                if !PROTOCOLS.contains(&p) {
                    e.push(format!(
                        "protocol must be one of {}, got `{p}`",
                        PROTOCOLS.join(", ")
                    ));
                }
            }
            if let Some(x) = d.attr("enforce") {
                if !matches!(x, "enforce" | "audit") {
                    e.push(format!("enforce must be `enforce` or `audit`, got `{x}`"));
                }
            }
            if let Some(t) = d.attr("tls") {
                if t != "skip" {
                    e.push(format!("tls must be `skip` when present, got `{t}`"));
                }
            }
            if let Some(b) = d.attr("bin") {
                if !b.starts_with('/') {
                    e.push(format!("bin must be an absolute real path, got `{b}`"));
                }
            }
            if let Some(r) = d.attr("rule") {
                if r.is_empty()
                    || r.starts_with("_provider_")
                    || !r
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    e.push(format!("rule `{r}` must match [A-Za-z0-9_-]+ and not start with `_provider_` (reserved by OpenShell)"));
                }
            }
            let allows = d.attrs.multi.get("allow").map(Vec::as_slice).unwrap_or(&[]);
            let denies = d.attrs.multi.get("deny").map(Vec::as_slice).unwrap_or(&[]);
            let graphql = d.attr("protocol") == Some("graphql");
            for a in allows {
                let check = if graphql {
                    check_graphql_op(a, "allow")
                } else {
                    check_allow(a, "allow")
                };
                if let Err(m) = check {
                    e.push(m);
                }
            }
            for a in denies {
                let check = if graphql {
                    check_graphql_op(a, "deny")
                } else {
                    check_allow(a, "deny")
                };
                if let Err(m) = check {
                    e.push(m);
                }
            }
            if d.attr("access").is_some() && !allows.is_empty() {
                e.push("`access` and `allow` are mutually exclusive in OpenShell (access presets vs explicit rules)".into());
            }
            if d.attr("protocol") == Some("mcp")
                && allows.is_empty()
                && denies.is_empty()
                && d.attr("access").is_none()
            {
                e.push(
                    "protocol mcp needs allow= rules or access= (access is emitted as mcp.allow_all_known_mcp_methods: true, never as an access preset)"
                        .into(),
                );
            }
            if let Some(p) = d.attr("protocol") {
                if matches!(p, "rest" | "websocket" | "graphql")
                    && allows.is_empty()
                    && denies.is_empty()
                    && d.attr("access").is_none()
                {
                    e.push(format!(
                        "protocol {p} requires allow= rules or an access preset to define allowed traffic"
                    ));
                }
            }
            let inspected = d.attr("access").is_some() || !allows.is_empty() || !denies.is_empty();
            if inspected && d.attr("protocol") == Some("tcp") {
                e.push(
                    "access/allow/deny need request inspection; protocol=tcp is uninspected L4"
                        .into(),
                );
            }
            if d.attr("tls") == Some("skip")
                && (inspected || d.attr("protocol").is_some_and(|p| p != "tcp"))
            {
                e.push("tls=skip relays without TLS termination, so it cannot be combined with request inspection".into());
            }
            if !denies.is_empty() && allows.is_empty() && d.attr("access").is_none() {
                e.push(
                    "deny without allow/access: deny rules only subtract from allowed requests"
                        .into(),
                );
            }
        }
        Kind::Fs { write, path } => {
            if *write && !reason_ok {
                e.push("fs write requires reason=\"...\"".into());
            }
            if let Err(m) = check_fs_path(path, *write) {
                e.push(m);
            }
        }
        Kind::Bin { path } => {
            if !path.starts_with('/') {
                e.push(format!("bin must be an absolute real path, got `{path}`"));
            }
        }
        Kind::Process => {
            if d.attr("user").is_none() && d.attr("group").is_none() {
                e.push("process needs user= and/or group=".into());
            }
            for k in ["user", "group"] {
                if let Some(v) = d.attr(k) {
                    if let Err(m) = check_identity(v) {
                        e.push(format!("{k}: {m}"));
                    }
                }
            }
        }
        Kind::Ignore => {
            if !reason_ok {
                e.push(
                    "ignore requires reason=\"...\" (explain why this call needs no policy entry)"
                        .into(),
                );
            }
            if let Some(h) = d.attr("host") {
                if d.attr("kind") != Some("net") {
                    e.push("ignore host=... needs kind=net".into());
                }
                if let Err(m) = check_host(h) {
                    e.push(format!("ignore host: {m}"));
                }
            }
        }
        Kind::Provider { .. } => {
            if let Some(p) = d.attr("profile").or(d.attr("name")) {
                if p.is_empty()
                    || !p
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                {
                    e.push(format!("`{p}` must match [A-Za-z0-9_-]+"));
                }
            }
        }
    }
    e
}

/// Document-level semantic checks on a parsed policy. Returns (errors, warnings).
pub fn check_document(doc: &PolicyDocument) -> (Vec<String>, Vec<String>) {
    let mut errs = Vec::new();
    let mut warns = Vec::new();
    if let Some(fs) = &doc.filesystem_policy {
        let total = fs.read_only.len() + fs.read_write.len();
        if total > MAX_FS_PATHS {
            errs.push(format!(
                "filesystem_policy lists {total} paths; the limit is {MAX_FS_PATHS}"
            ));
        }
        for p in &fs.read_only {
            if let Err(m) = check_fs_path(p, false) {
                errs.push(format!("filesystem_policy.read_only: {m}"));
            }
        }
        for p in &fs.read_write {
            if let Err(m) = check_fs_path(p, true) {
                errs.push(format!("filesystem_policy.read_write: {m}"));
            }
        }
    }
    if let Some(p) = &doc.process {
        for (k, v) in [
            ("run_as_user", &p.run_as_user),
            ("run_as_group", &p.run_as_group),
        ] {
            if !v.is_empty() {
                if let Err(m) = check_identity(v) {
                    errs.push(format!("process.{k}: {m}"));
                }
            }
        }
    }
    for (key, rule) in &doc.network_policies {
        if key.starts_with("_provider_") {
            errs.push(format!(
                "network_policies.{key}: rule keys starting with `_provider_` are reserved"
            ));
        }
        if rule.binaries.is_empty() {
            warns.push(format!("network_policies.{key}: empty `binaries` matches no executable, so the rule allows nothing"));
        }
        for (i, ep) in rule.endpoints.iter().enumerate() {
            let at = format!("network_policies.{key}.endpoints[{i}]");
            if ep.host.is_empty() && ep.allowed_ips.is_empty() {
                errs.push(format!("{at}: needs `host` (or `allowed_ips`)"));
            }
            if !ep.host.is_empty() {
                if let Err(m) = check_host(&ep.host) {
                    errs.push(format!("{at}: {m}"));
                }
            }
            if ep.port != 0 && !ep.ports.is_empty() {
                errs.push(format!("{at}: `port` and `ports` are mutually exclusive"));
            }
            if ep.effective_ports().is_empty() {
                errs.push(format!("{at}: needs `port` or `ports`"));
            }
            if ep.tls == "skip"
                && !ep.protocol.is_empty()
                && !ep.protocol.eq_ignore_ascii_case("tcp")
            {
                errs.push(format!(
                    "{at}: `tls: skip` cannot be used with a request protocol"
                ));
            }
            if !ep.access.is_empty() && !ep.rules.is_empty() {
                errs.push(format!("{at}: `access` and `rules` are mutually exclusive"));
            }
            if ep.protocol.eq_ignore_ascii_case("tcp")
                && ep.host.parse::<std::net::IpAddr>().is_ok()
            {
                errs.push(format!(
                    "{at}: protocol tcp needs a DNS host, not an IP literal"
                ));
            }
            if ep.is_l4() && (!ep.access.is_empty() || !ep.rules.is_empty()) {
                errs.push(format!(
                    "{at}: access/rules need `protocol` (rest, graphql, ...)"
                ));
            }
        }
    }
    (errs, warns)
}

/// Formats a miette report and its whole cause chain on one line, joined with `: `.
pub fn miette_str(e: &miette::Report) -> String {
    e.chain()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(": ")
}

/// Runs the OpenShell gateway validator on an authored policy.
///
/// `Ok(empty)` means `validate_and_canonicalize_sandbox_policy` accepted it. `Ok(messages)` are
/// the violation strings (the same text the gateway reports). `Err` is a parse or conversion
/// failure.
///
/// # Errors
///
/// Returns the parse error when the YAML is not a sandbox policy the gateway can load.
pub fn gateway_violations(yaml: &str) -> Result<Vec<String>, String> {
    let policy = openshell_policy::parse_sandbox_policy(yaml).map_err(|e| miette_str(&e))?;
    match openshell_policy::validate_and_canonicalize_sandbox_policy(policy) {
        Ok(_) => Ok(Vec::new()),
        Err(e) => Ok(e.violations().iter().map(ToString::to_string).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert!(check_host("api.openai.com").is_ok());
        assert!(check_host("*.example.com").is_ok());
        assert!(check_host("**.example.com").is_ok());
        assert!(check_host("api-*.example.com").is_ok());
        assert!(check_host("*.com").is_err());
        assert!(check_host("a.**.example.com").is_err());
        assert!(check_host("a.b*.example.com").is_err());
        assert!(check_host("localhost").is_err());
        assert!(check_host("169.254.169.254").is_err());
        assert!(check_host("127.0.0.1").is_err());
        assert!(check_host("bad host").is_err());
    }

    #[test]
    fn fs_paths() {
        assert!(check_fs_path("/workspace/data", true).is_ok());
        assert!(check_fs_path("workspace", false).is_err());
        assert!(check_fs_path("/a/../b", false).is_err());
        assert!(check_fs_path("/", true).is_err());
        assert!(check_fs_path("//", true).is_err());
        assert!(check_fs_path("/", false).is_ok());
    }

    #[test]
    fn identities() {
        assert!(check_identity("sandbox").is_ok());
        assert!(check_identity("1500").is_ok());
        assert!(check_identity("0").is_err());
        assert!(check_identity("root").is_err());
        assert!(check_identity("4294967295").is_err());
    }

    #[test]
    fn allow_format() {
        assert!(check_allow("GET:/v1/**", "allow").is_ok());
        assert!(check_allow("*:/**", "allow").is_ok());
        assert!(check_allow("get:/x", "allow").is_err());
        assert!(check_allow("GET", "allow").is_err());
        assert!(check_allow("GET:v1", "allow").is_err());
        assert!(check_graphql_op("query", "allow").is_ok());
        assert!(check_graphql_op("mutation:CreateIssue", "allow").is_ok());
        assert!(check_graphql_op("subscription", "deny").is_ok());
        assert!(check_graphql_op("POST:/graphql", "allow").is_err());
        assert!(check_graphql_op("query:", "allow").is_err());
        assert!(check_graphql_op("query:/path", "allow").is_err());
    }
}
