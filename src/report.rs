//! Human-readable rendering of lint findings (rustc / miette style): a header per finding with severity and category
//! icons, a `-->` location, a source snippet with line-number gutter and an underline, then `= help:` / `= note:` lines.
//!
//! Machine formats (`--format json`, `--format compact`) never go through this module.

use crate::lint::{Finding, Severity};
use std::collections::BTreeMap;

/// How to draw the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    /// Emit ANSI colour escapes.
    pub color: bool,
    /// Emit emoji/Unicode icons (otherwise plain ASCII).
    pub icons: bool,
}

/// Icon for a finding category. Covers every lint category (`net`, `fs`, `env`, `exec`, `directive`, `policy`) plus the
/// drift/stale names reserved for the other subcommands.
pub fn category_icon(cat: &str) -> &'static str {
    match cat {
        "net" => "🌐",
        "fs" => "📁",
        "env" => "🔑",
        "exec" => "⚙️",
        "directive" => "📝",
        "policy" => "📜",
        "drift" | "stale" => "🧹",
        _ => "•",
    }
}

pub fn severity_icon(s: Severity) -> &'static str {
    match s {
        Severity::Error => "✖",
        Severity::Warning => "⚠",
        Severity::Info => "ℹ",
    }
}

fn severity_word(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

pub(crate) fn severity_sgr(s: Severity) -> &'static str {
    match s {
        Severity::Error => "1;31",
        Severity::Warning => "1;33",
        Severity::Info => "1;36",
    }
}

/// `--color` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

/// Decide whether to colour. `always`/`never` are explicit and win over the environment; `auto` colours only a terminal,
/// honours `NO_COLOR` (any non-empty value disables), `CLICOLOR_FORCE` (non-empty, not `0`, forces) and `TERM=dumb`.
pub fn resolve_color(
    mode: ColorMode,
    stdout_is_tty: bool,
    env: &dyn Fn(&str) -> Option<String>,
) -> bool {
    match mode {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => {
            if env("NO_COLOR").is_some_and(|v| !v.is_empty()) {
                return false;
            }
            if env("CLICOLOR_FORCE").is_some_and(|v| !v.is_empty() && v != "0") {
                return true;
            }
            stdout_is_tty && env("TERM").as_deref() != Some("dumb")
        }
    }
}

/// Are icons safe to print? Needs a UTF-8 locale and a terminal that is not `TERM=dumb`.
pub fn resolve_icons(env: &dyn Fn(&str) -> Option<String>) -> bool {
    if env("TERM").as_deref() == Some("dumb") {
        return false;
    }
    let loc = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(|k| env(k))
        .find(|v| !v.is_empty())
        .unwrap_or_default()
        .to_ascii_lowercase();
    loc.contains("utf-8") || loc.contains("utf8")
}

impl Style {
    pub fn paint(&self, sgr: &str, text: &str) -> String {
        if self.color && !text.is_empty() {
            format!("\x1b[{sgr}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
    pub fn gutter(&self, text: &str) -> String {
        self.paint("1;34", text)
    }
}

const CONTEXT_BEFORE: usize = 2;
const CONTEXT_AFTER: usize = 1;
const TAB: &str = "    ";
/// Visible width of a source line before it is windowed around the underline.
const MAX_WIDTH: usize = 110;
/// Longest underline; a call that spans many columns (or lines) is marked at its start.
const MAX_UNDERLINE: usize = 48;

pub(crate) fn expand_tabs(s: &str) -> String {
    s.trim_end().replace('\t', TAB)
}

/// Byte index clamped to the string and moved back onto a char boundary.
fn floor_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Display column (0-based chars, tabs expanded) of byte offset `i` in `raw`, and the width of `raw[i..j]`.
fn cols(raw: &str, i: usize, j: usize) -> (usize, usize) {
    let i = floor_boundary(raw, i);
    let j = floor_boundary(raw, j).max(i);
    let w = |s: &str| expand_tabs_keep(s).chars().count();
    (w(&raw[..i]), w(&raw[i..j]))
}

fn expand_tabs_keep(s: &str) -> String {
    s.replace('\t', TAB)
}

/// The byte range to underline on `raw`: the finding's span, else a `@openshell` directive, else the trimmed line.
fn underline_range(f: &Finding, raw: &str) -> (usize, usize) {
    if let Some(sp) = f.span {
        let start = floor_boundary(raw, sp.col);
        let end = sp
            .end
            .map_or(raw.trim_end().len(), |e| floor_boundary(raw, e));
        return (
            start,
            if end > start {
                end
            } else {
                raw.trim_end().len().max(start + 1)
            },
        );
    }
    let t = raw.trim_end();
    if let Some(i) = t.find(crate::directive::MARKER) {
        return (i, t.len());
    }
    let lead = t.len() - t.trim_start().len();
    (lead, t.len().max(lead + 1))
}

pub(crate) fn window(text: &str, off: usize, ellipsis: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if off > 0 && chars.len() <= off {
        return String::new();
    }
    let start = off.min(chars.len());
    let end = (start + MAX_WIDTH).min(chars.len());
    let mut s = String::new();
    if off > 0 {
        s.push_str(ellipsis);
    }
    s.extend(&chars[start..end]);
    if end < chars.len() {
        s.push_str(ellipsis);
    }
    s
}

pub(crate) fn digits(n: usize) -> usize {
    n.max(1).to_string().len()
}

/// Render `findings` (any order). `lines_of(file)` supplies the source lines for the snippet; a file it cannot supply
/// (policy files, project-wide findings) is shown without one. `n_directives` / `n_files` feed the summary line.
pub fn render(
    findings: &[Finding],
    lines_of: &dyn Fn(&str) -> Option<Vec<String>>,
    style: Style,
    n_directives: usize,
) -> String {
    let mut out = String::new();
    if findings.is_empty() {
        let tick = if style.icons { "✓ " } else { "" };
        out.push_str(&style.paint(
            "1;32",
            &format!("{tick}no findings ({n_directives} directive(s))"),
        ));
        out.push('\n');
        return out;
    }
    let ellipsis = if style.icons { "…" } else { "..." };
    let mut groups: BTreeMap<(bool, &str), Vec<&Finding>> = BTreeMap::new();
    for f in findings {
        groups
            .entry((f.file == ".", f.file.as_str()))
            .or_default()
            .push(f);
    }
    for ((project_wide, file), mut fs) in groups {
        fs.sort_by_key(|f| (f.line, f.severity));
        let head = if project_wide {
            format!("{}project-wide", if style.icons { "🗂  " } else { "" })
        } else {
            format!("{}{file}", if style.icons { "📄 " } else { "" })
        };
        let n = fs.len();
        out.push_str(&style.paint("1", &head));
        out.push_str(&style.paint(
            "2",
            &format!("  ({n} finding{})", if n == 1 { "" } else { "s" }),
        ));
        out.push_str("\n\n");
        let lines = lines_of(file);
        for f in fs {
            render_one(&mut out, f, lines.as_deref(), style, ellipsis);
            out.push('\n');
        }
    }
    out.push_str(&summary(findings, style, n_directives));
    out
}

fn render_one(
    out: &mut String,
    f: &Finding,
    lines: Option<&[String]>,
    style: Style,
    ellipsis: &str,
) {
    let sev = f.severity;
    // header
    let mut icons = String::new();
    if style.icons {
        icons.push_str(severity_icon(sev));
        icons.push(' ');
        icons.push_str(category_icon(f.category));
        icons.push(' ');
    }
    out.push_str(&style.paint(
        severity_sgr(sev),
        &format!("{icons}{}[{}]", severity_word(sev), f.category),
    ));
    out.push_str(&style.paint(
        "1",
        &format!(": {}", f.title.as_deref().unwrap_or(&f.message)),
    ));
    out.push('\n');

    // which source lines to show
    let shown = lines.filter(|l| f.line >= 1 && f.line <= l.len());
    let (lo, hi) = match shown {
        Some(l) => (
            f.line.saturating_sub(CONTEXT_BEFORE).max(1),
            (f.line + CONTEXT_AFTER).min(l.len()),
        ),
        None => (f.line, f.line),
    };
    let w = digits(hi);
    let pad = " ".repeat(w);

    // location
    let mut col1 = 0;
    if let Some(l) = shown {
        let raw = &l[f.line - 1];
        let (s, _) = underline_range(f, raw);
        col1 = cols(raw, s, s).0 + 1;
    }
    let loc = if f.file == "." {
        "(project-wide)".to_string()
    } else if f.line == 0 {
        f.file.clone()
    } else if col1 > 0 {
        format!("{}:{}:{col1}", f.file, f.line)
    } else {
        format!("{}:{}", f.file, f.line)
    };
    out.push_str(&format!("{pad}{} {loc}\n", style.gutter("-->")));

    if let Some(l) = shown {
        let bar = style.gutter("|");
        out.push_str(&format!("{pad} {bar}\n"));
        let raw_p = &l[f.line - 1];
        let (s, e) = underline_range(f, raw_p);
        let (c0, cw) = cols(raw_p, s, e);
        let off = if expand_tabs(raw_p).chars().count() > MAX_WIDTH && c0 > 60 {
            c0 - 40
        } else {
            0
        };
        for n in lo..=hi {
            let text = window(&expand_tabs(&l[n - 1]), off, ellipsis);
            let num = style.gutter(&format!("{n:>w$}"));
            out.push_str(format!("{num} {bar} {text}").trim_end());
            out.push('\n');
            if n == f.line {
                let lead = c0 - off + if off > 0 { ellipsis.chars().count() } else { 0 };
                let room = MAX_WIDTH.saturating_sub(c0 - off).max(1);
                let carets = "^".repeat(cw.clamp(1, room.min(MAX_UNDERLINE)));
                let label = f
                    .label
                    .as_deref()
                    .map(|x| format!(" {x}"))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "{pad} {bar} {}{}\n",
                    " ".repeat(lead),
                    style.paint(severity_sgr(sev), &format!("{carets}{label}"))
                ));
            }
        }
        out.push_str(&format!("{pad} {bar}\n"));
    }
    for (key, text) in [("help", &f.help), ("note", &f.note)] {
        let Some(text) = text else { continue };
        let tag = style.paint(
            if key == "help" { "1;32" } else { "1;36" },
            &format!("{key}:"),
        );
        let eq = style.gutter("=");
        let cont = " ".repeat(w + 3 + key.len() + 2);
        for (i, line) in text.lines().enumerate() {
            if i == 0 {
                out.push_str(&format!("{pad} {eq} {tag} {line}\n"));
            } else {
                out.push_str(&format!("{cont}{line}\n"));
            }
        }
    }
}

pub(crate) fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `✖ 2 errors · ⚠ 5 warnings · ℹ 1 info` plus a per-category line.
pub fn summary(findings: &[Finding], style: Style, n_directives: usize) -> String {
    let count = |s: Severity| findings.iter().filter(|f| f.severity == s).count();
    let mut parts = Vec::new();
    for (s, one, many) in [
        (Severity::Error, "error", "errors"),
        (Severity::Warning, "warning", "warnings"),
        (Severity::Info, "info", "info"),
    ] {
        let n = count(s);
        if n > 0 {
            let icon = if style.icons {
                format!("{} ", severity_icon(s))
            } else {
                String::new()
            };
            parts.push(style.paint(severity_sgr(s), &format!("{icon}{}", plural(n, one, many))));
        }
    }
    let sep = if style.icons { " · " } else { ", " };
    let mut files: Vec<&str> = findings.iter().map(|f| f.file.as_str()).collect();
    files.sort_unstable();
    files.dedup();
    let mut s = format!(
        "{}  {}\n",
        parts.join(sep),
        style.paint(
            "2",
            &format!(
                "({} in {}; {})",
                plural(findings.len(), "finding", "findings"),
                plural(files.len(), "file", "files"),
                plural(n_directives, "directive", "directives")
            )
        )
    );
    let mut cats: BTreeMap<&str, usize> = BTreeMap::new();
    for f in findings {
        *cats.entry(f.category).or_default() += 1;
    }
    let cat_parts: Vec<String> = cats
        .iter()
        .map(|(c, n)| {
            if style.icons {
                format!("{} {c} {n}", category_icon(c))
            } else {
                format!("{c} {n}")
            }
        })
        .collect();
    s.push_str(&format!("by category: {}\n", cat_parts.join(sep)));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn colour_resolution() {
        let none = env(&[]);
        assert!(resolve_color(ColorMode::Auto, true, &none));
        assert!(!resolve_color(ColorMode::Auto, false, &none));
        assert!(!resolve_color(
            ColorMode::Auto,
            true,
            &env(&[("NO_COLOR", "1")])
        ));
        assert!(resolve_color(
            ColorMode::Auto,
            true,
            &env(&[("NO_COLOR", "")])
        ));
        assert!(!resolve_color(
            ColorMode::Auto,
            true,
            &env(&[("TERM", "dumb")])
        ));
        assert!(resolve_color(
            ColorMode::Auto,
            false,
            &env(&[("CLICOLOR_FORCE", "1")])
        ));
        assert!(!resolve_color(
            ColorMode::Auto,
            false,
            &env(&[("CLICOLOR_FORCE", "0")])
        ));
        assert!(resolve_color(
            ColorMode::Always,
            false,
            &env(&[("NO_COLOR", "1")])
        ));
        assert!(!resolve_color(ColorMode::Never, true, &none));
    }

    #[test]
    fn icon_resolution() {
        assert!(resolve_icons(&env(&[("LANG", "en_US.UTF-8")])));
        assert!(resolve_icons(&env(&[("LC_ALL", "C.utf8"), ("LANG", "C")])));
        assert!(!resolve_icons(&env(&[("LANG", "C")])));
        assert!(!resolve_icons(&env(&[])));
        assert!(!resolve_icons(&env(&[
            ("LANG", "en_US.UTF-8"),
            ("TERM", "dumb")
        ])));
    }

    #[test]
    fn every_lint_category_has_an_icon() {
        for c in ["net", "fs", "env", "exec", "directive", "policy"] {
            assert_ne!(category_icon(c), "•", "{c}");
        }
    }

    #[test]
    fn tabs_and_multibyte_do_not_misplace_the_underline() {
        let raw = "\tconst s = \"é\"; fetch(u);";
        let i = raw.find("fetch").unwrap();
        let (c, w) = cols(raw, i, raw.len() - 1);
        assert_eq!(c, 4 + "const s = \"é\"; ".chars().count());
        assert_eq!(w, "fetch(u)".chars().count());
    }
}
