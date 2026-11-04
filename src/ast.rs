//! Tree-sitter call-site extraction for JavaScript/TypeScript, Python, Go and shell.
//!
//! Unlike the regex engine this sees real syntax: `fetch` as a method of another object is not the global fetch, a string that
//! merely contains `open(` is not a call, `open(p, "r")` is not a write, and the *path argument* of a write can be classified
//! (absolute literal, temp directory, or dynamic) so the filesystem lint only speaks when it can say something useful.

use crate::lexer::Language;
use regex::Regex;
use std::sync::OnceLock;
use tree_sitter::{Node, Parser};

/// What is statically known about the path a filesystem write targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pth {
    /// Absolute path (or the absolute literal prefix of a computed path such as `` `/data/${id}` ``).
    Literal(String),
    /// Built from a temp-dir expression: covered by the sandbox baseline (`/tmp`).
    Temp,
    /// Relative, computed, or otherwise not statically known.
    Dynamic,
}

/// A call site (or URL literal) that may need a policy entry.
#[derive(Debug, Clone)]
pub struct Site {
    /// 1-based line of the node start.
    pub line: usize,
    /// 0-based byte column of the node start within its line.
    pub col: usize,
    /// 0-based byte column where the node ends on the same line (`None`: it continues on later lines).
    pub end_col: Option<usize>,
    /// `net` | `fs` | `env` | `exec`
    pub cat: &'static str,
    /// Short description of the call for messages, e.g. `fetch()` or `filesystem write (writeFile)`.
    pub what: String,
    /// Environment variable name (cat = env) when it is a literal.
    pub env: Option<String>,
    /// `host:port` pairs found in the URL-ish arguments (net).
    pub urls: Vec<String>,
    /// Target of a filesystem write (cat = fs).
    pub path: Option<Pth>,
    /// true: a call; false: a bare URL literal.
    pub call: bool,
}

/// Returns the (cached) regex that recognises temp-directory expressions such as `os.tmpdir()` or `mktemp`.
fn temp_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)tmp|tempdir|tempfile|temp_|mkdtemp|mktemp|mkstemp|createtemp|\btemp\b")
            .unwrap()
    })
}

/// Returns the source text of `n`; `src` must be the text the tree was parsed from.
fn txt<'a>(n: Node, src: &'a str) -> &'a str {
    &src[n.byte_range()]
}

/// Returns the 1-based line where `n` starts.
fn line_of(n: Node) -> usize {
    n.start_position().row + 1
}

/// Node kinds of string literals across the supported grammars.
const STRING_KINDS: &[&str] = &[
    "string",
    "template_string",
    "interpreted_string_literal",
    "raw_string_literal",
    "raw_string",
];
/// Node kinds of interpolations inside strings (JS `${}`, Python f-string `{}`, shell `$x` / `$(...)`).
const SUBST_KINDS: &[&str] = &[
    "template_substitution",
    "interpolation",
    "expansion",
    "simple_expansion",
    "command_substitution",
];

/// Returns whether a string node has an interpolation among its direct children.
fn has_subst(n: Node) -> bool {
    let mut c = n.walk();
    let r = n
        .named_children(&mut c)
        .any(|k| SUBST_KINDS.contains(&k.kind()));
    r
}

/// Removes Python string prefixes (`r`, `b`, `f`, `u` in any case) and one pair of surrounding quotes (`"""`, `'''`,
/// `"`, `'` or `` ` ``); text without matching quotes is returned with only the prefixes removed.
fn strip_quotes(t: &str) -> String {
    let t = t.trim_start_matches(['r', 'R', 'b', 'B', 'f', 'F', 'u', 'U']);
    for q in ["\"\"\"", "'''"] {
        if t.len() >= 6 && t.starts_with(q) && t.ends_with(q) {
            return t[3..t.len() - 3].to_string();
        }
    }
    let first = t.chars().next();
    if t.len() >= 2 && matches!(first, Some('"' | '\'' | '`')) && t.ends_with(first.unwrap()) {
        return t[1..t.len() - 1].to_string();
    }
    t.to_string()
}

/// The value of a string literal node without substitutions.
fn str_lit(n: Node, src: &str) -> Option<String> {
    if STRING_KINDS.contains(&n.kind()) && !has_subst(n) {
        Some(strip_quotes(txt(n, src)))
    } else {
        None
    }
}

/// Returns the named argument nodes of a call, without comments; empty when it has no `arguments` field.
fn named_args<'t>(call: Node<'t>) -> Vec<Node<'t>> {
    let Some(a) = call.child_by_field_name("arguments") else {
        return vec![];
    };
    let mut c = a.walk();
    let v = a
        .named_children(&mut c)
        .filter(|k| k.kind() != "comment")
        .collect();
    v
}

/// Returns whether path `p` is `root` or inside it; the root `/` contains every path.
fn under(p: &str, root: &str) -> bool {
    let r = root.trim_end_matches('/');
    p == r || p.starts_with(&format!("{r}/")) || r.is_empty()
}

/// Classify the path-valued argument of a write.
fn classify_path(n: Node, src: &str) -> Pth {
    if let Some(s) = str_lit(n, src) {
        return if s.starts_with('/') {
            Pth::Literal(s)
        } else {
            Pth::Dynamic
        };
    }
    let t = txt(n, src);
    // `path.join("/abs", x)` / `os.path.join("/abs", x)` / `Path("/abs")`: the first literal argument decides.
    if matches!(n.kind(), "call_expression" | "call") {
        if let Some(first) = named_args(n).first() {
            if let Some(s) = str_lit(*first, src) {
                if s.starts_with('/') {
                    return Pth::Literal(s);
                }
            }
        }
    }
    // template literal / f-string with an absolute literal head
    if matches!(n.kind(), "template_string" | "string") {
        let body = strip_quotes(t);
        if let Some(i) = body.find("${").or_else(|| body.find('{')) {
            if body.starts_with('/') && i > 1 {
                return Pth::Literal(body[..i].to_string());
            }
        }
    }
    if temp_re().is_match(t) {
        return Pth::Temp;
    }
    Pth::Dynamic
}

/// Returns whether a write to `p` is already allowed: temp paths, `/tmp`, `/dev/null`, or anything under one of the
/// declared `fs write` roots. A dynamic path is never covered.
pub fn is_covered_path(p: &Pth, fs_roots: &[String]) -> bool {
    match p {
        Pth::Temp => true,
        Pth::Dynamic => false,
        Pth::Literal(s) => {
            under(s, "/tmp") || s == "/dev/null" || fs_roots.iter().any(|r| under(s, r))
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------

/// Appends a call [`Site`] for node `n` with no details filled in and returns its index, so the caller can set them.
fn push(out: &mut Vec<Site>, n: Node, cat: &'static str, what: impl Into<String>) -> usize {
    let (a, b) = (n.start_position(), n.end_position());
    out.push(Site {
        line: line_of(n),
        col: a.column,
        end_col: (a.row == b.row).then_some(b.column),
        cat,
        what: what.into(),
        env: None,
        urls: vec![],
        path: None,
        call: true,
    });
    out.len() - 1
}

/// Returns the unique `host:port` pairs of URLs in the first two of `nodes` (the arguments that usually hold the URL).
fn urls_in(nodes: &[Node], src: &str) -> Vec<String> {
    let mut v = Vec::new();
    for n in nodes.iter().take(2) {
        for h in crate::lint::url_hosts(txt(*n, src)) {
            if !v.contains(&h) {
                v.push(h);
            }
        }
    }
    v
}

/// Parse and extract sites. `None` when the grammar cannot be loaded or parsing fails outright.
///
/// Also `None` for the generic `#` / `//` languages, which have no grammar. Every node is visited in document order and
/// handed to the language handler and to the URL-literal check.
pub fn sites(lang: Language, rel: &str, text: &str) -> Option<Vec<Site>> {
    if matches!(lang, Language::Hash | Language::Slash) {
        return None; // no grammar: directives only, no call-site heuristics
    }
    let mut parser = Parser::new();
    parser.set_language(&language_for(lang, rel)).ok()?;
    let tree = parser.parse(text, None)?;
    let mut out = Vec::new();
    let mut imp = FsImports::new();
    let mut cur = tree.root_node().walk();
    'outer: loop {
        let n = cur.node();
        match lang {
            Language::JavaScript => js(n, text, &mut out, &mut imp),
            Language::Python => py(n, text, &mut out),
            Language::Go => go(n, text, &mut out),
            Language::Shell => sh(n, text, &mut out),
            Language::Hash | Language::Slash => {}
        }
        url_literal(n, text, &mut out);
        if cur.goto_first_child() {
            continue;
        }
        loop {
            if cur.goto_next_sibling() {
                continue 'outer;
            }
            if !cur.goto_parent() {
                break 'outer;
            }
        }
    }
    Some(out)
}

/// Returns the tree-sitter grammar for a file: TypeScript or TSX by extension within the JavaScript family, bash for
/// shell (and, unused, for the generic languages).
fn language_for(lang: Language, rel: &str) -> tree_sitter::Language {
    match lang {
        Language::JavaScript => match rel.rsplit('.').next().unwrap_or("") {
            "ts" | "mts" | "cts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
            _ => tree_sitter_javascript::LANGUAGE.into(),
        },
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        Language::Shell | Language::Hash | Language::Slash => tree_sitter_bash::LANGUAGE.into(),
    }
}

/// Comments from the real syntax tree, one entry per line with the leader removed. The hand-written lexer cannot
/// tell a JS regex literal from a division, so a stray quote inside a regex can turn the rest of a big file into a
/// "string" and hide its directives; the grammar does not have that problem.
///
/// `None` for the generic `#` / `//` languages or when the grammar cannot be loaded or parsing fails.
pub fn comments(lang: Language, rel: &str, text: &str) -> Option<Vec<crate::lexer::Comment>> {
    if matches!(lang, Language::Hash | Language::Slash) {
        return None;
    }
    let mut parser = Parser::new();
    parser.set_language(&language_for(lang, rel)).ok()?;
    let tree = parser.parse(text, None)?;
    let mut out = Vec::new();
    let mut cur = tree.root_node().walk();
    'outer: loop {
        let n = cur.node();
        if n.kind() == "comment" {
            let t = txt(n, text);
            let first = line_of(n);
            for (i, l) in t.lines().enumerate() {
                let l = l.trim();
                let l = if i == 0 {
                    l.strip_prefix("//")
                        .or_else(|| l.strip_prefix("/*"))
                        .or_else(|| l.strip_prefix('#'))
                        .unwrap_or(l)
                } else {
                    l.strip_prefix('*').unwrap_or(l)
                };
                let l = l.strip_suffix("*/").unwrap_or(l);
                out.push(crate::lexer::Comment {
                    line: first + i,
                    text: l.to_string(),
                });
            }
        } else if cur.goto_first_child() {
            continue 'outer;
        }
        loop {
            if cur.goto_next_sibling() {
                continue 'outer;
            }
            if !cur.goto_parent() {
                break 'outer;
            }
        }
    }
    Some(out)
}

/// Callees whose string arguments are text for people (log lines, user messages, error text, CLI help), not destinations.
fn message_callee(name: &str) -> bool {
    let last = name.rsplit(['.', ':']).next().unwrap_or(name);
    matches!(
        name,
        "print" | "echo" | "printf" | "puts" | "println" | "errors.New" | "fmt.Errorf"
    ) || matches!(
        last,
        "print"
            | "debug"
            | "info"
            | "warn"
            | "warning"
            | "error"
            | "exception"
            | "critical"
            | "fatal"
            | "trace"
            | "log"
            | "logf"
            | "Printf"
            | "Println"
            | "Print"
            | "Fprintf"
            | "Fprintln"
            | "Errorf"
            | "Fatalf"
            | "Fatal"
            | "Panicf"
            | "add_argument"
            | "add_parser"
            | "ArgumentParser"
            | "echo"
            | "secho"
            | "print_help"
    ) || last.ends_with("Error")
        || last.ends_with("Exception")
        || last.ends_with("Warning")
}

/// Variable names that hold text for people.
const MESSAGE_NAMES: &[&str] = &[
    "help", "usage", "descr", "doc", "epilog", "message", "msg", "error", "err", "warn", "banner",
    "notice",
];

/// True when the string literal is a docstring or bare string statement, part of a `raise`/`throw`, or an argument of a
/// logging / printing / error-constructing / argument-help call, e.g. `raise ValueError("see https://docs.x.io")`.
fn message_context(n: Node, src: &str) -> bool {
    let mut cur = n.parent();
    let mut hops = 0;
    while let Some(p) = cur {
        hops += 1;
        if hops > 8 {
            return false;
        }
        match p.kind() {
            "expression_statement" => {
                // a bare string statement: a docstring (Python) or a no-op
                return p.named_child_count() == 1
                    && p.named_child(0).is_some_and(|c| c.id() == n.id());
            }
            "raise_statement" | "throw_statement" => return true,
            "call" | "call_expression" | "new_expression" | "command" => {
                let callee = p
                    .child_by_field_name("function")
                    .or_else(|| p.child_by_field_name("constructor"))
                    .or_else(|| p.child_by_field_name("name"))
                    .map(|c| txt(c, src).trim().to_string())
                    .unwrap_or_default();
                return message_callee(&callee);
            }
            "assignment"
            | "variable_declarator"
            | "short_var_declaration"
            | "const_spec"
            | "var_spec" => {
                // `_HELP_TEXT = """..."""`, `msg = (...)`, `const usage = "..."`: named for people, not for a connection
                let name = ["left", "name"]
                    .iter()
                    .find_map(|f| p.child_by_field_name(f))
                    .map(|c| txt(c, src).to_ascii_lowercase())
                    .unwrap_or_default();
                return MESSAGE_NAMES.iter().any(|k| name.contains(k));
            }
            "keyword_argument" => {
                let name = p
                    .child_by_field_name("name")
                    .map(|c| txt(c, src).to_ascii_lowercase())
                    .unwrap_or_default();
                if matches!(
                    name.as_str(),
                    "help"
                        | "description"
                        | "epilog"
                        | "usage"
                        | "message"
                        | "msg"
                        | "title"
                        | "label"
                ) {
                    return true;
                }
            }
            "pair" | "dictionary" => return false,
            _ => {}
        }
        cur = p.parent();
    }
    false
}

/// Records a string literal or shell word with a URL (up to 600 bytes) as a `net` site with `call: false`, unless it
/// is text for people (see [`message_context`]).
fn url_literal(n: Node, src: &str, out: &mut Vec<Site>) {
    let k = n.kind();
    if !(STRING_KINDS.contains(&k) || k == "word") {
        return;
    }
    let t = txt(n, src);
    if !t.contains("://") || t.len() > 600 {
        return;
    }
    if message_context(n, src) {
        return;
    }
    let hosts = crate::lint::url_hosts(t);
    if !hosts.is_empty() {
        let i = push(out, n, "net", "URL literal");
        out[i].urls = hosts;
        out[i].call = false;
    }
}

// ---- JavaScript / TypeScript -------------------------------------------------------------------------------------------

/// Returns a JS member chain as dotted text (`globalThis.fetch`), looking through `!`, parentheses and `as`; `None`
/// for anything else, such as a call or computed member in the chain.
fn js_dotted(n: Node, src: &str) -> Option<String> {
    match n.kind() {
        "identifier" | "property_identifier" | "this" | "shorthand_property_identifier" => {
            Some(txt(n, src).to_string())
        }
        "member_expression" => {
            let o = js_dotted(n.child_by_field_name("object")?, src)?;
            let p = n.child_by_field_name("property")?;
            Some(format!("{o}.{}", txt(p, src)))
        }
        "non_null_expression" | "parenthesized_expression" | "as_expression" => {
            js_dotted(n.named_child(0)?, src)
        }
        _ => None,
    }
}

/// Splits a member expression into its object (dotted, when it is a plain chain) and property name.
fn js_member_parts(n: Node, src: &str) -> Option<(Option<String>, String)> {
    if n.kind() == "member_expression" {
        let obj = n
            .child_by_field_name("object")
            .and_then(|o| js_dotted(o, src));
        let prop = txt(n.child_by_field_name("property")?, src).to_string();
        Some((obj, prop))
    } else {
        None
    }
}

/// fs write functions recognised when called by their bare name.
const JS_FS_BARE: &[&str] = &[
    "writeFile",
    "writeFileSync",
    "appendFile",
    "appendFileSync",
    "createWriteStream",
    "mkdirSync",
    "mkdtemp",
    "mkdtempSync",
    "copyFileSync",
    "rmSync",
    "unlinkSync",
    "renameSync",
];
/// fs write methods recognised on an fs-like object (see [`js_fs_object`]) or as imported names.
const JS_FS_MEMBER: &[&str] = &[
    "writeFile",
    "writeFileSync",
    "appendFile",
    "appendFileSync",
    "createWriteStream",
    "mkdir",
    "mkdirSync",
    "mkdtemp",
    "mkdtempSync",
    "unlink",
    "unlinkSync",
    "rename",
    "renameSync",
    "copyFile",
    "copyFileSync",
    "rm",
    "rmSync",
    "cp",
    "cpSync",
    "truncate",
    "truncateSync",
    "symlink",
    "symlinkSync",
    "outputFile",
    "outputFileSync",
    "ensureDir",
    "emptyDir",
    "move",
    "copy",
];
/// fs functions taking `(source, destination)`: the second argument is the path written.
const JS_FS_TWO_PATH: &[&str] = &[
    "copyFile",
    "copyFileSync",
    "rename",
    "renameSync",
    "cp",
    "cpSync",
    "symlink",
    "symlinkSync",
    "move",
    "copy",
];

/// Returns whether a dotted object name looks like node's fs module (`fs`, `fsp`, `fse`, `fs.promises`, anything
/// ending in `fs`, ...).
fn js_fs_object(o: &str) -> bool {
    let last = o.rsplit('.').next().unwrap_or(o).to_ascii_lowercase();
    matches!(
        last.as_str(),
        "fs" | "fsp" | "fspromises" | "promises" | "fse" | "fsextra" | "fsx" | "nodefs"
    ) || last.ends_with("fs")
        || o.to_ascii_lowercase().ends_with("fs.promises")
}

/// Local names bound to functions of node's fs modules (`import { mkdir as mk } from "node:fs/promises"`).
type FsImports = std::collections::BTreeMap<String, String>;

/// Returns whether an import specifier names an fs module (`fs`, `fs/promises`, `fs-extra`, `graceful-fs`, with or
/// without `node:`).
fn is_fs_module(m: &str) -> bool {
    matches!(
        m.trim_start_matches("node:"),
        "fs" | "fs/promises" | "fs-extra" | "graceful-fs"
    )
}

/// JavaScript/TypeScript handler for one node: network clients (`fetch`, node http, sockets, axios, `WebSocket`),
/// subprocesses, fs writes, and `process.env` reads.
///
/// fs functions imported or destructured from an fs module are recorded in `imp`, so calls through those local names
/// are recognised later in the walk.
fn js(n: Node, src: &str, out: &mut Vec<Site>, imp: &mut FsImports) {
    match n.kind() {
        "import_statement" => {
            let Some(m) = n
                .child_by_field_name("source")
                .and_then(|x| str_lit(x, src))
            else {
                return;
            };
            if !is_fs_module(&m) {
                return;
            }
            let mut stack = vec![n];
            while let Some(x) = stack.pop() {
                if x.kind() == "import_specifier" {
                    if let Some(name) = x.child_by_field_name("name") {
                        let local = x.child_by_field_name("alias").unwrap_or(name);
                        imp.insert(txt(local, src).to_string(), txt(name, src).to_string());
                    }
                } else {
                    let mut c = x.walk();
                    stack.extend(x.named_children(&mut c));
                }
            }
        }
        "new_expression" => {
            let Some(c) = n.child_by_field_name("constructor") else {
                return;
            };
            let Some(d) = js_dotted(c, src) else { return };
            let what = match d.as_str() {
                "WebSocket" | "ws.WebSocket" => "WebSocket client",
                "EventSource" => "EventSource",
                "XMLHttpRequest" => "XMLHttpRequest",
                _ => return,
            };
            let args = named_args(n);
            let i = push(out, n, "net", what);
            out[i].urls = urls_in(&args, src);
        }
        "call_expression" => {
            let Some(f) = n.child_by_field_name("function") else {
                return;
            };
            let args = named_args(n);
            let dotted = js_dotted(f, src);
            let parts = js_member_parts(f, src);
            if let Some(d) = &dotted {
                let net = match d.as_str() {
                    "fetch" | "globalThis.fetch" | "window.fetch" | "self.fetch"
                    | "undici.fetch" | "undici.request" => Some("fetch()"),
                    "http.request" | "http.get" | "https.request" | "https.get"
                    | "http2.connect" | "httpRequest" | "httpsRequest" => {
                        Some("node http(s) request")
                    }
                    "net.connect" | "net.createConnection" | "tls.connect" => Some("raw socket"),
                    x if x == "axios" || x.starts_with("axios.") => Some("axios"),
                    _ => None,
                };
                if let Some(w) = net {
                    let i = push(out, n, "net", w);
                    out[i].urls = urls_in(&args, src);
                }
                let exec = matches!(
                    d.as_str(),
                    "spawn"
                        | "spawnSync"
                        | "execFile"
                        | "execFileSync"
                        | "execSync"
                        | "fork"
                        | "execa"
                        | "execaSync"
                ) || matches!(
                    parts.as_ref().and_then(|p| p.0.as_deref()),
                    Some("child_process" | "cp" | "childProcess" | "execa" | "node:child_process")
                );
                if exec {
                    push(out, n, "exec", "subprocess");
                }
            }
            // fs writes
            let fs_name = match (&parts, &dotted) {
                (Some((Some(obj), name)), _)
                    if js_fs_object(obj) && JS_FS_MEMBER.contains(&name.as_str()) =>
                {
                    Some(name.clone())
                }
                (None, Some(d))
                    if imp
                        .get(d)
                        .is_some_and(|o| JS_FS_MEMBER.contains(&o.as_str())) =>
                {
                    imp.get(d).cloned()
                }
                (None, Some(d)) if JS_FS_BARE.contains(&d.as_str()) => Some(d.clone()),
                _ => None,
            };
            if let Some(name) = fs_name {
                let idx = if JS_FS_TWO_PATH.contains(&name.as_str()) {
                    1.min(args.len().saturating_sub(1))
                } else {
                    0
                };
                let p = args
                    .get(idx)
                    .map(|a| classify_path(*a, src))
                    .unwrap_or(Pth::Dynamic);
                let i = push(out, n, "fs", format!("filesystem write ({name})"));
                out[i].path = Some(p);
            }
        }
        "member_expression" => {
            // process.env.NAME
            if let Some(o) = n.child_by_field_name("object") {
                if js_dotted(o, src).as_deref() == Some("process.env") {
                    if let Some(p) = n.child_by_field_name("property") {
                        let i = push(out, n, "env", "environment read");
                        out[i].env = Some(txt(p, src).to_string());
                    }
                }
            }
        }
        "subscript_expression" => {
            if let (Some(o), Some(ix)) = (
                n.child_by_field_name("object"),
                n.child_by_field_name("index"),
            ) {
                if js_dotted(o, src).as_deref() == Some("process.env") {
                    let i = push(out, n, "env", "environment read");
                    out[i].env = str_lit(ix, src);
                }
            }
        }
        "variable_declarator" => {
            // const { A, B: b } = process.env
            let (Some(name), Some(val)) = (
                n.child_by_field_name("name"),
                n.child_by_field_name("value"),
            ) else {
                return;
            };
            // const { mkdir } = require("node:fs/promises")
            let req = if val.kind() == "await_expression" {
                val.named_child(0).unwrap_or(val)
            } else {
                val
            };
            if name.kind() == "object_pattern" && req.kind() == "call_expression" {
                let is_req = req
                    .child_by_field_name("function")
                    .is_some_and(|f| matches!(txt(f, src), "require" | "import"));
                if is_req
                    && named_args(req)
                        .first()
                        .and_then(|a| str_lit(*a, src))
                        .is_some_and(|m| is_fs_module(&m))
                {
                    let mut c = name.walk();
                    for k in name.named_children(&mut c) {
                        match k.kind() {
                            "shorthand_property_identifier_pattern" => {
                                imp.insert(txt(k, src).to_string(), txt(k, src).to_string());
                            }
                            "pair_pattern" => {
                                if let (Some(key), Some(v)) =
                                    (k.child_by_field_name("key"), k.child_by_field_name("value"))
                                {
                                    imp.insert(txt(v, src).to_string(), txt(key, src).to_string());
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            if name.kind() == "object_pattern"
                && js_dotted(val, src).as_deref() == Some("process.env")
            {
                let mut c = name.walk();
                for k in name.named_children(&mut c) {
                    let key = match k.kind() {
                        "shorthand_property_identifier_pattern" => Some(txt(k, src).to_string()),
                        "pair_pattern" => k
                            .child_by_field_name("key")
                            .map(|x| txt(x, src).to_string()),
                        "object_assignment_pattern" => k
                            .child_by_field_name("left")
                            .map(|x| txt(x, src).to_string()),
                        _ => None,
                    };
                    if let Some(key) = key {
                        let i = push(out, n, "env", "environment read");
                        out[i].env = Some(key);
                    }
                }
            }
        }
        _ => {}
    }
}

// ---- Python ------------------------------------------------------------------------------------------------------------

/// Returns a Python attribute chain of identifiers as dotted text (`os.environ.get`).
fn py_dotted(n: Node, src: &str) -> Option<String> {
    match n.kind() {
        "identifier" => Some(txt(n, src).to_string()),
        "attribute" => {
            let o = py_dotted(n.child_by_field_name("object")?, src)?;
            Some(format!(
                "{o}.{}",
                txt(n.child_by_field_name("attribute")?, src)
            ))
        }
        _ => None,
    }
}

/// Returns the value node of the keyword argument `name=` among `args`.
fn py_kwarg<'t>(args: &[Node<'t>], name: &str, src: &str) -> Option<Node<'t>> {
    args.iter().find_map(|a| {
        if a.kind() == "keyword_argument" && txt(a.child_by_field_name("name")?, src) == name {
            a.child_by_field_name("value")
        } else {
            None
        }
    })
}

/// Python handler for one node: HTTP clients and sockets, subprocesses, environment reads, and fs writes (`open` with
/// a literal write mode, `shutil`, `os`, `Path.write_text`/`write_bytes`).
fn py(n: Node, src: &str, out: &mut Vec<Site>) {
    match n.kind() {
        "call" => {
            let Some(f) = n.child_by_field_name("function") else {
                return;
            };
            let args = named_args(n);
            let dotted = py_dotted(f, src);
            let last = if f.kind() == "attribute" {
                f.child_by_field_name("attribute")
                    .map(|x| txt(x, src).to_string())
            } else {
                dotted.clone()
            };
            if let Some(d) = &dotted {
                let net = if let Some(rest) = d.strip_prefix("requests.") {
                    matches!(
                        rest,
                        "get"
                            | "post"
                            | "put"
                            | "patch"
                            | "delete"
                            | "head"
                            | "request"
                            | "Session"
                    )
                    .then_some("requests")
                } else if let Some(rest) = d.strip_prefix("httpx.") {
                    matches!(
                        rest,
                        "get"
                            | "post"
                            | "put"
                            | "patch"
                            | "delete"
                            | "head"
                            | "request"
                            | "stream"
                            | "Client"
                            | "AsyncClient"
                    )
                    .then_some("httpx")
                } else if matches!(
                    d.as_str(),
                    "urllib.request.urlopen"
                        | "urllib.request.Request"
                        | "urlopen"
                        | "request.urlopen"
                ) {
                    Some("urllib")
                } else if matches!(d.as_str(), "aiohttp.ClientSession") {
                    Some("aiohttp")
                } else if matches!(
                    d.as_str(),
                    "socket.create_connection"
                        | "websockets.connect"
                        | "websocket.create_connection"
                ) {
                    Some("raw socket")
                } else {
                    None
                };
                if let Some(w) = net {
                    let i = push(out, n, "net", w);
                    out[i].urls = urls_in(&args, src);
                }
                if d.starts_with("subprocess.") || matches!(d.as_str(), "os.system" | "os.popen") {
                    push(out, n, "exec", "subprocess");
                }
                if matches!(
                    d.as_str(),
                    "os.getenv" | "os.environ.get" | "environ.get" | "getenv"
                ) {
                    let i = push(out, n, "env", "environment read");
                    out[i].env = args.first().and_then(|a| str_lit(*a, src));
                }
                // fs
                let is_open = matches!(d.as_str(), "open" | "io.open" | "codecs.open");
                if is_open {
                    let mode = args
                        .get(1)
                        .copied()
                        .or_else(|| py_kwarg(&args, "mode", src));
                    let writes = mode
                        .and_then(|m| str_lit(m, src))
                        .is_some_and(|m| m.chars().any(|c| matches!(c, 'w' | 'a' | 'x' | '+')));
                    if writes {
                        let p = args
                            .first()
                            .map(|a| classify_path(*a, src))
                            .unwrap_or(Pth::Dynamic);
                        let i = push(out, n, "fs", "open() for writing");
                        out[i].path = Some(p);
                    }
                } else if let Some(rest) = d.strip_prefix("shutil.") {
                    if matches!(
                        rest,
                        "copy" | "copy2" | "copyfile" | "copytree" | "move" | "rmtree"
                    ) {
                        let idx = if rest == "rmtree" {
                            0
                        } else {
                            1.min(args.len().saturating_sub(1))
                        };
                        let p = args
                            .get(idx)
                            .map(|a| classify_path(*a, src))
                            .unwrap_or(Pth::Dynamic);
                        let i = push(out, n, "fs", format!("filesystem write (shutil.{rest})"));
                        out[i].path = Some(p);
                    }
                } else if let Some(rest) = d.strip_prefix("os.") {
                    if matches!(
                        rest,
                        "remove" | "rename" | "replace" | "makedirs" | "mkdir" | "unlink" | "rmdir"
                    ) {
                        let idx = if matches!(rest, "rename" | "replace") {
                            1.min(args.len().saturating_sub(1))
                        } else {
                            0
                        };
                        let p = args
                            .get(idx)
                            .map(|a| classify_path(*a, src))
                            .unwrap_or(Pth::Dynamic);
                        let i = push(out, n, "fs", format!("filesystem write (os.{rest})"));
                        out[i].path = Some(p);
                    }
                }
            }
            if let (Some(l), true) = (&last, f.kind() == "attribute") {
                if matches!(l.as_str(), "write_text" | "write_bytes") {
                    let recv = f.child_by_field_name("object");
                    let p = recv.map(|r| classify_path(r, src)).unwrap_or(Pth::Dynamic);
                    let i = push(out, n, "fs", format!("filesystem write ({l})"));
                    out[i].path = Some(p);
                }
            }
        }
        "subscript" => {
            if let (Some(v), Some(s)) = (
                n.child_by_field_name("value"),
                n.child_by_field_name("subscript"),
            ) {
                if matches!(py_dotted(v, src).as_deref(), Some("os.environ" | "environ")) {
                    let i = push(out, n, "env", "environment read");
                    out[i].env = str_lit(s, src);
                }
            }
        }
        _ => {}
    }
}

// ---- Go ----------------------------------------------------------------------------------------------------------------

/// Returns a Go selector chain as dotted text (`http.DefaultClient.Get`).
fn go_dotted(n: Node, src: &str) -> Option<String> {
    match n.kind() {
        "identifier" | "package_identifier" | "field_identifier" => Some(txt(n, src).to_string()),
        "selector_expression" => {
            let o = go_dotted(n.child_by_field_name("operand")?, src)?;
            Some(format!("{o}.{}", txt(n.child_by_field_name("field")?, src)))
        }
        _ => None,
    }
}

/// Go handler for one call expression: `net/http` clients and dialers, `os.Getenv`/`LookupEnv`, `exec.Command`, and
/// `os`/`ioutil` writes (`OpenFile` only with a write flag).
///
/// For `http.NewRequest[WithContext]` the URL is looked for after the method (and context) arguments.
fn go(n: Node, src: &str, out: &mut Vec<Site>) {
    if n.kind() != "call_expression" {
        return;
    }
    let Some(f) = n.child_by_field_name("function") else {
        return;
    };
    let Some(d) = go_dotted(f, src) else { return };
    let args = named_args(n);
    let net = match d.as_str() {
        "http.Get"
        | "http.Post"
        | "http.PostForm"
        | "http.Head"
        | "http.NewRequest"
        | "http.NewRequestWithContext"
        | "http.DefaultClient.Get"
        | "http.DefaultClient.Do"
        | "http.DefaultClient.Post" => Some("net/http client"),
        "net.Dial"
        | "net.DialTimeout"
        | "tls.Dial"
        | "websocket.Dial"
        | "websocket.DefaultDialer.Dial" => Some("raw socket"),
        _ => None,
    };
    if let Some(w) = net {
        let i = push(out, n, "net", w);
        let from = if d == "http.NewRequest" {
            1
        } else if d == "http.NewRequestWithContext" {
            2
        } else {
            0
        };
        out[i].urls = urls_in(&args[from.min(args.len())..], src);
    }
    if matches!(d.as_str(), "os.Getenv" | "os.LookupEnv") {
        let i = push(out, n, "env", "environment read");
        out[i].env = args.first().and_then(|a| str_lit(*a, src));
    }
    if matches!(d.as_str(), "exec.Command" | "exec.CommandContext") {
        push(out, n, "exec", "subprocess");
    }
    if let Some(name) = d.strip_prefix("os.").or_else(|| d.strip_prefix("ioutil.")) {
        let write = match name {
            "WriteFile" | "Create" | "MkdirAll" | "Mkdir" | "MkdirTemp" | "CreateTemp"
            | "Remove" | "RemoveAll" | "Rename" => true,
            "OpenFile" => args.get(1).is_some_and(|f| {
                let t = txt(*f, src);
                ["O_WRONLY", "O_RDWR", "O_CREATE", "O_APPEND", "O_TRUNC"]
                    .iter()
                    .any(|k| t.contains(k))
            }),
            _ => false,
        };
        if write {
            let idx = if name == "Rename" {
                1.min(args.len().saturating_sub(1))
            } else {
                0
            };
            let p = if matches!(name, "MkdirTemp" | "CreateTemp") {
                Pth::Temp
            } else {
                args.get(idx)
                    .map(|a| classify_path(*a, src))
                    .unwrap_or(Pth::Dynamic)
            };
            let i = push(out, n, "fs", format!("filesystem write (os.{name})"));
            out[i].path = Some(p);
        }
    }
}

// ---- Shell -------------------------------------------------------------------------------------------------------------

/// Returns the `argument` children of a shell command.
fn sh_args<'t>(cmd: Node<'t>) -> Vec<Node<'t>> {
    let mut c = cmd.walk();
    let v = cmd.children_by_field_name("argument", &mut c).collect();
    v
}

/// Classifies a shell word as a write target: an absolute path (or the literal prefix before its first `$`), a temp
/// path, or dynamic. `-` and plain numbers are dynamic.
fn sh_path(n: Node, src: &str) -> Pth {
    let t = txt(n, src);
    let unq = strip_quotes(t);
    if unq == "-" || unq.parse::<u32>().is_ok() {
        return Pth::Dynamic;
    }
    if unq.starts_with('/') && !unq.contains('$') && !unq.contains('`') {
        return Pth::Literal(unq);
    }
    if let Some(i) = unq.find('$') {
        if unq.starts_with('/') && i > 1 {
            return Pth::Literal(unq[..i].to_string());
        }
    }
    if temp_re().is_match(t) {
        return Pth::Temp;
    }
    Pth::Dynamic
}

/// Shell handler for one node: network commands (curl/wget, remote git, package managers, docker pull/push),
/// file-writing commands (target = last non-option argument), `>`/`>>`/`&>` redirections to files, and expansions of
/// upper-case variables as environment reads.
fn sh(n: Node, src: &str, out: &mut Vec<Site>) {
    match n.kind() {
        "command" => {
            let Some(nm) = n.child_by_field_name("name") else {
                return;
            };
            let name = txt(nm, src).rsplit('/').next().unwrap_or("").to_string();
            let args = sh_args(n);
            let first = args
                .first()
                .map(|a| txt(*a, src).to_string())
                .unwrap_or_default();
            let net = match name.as_str() {
                "curl" | "wget" => Some("curl/wget"),
                "git"
                    if matches!(
                        first.as_str(),
                        "clone" | "fetch" | "pull" | "push" | "ls-remote"
                    ) =>
                {
                    Some("git remote operation")
                }
                "npm" | "pnpm" | "yarn"
                    if matches!(
                        first.as_str(),
                        "install" | "i" | "add" | "ci" | "publish" | "view"
                    ) =>
                {
                    Some("package manager")
                }
                "pip" | "pip3" if first == "install" => Some("package manager"),
                "apt" | "apt-get" if matches!(first.as_str(), "install" | "update") => {
                    Some("package manager")
                }
                "docker" if matches!(first.as_str(), "pull" | "push") => Some("container registry"),
                _ => None,
            };
            if let Some(w) = net {
                let i = push(out, n, "net", w);
                let mut v = Vec::new();
                for a in &args {
                    for h in crate::lint::url_hosts(txt(*a, src)) {
                        if !v.contains(&h) {
                            v.push(h);
                        }
                    }
                }
                out[i].urls = v;
            }
            if matches!(
                name.as_str(),
                "mkdir"
                    | "rm"
                    | "cp"
                    | "mv"
                    | "touch"
                    | "tee"
                    | "chmod"
                    | "chown"
                    | "ln"
                    | "install"
                    | "rsync"
                    | "dd"
            ) {
                let paths: Vec<Node> = args
                    .iter()
                    .copied()
                    .filter(|a| !txt(*a, src).starts_with('-'))
                    .collect();
                if let Some(last) = paths.last() {
                    let i = push(out, n, "fs", format!("filesystem write ({name})"));
                    out[i].path = Some(sh_path(*last, src));
                }
            }
        }
        "file_redirect" => {
            let t = txt(n, src).trim_start();
            let t = t.trim_start_matches(|c: char| c.is_ascii_digit());
            if !(t.starts_with('>') || t.starts_with("&>")) || t.starts_with(">&") {
                return;
            }
            let Some(d) = n.child_by_field_name("destination") else {
                return;
            };
            let dt = strip_quotes(txt(d, src));
            if matches!(
                dt.as_str(),
                "/dev/null" | "/dev/stderr" | "/dev/stdout" | "/dev/tty"
            ) || dt.starts_with('&')
                || dt.parse::<u32>().is_ok()
            {
                return;
            }
            let i = push(out, n, "fs", "output redirection");
            out[i].path = Some(sh_path(d, src));
        }
        "variable_name" => {
            let parent_ok = n
                .parent()
                .is_some_and(|p| matches!(p.kind(), "simple_expansion" | "expansion"));
            let t = txt(n, src);
            if parent_ok && t.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                let i = push(out, n, "env", "environment read");
                out[i].env = Some(t.to_string());
            }
        }
        _ => {}
    }
}
