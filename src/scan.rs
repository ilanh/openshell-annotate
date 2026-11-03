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
}

impl Scan {
    /// One human-readable note per extension that was skipped, for stderr.
    pub fn unscanned_notes(&self) -> Vec<String> {
        self.unscanned
            .iter()
            .map(|(ext, n)| {
                format!(
                    "{n} .{ext} file{} not scanned (no comment syntax known); if its comments start with `#` or `//`, set `[scan.comment_leaders] {ext} = \"hash\"` (or \"slash\") in openshell-annotate.toml to read @openshell comments there",
                    if *n == 1 { " was" } else { "s were" }
                )
            })
            .collect()
    }
}

pub fn rel_path(root: &Path, p: &Path) -> String {
    let r = p.strip_prefix(root).unwrap_or(p);
    r.components()
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
            if Language::detect(path, &cfg.scan.comment_leaders).is_none() {
                let rel = rel_path(root, path);
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
            let rel = rel_path(root, path);
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
