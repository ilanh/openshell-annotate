//! Linter: directive validity + heuristic detection of un-annotated network, filesystem-write,
//! secret and subprocess call sites.
//!
//! Call-site detection is regex based on the lexer's code mask (comments and string contents
//! blanked), so matches inside strings/comments are ignored. It is a heuristic: dynamic URLs and
//! SDK-hidden HTTP calls need a directive or `@openshell ignore`. A tree-sitter backend is the planned
//! upgrade (see docs/design.md).

use crate::ast::{self, Pth, Site};
use crate::config::Config;
use crate::directive::{Directive, Kind};
use crate::lexer::Language;
use crate::scan::{Scan, Source};
use crate::validate;
use regex::Regex;
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub file: String,
    pub line: usize,
    pub severity: Severity,
    pub category: &'static str,
    pub message: String,
}

const DEFAULT_EXCLUDES: &[&str] = &[
    "/test/",
    "/tests/",
    "/__tests__/",
    ".test.",
    ".spec.",
    "_test.go",
    "/fixtures/",
    "/__mocks__/",
    ".d.ts",
    "/e2e/",
];

struct Pat {
    cat: &'static str,
    code: Regex,
    /// Applied to the raw line from the code match start; must match for a hit.
    raw: Option<Regex>,
    /// Applied to the raw line from the code match start; a match suppresses the hit.
    not_raw: Option<Regex>,
    what: &'static str,
}

fn p(cat: &'static str, what: &'static str, code: &str) -> Pat {
    Pat {
        cat,
        code: Regex::new(code).unwrap(),
        raw: None,
        not_raw: None,
        what,
    }
}

fn patterns(lang: Language) -> &'static Vec<Pat> {
    static JS: OnceLock<Vec<Pat>> = OnceLock::new();
    static PY: OnceLock<Vec<Pat>> = OnceLock::new();
    static GO: OnceLock<Vec<Pat>> = OnceLock::new();
    static SH: OnceLock<Vec<Pat>> = OnceLock::new();
    match lang {
        Language::JavaScript => JS.get_or_init(|| {
            vec![
                p("net", "fetch()", r"(?:^|[^.\w])fetch\s*\(|globalThis\.fetch\s*\("),
                p("net", "axios", r"\baxios\s*[.(]"),
                p("net", "node http(s) request", r"\b(?:https?\.(?:request|get)|httpsRequest|httpRequest)\s*\("),
                p("net", "WebSocket client", r"new\s+WebSocket\s*\("),
                p("net", "EventSource", r"new\s+EventSource\s*\("),
                p("net", "raw socket", r"\b(?:net\.(?:connect|createConnection)|tls\.connect)\s*\("),
                p("fs", "filesystem write", r"\b(?:writeFile|writeFileSync|appendFile|appendFileSync|createWriteStream|mkdir|mkdirSync|mkdtemp|mkdtempSync|unlink|unlinkSync|rename|renameSync|copyFile|copyFileSync|rmSync|truncate|truncateSync|symlink|symlinkSync)\s*\(|\bfs\w*\.(?:rm|cp|cpSync)\s*\("),
                p("env", "environment read", r"\bprocess\.env\b"),
                p("exec", "subprocess", r"(?:^|[^.\w])(?:spawn|spawnSync|execFile|execFileSync|execSync|fork)\s*\(|\bchild_process\.\w+\s*\("),
            ]
        }),
        Language::Python => PY.get_or_init(|| {
            let mut open = p("fs", "open() for writing", r"\bopen\s*\(");
            open.raw = Some(Regex::new(r#"^open\s*\([^)]*,\s*(?:mode\s*=\s*)?['"][rbt]*[wax+][^'"]*['"]"#).unwrap());
            vec![
                p("net", "requests", r"\brequests\.(?:get|post|put|patch|delete|head|request|Session)\b"),
                p("net", "httpx", r"\bhttpx\.\w+"),
                p("net", "urllib", r"\burllib\.request\.|\burlopen\s*\("),
                p("net", "aiohttp", r"\baiohttp\.ClientSession\b"),
                p("net", "raw socket", r"\bsocket\.create_connection\s*\(|\bwebsockets\.connect\s*\("),
                open,
                p("fs", "filesystem write", r"\.write_text\s*\(|\.write_bytes\s*\(|\bshutil\.(?:copy\w*|move|rmtree)\s*\(|\bos\.(?:remove|rename|makedirs|mkdir|unlink|rmdir)\s*\("),
                p("env", "environment read", r"\bos\.(?:environ|getenv)\b"),
                p("exec", "subprocess", r"\bsubprocess\.\w+\s*\(|\bos\.(?:system|popen)\s*\("),
            ]
        }),
        Language::Go => GO.get_or_init(|| {
            vec![
                p("net", "net/http client", r"\bhttp\.(?:Get|Post|PostForm|Head|NewRequest|NewRequestWithContext|DefaultClient)\b"),
                p("net", "raw socket", r"\b(?:net\.(?:Dial|DialTimeout)|tls\.Dial|websocket\.Dial)\s*\("),
                p("fs", "filesystem write", r"\bos\.(?:WriteFile|Create|OpenFile|MkdirAll|Mkdir|MkdirTemp|CreateTemp|Remove|RemoveAll|Rename)\s*\(|\bioutil\.WriteFile\s*\("),
                p("env", "environment read", r"\bos\.Getenv\s*\("),
                p("exec", "subprocess", r"\bexec\.Command(?:Context)?\s*\("),
            ]
        }),
        Language::Shell => SH.get_or_init(|| {
            let mut redir = p("fs", "output redirection", r"(?:^|[^>&0-9<\-=])>>?\s*[^&\s>]");
            redir.not_raw = Some(Regex::new(r"^>>?\s*(?:/dev/(?:null|stderr|stdout|tty)|&)").unwrap());
            vec![
                p("net", "curl/wget", r"\b(?:curl|wget)\b"),
                p("net", "git remote operation", r"\bgit\s+(?:clone|fetch|pull|push|ls-remote)\b"),
                p("net", "package manager", r"\b(?:npm|pnpm|yarn)\s+(?:install|i|add|ci|publish|view)\b|\bpip3?\s+install\b|\bapt(?:-get)?\s+(?:install|update)\b"),
                p("net", "container registry", r"\bdocker\s+(?:pull|push)\b"),
                redir,
                p("fs", "filesystem write", r"(?:^|[;&|(]\s*|\s)(?:mkdir|rm|cp|mv|touch|tee|chmod|chown|ln)\s+"),
                p("env", "environment read", r"\$\{?[A-Z]"),
            ]
        }),
    }
}

fn env_name_re(lang: Language) -> Regex {
    Regex::new(match lang {
        Language::JavaScript => r#"process\.env(?:\.|\[\s*["'])([A-Z][A-Z0-9_]*)"#,
        Language::Python => {
            r#"os\.(?:environ(?:\.get)?\s*[\(\[]|getenv\s*\()\s*["']([A-Z][A-Z0-9_]*)["']"#
        }
        Language::Go => r#"os\.Getenv\(\s*"([A-Z][A-Z0-9_]*)""#,
        Language::Shell => r"\$\{?([A-Z][A-Z0-9_]*)",
    })
    .unwrap()
}

pub fn secret_like(name: &str) -> bool {
    let parts: Vec<&str> = name.split('_').collect();
    let last = *parts.last().unwrap_or(&"");
    if matches!(
        last,
        "FILE"
            | "PATH"
            | "DIR"
            | "URL"
            | "URI"
            | "ENV"
            | "HOST"
            | "HOSTS"
            | "PORT"
            | "ID"
            | "NAME"
            | "MS"
            | "SECONDS"
            | "LIMIT"
            | "WINDOW"
            | "TTL"
            | "TIMEOUT"
            | "ENABLED"
            | "DISABLED"
    ) {
        return false;
    }
    parts.iter().any(|t| {
        matches!(
            *t,
            "KEY"
                | "TOKEN"
                | "SECRET"
                | "PASSWORD"
                | "PASSWD"
                | "CREDENTIAL"
                | "CREDENTIALS"
                | "PAT"
                | "APIKEY"
        )
    })
}

pub(crate) fn url_hosts(raw: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?:https?|wss?)://([A-Za-z0-9][A-Za-z0-9.\-]*[A-Za-z0-9])(?::(\d+))?").unwrap()
    });
    let mut v = Vec::new();
    for c in re.captures_iter(raw) {
        let whole = c.get(0).unwrap().as_str();
        let default = if whole.starts_with("https") || whole.starts_with("wss") {
            "443"
        } else {
            "80"
        };
        let host = &c[1];
        if host.contains('.') && !host.starts_with("127.") && host != "localhost" {
            v.push(format!(
                "{host}:{}",
                c.get(2).map_or(default, |m| m.as_str())
            ));
        }
    }
    v.dedup();
    v
}

fn excluded(rel: &str, cfg: &Config) -> bool {
    let path = format!("/{rel}");
    DEFAULT_EXCLUDES.iter().any(|x| path.contains(x))
        || cfg
            .lint
            .exclude
            .iter()
            .any(|x| crate::config::path_matches(rel, x))
}

pub fn lint(scan: &Scan, cfg: &Config) -> Vec<Finding> {
    let mut out = Vec::new();
    for e in &scan.errors {
        out.push(Finding {
            file: e.file.clone(),
            line: e.line,
            severity: Severity::Error,
            category: "directive",
            message: e.message.clone(),
        });
    }
    for d in &scan.directives {
        for m in validate::check_directive(d) {
            out.push(Finding {
                file: d.file.clone(),
                line: d.line,
                severity: Severity::Error,
                category: "directive",
                message: m,
            });
        }
    }
    let provider_envs: BTreeSet<&str> = scan
        .directives
        .iter()
        .filter_map(|d| {
            if let Kind::Provider { env } = &d.kind {
                Some(env.as_str())
            } else {
                None
            }
        })
        .collect();
    let fs_roots: Vec<String> = scan
        .directives
        .iter()
        .filter_map(|d| match &d.kind {
            Kind::Fs { write: true, path } => Some(path.clone()),
            _ => None,
        })
        .collect();
    let egress: Vec<(String, Vec<u16>)> = scan
        .directives
        .iter()
        .filter_map(|d| match &d.kind {
            Kind::Egress { host, ports } => Some((host.clone(), ports.clone())),
            _ => None,
        })
        .collect();
    let mut dyn_fs: Vec<(String, Vec<usize>)> = Vec::new();
    for src in &scan.sources {
        if excluded(&src.rel, cfg) {
            continue;
        }
        let here: Vec<&Directive> = scan
            .directives
            .iter()
            .filter(|d| d.file == src.rel)
            .collect();
        let covered = |line: usize, cat: &str| -> bool {
            here.iter().any(|d| {
                d.covers().contains(&cat)
                    && (d.file_scoped() || (d.line <= line && line - d.line <= cfg.lint.window))
            })
        };
        if cfg.lint.engine != "regex" {
            if let Some(sites) = ast::sites(src.lang, &src.rel, &src.lexed.raw.join("\n")) {
                lint_sites(
                    src,
                    sites,
                    cfg,
                    &Ctx {
                        provider_envs: &provider_envs,
                        fs_roots: &fs_roots,
                        egress: &egress,
                    },
                    &covered,
                    &mut out,
                    &mut dyn_fs,
                );
                continue;
            }
        }
        let env_re = env_name_re(src.lang);
        for (i, code) in src.lexed.code.iter().enumerate() {
            let line = i + 1;
            let raw = src.lexed.raw.get(i).map(String::as_str).unwrap_or("");
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            for pat in patterns(src.lang) {
                if !cfg.lint.categories.is_empty()
                    && !cfg.lint.categories.iter().any(|c| c == pat.cat)
                {
                    continue;
                }
                if pat.cat != "env" && seen.contains(pat.cat) {
                    continue;
                }
                let Some(m) = pat.code.find(code) else {
                    continue;
                };
                let raw_full: String = {
                    let off = code[..m.start()].chars().count();
                    raw.chars().skip(off).collect()
                };
                let raw_from = raw_full.trim_start_matches(|c: char| {
                    !(c.is_alphanumeric() || c == '>' || c == '$' || c == '_')
                });
                if let Some(r) = &pat.raw {
                    if !r.is_match(raw_from) {
                        continue;
                    }
                }
                if let Some(r) = &pat.not_raw {
                    // For redirections the regex consumed the char before `>`; re-anchor at `>`.
                    let at = raw_from
                        .find('>')
                        .map(|k| &raw_from[k..])
                        .unwrap_or(raw_from);
                    if r.is_match(at) {
                        continue;
                    }
                }
                match pat.cat {
                    "env" => {
                        let names: Vec<String> = env_re
                            .captures_iter(&raw_full)
                            .map(|c| c[1].to_string())
                            .filter(|n| secret_like(n))
                            .collect();
                        for n in names {
                            if provider_envs.contains(n.as_str()) || covered(line, "env") {
                                continue;
                            }
                            out.push(Finding {
                                file: src.rel.clone(),
                                line,
                                severity: Severity::Warning,
                                category: "env",
                                message: format!("reads secret-like variable {n} with no `@openshell provider {n}`; if a provider supplies it, declare it; otherwise `@openshell ignore kind=env reason=\"...\"`"),
                            });
                        }
                    }
                    "exec" => {
                        if !covered(line, "exec") {
                            out.push(Finding { file: src.rel.clone(), line, severity: Severity::Info, category: "exec", message: format!("{}: the spawned program is the binary that opens its own connections; if it talks to the network add `bin=` on its egress, else `@openshell ignore kind=exec reason=\"...\"`", pat.what) });
                        }
                    }
                    cat => {
                        if covered(line, cat) {
                            continue;
                        }
                        let hint = if cat == "net" {
                            let hosts = url_hosts(raw);
                            if hosts.is_empty() {
                                " (no literal URL on this line: declare the host with @openshell egress, or @openshell ignore kind=net)".to_string()
                            } else {
                                format!(
                                    " (suggest: @openshell egress {} reason=\"...\")",
                                    hosts.join(", ")
                                )
                            }
                        } else {
                            " (add `@openshell fs write <absolute in-sandbox path> reason=\"...\"`, or `@openshell ignore kind=fs`)".to_string()
                        };
                        out.push(Finding {
                            file: src.rel.clone(),
                            line,
                            severity: Severity::Warning,
                            category: if cat == "net" { "net" } else { "fs" },
                            message: format!("{} without an @openshell directive{hint}", pat.what),
                        });
                    }
                }
                seen.insert(pat.cat);
            }
        }
    }
    if cfg.lint.fs_dynamic == "summary" && !dyn_fs.is_empty() && !fs_roots.is_empty() {
        let total: usize = dyn_fs.iter().map(|d| d.1.len()).sum();
        dyn_fs.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
        let top: Vec<String> = dyn_fs
            .iter()
            .take(5)
            .map(|(f, l)| format!("{f} ({})", l.len()))
            .collect();
        out.push(Finding {
            file: ".".into(),
            line: 0,
            severity: Severity::Info,
            category: "fs",
            message: format!(
                "{total} filesystem write(s) in {} file(s) use non-literal paths and cannot be checked statically; they must land under the declared fs write roots ({}) or /tmp. Most: {}. Set [lint] fs_dynamic = \"info\" for one line per file.",
                dyn_fs.len(),
                fs_roots.join(", "),
                top.join(", ")
            ),
        });
    }
    out.sort_by(|a, b| (a.severity, &a.file, a.line).cmp(&(b.severity, &b.file, b.line)));
    out
}

struct Ctx<'a> {
    provider_envs: &'a BTreeSet<&'a str>,
    fs_roots: &'a [String],
    egress: &'a [(String, Vec<u16>)],
}

fn host_known(ctx: &Ctx, hp: &str) -> bool {
    let Some((h, p)) = hp.rsplit_once(':') else {
        return false;
    };
    let p: u16 = p.parse().unwrap_or(443);
    ctx.egress
        .iter()
        .any(|(eh, ports)| crate::diff_logs::host_matches(eh, h) && ports.contains(&p))
}

fn placeholder_host(hp: &str) -> bool {
    let h = hp.rsplit_once(':').map_or(hp, |x| x.0);
    h.contains("example.")
        || h.ends_with(".test")
        || h.ends_with(".local")
        || h.ends_with(".internal")
        || matches!(
            h,
            "www.w3.org" | "w3.org" | "json-schema.org" | "schemas.xmlsoap.org" | "xmlns.com"
        )
}

/// AST-site lint for one source file.
fn lint_sites(
    src: &Source,
    mut sites: Vec<Site>,
    cfg: &Config,
    ctx: &Ctx,
    covered: &dyn Fn(usize, &str) -> bool,
    out: &mut Vec<Finding>,
    dyn_fs: &mut Vec<(String, Vec<usize>)>,
) {
    sites.sort_by_key(|s| s.line);
    let want =
        |c: &str| cfg.lint.categories.is_empty() || cfg.lint.categories.iter().any(|x| x == c);
    let mut net_lines: BTreeSet<usize> = BTreeSet::new();
    let mut seen_env: BTreeSet<(usize, String)> = BTreeSet::new();
    let mut file_dyn: Vec<usize> = Vec::new();
    let mut push = |line: usize, severity: Severity, category: &'static str, message: String| {
        out.push(Finding {
            file: src.rel.clone(),
            line,
            severity,
            category,
            message,
        });
    };
    // calls first so that a URL literal on the same line as a flagged call is not reported twice
    sites.sort_by_key(|s| (s.line, !s.call));
    for s in &sites {
        if !want(s.cat) {
            continue;
        }
        match s.cat {
            "env" => {
                let Some(n) = &s.env else { continue };
                if !secret_like(n)
                    || ctx.provider_envs.contains(n.as_str())
                    || covered(s.line, "env")
                    || !seen_env.insert((s.line, n.clone()))
                {
                    continue;
                }
                push(s.line, Severity::Warning, "env", format!("reads secret-like variable {n} with no `@openshell provider {n}`; if a provider supplies it, declare it; otherwise `@openshell ignore kind=env reason=\"...\"`"));
            }
            "exec" if !covered(s.line, "exec") => {
                push(s.line, Severity::Info, "exec", format!("{}: the spawned program is the binary that opens its own connections; if it talks to the network add `bin=` on its egress, else `@openshell ignore kind=exec reason=\"...\"`", s.what));
            }
            "net" => {
                if covered(s.line, "net") {
                    continue;
                }
                let unknown: Vec<&String> = s
                    .urls
                    .iter()
                    .filter(|u| !host_known(ctx, u) && (s.call || !placeholder_host(u)))
                    .collect();
                if s.call {
                    if !s.urls.is_empty() && unknown.is_empty() {
                        net_lines.insert(s.line);
                        continue;
                    }
                    let hint = if unknown.is_empty() {
                        " (no literal URL in the arguments: declare the host with @openshell egress, or @openshell ignore kind=net)".to_string()
                    } else {
                        format!(
                            " (suggest: @openshell egress {} reason=\"...\")",
                            unknown
                                .iter()
                                .map(|x| x.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    };
                    net_lines.insert(s.line);
                    push(
                        s.line,
                        Severity::Warning,
                        "net",
                        format!("{} without an @openshell directive{hint}", s.what),
                    );
                } else if !unknown.is_empty() && !net_lines.contains(&s.line) {
                    net_lines.insert(s.line);
                    push(s.line, Severity::Info, "net", format!("URL literal to {} has no matching @openshell egress (suggest: @openshell egress {} reason=\"...\")", unknown[0], unknown.iter().map(|x| x.as_str()).collect::<Vec<_>>().join(", ")));
                }
            }
            "fs" => {
                let p = s.path.clone().unwrap_or(Pth::Dynamic);
                if ast::is_covered_path(&p, ctx.fs_roots) || covered(s.line, "fs") {
                    continue;
                }
                match p {
                    Pth::Literal(l) => push(s.line, Severity::Warning, "fs", format!("{} to {l}, which is outside the sandbox baseline (/tmp) and every declared fs write root (add `@openshell fs write <dir> reason=\"...\"` or `@openshell ignore kind=fs`)", s.what)),
                    _ => file_dyn.push(s.line),
                }
            }
            _ => {}
        }
    }
    if !file_dyn.is_empty() && cfg.lint.fs_dynamic != "off" {
        let first = file_dyn[0];
        let n = file_dyn.len();
        let no_roots = ctx.fs_roots.is_empty();
        if no_roots || cfg.lint.fs_dynamic == "warn" {
            push(first, Severity::Warning, "fs", format!("{n} filesystem write(s) with non-literal paths (first here) and {}: declare where the app may write (`@openshell fs write <dir> reason=\"...\"`) or `@openshell ignore kind=fs scope=file`", if no_roots { "no `@openshell fs write` root declared anywhere" } else { "fs_dynamic = warn" }));
        } else if cfg.lint.fs_dynamic == "info" {
            push(first, Severity::Info, "fs", format!("{n} filesystem write(s) with non-literal paths (first here); they must land under a declared fs write root or /tmp"));
        } else {
            dyn_fs.push((src.rel.clone(), file_dyn));
        }
    }
}

#[cfg(test)]
mod secret_name_tests {
    use super::secret_like;

    #[test]
    fn secret_names() {
        for n in [
            "OPENAI_API_KEY",
            "GITHUB_TOKEN",
            "BETTER_AUTH_SECRET",
            "DB_PASSWORD",
        ] {
            assert!(secret_like(n), "{n}");
        }
        for n in [
            "PAPERCLIP_MCP_GATEWAY_TOKEN_REQUEST_LIMIT",
            "PAPERCLIP_TOKEN_BROKER_ALLOWED_HOSTS",
            "TOKEN_FILE",
            "AWS_CONTAINER_CREDENTIALS_FULL_URI",
            "PATH",
        ] {
            assert!(!secret_like(n), "{n}");
        }
    }
}
