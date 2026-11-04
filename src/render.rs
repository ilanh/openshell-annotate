//! Human-readable output for `scan`, `diff-logs` and `generate`, in the same visual language as [`crate::report`]
//! (lint): category icons, a `-->` location with a source snippet, `= key: value` detail lines and a summary.
//!
//! Machine formats (`--format json`, `--format compact`) never go through this module.

use crate::diff_logs::{self, Report};
use crate::directive::{Directive, Kind};
use crate::generate::Generated;
use crate::lint::Severity;
use crate::report::{
    category_icon, digits, expand_tabs, plural, severity_icon, severity_sgr, window, Style,
};
use crate::scan::Scan;
use std::collections::BTreeMap;

/// Supplies the source lines of a file (relative path), if they can be read.
pub type LinesOf<'a> = &'a dyn Fn(&str) -> Option<Vec<String>>;

/// The lint category a directive belongs to (decides its icon and colour).
pub fn kind_category(k: &Kind) -> &'static str {
    match k {
        Kind::Egress { .. } => "net",
        Kind::Fs { .. } => "fs",
        Kind::Provider { .. } => "env",
        Kind::Bin { .. } | Kind::Process => "exec",
        Kind::Ignore => "directive",
    }
}

fn category_sgr(cat: &str) -> &'static str {
    match cat {
        "net" => "1;36",
        "fs" => "1;35",
        "env" => "1;33",
        "exec" => "1;32",
        _ => "1",
    }
}

fn sep(style: Style) -> &'static str {
    if style.icons {
        " · "
    } else {
        ", "
    }
}

fn arrow(style: Style) -> &'static str {
    if style.icons {
        "→"
    } else {
        "->"
    }
}

fn bullet(style: Style) -> &'static str {
    if style.icons {
        "•"
    } else {
        "-"
    }
}

fn ellipsis(style: Style) -> &'static str {
    if style.icons {
        "…"
    } else {
        "..."
    }
}

/// `"🌐 "` when icons are on, else nothing.
fn icon_prefix(style: Style, cat: &str) -> String {
    if style.icons {
        format!("{} ", category_icon(cat))
    } else {
        String::new()
    }
}

/// Wrap `text` at word boundaries to at most `width` characters per line (a longer word stays whole).
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

const WRAP: usize = 96;

// ---- source snippets -------------------------------------------------------------------------------------------------

/// One row of a snippet: an existing line (`num`), or a line that would be inserted (`added`).
struct Row {
    num: Option<usize>,
    added: bool,
    text: String,
}

fn existing(lines: &[String], n: usize, style: Style) -> Row {
    Row {
        num: Some(n),
        added: false,
        text: window(&expand_tabs(&lines[n - 1]), 0, ellipsis(style)),
    }
}

/// `--> loc`, then the rows between gutter bars. Returns the gutter width (for aligned `= key:` lines).
fn snippet(out: &mut String, style: Style, loc: &str, rows: &[Row]) -> usize {
    let w = rows.iter().filter_map(|r| r.num).max().map_or(1, digits);
    let pad = " ".repeat(w);
    out.push_str(&format!("{pad}{} {loc}\n", style.gutter("-->")));
    if rows.is_empty() {
        return w;
    }
    let bar = style.gutter("|");
    out.push_str(&format!("{pad} {bar}\n"));
    for r in rows {
        let cell = if r.added {
            style.paint("1;32", &format!("{:>w$}", "+"))
        } else {
            style.gutter(&format!("{:>w$}", r.num.unwrap_or(0)))
        };
        let text = if r.added {
            style.paint("32", &r.text)
        } else {
            r.text.clone()
        };
        out.push_str(format!("{cell} {bar} {text}").trim_end());
        out.push('\n');
    }
    out.push_str(&format!("{pad} {bar}\n"));
    w
}

/// `= key: text` lines (wrapped) under a snippet; `w` is the snippet's gutter width.
fn detail(out: &mut String, style: Style, w: usize, key: &str, sgr: &str, text: &str) {
    detail_wrapped(out, style, w, key, sgr, text, true);
}

/// Like [`detail`]; `wrap = false` keeps the text on one line (for copy-pasteable directives).
fn detail_wrapped(
    out: &mut String,
    style: Style,
    w: usize,
    key: &str,
    sgr: &str,
    text: &str,
    wrap_text: bool,
) {
    let pad = " ".repeat(w);
    let tag = style.paint(sgr, &format!("{key}:"));
    let eq = style.gutter("=");
    let cont = " ".repeat(w + 3 + key.len() + 2);
    let lines = if wrap_text {
        wrap(text, WRAP)
    } else {
        vec![text.to_string()]
    };
    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            out.push_str(&format!("{pad} {eq} {tag} {line}\n"));
        } else {
            out.push_str(&format!("{cont}{line}\n"));
        }
    }
}

fn split_loc(loc: &str) -> Option<(&str, usize)> {
    let (f, l) = loc.rsplit_once(':')?;
    Some((f, l.parse().ok()?))
}

// ---- scan ----------------------------------------------------------------------------------------------------------------

/// What a directive is about, after its verb: `host:ports`, the env var, the path, or for `process` and `ignore`
/// the attributes that say what they do (`user=sandbox group=sandbox`, `kind=net scope=file`). Empty when there is
/// nothing to show (a bare `process`).
pub fn directive_detail(d: &Directive) -> String {
    let attrs = |keys: &[&str]| {
        keys.iter()
            .filter_map(|k| d.attr(k).map(|v| format!("{k}={v}")))
            .collect::<Vec<_>>()
            .join(" ")
    };
    match &d.kind {
        Kind::Egress { host, ports } => format!(
            "{host}:{}",
            ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Kind::Provider { env } => env.clone(),
        Kind::Fs { path, .. } => path.clone(),
        Kind::Bin { path } => path.clone(),
        Kind::Process => attrs(&["user", "group"]),
        Kind::Ignore => attrs(&["kind", "host", "scope"]),
    }
}

/// `scan` report: directives grouped by file, each with its category icon, `file:line` and the source line.
pub fn scan_human(scan: &Scan, lines_of: LinesOf, style: Style) -> String {
    let mut out = String::new();
    let n = scan.directives.len();
    let nfiles = scan.sources.len();
    if n == 0 {
        let mark = if style.icons { "ℹ " } else { "" };
        out.push_str(&style.paint(
            "1;33",
            &format!(
                "{mark}no @openshell directives found ({})",
                plural(nfiles, "file scanned", "files scanned")
            ),
        ));
        out.push('\n');
        push_scan_errors(&mut out, scan, style);
        return out;
    }
    let mut groups: BTreeMap<&str, Vec<&Directive>> = BTreeMap::new();
    for d in &scan.directives {
        groups.entry(d.file.as_str()).or_default().push(d);
    }
    for (file, mut ds) in groups {
        ds.sort_by_key(|d| d.line);
        let head = format!("{}{file}", if style.icons { "📄 " } else { "" });
        out.push_str(&style.paint("1", &head));
        out.push_str(&style.paint(
            "2",
            &format!("  ({})", plural(ds.len(), "directive", "directives")),
        ));
        out.push_str("\n\n");
        let lines = lines_of(file);
        for d in ds {
            let cat = kind_category(&d.kind);
            let mut h = style.paint(
                category_sgr(cat),
                &format!("{}{}", icon_prefix(style, cat), d.kind.name()),
            );
            let det = directive_detail(d);
            if !det.is_empty() {
                h.push(' ');
                h.push_str(&style.paint("1", &det));
            }
            if let Some(r) = d.reason() {
                h.push_str(&style.paint("2", &format!("  # {r}")));
            }
            out.push_str(&h);
            out.push('\n');
            let rows: Vec<Row> = match lines.as_deref() {
                Some(l) if d.line >= 1 && d.line <= l.len() => vec![existing(l, d.line, style)],
                _ => vec![],
            };
            snippet(&mut out, style, &d.loc(), &rows);
            out.push('\n');
        }
    }
    let mark = if style.icons { "✔ " } else { "" };
    out.push_str(&style.paint(
        "1;32",
        &format!(
            "{mark}{} in {}",
            plural(n, "directive", "directives"),
            plural(nfiles, "file", "files")
        ),
    ));
    out.push('\n');
    let mut cats: BTreeMap<&str, usize> = BTreeMap::new();
    for d in &scan.directives {
        *cats.entry(kind_category(&d.kind)).or_default() += 1;
    }
    out.push_str(&format!("by category: {}\n", category_counts(&cats, style)));
    push_scan_errors(&mut out, scan, style);
    out
}

fn push_scan_errors(out: &mut String, scan: &Scan, style: Style) {
    if !scan.errors.is_empty() {
        let mark = if style.icons { "✖ " } else { "" };
        out.push_str(&style.paint(
            "1;31",
            &format!(
                "{mark}{} (details on stderr)",
                plural(scan.errors.len(), "parse error", "parse errors")
            ),
        ));
        out.push('\n');
    }
}

fn category_counts(cats: &BTreeMap<&str, usize>, style: Style) -> String {
    cats.iter()
        .map(|(c, n)| {
            if style.icons {
                format!("{} {c} {n}", category_icon(c))
            } else {
                format!("{c} {n}")
            }
        })
        .collect::<Vec<_>>()
        .join(sep(style))
}

// ---- diff-logs -----------------------------------------------------------------------------------------------------------

fn hostport(host: &str, port: Option<u16>) -> String {
    format!("{host}:{}", port.map_or("?".into(), |p| p.to_string()))
}

/// Head line `<sev icon> <category icon> <label>: <what>  ×N  binary`.
fn entry_head(
    style: Style,
    sev: Severity,
    cat: &str,
    label: &str,
    what: &str,
    tail: &str,
) -> String {
    let mut icons = String::new();
    if style.icons {
        icons = format!("{} {} ", severity_icon(sev), category_icon(cat));
    }
    let mut s = style.paint(severity_sgr(sev), &format!("{icons}{label}"));
    s.push_str(&style.paint("1", &format!(": {what}")));
    if !tail.is_empty() {
        s.push_str(&style.paint("2", &format!("  {tail}")));
    }
    s
}

fn count_binary(count: usize, binary: Option<&str>, style: Style) -> String {
    let x = if style.icons { "×" } else { "x" };
    match binary.filter(|b| !b.is_empty()) {
        Some(b) => format!("{x}{count}  {b}"),
        None => format!("{x}{count}"),
    }
}

fn section(out: &mut String, style: Style, icon: &str, title: &str, n: usize) {
    let icon = if style.icons && !icon.is_empty() {
        format!("{icon} ")
    } else {
        String::new()
    };
    out.push_str(&style.paint("1", &format!("{icon}{title}")));
    out.push_str(&style.paint("2", &format!("  ({n})")));
    out.push_str("\n\n");
}

/// `diff-logs` report: what the logs say, grouped by what to do about it, with the line each fix would go on.
/// `observed_file` is where `--write` parks suggestions no source mentions; `logs_hint` names the inputs for the
/// follow-up command in the footer.
pub fn diff_logs_human(
    rep: &Report,
    scan: &Scan,
    lines_of: LinesOf,
    observed_file: &str,
    logs_hint: &str,
    style: Style,
) -> String {
    let mut out = String::new();
    let magnifier = if style.icons { "🔍 " } else { "" };
    out.push_str(&style.paint("1", &format!("{magnifier}diff-logs")));
    out.push_str(&format!(
        "  {}: {}{}{}\n\n",
        plural(rep.events, "event", "events"),
        style.paint(
            "1;31",
            &format!(
                "{}{} denied",
                if style.icons { "✖ " } else { "" },
                rep.denied
            )
        ),
        sep(style),
        style.paint(
            "1;32",
            &format!(
                "{}{} allowed",
                if style.icons { "✔ " } else { "" },
                rep.allowed
            )
        ),
    ));
    let lang_of = |file: &str| scan.sources.iter().find(|s| s.rel == file).map(|s| s.lang);

    let kinds = [
        (
            "missing_egress",
            "missing egress",
            Severity::Error,
            "Missing directives",
            "➕",
        ),
        (
            "binary_mismatch",
            "binary mismatch",
            Severity::Warning,
            "Binary not covered",
            "⚙️",
        ),
        (
            "l7_allow",
            "l7 allow",
            Severity::Warning,
            "L7 rules too narrow",
            "🔒",
        ),
        (
            "tls_failure",
            "tls failure",
            Severity::Warning,
            "Allowed, then failed (TLS)",
            "🔐",
        ),
        (
            "manual",
            "manual review",
            Severity::Info,
            "Needs a manual look",
            "👀",
        ),
    ];
    for (kind, label, sev, title, icon) in kinds {
        let group: Vec<_> = rep.suggestions.iter().filter(|g| g.kind == kind).collect();
        if group.is_empty() {
            continue;
        }
        section(&mut out, style, icon, title, group.len());
        for g in group {
            out.push_str(&entry_head(
                style,
                sev,
                "net",
                label,
                &hostport(&g.host, g.port),
                &count_binary(g.count, g.binary.as_deref(), style),
            ));
            out.push('\n');
            let placed = g.place.as_deref().and_then(split_loc);
            let src = placed.and_then(|(f, l)| lines_of(f).map(|ls| (f, l, ls)));
            let mut w = 1;
            match (&src, placed) {
                (Some((f, l, ls)), _) if *l >= 1 && *l <= ls.len() => {
                    let mut rows = Vec::new();
                    if let Some(d) = &g.directive {
                        if *l > 1 {
                            rows.push(existing(ls, *l - 1, style));
                        }
                        let indent: String = ls[*l - 1]
                            .chars()
                            .take_while(|c| c.is_whitespace())
                            .collect();
                        let lead = lang_of(f).map_or("#", diff_logs::leader);
                        rows.push(Row {
                            num: None,
                            added: true,
                            text: format!("{indent}{lead} {d}"),
                        });
                        rows.push(existing(ls, *l, style));
                    } else {
                        if *l > 1 {
                            rows.push(existing(ls, *l - 1, style));
                        }
                        rows.push(existing(ls, *l, style));
                        if *l < ls.len() {
                            rows.push(existing(ls, *l + 1, style));
                        }
                    }
                    w = snippet(&mut out, style, &format!("{f}:{l}"), &rows);
                }
                (_, Some((f, l))) => {
                    w = snippet(&mut out, style, &format!("{f}:{l}"), &[]);
                }
                _ => {}
            }
            detail(&mut out, style, w, "why", "1;36", &g.note);
            if g.directive.is_some() && placed.is_none() {
                detail(
                    &mut out,
                    style,
                    w,
                    "where",
                    "1;36",
                    &format!(
                        "no source line mentions this host; `--write` appends it to {observed_file}"
                    ),
                );
            }
            if let Some(d) = &g.directive {
                if src.is_none() {
                    detail_wrapped(&mut out, style, w, "add", "1;32", d, false);
                }
            }
            out.push('\n');
        }
    }

    if !rep.stale.is_empty() {
        section(&mut out, style, "🧹", "Stale directives", rep.stale.len());
        for st in &rep.stale {
            out.push_str(&entry_head(
                style,
                Severity::Warning,
                "stale",
                "stale",
                &st.target,
                st.binary.as_deref().unwrap_or(""),
            ));
            out.push('\n');
            let mut w = 1;
            if let Some((f, l)) = split_loc(&st.directive) {
                let rows: Vec<Row> = match lines_of(f) {
                    Some(ls) if l >= 1 && l <= ls.len() => vec![existing(&ls, l, style)],
                    _ => vec![],
                };
                w = snippet(&mut out, style, &st.directive, &rows);
            }
            detail(
                &mut out,
                style,
                w,
                "note",
                "1;36",
                "never matched in these logs (advisory: logs are lossy, remove only after a run that exercised this feature)",
            );
            out.push('\n');
        }
    }

    if !rep.declined.is_empty() {
        section(
            &mut out,
            style,
            "🚫",
            "Declined on purpose",
            rep.declined.len(),
        );
        for d in &rep.declined {
            out.push_str(&entry_head(
                style,
                Severity::Info,
                "directive",
                "declined",
                &hostport(&d.host, d.port),
                &count_binary(d.count, None, style),
            ));
            out.push('\n');
            let mut w = 1;
            if let Some((f, l)) = split_loc(&d.directive) {
                let rows: Vec<Row> = match lines_of(f) {
                    Some(ls) if l >= 1 && l <= ls.len() => vec![existing(&ls, l, style)],
                    _ => vec![],
                };
                w = snippet(&mut out, style, &d.directive, &rows);
            } else {
                detail(&mut out, style, w, "by", "1;36", &d.directive);
            }
            detail(&mut out, style, w, "reason", "1;36", &d.reason);
            out.push('\n');
        }
    }

    if !rep.non_actionable.is_empty() {
        section(
            &mut out,
            style,
            "⛔",
            "Not actionable",
            rep.non_actionable.len(),
        );
        for n in &rep.non_actionable {
            out.push_str(&entry_head(
                style,
                Severity::Info,
                "net",
                "blocked",
                &hostport(&n.host, n.port),
                &count_binary(n.count, None, style),
            ));
            out.push('\n');
            detail(&mut out, style, 1, "why", "1;36", &n.why);
            out.push('\n');
        }
    }

    // summary
    let writable = rep
        .suggestions
        .iter()
        .filter(|g| g.directive.is_some())
        .count();
    if rep.suggestions.is_empty() {
        let mark = if style.icons { "✓ " } else { "" };
        out.push_str(&style.paint(
            "1;32",
            &format!("{mark}every event is explained by the directives"),
        ));
    } else {
        let mark = if style.icons { "✖ " } else { "" };
        out.push_str(&style.paint(
            "1;31",
            &format!(
                "{mark}{}",
                plural(rep.suggestions.len(), "suggestion", "suggestions")
            ),
        ));
    }
    let mut parts = Vec::new();
    if !rep.stale.is_empty() {
        parts.push(plural(
            rep.stale.len(),
            "stale directive",
            "stale directives",
        ));
    }
    if !rep.declined.is_empty() {
        parts.push(format!("{} declined", rep.declined.len()));
    }
    if !rep.non_actionable.is_empty() {
        parts.push(format!("{} not actionable", rep.non_actionable.len()));
    }
    if !parts.is_empty() {
        out.push_str(&style.paint("2", &format!("  ({})", parts.join(sep(style)))));
    }
    out.push('\n');
    if writable > 0 {
        let mark = if style.icons { "💡 " } else { "" };
        out.push_str(&format!(
            "{mark}next: `openshell-annotate diff-logs {logs_hint} --write` inserts {} (review, then regenerate the policy)\n",
            plural(writable, "directive", "directives")
        ));
    }
    out.push_str(&style.paint(
        "2",
        "note: logs are best-effort and lossy; absence of an event is not proof. Filesystem denials are not logged by OpenShell 0.1.6 (#4270).\n",
    ));
    out
}

// ---- generate ------------------------------------------------------------------------------------------------------------

fn from_locs(dirs: &[Directive], pred: impl Fn(&Directive) -> bool) -> String {
    let locs: Vec<String> = dirs
        .iter()
        .filter(|d| pred(d))
        .map(Directive::loc)
        .collect();
    match locs.len() {
        0 => String::new(),
        1..=3 => locs.join(", "),
        n => format!("{}, +{} more", locs[..3].join(", "), n - 3),
    }
}

fn norm_path(p: &str) -> String {
    if p.len() > 1 {
        p.trim_end_matches('/').to_string()
    } else {
        p.to_string()
    }
}

fn label(out: &mut String, style: Style, cat: &str, name: &str, text: &str) {
    out.push_str(&style.paint(
        category_sgr(cat),
        &format!("{}{name:<11}", icon_prefix(style, cat)),
    ));
    out.push_str(text);
    out.push('\n');
}

/// Summary of what `generate` produced, meant for stderr: policy YAML itself is never touched. `policy_dest` /
/// `providers_dest` are the output paths (`None` policy = stdout).
pub fn generate_human(
    g: &Generated,
    dirs: &[Directive],
    policy_dest: Option<&str>,
    providers_dest: Option<&str>,
    style: Style,
) -> String {
    let mut out = String::new();
    let st = &g.stats;
    let mark = if style.icons { "✔ " } else { "" };
    out.push_str(&style.paint(
        "1;32",
        &format!(
            "{mark}generated a policy from {}",
            plural(st.directives, "directive", "directives")
        ),
    ));
    out.push_str("\n\n");

    let to = arrow(style);
    label(
        &mut out,
        style,
        "policy",
        "policy",
        &format!("{to} {}", policy_dest.unwrap_or("stdout")),
    );

    // network
    label(
        &mut out,
        style,
        "net",
        "network",
        &format!(
            "{}{}{}{}",
            plural(st.network_rules, "rule", "rules"),
            sep(style),
            plural(st.endpoints, "endpoint", "endpoints"),
            format_args!(
                "{}{}",
                sep(style),
                plural(st.binaries, "binary path", "binary paths")
            )
        ),
    );
    for (name, rule) in &g.doc.network_policies {
        let mut eps = Vec::new();
        for e in &rule.endpoints {
            let mut ports: Vec<u16> = e.ports.clone();
            if e.port != 0 {
                ports.push(e.port);
            }
            ports.sort_unstable();
            ports.dedup();
            let mut s = format!(
                "{}:{}",
                e.host,
                ports
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            );
            let mut attrs = Vec::new();
            for (k, v) in [
                ("protocol", &e.protocol),
                ("access", &e.access),
                ("enforcement", &e.enforcement),
            ] {
                if !v.is_empty() {
                    attrs.push(format!("{k}={v}"));
                }
            }
            if !e.rules.is_empty() {
                attrs.push(plural(e.rules.len(), "L7 rule", "L7 rules"));
            }
            if !attrs.is_empty() {
                s.push_str(&format!(" [{}]", attrs.join(" ")));
            }
            eps.push(s);
        }
        let hosts: Vec<&str> = rule.endpoints.iter().map(|e| e.host.as_str()).collect();
        out.push_str(&format!(
            "   {} {}\n",
            bullet(style),
            style.paint("1", name)
        ));
        out.push_str(&format!("       {to} {}\n", eps.join("; ")));
        let bins: Vec<&str> = rule.binaries.iter().map(|b| b.path.as_str()).collect();
        if !bins.is_empty() {
            out.push_str(&format!("       bins: {}\n", bins.join(", ")));
        }
        let from = from_locs(
            dirs,
            |d| matches!(&d.kind, Kind::Egress { host, .. } if hosts.contains(&host.as_str())),
        );
        if !from.is_empty() {
            out.push_str(&style.paint("2", &format!("       from {from}\n")));
        }
    }

    // filesystem
    if let Some(fs) = &g.doc.filesystem_policy {
        let mut text = format!(
            "{} read-only{}{} read-write",
            fs.read_only.len(),
            sep(style),
            fs.read_write.len()
        );
        if fs.include_workdir {
            text.push_str(&format!("{}workdir included", sep(style)));
        }
        label(&mut out, style, "fs", "filesystem", &text);
        for (tag, paths, write) in [("ro", &fs.read_only, false), ("rw", &fs.read_write, true)] {
            for p in paths {
                let from = from_locs(
                    dirs,
                    |d| matches!(&d.kind, Kind::Fs { write: w, path } if *w == write && norm_path(path) == *p),
                );
                let from = if from.is_empty() {
                    "system_paths default".to_string()
                } else {
                    from
                };
                out.push_str(&format!(
                    "   {} {tag}  {}  {}\n",
                    bullet(style),
                    style.paint("1", p),
                    style.paint("2", &format!("from {from}"))
                ));
            }
        }
    }

    // providers
    let pv = &g.providers;
    if !pv.providers.is_empty() || !pv.unmapped.is_empty() {
        let mut text = format!(
            "{}{}{} unmapped",
            plural(st.providers, "provider", "providers"),
            sep(style),
            st.unmapped_providers
        );
        if let Some(p) = providers_dest {
            text.push_str(&format!(" {to} {p}"));
        }
        label(&mut out, style, "env", "providers", &text);
        for p in &pv.providers {
            let prof = p
                .profile
                .as_deref()
                .map(|x| format!(" (profile {x})"))
                .unwrap_or_default();
            out.push_str(&format!(
                "   {} {}{prof}: {}  {}\n",
                bullet(style),
                style.paint("1", &p.instance),
                p.credentials.join(", "),
                style.paint("2", &format!("from {}", p.sources.join(", ")))
            ));
        }
        for u in &pv.unmapped {
            out.push_str(&format!(
                "   {} {} {}  {}\n",
                bullet(style),
                style.paint("1;33", &u.env),
                style.paint("1;33", "(no matching profile)"),
                style.paint("2", &format!("from {}", u.sources.join(", ")))
            ));
        }
    } else if let Some(p) = providers_dest {
        label(
            &mut out,
            style,
            "env",
            "providers",
            &format!("none declared {to} {p} (empty)"),
        );
    }

    // process
    if let Some(p) = &g.doc.process {
        label(
            &mut out,
            style,
            "exec",
            "process",
            &format!(
                "run_as_user={}{}run_as_group={}",
                p.run_as_user,
                sep(style),
                p.run_as_group
            ),
        );
    }

    if !g.warnings.is_empty() {
        out.push('\n');
        let mark = if style.icons { "⚠ " } else { "" };
        out.push_str(&style.paint(
            "1;33",
            &format!("{mark}{}", plural(g.warnings.len(), "warning", "warnings")),
        ));
        out.push('\n');
        for w in &g.warnings {
            for (i, line) in wrap(w, WRAP).iter().enumerate() {
                let lead = if i == 0 {
                    format!("{} ", bullet(style))
                } else {
                    "  ".to_string()
                };
                out.push_str(&format!("   {lead}{line}\n"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Language;

    #[test]
    fn wrap_breaks_at_words_and_keeps_long_words() {
        assert_eq!(wrap("aa bb cc", 5), vec!["aa bb", "cc"]);
        assert_eq!(wrap("abcdefgh ij", 4), vec!["abcdefgh", "ij"]);
        assert!(wrap("   ", 10).is_empty());
    }

    #[test]
    fn every_directive_kind_has_an_icon() {
        for k in [
            Kind::Egress {
                host: "h".into(),
                ports: vec![1],
            },
            Kind::Fs {
                write: true,
                path: "/p".into(),
            },
            Kind::Provider { env: "E".into() },
            Kind::Bin { path: "/b".into() },
            Kind::Process,
            Kind::Ignore,
        ] {
            assert_ne!(category_icon(kind_category(&k)), "•");
        }
    }

    #[test]
    fn split_loc_takes_the_last_colon() {
        assert_eq!(split_loc("src/a.ts:12"), Some(("src/a.ts", 12)));
        assert_eq!(split_loc("nope"), None);
        assert_eq!(split_loc("a:b"), None);
    }

    #[test]
    fn language_leader_is_exposed_for_previews() {
        assert_eq!(diff_logs::leader(Language::Python), "#");
        assert_eq!(diff_logs::leader(Language::Go), "//");
    }
}
