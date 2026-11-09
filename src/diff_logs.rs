//! `diff-logs`: map OpenShell allow/deny events (shorthand lines from `openshell logs`, or OCSF JSON lines) back to
//! `@openshell` directives by binary, host and port. Missing directives become suggestions (optionally written),
//! directives that never matched an event are reported as stale.
//!
//! Delivery of these logs is best-effort: *absence of a deny or an allow is not proof*. Filesystem denials are not logged by
//! OpenShell 0.1.7 (upstream #4270), so only network and L7 HTTP events are handled.

use crate::config::Config;
use crate::directive::{Directive, Kind};
use crate::generate::find_bin;
use crate::lexer::Language;
use crate::scan::Scan;
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

/// Outcome of a logged connection or request.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Allowed,
    Denied,
    /// `NET:FAIL`: the proxy accepted the connection (usually right after an ALLOWED) and then it failed, e.g. the
    /// client rejected the OpenShell TLS-termination certificate.
    Failed,
}

/// One sandbox network event parsed from a log line.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub action: Action,
    /// `net` (L4 connection) or `http` (L7 request).
    pub class: &'static str,
    /// Absolute path of the process that connected, when the log names one.
    pub binary: Option<String>,
    /// Destination host name (or IP when no name is logged).
    pub host: String,
    pub port: Option<u16>,
    /// HTTP method (L7 events).
    pub method: Option<String>,
    /// Request path without query or fragment (L7 events).
    pub path: Option<String>,
    /// Name of the policy rule that decided, when logged (`-` counts as none).
    pub policy: Option<String>,
    /// The log's reason or status detail.
    pub reason: Option<String>,
    /// Log file name, or `<stdin>`.
    pub source: String,
    /// 1-based line in [`Event::source`].
    pub line: usize,
}

/// Removes ANSI colour escape sequences.
fn strip_ansi(s: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("\x1b\\[[0-9;]*m").unwrap())
        .replace_all(s, "")
        .into_owned()
}

/// Matches a shorthand `NET:OPEN ... ALLOWED|DENIED bin(pid) -> host:port [policy:..] [reason:..]` line.
fn net_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"NET:OPEN \[\w+\] (ALLOWED|DENIED) (\S+?)\((\d+)\) -> (\S+?):(\d+)(?: \[policy:(\S+?) engine:(\w+)\])?(?: \[reason:(.*?)\])?\s*$").unwrap()
    })
}
/// Matches a shorthand `NET:FAIL ... [bin(pid) -> ]host:port` line.
fn fail_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"NET:FAIL \[\w+\] (?:(\S+?)\((\d+)\) -> )?(\S+?):(\d+)(?:\s|$)").unwrap()
    })
}
/// Matches a shorthand `HTTP:<METHOD> ... ALLOWED|DENIED METHOD http(s)://host[:port][/path] ...` line.
fn http_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"HTTP:([A-Z]+) \[\w+\] (ALLOWED|DENIED) ([A-Z]+) (https?)://([^/ :]+)(?::(\d+))?(/\S*)?(?: \[policy:(\S+?) engine:(\w+)\])?(?: \[reason:(.*?)\])?\s*$").unwrap()
    })
}

/// Returns [`Action::Allowed`] for `allowed` (any case), [`Action::Denied`] for anything else.
fn action_of(s: &str) -> Action {
    if s.eq_ignore_ascii_case("allowed") {
        Action::Allowed
    } else {
        Action::Denied
    }
}

/// Cuts a URL path at its query string or fragment.
fn strip_query(p: &str) -> String {
    p.split(['?', '#']).next().unwrap_or(p).to_string()
}

/// Converts one OCSF record into an [`Event`].
///
/// Only network activity (class 4001; activity 4 is a failure) and HTTP activity (class 4002) are used. Returns `None`
/// for other classes, records without a destination host or an action, and the binary is dropped unless it is an
/// absolute path.
///
/// Network "Refuse" records (activity 5, e.g. `status_detail: policy_dns_ineligible` when policy DNS refuses a name the
/// policy does not allow) are skipped as well: they carry no port or process, and the connection attempt that follows
/// is logged separately as a denied Open with host, port and binary. The shorthand parser ignores `NET:REFUSE` too.
fn parse_json_line(v: &serde_json::Value, source: &str, line: usize) -> Option<Event> {
    let class_uid = v.get("class_uid")?.as_u64()?;
    if class_uid != 4001 && class_uid != 4002 {
        return None;
    }
    if class_uid == 4001
        && (v.get("activity_id").and_then(|a| a.as_u64()) == Some(5)
            || v.get("status_detail").and_then(|d| d.as_str()) == Some("policy_dns_ineligible"))
    {
        return None;
    }
    let failed = class_uid == 4001 && v.get("activity_id").and_then(|a| a.as_u64()) == Some(4);
    let action = match v.get("action").and_then(|a| a.as_str()) {
        _ if failed => Action::Failed,
        Some(a) => action_of(a),
        None => match v.get("action_id").and_then(|a| a.as_u64()) {
            Some(1) => Action::Allowed,
            Some(2) => Action::Denied,
            _ => return None,
        },
    };
    let dst = v.get("dst_endpoint");
    let s = |o: Option<&serde_json::Value>, k: &str| {
        o.and_then(|x| x.get(k))
            .and_then(|x| x.as_str())
            .map(str::to_string)
    };
    let http = v.get("http_request");
    let url = http.and_then(|h| h.get("url"));
    let host = s(dst, "domain")
        .or_else(|| s(dst, "hostname"))
        .or_else(|| s(url, "hostname"))
        .or_else(|| s(dst, "ip"))?;
    let port = dst
        .and_then(|d| d.get("port"))
        .and_then(|p| p.as_u64())
        .or_else(|| url.and_then(|u| u.get("port")).and_then(|p| p.as_u64()))
        .map(|p| p as u16);
    let proc_ = v.get("actor").and_then(|a| a.get("process"));
    let binary = proc_
        .and_then(|p| p.get("file"))
        .and_then(|f| f.get("path"))
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .or_else(|| s(proc_, "name"))
        .filter(|b| b.starts_with('/'));
    Some(Event {
        action,
        class: if class_uid == 4002 { "http" } else { "net" },
        binary,
        host,
        port,
        method: s(http, "http_method"),
        path: s(url, "path").map(|p| strip_query(&p)),
        policy: s(v.get("firewall_rule"), "name"),
        reason: v
            .get("status_detail")
            .and_then(|x| x.as_str())
            .map(str::to_string),
        source: source.to_string(),
        line,
    })
}

/// Parse a log file's text: OCSF JSON lines (also when prefixed, e.g. `OCSF-JSON {...}`) and shorthand lines.
///
/// ANSI colours are stripped first; lines that match neither format are ignored. An HTTP shorthand line without a
/// port gets the scheme's default port.
pub fn parse_logs(text: &str, source: &str) -> Vec<Event> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = strip_ansi(raw);
        if let Some(b) = line.find('{') {
            if line[b..].contains("\"class_uid\"") {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line[b..]) {
                    if let Some(e) = parse_json_line(&v, source, i + 1) {
                        out.push(e);
                    }
                    continue;
                }
            }
        }
        if let Some(c) = net_re().captures(&line) {
            out.push(Event {
                action: action_of(&c[1]),
                class: "net",
                binary: Some(c[2].to_string()),
                host: c[4].to_string(),
                port: c[5].parse().ok(),
                method: None,
                path: None,
                policy: c
                    .get(6)
                    .map(|m| m.as_str().to_string())
                    .filter(|p| p != "-"),
                reason: c.get(8).map(|m| m.as_str().to_string()),
                source: source.to_string(),
                line: i + 1,
            });
        } else if let Some(c) = fail_re().captures(&line) {
            out.push(Event {
                action: Action::Failed,
                class: "net",
                binary: c.get(1).map(|m| m.as_str().to_string()),
                host: c[3].to_string(),
                port: c[4].parse().ok(),
                method: None,
                path: None,
                policy: None,
                reason: None,
                source: source.to_string(),
                line: i + 1,
            });
        } else if let Some(c) = http_re().captures(&line) {
            let port = c
                .get(6)
                .and_then(|m| m.as_str().parse().ok())
                .or(Some(if &c[4] == "https" { 443 } else { 80 }));
            out.push(Event {
                action: action_of(&c[2]),
                class: "http",
                binary: None,
                host: c[5].to_string(),
                port,
                method: Some(c[3].to_string()),
                path: Some(strip_query(c.get(7).map(|m| m.as_str()).unwrap_or("/"))),
                policy: c
                    .get(8)
                    .map(|m| m.as_str().to_string())
                    .filter(|p| p != "-"),
                reason: c.get(10).map(|m| m.as_str().to_string()),
                source: source.to_string(),
                line: i + 1,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------------

/// An egress directive with its target and resolved binary, ready to match against events.
#[derive(Debug, Clone)]
pub struct Row {
    pub dir: Directive,
    pub host: String,
    pub ports: Vec<u16>,
    /// The binary the directive resolves to (as for `generate`); `None` when it cannot be determined.
    pub bin: Option<String>,
}

/// Returns one [`Row`] per egress directive of the scan.
pub fn rows(scan: &Scan, cfg: &Config) -> Vec<Row> {
    let bins: Vec<&Directive> = scan
        .directives
        .iter()
        .filter(|d| matches!(d.kind, Kind::Bin { .. }))
        .collect();
    scan.directives
        .iter()
        .filter_map(|d| match &d.kind {
            Kind::Egress { host, ports } => Some(Row {
                dir: d.clone(),
                host: host.clone(),
                ports: ports.clone(),
                bin: find_bin(d, &bins, cfg).ok().map(|b| b.0),
            }),
            _ => None,
        })
        .collect()
}

/// Returns whether `host` matches the directive host `pat`, case-insensitively; a leading `*` matches exactly one
/// label.
pub fn host_matches(pat: &str, host: &str) -> bool {
    let (p, h) = (pat.to_ascii_lowercase(), host.to_ascii_lowercase());
    if let Some(suffix) = p.strip_prefix('*') {
        // `*.example.com` matches exactly one more label.
        return h.ends_with(suffix)
            && h.len() > suffix.len()
            && !h[..h.len() - suffix.len()].contains('.');
    }
    p == h
}

/// The supervisor matches a connection against the binary *and its ancestors*, so a helper spawned by an allowed
/// binary shows up in logs under its own path (git -> git-remote-http).
fn bin_matches(row_bin: &str, ev_bin: &str) -> bool {
    if row_bin == ev_bin {
        return true;
    }
    let base = ev_bin.rsplit('/').next().unwrap_or("");
    row_bin.ends_with("/git") && ev_bin.contains("/git-core/") && base.starts_with("git-")
}

/// Returns the `network_policies` key the row's directive generates (`rule=` or the host slug).
fn row_rule_name(r: &Row) -> String {
    r.dir
        .attr("rule")
        .map(str::to_string)
        .unwrap_or_else(|| crate::generate::slug(&r.host))
}

/// Returns whether the row's host matches and, when the event has a port, the row lists it.
fn row_matches_target(r: &Row, host: &str, port: Option<u16>) -> bool {
    host_matches(&r.host, host) && port.is_none_or(|p| r.ports.contains(&p))
}

/// Returns why a denial can never be fixed by a directive (SSRF protection, loopback, link-local, cloud metadata,
/// the OpenShell control plane), or `None` when it can.
fn non_actionable(e: &Event) -> Option<&'static str> {
    let reason = e.reason.as_deref().unwrap_or("").to_ascii_lowercase();
    if reason.contains("ssrf") || reason.contains("always") || reason.contains("blocked address") {
        return Some("always-blocked destination (SSRF protection)");
    }
    let h = e.host.as_str();
    if h == "localhost"
        || h.starts_with("127.")
        || h.starts_with("169.254.")
        || h == "::1"
        || h == "metadata.google.internal"
    {
        return Some("loopback/link-local/metadata can never be allowed");
    }
    if h == "host.openshell.internal" {
        return Some("OpenShell control plane");
    }
    None
}

/// Something to change so the logged denials (or failures) stop.
#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    /// `missing_egress` | `binary_mismatch` | `l7_allow` | `tls_failure` | `manual`
    pub kind: &'static str,
    pub host: String,
    pub port: Option<u16>,
    pub binary: Option<String>,
    /// Number of events behind the suggestion.
    pub count: usize,
    /// Directive text without the comment leader.
    pub directive: Option<String>,
    /// `file:line` where the directive belongs: the directive it extends, or a source line that mentions the host.
    pub place: Option<String>,
    /// Explanation for the user.
    pub note: String,
}

/// An egress directive that no event in the logs matched.
#[derive(Debug, Clone, Serialize)]
pub struct Stale {
    /// `file:line` of the directive.
    pub directive: String,
    /// `host:port[,port...]`.
    pub target: String,
    pub binary: Option<String>,
}

/// Denials no directive can fix, grouped by destination and reason.
#[derive(Debug, Clone, Serialize)]
pub struct NonActionable {
    pub host: String,
    pub port: Option<u16>,
    pub count: usize,
    pub why: String,
}

/// Denials that match an `@openshell ignore kind=net host=...` directive: deliberately not allowed.
#[derive(Debug, Clone, Serialize)]
pub struct Declined {
    pub host: String,
    pub port: Option<u16>,
    pub count: usize,
    /// `file:line` of the `ignore` directive.
    pub directive: String,
    /// The `ignore` directive's reason.
    pub reason: String,
}

/// Result of [`analyze`].
#[derive(Debug, Default, Serialize)]
pub struct Report {
    /// All parsed events.
    pub events: usize,
    pub denied: usize,
    pub allowed: usize,
    /// `NET:FAIL` events (connection allowed, then failed).
    #[serde(default)]
    pub failed: usize,
    pub suggestions: Vec<Suggestion>,
    pub stale: Vec<Stale>,
    pub non_actionable: Vec<NonActionable>,
    pub declined: Vec<Declined>,
}

/// `GET:/v1/**` style rule vs. a method and path (`**` spans segments, `*` one segment).
fn allow_covers(rule: &str, method: &str, path: &str) -> bool {
    let Some((m, glob)) = rule.split_once(':') else {
        return false;
    };
    if m != "*" && !m.eq_ignore_ascii_case(method) {
        return false;
    }
    let mut re = String::from("^");
    let mut it = glob.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '*' if it.peek() == Some(&'*') => {
                it.next();
                re.push_str(".*");
            }
            '*' => re.push_str("[^/]*"),
            c => re.push_str(&regex::escape(&c.to_string())),
        }
    }
    re.push('$');
    Regex::new(&re).is_ok_and(|r| r.is_match(path))
}

/// Returns the endpoint-setting attributes of a directive (`rule`, `access`, `protocol`, `enforce`, `tls`, `allow`,
/// `deny`) as ` key=value` text, so a suggested directive for another binary keeps the same settings.
fn spec_attrs(d: &Directive) -> String {
    let mut s = String::new();
    for k in ["rule", "access", "protocol", "enforce", "tls"] {
        if let Some(v) = d.attr(k) {
            s.push_str(&format!(" {k}={v}"));
        }
    }
    for k in ["allow", "deny"] {
        for v in d.attrs.multi.get(k).into_iter().flatten() {
            s.push_str(&format!(" {k}=\"{v}\""));
        }
    }
    s
}

/// Returns the `file:line` of the first source line that mentions `host` outside a directive, preferring files that do
/// not look like tests, fixtures or scripts. Files matching `[diff-logs].exclude` are skipped.
fn find_mention(scan: &Scan, host: &str, cfg: &Config) -> Option<String> {
    let mut best: Option<String> = None;
    for src in &scan.sources {
        if cfg
            .diff_logs
            .exclude
            .iter()
            .any(|x| crate::config::path_matches(&src.rel, x))
        {
            continue;
        }
        let test_like = src.rel.contains("test")
            || src.rel.contains("fixture")
            || src.rel.contains("/scripts/");
        for (i, raw) in src.lexed.raw.iter().enumerate() {
            if raw.contains(host) && !raw.contains("@openshell") {
                let loc = format!("{}:{}", src.rel, i + 1);
                if !test_like {
                    return Some(loc);
                }
                best.get_or_insert(loc);
            }
        }
    }
    best
}

/// How many log lines after an ALLOWED connection a `NET:FAIL` is still attributed to it.
const FAIL_WINDOW: usize = 10;

/// `NET:OPEN ALLOWED host:443` followed by `NET:FAIL <synthetic ip>:443`: the policy let the connection through but the
/// client then dropped it. With TLS termination this is almost always a client that does not trust the OpenShell CA.
fn tls_failures(events: &[Event], rows: &[Row], scan: &Scan, cfg: &Config, rep: &mut Report) {
    type Key = (Option<String>, String, Option<u16>);
    let mut groups: BTreeMap<Key, usize> = BTreeMap::new();
    let mut last: Option<&Event> = None;
    for e in events {
        match e.action {
            Action::Allowed if e.class == "net" => last = Some(e),
            Action::Failed => {
                if let Some(a) = last.filter(|a| {
                    a.source == e.source && e.line > a.line && e.line - a.line <= FAIL_WINDOW
                }) {
                    *groups
                        .entry((a.binary.clone(), a.host.clone(), a.port))
                        .or_default() += 1;
                }
            }
            _ => {}
        }
    }
    for ((bin, host, port), count) in groups {
        let row = rows.iter().find(|r| {
            row_matches_target(r, &host, port)
                && match (&r.bin, &bin) {
                    (Some(a), Some(b)) => bin_matches(a, b),
                    _ => true,
                }
        });
        let skip = row.is_some_and(|r| r.dir.attr("tls") == Some("skip"));
        let place = row
            .map(|r| r.dir.loc())
            .or_else(|| find_mention(scan, &host, cfg));
        let (kind, note) = if skip {
            (
                "manual",
                format!(
                    "connection allowed but failed {count}x although tls=skip is set at {}: not a certificate problem; check that the upstream is reachable and the port is right",
                    place.clone().unwrap_or_default()
                ),
            )
        } else {
            (
                "tls_failure",
                format!(
                    "connection was ALLOWED and then failed ({count}x): the client most likely does not trust the OpenShell TLS-termination CA (rustls with bundled webpki-roots, pinned certificates, a Java truststore). Use `tls=skip` on the egress directive (relays without inspection, so L7 rules stop applying) or make the client trust the sandbox CA bundle"
                ),
            )
        };
        rep.suggestions.push(Suggestion {
            kind,
            host,
            port,
            binary: bin,
            count,
            directive: None,
            place,
            note,
        });
    }
}

/// Matches the events against the egress directives of the scan.
///
/// Denials are grouped by binary, host and port. Those matching an `ignore host=` directive are declined, those no
/// directive can fix are non-actionable; the rest become suggestions: a new egress when no directive covers the
/// host:port, the same settings for another binary, `allow=` rules for L7 denials, or a manual note (e.g. when the
/// deployed policy looks stale). Egress directives that no event matched are stale (only when there are events).
pub fn analyze(events: &[Event], scan: &Scan, cfg: &Config) -> Report {
    let rows = rows(scan, cfg);
    let mut rep = Report {
        events: events.len(),
        denied: events.iter().filter(|e| e.action == Action::Denied).count(),
        allowed: events
            .iter()
            .filter(|e| e.action == Action::Allowed)
            .count(),
        failed: events.iter().filter(|e| e.action == Action::Failed).count(),
        ..Default::default()
    };
    tls_failures(events, &rows, scan, cfg, &mut rep);

    // Denials grouped by (binary, host, port).
    type Key = (Option<String>, String, Option<u16>);
    let mut groups: BTreeMap<Key, (usize, BTreeSet<(String, String)>)> = BTreeMap::new();
    let mut na: BTreeMap<(String, Option<u16>, String), usize> = BTreeMap::new();
    let ignores: Vec<(&Directive, &str)> = scan
        .directives
        .iter()
        .filter(|d| matches!(d.kind, Kind::Ignore))
        .filter_map(|d| d.attr("host").map(|h| (d, h)))
        .collect();
    let mut declined: BTreeMap<(String, Option<u16>, String), (usize, String)> = BTreeMap::new();
    for e in events.iter().filter(|e| e.action == Action::Denied) {
        if let Some((d, _)) = ignores.iter().find(|(_, h)| host_matches(h, &e.host)) {
            let g = declined
                .entry((e.host.clone(), e.port, d.loc()))
                .or_insert((0, d.reason().unwrap_or_default().to_string()));
            g.0 += 1;
            continue;
        }
        if let Some(why) = non_actionable(e) {
            *na.entry((e.host.clone(), e.port, why.to_string()))
                .or_default() += 1;
            continue;
        }
        let g = groups
            .entry((e.binary.clone(), e.host.clone(), e.port))
            .or_default();
        g.0 += 1;
        if e.class == "http" {
            g.1.insert((
                e.method.clone().unwrap_or_default(),
                e.path.clone().unwrap_or_default(),
            ));
        }
    }
    for ((host, port, directive), (count, reason)) in declined {
        rep.declined.push(Declined {
            host,
            port,
            count,
            directive,
            reason,
        });
    }
    for ((host, port, why), count) in na {
        rep.non_actionable.push(NonActionable {
            host,
            port,
            count,
            why,
        });
    }

    let mut seen_directives: BTreeSet<String> = BTreeSet::new();
    for ((bin, host, port), (count, l7)) in &groups {
        let matching: Vec<&Row> = rows
            .iter()
            .filter(|r| row_matches_target(r, host, *port))
            .collect();
        let target = format!("{host}:{}", port.map_or("?".into(), |p| p.to_string()));
        let place = find_mention(scan, host, cfg);
        let by = bin.clone().unwrap_or_else(|| "unknown binary".into());
        let bin_attr = bin
            .as_ref()
            .map(|b| format!(" bin={b}"))
            .unwrap_or_default();
        if matching.is_empty() {
            if port.is_none() {
                rep.suggestions.push(Suggestion { kind: "manual", host: host.clone(), port: *port, binary: bin.clone(), count: *count, directive: None, place, note: "denied connection without a port; add an egress directive with the right host:port".into() });
                continue;
            }
            let mut d = format!(
                "@openshell egress {target} reason=\"observed in sandbox log: {by} (confirm it is needed)\"{bin_attr}"
            );
            for (m, p) in l7 {
                if !m.is_empty() {
                    d.push_str(&format!(" allow=\"{m}:{p}\""));
                }
            }
            if seen_directives.insert(d.clone()) {
                rep.suggestions.push(Suggestion {
                    kind: "missing_egress",
                    host: host.clone(),
                    port: *port,
                    binary: bin.clone(),
                    count: *count,
                    directive: Some(d),
                    place,
                    note: if bin.is_none() {
                        "no binary in the event: add bin= or rely on [languages]".into()
                    } else {
                        "no directive covers this host:port".into()
                    },
                });
            }
            continue;
        }
        let bin_ok = bin.as_ref().is_none_or(|b| {
            matching
                .iter()
                .any(|r| r.bin.as_deref().is_some_and(|rb| bin_matches(rb, b)))
        });
        if !bin_ok {
            let r = matching[0];
            let d = format!(
                "@openshell egress {target} reason=\"observed in sandbox log: {by} (confirm it is needed)\"{bin_attr}{}",
                spec_attrs(&r.dir)
            );
            if seen_directives.insert(d.clone()) {
                rep.suggestions.push(Suggestion {
                    kind: "binary_mismatch",
                    host: host.clone(),
                    port: *port,
                    binary: bin.clone(),
                    count: *count,
                    directive: Some(d),
                    place: Some(r.dir.loc()),
                    note: format!(
                        "host:port is annotated at {} but for {}; this binary is not in the rule",
                        r.dir.loc(),
                        r.bin.clone().unwrap_or_else(|| "?".into())
                    ),
                });
            }
            continue;
        }
        // host:port and binary are covered: remaining denials are L7 (method/path not permitted).
        for (m, p) in l7 {
            let r = matching
                .iter()
                .find(|r| {
                    bin.is_none()
                        || r.bin
                            .as_deref()
                            .zip(bin.as_deref())
                            .is_some_and(|(a, b)| bin_matches(a, b))
                })
                .unwrap_or(&matching[0]);
            if r.dir.attr("access").is_some() {
                rep.suggestions.push(Suggestion { kind: "manual", host: host.clone(), port: *port, binary: bin.clone(), count: *count, directive: None, place: Some(r.dir.loc()), note: format!("{m} {p} denied under access={}; raise access or switch {} to allow=\"...\" rules", r.dir.attr("access").unwrap(), r.dir.loc()) });
                continue;
            }
            let covered = matching.iter().any(|r| {
                r.dir
                    .attrs
                    .multi
                    .get("allow")
                    .into_iter()
                    .flatten()
                    .any(|a| allow_covers(a, m, p))
            });
            if covered {
                rep.suggestions.push(Suggestion { kind: "manual", host: host.clone(), port: *port, binary: bin.clone(), count: *count, directive: None, place: Some(r.dir.loc()), note: format!("{m} {p} is already allowed by the directives, yet was denied: the deployed policy is stale (regenerate and apply it)") });
                continue;
            }
            let rb = r
                .bin
                .clone()
                .map(|b| format!(" bin={b}"))
                .unwrap_or_default();
            let rule = r
                .dir
                .attr("rule")
                .map(|x| format!(" rule={x}"))
                .unwrap_or_default();
            let d = format!(
                "@openshell egress {target} reason=\"observed L7 deny: {m} {p} (confirm it is needed)\" allow=\"{m}:{p}\"{rb}{rule}"
            );
            if seen_directives.insert(d.clone()) {
                rep.suggestions.push(Suggestion {
                    kind: "l7_allow",
                    host: host.clone(),
                    port: *port,
                    binary: bin.clone(),
                    count: *count,
                    directive: Some(d),
                    place: Some(r.dir.loc()),
                    note: format!("method/path not permitted by the rules at {}", r.dir.loc()),
                });
            }
        }
        if l7.is_empty() {
            rep.suggestions.push(Suggestion { kind: "manual", host: host.clone(), port: *port, binary: bin.clone(), count: *count, directive: None, place: Some(matching[0].dir.loc()), note: "denied although directive, host:port and binary match: the deployed policy is probably stale (regenerate and apply it) or the binary is matched through a different path".into() });
        }
    }

    // Stale directives: no event (allowed or denied) for their host:port (and binary, when the event names one).
    if !events.is_empty() {
        for r in &rows {
            let hit = events.iter().any(|e| {
                row_matches_target(r, &e.host, e.port)
                    && (match (&r.bin, &e.binary) {
                        (Some(a), Some(b)) => bin_matches(a, b),
                        _ => true,
                    } || e.policy.as_deref() == Some(row_rule_name(r).as_str()))
            });
            if !hit {
                rep.stale.push(Stale {
                    directive: r.dir.loc(),
                    target: format!(
                        "{}:{}",
                        r.host,
                        r.ports
                            .iter()
                            .map(u16::to_string)
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                    binary: r.bin.clone(),
                });
            }
        }
    }
    rep
}

/// Returns the line-comment leader to write a directive in a file of this language.
pub fn leader(lang: Language) -> &'static str {
    match lang {
        Language::Python | Language::Shell | Language::Hash => "#",
        _ => "//",
    }
}

/// Insert the suggested directives above their placement line; suggestions without a place go to `observed_file`.
/// Returns the list of files touched. `manual` suggestions are never written.
///
/// Inserted lines take the indentation of the line they go above and the comment leader of the file's language (`#`
/// for files that were not scanned). Entries already present in `observed_file` are not added again.
///
/// # Errors
///
/// Fails when a target file cannot be read or written, or the directory of `observed_file` cannot be created.
pub fn write_suggestions(
    rep: &Report,
    root: &std::path::Path,
    scan: &Scan,
    observed_file: &str,
) -> std::io::Result<Vec<String>> {
    let mut by_file: BTreeMap<String, Vec<(usize, String)>> = BTreeMap::new();
    let mut observed: Vec<String> = Vec::new();
    for s in &rep.suggestions {
        let Some(d) = &s.directive else { continue };
        if s.kind == "binary_mismatch" || s.kind == "l7_allow" {
            // placed next to the directive it extends
            if let Some(p) = &s.place {
                if let Some((f, l)) = p.rsplit_once(':') {
                    if let Ok(l) = l.parse::<usize>() {
                        by_file
                            .entry(f.to_string())
                            .or_default()
                            .push((l, d.clone()));
                        continue;
                    }
                }
            }
        }
        if s.kind == "missing_egress" {
            if let Some(p) = &s.place {
                if let Some((f, l)) = p.rsplit_once(':') {
                    if let Ok(l) = l.parse::<usize>() {
                        by_file
                            .entry(f.to_string())
                            .or_default()
                            .push((l, d.clone()));
                        continue;
                    }
                }
            }
        }
        observed.push(d.clone());
    }
    let mut touched = Vec::new();
    for (file, mut ins) in by_file {
        let path = root.join(&file);
        let text = std::fs::read_to_string(&path)?;
        let trailing_nl = text.ends_with('\n');
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        let lang = scan.sources.iter().find(|s| s.rel == file).map(|s| s.lang);
        let lead = lang.map(leader).unwrap_or("#");
        ins.sort_by_key(|b| std::cmp::Reverse(b.0));
        for (line, d) in ins {
            let i = line.saturating_sub(1).min(lines.len());
            let indent: String = lines
                .get(i)
                .map(|l| l.chars().take_while(|c| c.is_whitespace()).collect())
                .unwrap_or_default();
            lines.insert(i, format!("{indent}{lead} {d}"));
        }
        let mut out = lines.join("\n");
        if trailing_nl {
            out.push('\n');
        }
        std::fs::write(&path, out)?;
        touched.push(file);
    }
    if !observed.is_empty() {
        let path = root.join(observed_file);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            "# Egress observed in sandbox logs that no source line mentions (written by `openshell-annotate diff-logs --write`).\n# Review each entry, keep the reason truthful, then regenerate the policy.\n".to_string()
        });
        for d in observed {
            if !text.contains(&d) {
                text.push_str(&format!("# {d}\n"));
            }
        }
        std::fs::write(&path, text)?;
        touched.push(observed_file.to_string());
    }
    Ok(touched)
}

/// Renders the report in the stable compact text format (`--format compact`).
pub fn render_text(rep: &Report) -> String {
    let mut s =
        format!(
        "diff-logs: {} event(s) ({} denied, {} allowed); {} suggestion(s), {} stale directive(s)\n",
        rep.events, rep.denied, rep.allowed, rep.suggestions.len(), rep.stale.len()
    );
    s.push_str("note: logs are best-effort and lossy; absence of an event is not proof. Filesystem denials are not logged by OpenShell 0.1.7 (#4270).\n");
    for g in &rep.suggestions {
        let up = g.kind.to_ascii_uppercase();
        s.push_str(&format!(
            "\n{up:<16} {}:{} x{} {}\n  {}\n",
            g.host,
            g.port.map_or("?".into(), |p| p.to_string()),
            g.count,
            g.binary.as_deref().unwrap_or(""),
            g.note
        ));
        if let Some(d) = &g.directive {
            s.push_str(&format!("  suggest: {d}\n"));
        }
        if let Some(p) = &g.place {
            s.push_str(&format!("  place:   {p}\n"));
        }
    }
    for st in &rep.stale {
        s.push_str(&format!(
            "\nSTALE            {} {} (never matched in these logs)\n",
            st.directive, st.target
        ));
    }
    for d in &rep.declined {
        s.push_str(&format!(
            "\nDECLINED         {}:{} x{} by {} ({})\n",
            d.host,
            d.port.map_or("?".into(), |p| p.to_string()),
            d.count,
            d.directive,
            d.reason
        ));
    }
    for n in &rep.non_actionable {
        s.push_str(&format!(
            "\nNON-ACTIONABLE   {}:{} x{} {}\n",
            n.host,
            n.port.map_or("?".into(), |p| p.to_string()),
            n.count,
            n.why
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shorthand_net_and_http() {
        let t = "\x1b[2m[t]\x1b[0m [sandbox] [OCSF ] [ocsf] NET:OPEN [MED] DENIED /usr/bin/curl(5) -> a.example:8443 [policy:- engine:opa] [reason:transparent_tcp_policy_denied]\n\
[t] [sandbox] [OCSF ] [ocsf] HTTP:GET [MED] DENIED GET http://b.example/x?y=1 [policy:r engine:l7] [reason:L7_REQUEST deny]\n\
garbage line\n";
        let e = parse_logs(t, "x");
        assert_eq!(e.len(), 2);
        assert_eq!(
            (e[0].binary.as_deref(), e[0].host.as_str(), e[0].port),
            (Some("/usr/bin/curl"), "a.example", Some(8443))
        );
        assert_eq!(e[0].policy, None);
        assert_eq!(
            (e[1].port, e[1].path.as_deref(), e[1].binary.clone()),
            (Some(80), Some("/x"), None)
        );
    }

    #[test]
    fn parses_ocsf_json_and_skips_other_classes() {
        let t = "{\"class_uid\":4001,\"action_id\":2,\"dst_endpoint\":{\"ip\":\"1.2.3.4\",\"port\":22},\"actor\":{\"process\":{\"file\":{\"path\":\"/usr/bin/git\"}}}}\n{\"class_uid\":1007}\nOCSF-JSON {\"class_uid\":4002,\"action\":\"Allowed\",\"http_request\":{\"http_method\":\"GET\",\"url\":{\"hostname\":\"h.example\",\"path\":\"/p\"}},\"dst_endpoint\":{\"port\":443}}\n";
        let e = parse_logs(t, "x");
        assert_eq!(e.len(), 2);
        assert_eq!(
            (e[0].action, e[0].host.as_str(), e[0].binary.as_deref()),
            (Action::Denied, "1.2.3.4", Some("/usr/bin/git"))
        );
        assert_eq!(
            (e[1].action, e[1].class, e[1].host.as_str()),
            (Action::Allowed, "http", "h.example")
        );
    }

    #[test]
    fn skips_ocsf_network_refuse_records() {
        // Real OpenShell v0.1.7 (OCSF 1.8.0) policy-DNS refusal followed by the denied connection it precedes.
        let t = concat!(
            "{\"class_uid\":4001,\"activity_id\":5,\"activity_name\":\"Refuse\",\"status_detail\":\"policy_dns_ineligible\",",
            "\"dst_endpoint\":{\"domain\":\"example.com\"},\"action_id\":2,\"action\":\"Denied\",\"metadata\":{\"version\":\"1.8.0\"}}\n",
            "{\"class_uid\":4001,\"activity_id\":1,\"status_detail\":\"policy_dns_ineligible\",\"dst_endpoint\":{\"domain\":\"b.example\"},\"action\":\"Denied\"}\n",
            "{\"class_uid\":4001,\"activity_id\":1,\"status_detail\":\"transparent_tcp_policy_denied\",",
            "\"dst_endpoint\":{\"domain\":\"example.com\",\"ip\":\"198.18.0.3\",\"port\":443},",
            "\"actor\":{\"process\":{\"name\":\"/usr/bin/curl\",\"pid\":0}},\"action_id\":2,\"action\":\"Denied\",\"metadata\":{\"version\":\"1.8.0\"}}\n",
        );
        let e = parse_logs(t, "x");
        assert_eq!(e.len(), 1);
        assert_eq!(
            (
                e[0].action,
                e[0].host.as_str(),
                e[0].port,
                e[0].binary.as_deref()
            ),
            (
                Action::Denied,
                "example.com",
                Some(443),
                Some("/usr/bin/curl")
            )
        );
    }

    #[test]
    fn git_helpers_match_their_parent_binary() {
        assert!(bin_matches(
            "/usr/bin/git",
            "/usr/lib/git-core/git-remote-http"
        ));
        assert!(!bin_matches(
            "/usr/bin/curl",
            "/usr/lib/git-core/git-remote-http"
        ));
        assert!(!bin_matches("/usr/bin/git", "/usr/bin/curl"));
    }

    #[test]
    fn wildcard_hosts_match_one_label_and_globs() {
        assert!(host_matches("*.example.com", "a.example.com"));
        assert!(!host_matches("*.example.com", "a.b.example.com"));
        assert!(!host_matches("*.example.com", "example.com"));
        assert!(host_matches("API.Example.com", "api.example.COM"));
        assert!(allow_covers("GET:/v1/**", "get", "/v1/a/b"));
        assert!(!allow_covers("GET:/v1/*", "GET", "/v1/a/b"));
        assert!(allow_covers("*:/x", "DELETE", "/x"));
        assert!(!allow_covers("POST:/x", "GET", "/x"));
    }
}
