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

pub const TOOL: &str = concat!("openshell-annotate ", env!("CARGO_PKG_VERSION"));
const MAX_SOURCES_SHOWN: usize = 6;

pub struct Generated {
    pub policy_yaml: String,
    pub doc: PolicyDocument,
    pub providers: ProvidersOut,
    pub warnings: Vec<String>,
    pub stats: Stats,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Stats {
    pub directives: usize,
    pub network_rules: usize,
    pub endpoints: usize,
    pub binaries: usize,
    pub fs_read: usize,
    pub fs_write: usize,
    pub providers: usize,
    pub unmapped_providers: usize,
}

pub fn miette_str(e: &miette::Report) -> String {
    e.chain()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(": ")
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct Spec {
    protocol: Option<String>,
    access: Option<String>,
    enforcement: Option<String>,
    tls: Option<String>,
    allow: Vec<(String, String)>,
    deny: Vec<(String, String)>,
}

struct Ep {
    spec: Spec,
    spec_origin: String,
    sources: Vec<(String, String)>, // (loc, reason)
}

#[derive(Default)]
struct RuleAcc {
    eps: BTreeMap<(String, Vec<u16>), Ep>,
    bins: BTreeMap<String, String>, // path -> origin
}

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

fn split_rule(s: &str) -> (String, String) {
    let (m, p) = s.split_once(':').unwrap();
    (m.to_string(), p.to_string())
}

fn spec_of(d: &Directive, cfg: &Config) -> Spec {
    let allow: Vec<_> = d
        .attrs
        .multi
        .get("allow")
        .map(|v| v.iter().map(|s| split_rule(s)).collect())
        .unwrap_or_default();
    let deny: Vec<_> = d
        .attrs
        .multi
        .get("deny")
        .map(|v| v.iter().map(|s| split_rule(s)).collect())
        .unwrap_or_default();
    let access = d.attr("access").map(str::to_string);
    let inspected = access.is_some() || !allow.is_empty() || !deny.is_empty();
    let tls = d.attr("tls").map(str::to_string);
    let protocol = match d.attr("protocol") {
        Some(p) => Some(p.to_string()),
        None if inspected => Some("rest".to_string()),
        None => None,
    };
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

fn qs(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

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

fn clean(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > 160 {
        format!("{}...", t.chars().take(157).collect::<String>())
    } else {
        t
    }
}

fn comment_lines(sources: &[(String, String)], indent: &str, out: &mut String) {
    for (loc, reason) in sources.iter().take(MAX_SOURCES_SHOWN) {
        if reason.is_empty() {
            out.push_str(&format!("{indent}# {loc}\n"));
        } else {
            out.push_str(&format!("{indent}# {} ({loc})\n", clean(reason)));
        }
    }
    if sources.len() > MAX_SOURCES_SHOWN {
        out.push_str(&format!(
            "{indent}# ... and {} more call sites\n",
            sources.len() - MAX_SOURCES_SHOWN
        ));
    }
}

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
            if !s.allow.is_empty() {
                y.push_str("        rules:\n");
                for (m, p) in &s.allow {
                    y.push_str(&format!(
                        "          - allow:\n              method: {}\n              path: {}\n",
                        qs(m),
                        qs(p)
                    ));
                }
            }
            if !s.deny.is_empty() {
                y.push_str("        deny_rules:\n");
                for (m, p) in &s.deny {
                    y.push_str(&format!(
                        "          - method: {}\n            path: {}\n",
                        qs(m),
                        qs(p)
                    ));
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
