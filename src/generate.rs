//! Directives -> policy.yaml (with provenance comments) + providers output.
//!
//! The YAML is written as text on purpose: `openshell_policy_schema::serialize_policy` drops
//! comments and sorts keys. Every output is re-parsed with `openshell_policy_schema::parse_policy`
//! (unknown fields, duplicate keys, version, size limits) and passed through `validate::check_document`.

use crate::config::Config;
use crate::directive::{Directive, Kind};
use crate::providers::{self, Profile, ProvidersOut};
use crate::validate;
use openshell_policy_schema::{parse_policy, PolicyDocument};
use std::collections::{BTreeMap, BTreeSet};

/// Tool name and version, written into the header of generated files.
pub const TOOL: &str = concat!("openshell-annotate ", env!("CARGO_PKG_VERSION"));
/// Most source locations listed in the comment above one policy entry; the rest are counted.
const MAX_SOURCES_SHOWN: usize = 6;

/// Output of [`generate`].
pub struct Generated {
    /// The policy as written, with provenance comments.
    pub policy_yaml: String,
    /// [`Generated::policy_yaml`] parsed back by `openshell_policy_schema`.
    pub doc: PolicyDocument,
    pub providers: ProvidersOut,
    pub warnings: Vec<String>,
    pub stats: Stats,
}

/// Counts for the `generate` summary.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Stats {
    /// All directives passed in, of every kind.
    pub directives: usize,
    pub network_rules: usize,
    pub endpoints: usize,
    /// Binary entries summed over all network rules.
    pub binaries: usize,
    pub fs_read: usize,
    pub fs_write: usize,
    pub providers: usize,
    /// Credentials no provider profile declares.
    pub unmapped_providers: usize,
}

/// Formats a miette report and its whole cause chain on one line, joined with `: `.
pub fn miette_str(e: &miette::Report) -> String {
    e.chain()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(": ")
}

/// Endpoint settings of an egress directive; two directives for one host:port must agree on them.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Spec {
    protocol: Option<String>,
    access: Option<String>,
    /// `protocol=mcp` plus `access=` — MCP rejects access presets, so this is emitted as
    /// `mcp.allow_all_known_mcp_methods: true` instead of `access`.
    mcp_allow_all: bool,
    /// Only set for inspected (non-`tcp`) protocols.
    enforcement: Option<String>,
    tls: Option<String>,
    /// Clauses from `allow=`. HTTP rules fill method and path; GraphQL rules fill operation type.
    allow: Vec<L7Clause>,
    /// Clauses from `deny=`. HTTP rules fill method and path; GraphQL rules fill operation type.
    deny: Vec<L7Clause>,
}

/// One `allow=` / `deny=` clause. HTTP clauses set method and path. GraphQL clauses set
/// `operation_type` (and optionally `operation_name`) and leave method and path empty, so
/// `operation_type` is never emitted for any other protocol.
#[derive(Clone, PartialEq, Eq, Debug)]
struct L7Clause {
    method: String,
    path: String,
    operation_type: String,
    operation_name: String,
}

/// One endpoint being assembled: its settings and every directive that asks for it.
struct Ep {
    spec: Spec,
    /// Location of the directive that first set [`Ep::spec`], for conflict messages.
    spec_origin: String,
    sources: Vec<(String, String)>, // (loc, reason)
}

/// One `network_policies` rule being assembled.
#[derive(Default)]
struct RuleAcc {
    /// Endpoints keyed by host and sorted ports.
    eps: BTreeMap<(String, Vec<u16>), Ep>,
    bins: BTreeMap<String, String>, // path -> origin
}

/// Returns the default rule key for a host: lower-cased ASCII alphanumerics, every other run of characters collapsed
/// to one `_`, trimmed; `endpoint` if nothing is left.
pub(crate) fn slug(host: &str) -> String {
    let mut s = String::new();
    let mut last_us = true;
    for c in host.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
            last_us = false;
        } else if !last_us {
            s.push('_');
            last_us = true;
        }
    }
    let s = s.trim_matches('_').to_string();
    if s.is_empty() {
        "endpoint".into()
    } else {
        s
    }
}

/// Splits a validated `METHOD:/path` value at its first `:`.
///
/// # Panics
///
/// Panics if `s` has no `:`; [`validate::check_directive`] rejects such values before generation.
fn http_clause(s: &str) -> L7Clause {
    let (m, p) = s.split_once(':').unwrap();
    L7Clause {
        method: m.to_string(),
        path: p.to_string(),
        operation_type: String::new(),
        operation_name: String::new(),
    }
}

/// Splits a validated GraphQL `operation_type` or `operation_type:operationName` value.
///
/// [`validate::check_directive`] only lets `query`, `mutation`, or `subscription` through, with an
/// optional GraphQL operation name.
fn graphql_clause(s: &str) -> L7Clause {
    let (op, name) = match s.split_once(':') {
        Some((op, name)) => (op, name),
        None => (s, ""),
    };
    L7Clause {
        method: String::new(),
        path: String::new(),
        operation_type: op.to_string(),
        operation_name: name.to_string(),
    }
}

/// Writes the fields of one allow or deny clause. `first` is the prefix of the first field
/// (for a deny rule that is `          - `) and `rest` prefixes every later field.
fn push_clause_fields(rule: &L7Clause, first: &str, rest: &str, y: &mut String) {
    let mut leading = first;
    let mut push = |text: String| {
        y.push_str(leading);
        y.push_str(&text);
        y.push('\n');
        leading = rest;
    };
    if !rule.method.is_empty() {
        push(format!("method: {}", qs(&rule.method)));
    }
    if !rule.path.is_empty() {
        push(format!("path: {}", qs(&rule.path)));
    }
    if !rule.operation_type.is_empty() {
        push(format!("operation_type: {}", qs(&rule.operation_type)));
    }
    if !rule.operation_name.is_empty() {
        push(format!("operation_name: {}", qs(&rule.operation_name)));
    }
}

/// Builds the endpoint settings of an egress directive.
///
/// Any of `access`, `allow` or `deny` implies inspection, so the protocol defaults to `rest`; inspected protocols get
/// `enforce=` or else `[policy].enforcement`.
///
/// # Panics
///
/// Panics (via [`http_clause`]) on an HTTP `allow`/`deny` value without `:`, which validation rules out.
/// GraphQL clauses are `query`, `mutation`, or `subscription`, with an optional `:operationName`.
fn spec_of(d: &Directive, cfg: &Config) -> Spec {
    let graphql = d.attr("protocol") == Some("graphql");
    let clause = |s: &String| {
        if graphql {
            graphql_clause(s)
        } else {
            http_clause(s)
        }
    };
    let allow: Vec<_> = d
        .attrs
        .multi
        .get("allow")
        .map(|v| v.iter().map(clause).collect())
        .unwrap_or_default();
    let deny: Vec<_> = d
        .attrs
        .multi
        .get("deny")
        .map(|v| v.iter().map(clause).collect())
        .unwrap_or_default();
    let mut access = d.attr("access").map(str::to_string);
    let inspected = access.is_some() || !allow.is_empty() || !deny.is_empty();
    let tls = d.attr("tls").map(str::to_string);
    let protocol = match d.attr("protocol") {
        Some(p) => Some(p.to_string()),
        None if inspected => Some("rest".to_string()),
        None => None,
    };
    // MCP has no access presets. An `access=` value is the allow-all form, emitted as
    // `mcp.allow_all_known_mcp_methods` rather than `access`.
    let mcp_allow_all = protocol.as_deref() == Some("mcp") && access.is_some();
    if mcp_allow_all {
        access = None;
    }
    let enforcement = match protocol.as_deref() {
        Some("tcp") | None => None,
        Some(_) => Some(
            d.attr("enforce")
                .map(str::to_string)
                .unwrap_or_else(|| cfg.policy.enforcement.clone()),
        ),
    };
    Spec {
        protocol,
        access,
        mcp_allow_all,
        enforcement,
        tls,
        allow,
        deny,
    }
}

/// Two specs differ only in their allow/deny lists and neither uses an `access` preset.
fn mergeable(a: &Spec, b: &Spec) -> bool {
    a.access.is_none()
        && b.access.is_none()
        && a.protocol == b.protocol
        && a.enforcement == b.enforcement
        && a.tls == b.tls
        && (!a.allow.is_empty() || !a.deny.is_empty())
        && (!b.allow.is_empty() || !b.deny.is_empty())
}

/// Joins endpoints of one host whose port lists overlap (`api.nuget.org:443` and `api.nuget.org:443,8443`) into
/// one endpoint with the union of the ports, so a host:port is never listed twice. Settings must agree (allow/deny
/// lists that can be unioned count as agreeing); otherwise it is the same conflict as two directives on one port.
fn merge_overlapping_ports(rkey: &str, acc: &mut RuleAcc, errors: &mut Vec<String>) {
    loop {
        let keys: Vec<(String, Vec<u16>)> = acc.eps.keys().cloned().collect();
        let mut pair = None;
        'outer: for (i, a) in keys.iter().enumerate() {
            for b in &keys[i + 1..] {
                if a.0 == b.0 && a.1.iter().any(|p| b.1.contains(p)) {
                    pair = Some((a.clone(), b.clone()));
                    break 'outer;
                }
            }
        }
        let Some((ka, kb)) = pair else {
            return;
        };
        let a = acc.eps.remove(&ka).expect("key listed above");
        let b = acc.eps.remove(&kb).expect("key listed above");
        if a.spec != b.spec && !mergeable(&a.spec, &b.spec) {
            let shared: Vec<String> =
                ka.1.iter()
                    .filter(|p| kb.1.contains(p))
                    .map(u16::to_string)
                    .collect();
            errors.push(format!(
                "{}: conflicting settings for {}:{} in rule `{rkey}` (first declared at {}); make the directives agree or split them with rule=",
                b.spec_origin,
                ka.0,
                shared.join(","),
                a.spec_origin
            ));
            acc.eps.insert(ka, a);
            acc.eps.insert(kb, b);
            return;
        }
        let mut ports = ka.1;
        ports.extend(kb.1);
        ports.sort_unstable();
        ports.dedup();
        let mut spec = a.spec;
        for r in b.spec.allow {
            if !spec.allow.contains(&r) {
                spec.allow.push(r);
            }
        }
        for r in b.spec.deny {
            if !spec.deny.contains(&r) {
                spec.deny.push(r);
            }
        }
        let mut sources = a.sources;
        sources.extend(b.sources);
        acc.eps.insert(
            (ka.0, ports),
            Ep {
                spec,
                spec_origin: a.spec_origin,
                sources,
            },
        );
    }
}

/// Returns the binary for an egress directive and a description of where it came from.
///
/// In order: the directive's `bin=`; the nearest `@openshell bin` at or above it in the same file; a `scope=file`
/// `bin` anywhere in the file; the `[languages]` entry for its language (or, for the generic `#` / `//` languages,
/// for its file extension).
///
/// # Errors
///
/// Returns a message with the fixes when none of these gives a binary.
pub(crate) fn find_bin(
    d: &Directive,
    bins: &[&Directive],
    cfg: &Config,
) -> Result<(String, String), String> {
    if let Some(b) = d.attr("bin") {
        return Ok((b.to_string(), format!("bin= at {}", d.loc())));
    }
    let mut best: Option<&Directive> = None;
    for b in bins {
        if b.file == d.file && b.line <= d.line && best.is_none_or(|x| b.line > x.line) {
            best = Some(b);
        }
    }
    if best.is_none() {
        best = bins
            .iter()
            .copied()
            .find(|b| b.file == d.file && b.file_scoped());
    }
    if let Some(b) = best {
        if let Kind::Bin { path } = &b.kind {
            return Ok((path.clone(), format!("`bin` directive at {}", b.loc())));
        }
    }
    if let Some(lang) = d.lang {
        // Generic `#` / `//` languages are keyed by file extension (`[languages] rb = "/usr/bin/ruby"`).
        let key = match lang {
            crate::lexer::Language::Hash | crate::lexer::Language::Slash => {
                d.file.rsplit('.').next().unwrap_or("").to_ascii_lowercase()
            }
            _ => lang.config_key().to_string(),
        };
        if let Some(p) = cfg.languages.get(&key) {
            return Ok((p.clone(), format!("[languages].{key} default")));
        }
        return Err(format!(
            "{}: no executable known for this egress: add `bin=/real/path` or a `@openshell bin /real/path` line in the file, or set [languages].{key} in openshell-annotate.toml (an empty `binaries` list allows nothing)",
            d.loc(),
        ));
    }
    Err(format!("{}: cannot determine the executable", d.loc()))
}

/// Quotes `s` as a JSON string, which is also a valid YAML double-quoted scalar.
fn qs(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

/// Returns `k` as a YAML mapping key: bare when it is a plain identifier that YAML cannot read as a boolean or null,
/// quoted otherwise.
fn key(k: &str) -> String {
    let ok = !k.is_empty()
        && k.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !matches!(
            k.to_ascii_lowercase().as_str(),
            "true" | "false" | "yes" | "no" | "null" | "on" | "off" | "y" | "n"
        );
    if ok {
        k.to_string()
    } else {
        qs(k)
    }
}

/// Makes `s` safe for a one-line YAML comment: control characters become spaces and whitespace runs collapse.
fn clean(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Comment text width (columns) at which long reasons are wrapped onto further `#` lines.
const COMMENT_WIDTH: usize = 100;

/// Break `text` at spaces into lines of at most `width` characters; a single longer word stays whole.
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(l) if l.chars().count() + 1 + word.chars().count() <= width => {
                l.push(' ');
                l.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

/// Appends the provenance comment for a policy entry: `# reason (file:line)` per source (or just `# file:line`
/// without a reason), at most [`MAX_SOURCES_SHOWN`] of them, then a count of the rest.
fn comment_lines(sources: &[(String, String)], indent: &str, out: &mut String) {
    for (loc, reason) in sources.iter().take(MAX_SOURCES_SHOWN) {
        if reason.is_empty() {
            out.push_str(&format!("{indent}# {loc}\n"));
        } else {
            // The full reason is kept: it is the audit trail for the rule. Long ones wrap onto further comment lines.
            let text = format!("{} ({loc})", clean(reason));
            let width = COMMENT_WIDTH.saturating_sub(indent.len() + 2).max(40);
            for line in wrap_words(&text, width) {
                out.push_str(&format!("{indent}# {line}\n"));
            }
        }
    }
    if sources.len() > MAX_SOURCES_SHOWN {
        out.push_str(&format!(
            "{indent}# ... and {} more call sites\n",
            sources.len() - MAX_SOURCES_SHOWN
        ));
    }
}

/// Builds the policy YAML and the providers output from the directives.
///
/// Egress directives are grouped into rules by `rule=` or the host slug, endpoints by host and ports; endpoints
/// of one host whose ports overlap are joined, and differing `allow`/`deny` lists for one endpoint are unioned. An
/// `fs read` that is also an `fs write` is emitted only under `read_write`. Baseline system paths are added according
/// to `[policy].system_paths`. The YAML is then re-parsed with `parse_policy` and checked with
/// [`validate::check_document`].
///
/// # Errors
///
/// Returns every problem found: invalid directives (in that case nothing else is checked), egress directives with no
/// known binary, conflicting endpoint settings, different hosts mapping to one rule key, an invalid
/// `[policy].system_paths`, conflicting `process` user/group values, and generated YAML that fails schema parsing or
/// the document checks.
pub fn generate(
    dirs: &[Directive],
    cfg: &Config,
    profiles: &[Profile],
) -> Result<Generated, Vec<String>> {
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for d in dirs {
        for m in validate::check_directive(d) {
            errors.push(format!("{}: {m}", d.loc()));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let bins: Vec<&Directive> = dirs
        .iter()
        .filter(|d| matches!(d.kind, Kind::Bin { .. }))
        .collect();
    let mut rules: BTreeMap<String, RuleAcc> = BTreeMap::new();
    let mut slug_hosts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for d in dirs {
        let Kind::Egress { host, ports } = &d.kind else {
            continue;
        };
        let explicit = d.attr("rule").map(str::to_string);
        let rkey = explicit.clone().unwrap_or_else(|| slug(host));
        if explicit.is_none() {
            slug_hosts
                .entry(rkey.clone())
                .or_default()
                .insert(host.clone());
        }
        let (bin, origin) = match find_bin(d, &bins, cfg) {
            Ok(b) => b,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        let spec = spec_of(d, cfg);
        let acc = rules.entry(rkey.clone()).or_default();
        let mut sorted_ports = ports.clone();
        sorted_ports.sort_unstable();
        let ep = acc
            .eps
            .entry((host.clone(), sorted_ports))
            .or_insert_with(|| Ep {
                spec: spec.clone(),
                spec_origin: d.loc(),
                sources: vec![],
            });
        if ep.spec != spec && mergeable(&ep.spec, &spec) {
            // Same host:port needed by several call sites with different allow/deny lists: the policy is the union.
            for r in spec.allow.iter() {
                if !ep.spec.allow.contains(r) {
                    ep.spec.allow.push(r.clone());
                }
            }
            for r in spec.deny.iter() {
                if !ep.spec.deny.contains(r) {
                    ep.spec.deny.push(r.clone());
                }
            }
        } else if ep.spec != spec {
            errors.push(format!(
                "{}: conflicting settings for {host}:{} in rule `{rkey}` (first declared at {}); make the directives agree or split them with rule=",
                d.loc(),
                ports.iter().map(u16::to_string).collect::<Vec<_>>().join(","),
                ep.spec_origin
            ));
        }
        ep.sources
            .push((d.loc(), d.reason().unwrap_or("").to_string()));
        acc.bins.entry(bin).or_insert(origin);
    }
    for (rkey, acc) in rules.iter_mut() {
        merge_overlapping_ports(rkey, acc, &mut errors);
    }
    for (s, hosts) in &slug_hosts {
        if hosts.len() > 1 {
            errors.push(format!(
                "hosts {} all map to rule key `{s}`; their binaries would combine across every endpoint (OpenShell rules are binaries x endpoints). Add rule=<name> to separate them",
                hosts.iter().map(|h| format!("`{h}`")).collect::<Vec<_>>().join(", ")
            ));
        }
    }

    // Filesystem
    let mut fs_read: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut fs_write: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for d in dirs {
        if let Kind::Fs { write, path } = &d.kind {
            let p = if path.len() > 1 {
                path.trim_end_matches('/').to_string()
            } else {
                path.clone()
            };
            let m = if *write { &mut fs_write } else { &mut fs_read };
            m.entry(p)
                .or_default()
                .push((d.loc(), d.reason().unwrap_or("").to_string()));
        }
    }
    for p in fs_write.keys() {
        if fs_read.remove(p).is_some() {
            warnings.push(format!(
                "fs read {p} is subsumed by fs write {p}; emitted only under read_write"
            ));
        }
    }

    // Baseline system paths the supervisor does not add for policies without network rules.
    let sp = cfg.policy.system_paths.as_str();
    if !matches!(sp, "auto" | "always" | "never") {
        errors.push(format!(
            "[policy].system_paths = \"{sp}\": expected auto, always or never"
        ));
    }
    let add_baseline = match sp {
        "always" => true,
        "auto" => rules.is_empty(),
        _ => false,
    };
    if add_baseline {
        let why = "OpenShell proxy baseline: needed to exec anything; the supervisor adds it only when network rules exist".to_string();
        for p in ["/usr", "/lib", "/etc", "/proc", "/dev/urandom"] {
            if !fs_read.contains_key(p) && !fs_write.contains_key(p) {
                fs_read
                    .entry(p.to_string())
                    .or_default()
                    .push(("[policy].system_paths".to_string(), why.clone()));
            }
        }
        for p in ["/tmp", "/dev/null"] {
            if !fs_write.contains_key(p) {
                fs_read.remove(p);
                fs_write
                    .entry(p.to_string())
                    .or_default()
                    .push(("[policy].system_paths".to_string(), why.clone()));
            }
        }
    }

    // Process
    let mut user: Option<(String, String)> = None;
    let mut group: Option<(String, String)> = None;
    for d in dirs {
        if matches!(d.kind, Kind::Process) {
            for (k, slot) in [("user", &mut user), ("group", &mut group)] {
                if let Some(v) = d.attr(k) {
                    match slot {
                        Some((old, at)) if old != v => errors.push(format!(
                            "{}: process {k}={v} conflicts with {old} at {at}",
                            d.loc()
                        )),
                        Some(_) => {}
                        None => *slot = Some((v.to_string(), d.loc())),
                    }
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let prov = providers::resolve(dirs, cfg, profiles);
    for u in &prov.unmapped {
        warnings.push(format!(
            "provider {} has no profile (used at {}); see providers output",
            u.env,
            u.sources.join(", ")
        ));
    }
    for p in &prov.providers {
        for pe in &p.profile_endpoints {
            for (rk, acc) in &rules {
                for (h, ps) in acc.eps.keys() {
                    if ps.iter().any(|port| format!("{h}:{port}") == *pe) {
                        // Different binaries: the profile does not cover this app, so the explicit rule is needed.
                        if !p.profile_binaries.is_empty()
                            && !acc.bins.keys().any(|b| p.profile_binaries.contains(b))
                        {
                            continue;
                        }
                        warnings.push(format!(
                            "egress {pe} (rule `{rk}`) duplicates an endpoint the `{}` provider profile adds when attached; keep it only if you want an explicit rule for the binaries listed",
                            p.profile.as_deref().unwrap_or(&p.instance)
                        ));
                    }
                }
            }
        }
    }

    // Emit
    let mut y = String::new();
    y.push_str(&format!("# Generated by {TOOL} - DO NOT EDIT. Change the @openshell directives in source and regenerate.\n"));
    y.push_str("# Validated against openshell-policy-schema (OpenShell v0.1.6). Each entry's comment gives the reason and source file:line.\n");
    y.push_str("# Startup-only sections (filesystem_policy, landlock, process) need a sandbox recreate to change; network_policies are live.\n");
    y.push_str("version: 1\n");
    if !fs_read.is_empty() || !fs_write.is_empty() {
        y.push_str("filesystem_policy:\n");
        y.push_str(&format!(
            "  include_workdir: {}\n",
            cfg.policy.include_workdir
        ));
        for (name, m) in [("read_only", &fs_read), ("read_write", &fs_write)] {
            if m.is_empty() {
                continue;
            }
            y.push_str(&format!("  {name}:\n"));
            for (p, srcs) in m {
                comment_lines(srcs, "    ", &mut y);
                y.push_str(&format!("    - {}\n", qs(p)));
            }
        }
    }
    if user.is_some() || group.is_some() {
        y.push_str("process:\n");
        if let Some((u, at)) = &user {
            y.push_str(&format!("  # {at}\n  run_as_user: {}\n", qs(u)));
        }
        if let Some((g, at)) = &group {
            y.push_str(&format!("  # {at}\n  run_as_group: {}\n", qs(g)));
        }
    }
    let mut stats = Stats {
        directives: dirs.len(),
        ..Stats::default()
    };
    if !rules.is_empty() {
        y.push_str("network_policies:\n");
    }
    for (rk, acc) in &rules {
        stats.network_rules += 1;
        y.push_str(&format!("  {}:\n", key(rk)));
        y.push_str("    endpoints:\n");
        for ((host, ports), ep) in &acc.eps {
            stats.endpoints += 1;
            comment_lines(&ep.sources, "      ", &mut y);
            y.push_str(&format!("      - host: {}\n", qs(host)));
            if ports.len() == 1 {
                y.push_str(&format!("        port: {}\n", ports[0]));
            } else {
                y.push_str(&format!(
                    "        ports: [{}]\n",
                    ports
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            let s = &ep.spec;
            if let Some(p) = &s.protocol {
                y.push_str(&format!("        protocol: {p}\n"));
            }
            if let Some(t) = &s.tls {
                y.push_str(&format!("        tls: {t}\n"));
            }
            if let Some(e) = &s.enforcement {
                y.push_str(&format!("        enforcement: {e}\n"));
            }
            if let Some(a) = &s.access {
                y.push_str(&format!("        access: {a}\n"));
            }
            if s.mcp_allow_all {
                y.push_str("        mcp:\n          allow_all_known_mcp_methods: true\n");
            }
            if !s.allow.is_empty() {
                y.push_str("        rules:\n");
                for rule in &s.allow {
                    y.push_str("          - allow:\n");
                    push_clause_fields(rule, "              ", "              ", &mut y);
                }
            }
            if !s.deny.is_empty() {
                y.push_str("        deny_rules:\n");
                for rule in &s.deny {
                    push_clause_fields(rule, "          - ", "            ", &mut y);
                }
            }
        }
        y.push_str("    binaries:\n");
        for (b, origin) in &acc.bins {
            stats.binaries += 1;
            y.push_str(&format!("      - path: {}  # {origin}\n", qs(b)));
        }
    }
    stats.fs_read = fs_read.len();
    stats.fs_write = fs_write.len();
    stats.providers = prov.providers.len();
    stats.unmapped_providers = prov.unmapped.len();

    let doc = match parse_policy(&y) {
        Ok(d) => d,
        Err(e) => {
            return Err(vec![format!(
                "internal error: generated YAML failed openshell-policy-schema validation: {}",
                miette_str(&e)
            )])
        }
    };
    let (errs, warns) = validate::check_document(&doc);
    if !errs.is_empty() {
        return Err(errs);
    }
    warnings.extend(warns);
    if dirs.iter().all(|d| {
        !matches!(
            d.kind,
            Kind::Egress { .. } | Kind::Fs { .. } | Kind::Process
        )
    }) {
        warnings.push("no egress/fs/process directives found: the generated policy is empty (OpenShell then applies its built-in restrictive default)".into());
    }
    Ok(Generated {
        policy_yaml: y,
        doc,
        providers: prov,
        warnings,
        stats,
    })
}

#[cfg(test)]
mod wrap_tests {
    use super::*;

    #[test]
    fn wraps_at_words_and_never_drops_text() {
        let text = "alpha beta gamma delta epsilon zeta eta theta";
        let lines = wrap_words(text, 16);
        assert_eq!(lines.join(" "), text);
        assert!(lines.iter().all(|l| l.chars().count() <= 16), "{lines:?}");
        assert_eq!(wrap_words("short", 16), vec!["short"]);
        // one overlong word stays whole instead of being cut
        assert_eq!(
            wrap_words("a verylongwordhere b", 5),
            vec!["a", "verylongwordhere", "b"]
        );
    }
}
