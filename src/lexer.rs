//! Comment lexer for JS/TS, Python, Go and shell, plus generic `#` and `//` leaders for other languages.
//!
//! The lexer is string-aware: `//` or `#` inside a string literal, template
//! literal, Python triple-quoted string or shell heredoc is not a comment.
//! Besides comments it produces a per-line "code mask" (comments and string
//! contents blanked with spaces) so the linter can match call sites without
//! matching text inside strings or comments.
//!
//! Known limits (documented, not hidden): JS regex literals are not parsed, so
//! a regex containing an unbalanced backtick can confuse the lexer for the rest
//! of the file; Python docstrings are strings, so directives in them are not
//! recognised (use `#` comments); shell parameter expansions beyond `$#`/`${#`
//! are not modelled.

use std::path::Path;

/// How a source file is lexed; also selects the lint patterns and the `[languages]` default binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    JavaScript, // js jsx mjs cjs ts tsx mts cts
    Python,
    Go,
    Shell,
    /// Any other language whose line comments start with `#` (Ruby, Perl, R, ...), chosen by extension.
    Hash,
    /// Any other language whose comments start with `//` or `/* */` (Rust, Java, C#, Kotlin, ...), chosen by extension.
    Slash,
}

/// Extensions with a built-in comment leader beyond the four first-class languages.
/// Override or extend with `[scan.comment_leaders]` in `openshell-annotate.toml`.
pub const DEFAULT_LEADERS: &[(&str, &str)] = &[
    ("rb", "hash"),
    ("rake", "hash"),
    ("gemspec", "hash"),
    ("pl", "hash"),
    ("pm", "hash"),
    ("r", "hash"),
    ("rs", "slash"),
    ("java", "slash"),
    ("kt", "slash"),
    ("kts", "slash"),
    ("cs", "slash"),
    ("php", "slash"),
    ("swift", "slash"),
    ("scala", "slash"),
    ("dart", "slash"),
    ("c", "slash"),
    ("h", "slash"),
    ("cc", "slash"),
    ("cpp", "slash"),
    ("hpp", "slash"),
];

/// Source-code extensions we do not know a comment leader for; files with these are reported as "not scanned".
pub const KNOWN_UNSCANNED: &[&str] = &[
    "lua", "ex", "exs", "erl", "hs", "ml", "clj", "cljs", "jl", "zig", "groovy", "vb", "fs", "nim",
    "cr", "v", "sol", "tf",
];

/// Line-comment style of the `KNOWN_UNSCANNED` extensions, so the "not scanned" note can give advice that works:
/// `hash` / `slash` can be switched on with `[scan.comment_leaders]`; anything else is a leader this tool cannot read.
pub const UNSCANNED_COMMENT_STYLE: &[(&str, &str)] = &[
    ("ex", "hash"),
    ("exs", "hash"),
    ("jl", "hash"),
    ("nim", "hash"),
    ("cr", "hash"),
    ("tf", "hash"),
    ("zig", "slash"),
    ("groovy", "slash"),
    ("fs", "slash"),
    ("v", "slash"),
    ("sol", "slash"),
    ("lua", "--"),
    ("hs", "--"),
    ("erl", "%"),
    ("ml", "(* *)"),
    ("clj", ";"),
    ("cljs", ";"),
    ("vb", "'"),
];

impl Language {
    /// Language of a file, honouring `[scan.comment_leaders]` (extension -> `hash` | `slash` | `none`).
    /// First-class languages always win; `none` switches a built-in default off.
    pub fn detect(
        path: &Path,
        overrides: &std::collections::BTreeMap<String, String>,
    ) -> Option<Self> {
        if let Some(l) = Self::from_path(path) {
            return Some(l);
        }
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        let kind = overrides.get(&ext).map(String::as_str).or_else(|| {
            DEFAULT_LEADERS
                .iter()
                .find(|(e, _)| *e == ext)
                .map(|(_, k)| *k)
        })?;
        match kind {
            "hash" => Some(Language::Hash),
            "slash" => Some(Language::Slash),
            _ => None,
        }
    }

    /// Returns the first-class language of a file (JS/TS, Python, Go, shell), ignoring `[scan.comment_leaders]`.
    ///
    /// `Dockerfile` and `.sh`/`.bash`/`.zsh` files are shell.
    pub fn from_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?;
        if name == "Dockerfile"
            || name.ends_with(".sh")
            || name.ends_with(".bash")
            || name.ends_with(".zsh")
        {
            return Some(Language::Shell);
        }
        match path.extension()?.to_str()? {
            "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" => {
                Some(Language::JavaScript)
            }
            "py" => Some(Language::Python),
            "go" => Some(Language::Go),
            _ => None,
        }
    }

    /// Key used in `openshell-annotate.toml` `[languages]`.
    pub fn config_key(self) -> &'static str {
        match self {
            Language::JavaScript => "javascript",
            Language::Python => "python",
            Language::Go => "go",
            Language::Shell => "shell",
            Language::Hash => "hash",
            Language::Slash => "slash",
        }
    }
}

/// The comment text on one source line; a block comment yields one [`Comment`] per line it spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// 1-based line number.
    pub line: usize,
    /// Comment text for this line with the comment leader (`//`, `#`, `/*`, `*`) removed.
    pub text: String,
}

/// Result of [`lex`]: the comments of a file plus its raw and code-masked lines.
#[derive(Debug, Default)]
pub struct Lexed {
    /// Comments in source order.
    pub comments: Vec<Comment>,
    /// Original lines (no trailing newline).
    pub raw: Vec<String>,
    /// Lines with comments and string contents replaced by spaces (quotes kept).
    pub code: Vec<String>,
}

/// Lexes `src` as `lang`, returning its comments and the code mask.
pub fn lex(lang: Language, src: &str) -> Lexed {
    match lang {
        Language::JavaScript => lex_clike(src, false),
        Language::Go => lex_clike(src, true),
        Language::Python => lex_hash(src, true, false),
        Language::Shell => lex_hash(src, false, true),
        Language::Hash => lex_hash(src, false, false),
        Language::Slash => lex_clike(src, true),
    }
}

/// Builder that accumulates a [`Lexed`] one character and one line at a time.
struct Out {
    comments: Vec<Comment>,
    raw: Vec<String>,
    code: Vec<String>,
    /// Code mask of the current line so far.
    cur_code: String,
    /// Comment text of the current line so far; `Some` once a comment has started on it.
    cur_comment: Option<String>,
    /// 1-based number of the current line.
    line: usize,
}

impl Out {
    /// Starts at line 1 with the raw lines of `src` already split.
    fn new(src: &str) -> Self {
        Out {
            comments: Vec::new(),
            raw: src.lines().map(str::to_string).collect(),
            code: Vec::new(),
            cur_code: String::new(),
            cur_comment: None,
            line: 1,
        }
    }
    /// Appends a code character to the mask unchanged.
    fn code(&mut self, c: char) {
        self.cur_code.push(c);
    }
    /// Appends a blank for `c` to the mask, keeping tabs so columns still line up.
    fn blank(&mut self, c: char) {
        self.cur_code.push(if c == '\t' { '\t' } else { ' ' });
    }
    /// Appends `c` to the current comment and blanks it in the mask.
    fn comment_char(&mut self, c: char) {
        self.cur_comment.get_or_insert_with(String::new).push(c);
        self.blank(c);
    }
    /// Starts an empty comment on the current line.
    fn start_comment(&mut self) {
        self.cur_comment = Some(String::new());
    }
    /// Ends the current line: records its comment (if any) and mask line, then advances the line number.
    fn newline(&mut self) {
        if let Some(t) = self.cur_comment.take() {
            self.comments.push(Comment {
                line: self.line,
                text: t,
            });
        }
        self.code.push(std::mem::take(&mut self.cur_code));
        self.line += 1;
    }
    /// Flushes a last line without a trailing newline and returns the result.
    fn finish(mut self) -> Lexed {
        if self.cur_comment.is_some() || !self.cur_code.is_empty() {
            self.newline();
        }
        Lexed {
            comments: self.comments,
            raw: self.raw,
            code: self.code,
        }
    }
}

/// Removes the leading whitespace and one optional `*` of a block-comment line (` * text` -> `text`).
fn strip_block_leader(text: &str) -> String {
    let t = text.trim_start();
    let t = t.strip_prefix('*').unwrap_or(t);
    t.trim_start().to_string()
}

/// One level of the JS template-literal nesting stack in [`lex_clike`].
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Code(usize), // brace depth (for template `${ }`)
    /// Inside a `` ` `` template literal, outside any `${ }`.
    Template,
}

/// Lexes `//` and `/* */` comments, `'`/`"` strings and backtick strings.
///
/// With `go` set, a backtick opens a Go raw string (no interpolation); otherwise it opens a JS template literal whose
/// `${ }` parts are lexed as code, tracked by a stack of [`Mode`]s. An unterminated `'`/`"` string ends at the end of
/// its line.
fn lex_clike(src: &str, go: bool) -> Lexed {
    let chars: Vec<char> = src.chars().collect();
    let mut o = Out::new(src);
    let mut stack: Vec<Mode> = vec![Mode::Code(0)];
    let mut i = 0;
    let mut in_block = false;
    let mut in_line = false;
    let mut quote: Option<char> = None; // ' or "
    let mut raw_go = false;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '\n' {
            if in_block {
                // flush block comment line, keep block state
                if let Some(t) = o.cur_comment.take() {
                    o.cur_comment = Some(strip_block_leader(&t));
                }
            }
            in_line = false;
            quote = None; // unterminated string recovery
            o.newline();
            if in_block {
                o.start_comment();
            }
            i += 1;
            continue;
        }
        if in_line {
            o.comment_char(c);
            i += 1;
            continue;
        }
        if in_block {
            if c == '*' && next == Some('/') {
                in_block = false;
                if let Some(t) = o.cur_comment.take() {
                    let t = strip_block_leader(&t);
                    o.cur_comment = Some(t);
                }
                // close comment on this line
                if let Some(t) = o.cur_comment.take() {
                    o.comments.push(Comment {
                        line: o.line,
                        text: t.trim_end().to_string(),
                    });
                }
                o.blank('*');
                o.blank('/');
                i += 2;
            } else {
                o.comment_char(c);
                i += 1;
            }
            continue;
        }
        if raw_go {
            if c == '`' {
                raw_go = false;
                o.code(c);
            } else {
                o.blank(c);
            }
            i += 1;
            continue;
        }
        if let Some(q) = quote {
            if c == '\\' {
                o.blank(c);
                if let Some(n) = next {
                    if n != '\n' {
                        o.blank(n);
                        i += 2;
                        continue;
                    }
                }
                i += 1;
                continue;
            }
            if c == q {
                quote = None;
                o.code(c);
            } else {
                o.blank(c);
            }
            i += 1;
            continue;
        }
        match *stack.last().unwrap() {
            Mode::Template => {
                if c == '\\' {
                    o.blank(c);
                    if let Some(n) = next {
                        if n != '\n' {
                            o.blank(n);
                            i += 2;
                            continue;
                        }
                    }
                    i += 1;
                } else if c == '`' {
                    stack.pop();
                    o.code(c);
                    i += 1;
                } else if c == '$' && next == Some('{') {
                    stack.push(Mode::Code(0));
                    o.code('$');
                    o.code('{');
                    i += 2;
                } else {
                    o.blank(c);
                    i += 1;
                }
            }
            Mode::Code(depth) => {
                if c == '/' && next == Some('/') {
                    in_line = true;
                    o.start_comment();
                    o.blank(c);
                    o.blank('/');
                    i += 2;
                } else if c == '/' && next == Some('*') {
                    in_block = true;
                    o.start_comment();
                    o.blank(c);
                    o.blank('*');
                    i += 2;
                } else if c == '"' || c == '\'' {
                    quote = Some(c);
                    o.code(c);
                    i += 1;
                } else if c == '`' {
                    if go {
                        raw_go = true;
                    } else {
                        stack.push(Mode::Template);
                    }
                    o.code(c);
                    i += 1;
                } else if c == '{' {
                    *stack.last_mut().unwrap() = Mode::Code(depth + 1);
                    o.code(c);
                    i += 1;
                } else if c == '}' {
                    if depth == 0 && stack.len() > 1 {
                        stack.pop();
                    } else if depth > 0 {
                        *stack.last_mut().unwrap() = Mode::Code(depth - 1);
                    }
                    o.code(c);
                    i += 1;
                } else {
                    o.code(c);
                    i += 1;
                }
            }
        }
    }
    if in_block {
        if let Some(t) = o.cur_comment.take() {
            o.cur_comment = Some(strip_block_leader(&t));
        }
    }
    o.finish_clike()
}

impl Out {
    /// Same as [`Out::finish`], used by [`lex_clike`].
    fn finish_clike(mut self) -> Lexed {
        // Flush trailing line comment (no newline at EOF) with the line number.
        if self.cur_comment.is_some() || !self.cur_code.is_empty() {
            self.newline();
        }
        Lexed {
            comments: self.comments,
            raw: self.raw,
            code: self.code,
        }
    }
}

/// Lexes `#` comments and quoted strings for Python, shell and the generic `#` languages.
///
/// With `python` set, `#` always starts a comment outside strings and `'''`/`"""` strings may span lines. Otherwise
/// `#` starts a comment only at the start of a line or after whitespace or `; ( ) & |`, and with `shell` set never
/// after `$` (`$#`); shell heredoc bodies are blanked up to their terminator line. In Python and shell an unterminated
/// `'`/`"` string ends at the end of its line.
fn lex_hash(src: &str, python: bool, shell: bool) -> Lexed {
    let chars: Vec<char> = src.chars().collect();
    let mut o = Out::new(src);
    let mut i = 0;
    let mut in_comment = false;
    let mut quote: Option<char> = None;
    let mut triple: Option<char> = None;
    let mut heredoc: Option<String> = None;
    let mut line_start = true;
    let mut line_buf = String::new(); // code on current line, to detect heredoc starts
                                      // Pre-split lines for heredoc body detection.
    let lines: Vec<&str> = src.split('\n').collect();
    let mut cur_line_idx = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '\n' {
            if shell && heredoc.is_none() {
                if let Some(ci) = o
                    .cur_code
                    .chars()
                    .collect::<Vec<_>>()
                    .windows(2)
                    .position(|w| w == ['<', '<'])
                {
                    let raw_line = lines.get(cur_line_idx).copied().unwrap_or("");
                    let rest: String = raw_line.chars().skip(ci + 2).collect();
                    if let Some(tag) = find_heredoc_tag(&rest) {
                        heredoc = Some(tag);
                    }
                }
            }
            in_comment = false;
            if shell {
                quote = None; // keep multi-line quotes simple: only reset for comments
            }
            if python {
                quote = None;
            }
            o.newline();
            cur_line_idx += 1;
            line_start = true;
            line_buf.clear();
            i += 1;
            // heredoc body: consume lines until terminator
            if let Some(tag) = heredoc.clone() {
                while cur_line_idx < lines.len() {
                    let l = lines[cur_line_idx];
                    let is_end = l.trim() == tag;
                    for ch in l.chars() {
                        o.blank(ch);
                        i += 1;
                    }
                    if cur_line_idx + 1 < lines.len() {
                        o.newline();
                        i += 1; // the '\n'
                    }
                    cur_line_idx += 1;
                    if is_end {
                        heredoc = None;
                        break;
                    }
                }
            }
            continue;
        }
        if in_comment {
            o.comment_char(c);
            i += 1;
            continue;
        }
        if let Some(q) = triple {
            if c == q && next == Some(q) && chars.get(i + 2) == Some(&q) {
                triple = None;
                o.code(q);
                o.code(q);
                o.code(q);
                i += 3;
            } else {
                if c == '\\' {
                    o.blank(c);
                    if let Some(n) = next {
                        if n != '\n' {
                            o.blank(n);
                            i += 2;
                            continue;
                        }
                    }
                } else {
                    o.blank(c);
                }
                i += 1;
            }
            continue;
        }
        if let Some(q) = quote {
            if c == '\\' && !(shell && q == '\'') {
                o.blank(c);
                if let Some(n) = next {
                    if n != '\n' {
                        o.blank(n);
                        i += 2;
                        continue;
                    }
                }
                i += 1;
                continue;
            }
            if c == q {
                quote = None;
                o.code(c);
            } else {
                o.blank(c);
            }
            i += 1;
            continue;
        }
        // code
        let prev = line_buf.chars().last();
        let hash_starts_comment = c == '#'
            && if python {
                true
            } else {
                match prev {
                    None => true,
                    Some(p) => p.is_whitespace() || matches!(p, ';' | '(' | ')' | '&' | '|'),
                }
            }
            && !(shell && prev == Some('$'));
        if hash_starts_comment {
            // `#!` shebang is a comment too; harmless.
            in_comment = true;
            o.start_comment();
            o.blank(c);
            i += 1;
            line_start = false;
            continue;
        }
        if python && (c == '"' || c == '\'') && next == Some(c) && chars.get(i + 2) == Some(&c) {
            triple = Some(c);
            o.code(c);
            o.code(c);
            o.code(c);
            line_buf.push_str("   ");
            i += 3;
            continue;
        }
        if c == '"' || c == '\'' {
            quote = Some(c);
            o.code(c);
            line_buf.push(c);
            i += 1;
            continue;
        }
        let _ = line_start;
        line_start = false;
        o.code(c);
        line_buf.push(c);
        i += 1;
    }
    o.finish()
}

/// Returns the heredoc terminator from the text after `<<` (`-`, quotes and `\` before the word are skipped), or
/// `None` for a here-string `<<<` or when no word follows.
fn find_heredoc_tag(rest: &str) -> Option<String> {
    if rest.starts_with('<') {
        return None; // here-string <<<
    }
    let rest = rest.strip_prefix('-').unwrap_or(rest).trim_start();
    let rest = rest.trim_start_matches(['\'', '"', '\\']);
    let tag: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if tag.is_empty() {
        None
    } else {
        Some(tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(l: &Lexed) -> Vec<(usize, String)> {
        l.comments
            .iter()
            .map(|c| (c.line, c.text.trim().to_string()))
            .collect()
    }

    #[test]
    fn js_line_and_block_comments() {
        let l = lex(
            Language::JavaScript,
            "// a\nconst x = 1; // b\n/* c\n * d\n */\nfoo();\n",
        );
        let t: Vec<_> = texts(&l)
            .into_iter()
            .filter(|(_, s)| !s.is_empty())
            .collect();
        assert_eq!(
            t,
            vec![
                (1, "a".to_string()),
                (2, "b".into()),
                (3, "c".into()),
                (4, "d".into())
            ]
        );
    }

    #[test]
    fn js_slashes_in_strings_are_not_comments() {
        let l = lex(
            Language::JavaScript,
            "const u = \"https://x.test/a\"; // real\nconst t = `//no ${ a // yes\n} //no`;\n",
        );
        let t = texts(&l);
        assert!(t.contains(&(1, "real".into())));
        assert!(t.contains(&(2, "yes".into())), "{t:?}");
        assert!(!t
            .iter()
            .any(|(_, s)| s.contains("no") && !s.contains("yes") && s != "real"));
        assert!(!l.code[0].contains("https"));
        assert!(l.code[0].contains("const u"));
    }

    #[test]
    fn js_template_nested_and_code_mask() {
        let l = lex(
            Language::JavaScript,
            "const a = `x ${fetch(\"u\")} y`; // c\n",
        );
        assert!(l.code[0].contains("fetch("));
        assert_eq!(texts(&l), vec![(1, "c".into())]);
    }

    #[test]
    fn go_raw_strings_and_runes() {
        let l = lex(
            Language::Go,
            "s := `// not a comment\nstill raw`\n// real\nr := '/'\nhttp.Get(u) // t\n",
        );
        assert_eq!(texts(&l), vec![(3, "real".into()), (5, "t".into())]);
        assert!(l.code[4].contains("http.Get"));
    }

    #[test]
    fn python_hash_and_triple_quotes() {
        let l = lex(
            Language::Python,
            "x = '#no'\n\"\"\"doc # no\n@openshell egress x:1\n\"\"\"\n# yes\ny = 1  # also\n",
        );
        assert_eq!(texts(&l), vec![(5, "yes".into()), (6, "also".into())]);
    }

    #[test]
    fn shell_hash_rules() {
        let l = lex(
            Language::Shell,
            "echo $# \"a # b\" 'c # d' # e\nx=${#arr[@]}\nfoo#bar\n# f\n",
        );
        assert_eq!(texts(&l), vec![(1, "e".into()), (4, "f".into())]);
    }

    #[test]
    fn shell_heredoc_body_is_not_comments() {
        let l = lex(Language::Shell, "cat <<'EOF'\n# inside\nEOF\n# after\n");
        assert_eq!(texts(&l), vec![(4, "after".into())]);
        assert_eq!(l.code.len(), 4);
    }

    #[test]
    fn language_detection() {
        assert_eq!(
            Language::from_path(Path::new("a/b.tsx")),
            Some(Language::JavaScript)
        );
        assert_eq!(
            Language::from_path(Path::new("a/b.py")),
            Some(Language::Python)
        );
        assert_eq!(
            Language::from_path(Path::new("main.go")),
            Some(Language::Go)
        );
        assert_eq!(
            Language::from_path(Path::new("run.sh")),
            Some(Language::Shell)
        );
        assert_eq!(
            Language::from_path(Path::new("Dockerfile")),
            Some(Language::Shell)
        );
        assert_eq!(Language::from_path(Path::new("x.rs")), None);
    }
}
