//! Repository walk + directive extraction.

use crate::config::Config;
use crate::directive::{extract, Directive, ParseError};
use crate::lexer::{lex, Language, Lexed};
use anyhow::Result;
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "dist",
    "build",
    "target",
    ".git",
    "vendor",
    ".next",
    ".turbo",
];
const MAX_BYTES: u64 = 2 * 1024 * 1024;

pub struct Source {
    /// Path relative to the scan root, `/` separated.
    pub rel: String,
    pub lang: Language,
    pub lexed: Lexed,
}

#[derive(Default)]
pub struct Scan {
    pub sources: Vec<Source>,
    pub directives: Vec<Directive>,
    pub errors: Vec<ParseError>,
    /// Source files in languages with no known comment syntax, by extension (see `lexer::KNOWN_UNSCANNED`).
    pub unscanned: std::collections::BTreeMap<String, usize>,
    /// `[scan].exclude` / `[lint].exclude` patterns that matched no file in a whole-repo scan.
    pub dead_excludes: Vec<DeadExclude>,
}

impl Scan {
    /// One human-readable note per extension that was skipped, for stderr.
    pub fn unscanned_notes(&self) -> Vec<String> {
        self.unscanned
            .iter()
            .map(|(ext, n)| {
                let head = format!(
                    "{n} .{ext} file{} not scanned",
                    if *n == 1 { " was" } else { "s were" }
                );
                let style = crate::lexer::UNSCANNED_COMMENT_STYLE
                    .iter()
                    .find(|(e, _)| e == ext)
                    .map(|(_, s)| *s);
                match style {
                    Some(kind @ ("hash" | "slash")) => format!(
                        "{head} (no comment leader set); its comments start with `{}`, so set `[scan.comment_leaders] {ext} = \"{kind}\"` in openshell-annotate.toml to read @openshell comments there",
                        if kind == "hash" { "#" } else { "//" }
                    ),
                    Some(leader) => format!(
                        "{head}: its comments start with `{leader}`, which this tool does not read; put its directives in a `#` comment in a sidecar shell file (for example annotations.sh) and name the source file in the reason"
                    ),
                    None => format!(
                        "{head} (no comment syntax known); if its comments start with `#` or `//`, set `[scan.comment_leaders] {ext} = \"hash\"` (or \"slash\") in openshell-annotate.toml to read @openshell comments there"
                    ),
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadExclude {
    /// `scan` or `lint`.
    pub section: &'static str,
    pub pattern: String,
}

impl DeadExclude {
    /// True when the pattern uses regex syntax, which `exclude` does not support.
    pub fn looks_like_regex(&self) -> bool {
        let p = self.pattern.strip_prefix('^').unwrap_or(&self.pattern);
        p.contains(['(', ')', '|', '[', ']', '+', '?', '*', '\\', '$'])
    }

    pub fn note(&self) -> String {
        let hint = if self.looks_like_regex() {
            "exclude patterns are not regexes: `^prefix/` anchors at the repo root, anything else is a literal substring of `/<relative path>`; list one entry per directory"
        } else {
            "check the spelling: `^prefix/` anchors at the repo root, anything else is a literal substring of `/<relative path>`"
        };
        format!(
            "[{}].exclude pattern `{}` matches no file; {hint}",
            self.section, self.pattern
        )
    }
}

pub fn rel_path(root: &Path, p: &Path) -> String {
    let r = p.strip_prefix(root).unwrap_or(p);
    // `scan .` yields `./src/a.ts`; a leading `.` would break `^prefix` excludes and show up in policy comments
    r.components()
        .filter(|c| !matches!(c, std::path::Component::CurDir))
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

pub fn scan(root: &Path, paths: &[PathBuf], cfg: &Config) -> Result<Scan> {
    let mut out = Scan::default();
    let roots: Vec<PathBuf> = if paths.is_empty() {
        vec![root.to_path_buf()]
    } else {
        paths.to_vec()
    };
    let mut files = Vec::new();
    // exclude patterns that match nothing are almost always typos or regex syntax: report them (whole-repo scans only)
    let patterns: Vec<(&'static str, String)> = cfg
        .scan
        .exclude
        .iter()
        .map(|x| ("scan", x.clone()))
        .chain(cfg.lint.exclude.iter().map(|x| ("lint", x.clone())))
        .collect();
    let mut hit = vec![false; patterns.len()];
    let whole_repo = paths.is_empty() || paths.iter().all(|p| p == root || p == Path::new("."));
    for p in &roots {
        let mut wb = WalkBuilder::new(p);
        wb.hidden(true)
            .git_ignore(true)
            .git_global(false)
            .git_exclude(true)
            .require_git(false);
        wb.filter_entry(|e| {
            e.file_name().to_str().is_none_or(|n| {
                !(e.file_type().is_some_and(|t| t.is_dir()) && SKIP_DIRS.contains(&n))
            })
        });
        for ent in wb.build() {
            let ent = ent?;
            if !ent.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let path = ent.path();
            let rel = rel_path(root, path);
            for (i, x) in patterns.iter().enumerate() {
                if !hit[i] && crate::config::path_matches(&rel, &x.1) {
                    hit[i] = true;
                }
            }
            if Language::detect(path, &cfg.scan.comment_leaders).is_none() {
                let skipped = cfg
                    .scan
                    .exclude
                    .iter()
                    .any(|x| crate::config::path_matches(&rel, x));
                if let (Some(ext), false) = (path.extension().and_then(|e| e.to_str()), skipped) {
                    let ext = ext.to_ascii_lowercase();
                    if crate::lexer::KNOWN_UNSCANNED.contains(&ext.as_str())
                        || cfg.scan.comment_leaders.get(&ext).map(String::as_str) == Some("none")
                    {
                        *out.unscanned.entry(ext).or_insert(0) += 1;
                    }
                }
                continue;
            }
            if ent.metadata().map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
                continue;
            }
            if cfg
                .scan
                .exclude
                .iter()
                .any(|x| crate::config::path_matches(&rel, x))
            {
                continue;
            }
            files.push((rel, path.to_path_buf()));
        }
    }
    if whole_repo {
        for (i, (section, pattern)) in patterns.iter().enumerate() {
            if !hit[i] {
                out.dead_excludes.push(DeadExclude {
                    section,
                    pattern: pattern.clone(),
                });
            }
        }
    }
    files.sort();
    files.dedup_by(|a, b| a.0 == b.0);
    for (rel, path) in files {
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lang = Language::detect(&path, &cfg.scan.comment_leaders).unwrap();
        let mut lexed = lex(lang, &src);
        if lang == Language::JavaScript {
            // the grammar knows regex literals and JSX; the hand-written lexer does not
            if let Some(c) = crate::ast::comments(lang, &rel, &src) {
                lexed.comments = c;
            }
        }
        let (ds, es) = extract(&rel, lang, &lexed.comments);
        out.directives.extend(ds);
        out.errors.extend(es);
        out.sources.push(Source { rel, lang, lexed });
    }
    Ok(out)
}
