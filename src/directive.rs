//! `@openshell` directive grammar and parser.
//!
//! The grammar and the per-verb attribute tables live in [`crate::spec`] (single source of truth, also used to
//! generate `docs/DIRECTIVES.md`).
//!
//! A directive must be the first thing in a comment line (after the comment
//! leader), so prose that merely mentions `@openshell` is ignored.

use crate::lexer::{Comment, Language};
use crate::spec;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;

pub const MARKER: &str = "@openshell";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attrs {
    /// Single-valued attributes (last one wins is *not* allowed: duplicates are errors).
    pub single: BTreeMap<String, String>,
    /// Repeatable attributes: `allow`, `deny`.
    pub multi: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Kind {
    Egress { host: String, ports: Vec<u16> },
    Provider { env: String },
    Fs { write: bool, path: String },
    Bin { path: String },
    Process,
    Ignore,
}

impl Kind {
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Egress { .. } => "egress",
            Kind::Provider { .. } => "provider",
            Kind::Fs { write: true, .. } => "fs write",
            Kind::Fs { write: false, .. } => "fs read",
            Kind::Bin { .. } => "bin",
            Kind::Process => "process",
            Kind::Ignore => "ignore",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Directive {
    pub file: String,
    pub line: usize,
    #[serde(skip)]
    pub lang: Option<Language>,
    #[serde(flatten)]
    pub kind: Kind,
    pub attrs: Attrs,
}

impl Directive {
    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.single.get(k).map(String::as_str)
    }
    pub fn reason(&self) -> Option<&str> {
        self.attr("reason")
    }
    pub fn loc(&self) -> String {
        format!("{}:{}", self.file, self.line)
    }
    /// Lint category this directive covers (`net`, `fs`, `env`, `exec`).
    pub fn covers(&self) -> Vec<&'static str> {
        match &self.kind {
            Kind::Egress { .. } => vec!["net"],
            Kind::Fs { write: true, .. } => vec!["fs"],
            Kind::Provider { .. } => vec!["env"],
            Kind::Bin { .. } => vec!["exec"],
            Kind::Ignore => match self.attr("kind").unwrap_or("all") {
                "net" => vec!["net"],
                "fs" => vec!["fs"],
                "env" => vec!["env"],
                "exec" => vec!["exec"],
                _ => vec!["net", "fs", "env", "exec"],
            },
            _ => vec![],
        }
    }
    pub fn file_scoped(&self) -> bool {
        self.attr("scope") == Some("file")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParseError {
    pub file: String,
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.message)
    }
}

/// Extract directives from lexed comments.
pub fn extract(
    file: &str,
    lang: Language,
    comments: &[Comment],
) -> (Vec<Directive>, Vec<ParseError>) {
    let mut ds = Vec::new();
    let mut es = Vec::new();
    for c in comments {
        let t = c.text.trim();
        let Some(rest) = t.strip_prefix(MARKER) else {
            continue;
        };
        // `@openshell-foo` or `@openshellfoo` is not our marker.
        if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
            continue;
        }
        match parse_body(rest.trim()) {
            Ok((kind, attrs)) => ds.push(Directive {
                file: file.to_string(),
                line: c.line,
                lang: Some(lang),
                kind,
                attrs,
            }),
            Err(m) => es.push(ParseError {
                file: file.to_string(),
                line: c.line,
                message: m,
            }),
        }
    }
    (ds, es)
}

#[derive(Debug, PartialEq)]
enum Tok {
    Word(String),
    Pair(String, String),
}

fn tokenize(s: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        // An absolute path may contain `=` (e.g. pnpm store dirs): keep it as one word.
        let is_path = chars[start] == '/';
        while i < chars.len()
            && !chars[i].is_whitespace()
            && (is_path || (chars[i] != '=' && chars[i] != '"'))
        {
            i += 1;
        }
        if i < chars.len() && chars[i] == '=' {
            let key: String = chars[start..i].iter().collect();
            if key.is_empty() {
                return Err("attribute with empty key".into());
            }
            i += 1;
            let val: String = if i < chars.len() && chars[i] == '"' {
                i += 1;
                let mut v = String::new();
                let mut closed = false;
                while i < chars.len() {
                    match chars[i] {
                        '\\' if i + 1 < chars.len()
                            && (chars[i + 1] == '"' || chars[i + 1] == '\\') =>
                        {
                            v.push(chars[i + 1]);
                            i += 2;
                        }
                        '"' => {
                            closed = true;
                            i += 1;
                            break;
                        }
                        c => {
                            v.push(c);
                            i += 1;
                        }
                    }
                }
                if !closed {
                    return Err(format!("unterminated quoted value for `{key}`"));
                }
                if i < chars.len() && !chars[i].is_whitespace() {
                    return Err(format!("unexpected text after quoted value for `{key}`"));
                }
                v
            } else {
                let vs = i;
                while i < chars.len() && !chars[i].is_whitespace() {
                    if chars[i] == '"' {
                        return Err(format!(
                            "unexpected quote inside value of `{key}` (quote the whole value)"
                        ));
                    }
                    i += 1;
                }
                chars[vs..i].iter().collect()
            };
            out.push(Tok::Pair(key, val));
        } else if i < chars.len() && chars[i] == '"' {
            return Err("unexpected quote (values must be written key=\"value\")".into());
        } else {
            out.push(Tok::Word(chars[start..i].iter().collect()));
        }
    }
    Ok(out)
}

fn allowed_attrs(verb: &str) -> (Vec<&'static str>, Vec<&'static str>) {
    match spec::verb(verb) {
        Some(v) => (
            v.attrs
                .iter()
                .filter(|a| !a.multi)
                .map(|a| a.name)
                .chain(spec::COMMON_ATTRS.iter().map(|a| a.name))
                .collect(),
            v.attrs.iter().filter(|a| a.multi).map(|a| a.name).collect(),
        ),
        None => (vec![], vec![]),
    }
}

fn parse_body(body: &str) -> Result<(Kind, Attrs), String> {
    let toks = tokenize(body)?;
    let mut it = toks.into_iter();
    let verb = match it.next() {
        Some(Tok::Word(w)) => w,
        Some(Tok::Pair(k, _)) => {
            return Err(format!(
                "expected a directive keyword ({}), found attribute `{k}=`",
                spec::verb_names()
            ))
        }
        None => {
            return Err(format!(
                "empty directive; expected one of: {}",
                spec::verb_names()
            ))
        }
    };
    let mut words = Vec::new();
    let mut attrs = Attrs {
        single: BTreeMap::new(),
        multi: BTreeMap::new(),
    };
    let (single_ok, multi_ok) = allowed_attrs(&verb);
    if spec::verb(&verb).is_none() {
        return Err(format!(
            "unknown directive `{verb}`; expected one of: {}",
            spec::verb_names()
        ));
    }
    for t in it {
        match t {
            Tok::Word(w) => words.push(w),
            Tok::Pair(k, v) => {
                if multi_ok.contains(&k.as_str()) {
                    attrs.multi.entry(k).or_default().push(v);
                } else if single_ok.contains(&k.as_str()) {
                    if attrs.single.insert(k.clone(), v).is_some() {
                        return Err(format!("duplicate attribute `{k}`"));
                    }
                } else {
                    return Err(format!("unknown attribute `{k}` for `{verb}`"));
                }
            }
        }
    }
    let need = |n: usize, usage: &str| -> Result<(), String> {
        if words.len() != n {
            Err(format!("usage: {usage}"))
        } else {
            Ok(())
        }
    };
    if let Some(s) = attrs.single.get("scope") {
        if s != "file" && s != "next" {
            return Err(format!("`scope` must be `file` or `next`, got `{s}`"));
        }
    }
    let kind = match verb.as_str() {
        "egress" => {
            need(1, spec::verb("egress").unwrap().usage)?;
            let (host, ports) = parse_target(&words[0])?;
            Kind::Egress { host, ports }
        }
        "provider" => {
            need(1, spec::verb("provider").unwrap().usage)?;
            let env = words[0].clone();
            if !valid_env(&env) {
                return Err(format!(
                    "`{env}` is not a valid environment variable name ([A-Z_][A-Z0-9_]*)"
                ));
            }
            Kind::Provider { env }
        }
        "fs" => {
            need(2, spec::verb("fs").unwrap().usage)?;
            let write = match words[0].as_str() {
                "read" => false,
                "write" => true,
                o => return Err(format!("fs mode must be `read` or `write`, got `{o}`")),
            };
            Kind::Fs {
                write,
                path: words[1].clone(),
            }
        }
        "bin" => {
            need(1, spec::verb("bin").unwrap().usage)?;
            Kind::Bin {
                path: words[0].clone(),
            }
        }
        "process" => {
            need(0, spec::verb("process").unwrap().usage)?;
            Kind::Process
        }
        "ignore" => {
            need(0, spec::verb("ignore").unwrap().usage)?;
            if let Some(k) = attrs.single.get("kind") {
                if !matches!(k.as_str(), "net" | "fs" | "env" | "exec" | "all") {
                    return Err(format!(
                        "`kind` must be net, fs, env, exec or all, got `{k}`"
                    ));
                }
            }
            Kind::Ignore
        }
        _ => unreachable!(),
    };
    Ok((kind, attrs))
}

fn valid_env(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_uppercase() || c == '_')
        && cs.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn parse_target(t: &str) -> Result<(String, Vec<u16>), String> {
    let (host, ports) = t.rsplit_once(':').ok_or_else(|| {
        format!("`{t}`: a port is required (host:port); OpenShell rejects endpoints without one")
    })?;
    if host.is_empty() {
        return Err(format!("`{t}`: empty host"));
    }
    let mut ps = Vec::new();
    for p in ports.split(',') {
        let n: u32 = p
            .parse()
            .map_err(|_| format!("`{p}` is not a port number"))?;
        if !(1..=65535).contains(&n) {
            return Err(format!("port {n} is out of range 1..65535"));
        }
        let n = n as u16;
        if !ps.contains(&n) {
            ps.push(n);
        }
    }
    Ok((host.to_ascii_lowercase(), ps))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(s: &str) -> Result<(Kind, Attrs), String> {
        parse_body(s)
    }

    #[test]
    fn egress_basic() {
        let (k, a) =
            one("egress api.openai.com:443 reason=\"LLM calls\" access=read-write").unwrap();
        assert_eq!(
            k,
            Kind::Egress {
                host: "api.openai.com".into(),
                ports: vec![443]
            }
        );
        assert_eq!(a.single["reason"], "LLM calls");
        assert_eq!(a.single["access"], "read-write");
    }

    #[test]
    fn egress_ports_and_multi_allow() {
        let (k, a) =
            one("egress Example.com:80,443 allow=\"GET:/v1/**\" allow=\"POST:/v1/x\" reason=x")
                .unwrap();
        assert_eq!(
            k,
            Kind::Egress {
                host: "example.com".into(),
                ports: vec![80, 443]
            }
        );
        assert_eq!(a.multi["allow"], vec!["GET:/v1/**", "POST:/v1/x"]);
    }

    #[test]
    fn egress_requires_port() {
        assert!(one("egress api.openai.com reason=x")
            .unwrap_err()
            .contains("port is required"));
        assert!(one("egress a.test:99999 reason=x")
            .unwrap_err()
            .contains("out of range"));
        assert!(one("egress a.test:abc reason=x")
            .unwrap_err()
            .contains("not a port"));
    }

    #[test]
    fn provider_fs_bin_ignore() {
        assert_eq!(
            one("provider OPENAI_API_KEY").unwrap().0,
            Kind::Provider {
                env: "OPENAI_API_KEY".into()
            }
        );
        assert!(one("provider openai_key")
            .unwrap_err()
            .contains("valid environment"));
        assert_eq!(
            one("fs write /workspace/data reason=cache").unwrap().0,
            Kind::Fs {
                write: true,
                path: "/workspace/data".into()
            }
        );
        assert!(one("fs rw /x").unwrap_err().contains("`read` or `write`"));
        assert_eq!(
            one("bin /usr/local/bin/node").unwrap().0,
            Kind::Bin {
                path: "/usr/local/bin/node".into()
            }
        );
        assert_eq!(
            one("ignore kind=net reason=\"dynamic\" scope=file")
                .unwrap()
                .0,
            Kind::Ignore
        );
        assert!(one("ignore kind=bogus reason=x")
            .unwrap_err()
            .contains("kind"));
    }

    #[test]
    fn path_with_equals_sign() {
        assert_eq!(
            one("fs write /app/.pnpm/pkg@1_patch_hash=abc/lib reason=\"x\"")
                .unwrap()
                .0,
            Kind::Fs {
                write: true,
                path: "/app/.pnpm/pkg@1_patch_hash=abc/lib".into()
            }
        );
    }

    #[test]
    fn strict_unknowns() {
        assert!(one("egress a.test:1 foo=bar")
            .unwrap_err()
            .contains("unknown attribute `foo`"));
        assert!(one("frobnicate x")
            .unwrap_err()
            .contains("unknown directive"));
        assert!(one("").unwrap_err().contains("empty directive"));
        assert!(one("egress a.test:1 reason=x reason=y")
            .unwrap_err()
            .contains("duplicate"));
        assert!(one("fs write /x access=full")
            .unwrap_err()
            .contains("unknown attribute"));
        assert!(one("egress a.test:1 reason=\"open")
            .unwrap_err()
            .contains("unterminated"));
    }

    #[test]
    fn marker_must_lead_the_comment() {
        let cs = vec![
            Comment {
                line: 1,
                text: " @openshell egress a.test:443 reason=x".into(),
            },
            Comment {
                line: 2,
                text: " see @openshell egress b.test:443".into(),
            },
            Comment {
                line: 3,
                text: " @openshell-extra foo".into(),
            },
            Comment {
                line: 4,
                text: " @openshell bogus".into(),
            },
        ];
        let (ds, es) = extract("f.ts", Language::JavaScript, &cs);
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].line, 1);
        assert_eq!(es.len(), 1);
        assert_eq!(es[0].line, 4);
    }

    #[test]
    fn escaped_quote_in_reason() {
        let (_, a) = one(r#"egress a.test:1 reason="say \"hi\"""#).unwrap();
        assert_eq!(a.single["reason"], "say \"hi\"");
    }
}
