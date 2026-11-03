//! `openshell-annotate.toml` (strict: unknown keys are errors).

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

pub const CONFIG_FILE: &str = "openshell-annotate.toml";

/// Nearest directory at or above `start` that contains `openshell-annotate.toml`.
pub fn find_root(start: &Path) -> Option<std::path::PathBuf> {
    start
        .ancestors()
        .find(|d| d.join(CONFIG_FILE).is_file())
        .map(Path::to_path_buf)
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Default executable per language family (`javascript`, `python`, `go`, `shell`).
    /// Must be the *real path* of the binary inside the sandbox image (OpenShell does not follow symlinks).
    #[serde(default)]
    pub languages: BTreeMap<String, String>,
    #[serde(default)]
    pub policy: PolicyConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub lint: LintConfig,
    #[serde(default)]
    pub scan: ScanConfig,
    #[serde(default, rename = "diff-logs")]
    pub diff_logs: DiffLogsConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DiffLogsConfig {
    /// Path patterns (see `path_matches`) that `diff-logs` never uses as the place for a suggested directive, e.g. the
    /// probe script that drives the sandbox run. Directives *in* those files still count. The `--observed-file` is always skipped.
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    /// Emitted when any `fs` directive exists. A present `filesystem_policy` otherwise defaults it to false.
    #[serde(default = "yes")]
    pub include_workdir: bool,
    /// Default `enforcement` for inspected endpoints (OpenShell's own default is `audit`).
    #[serde(default = "enforce")]
    pub enforcement: String,
    /// Baseline system paths (`/usr /lib /etc /proc /dev/urandom` read, `/tmp /dev/null` write) that any process needs to start.
    /// OpenShell's supervisor adds them itself only when `network_policies` is non-empty, so a policy without egress would otherwise be unable to exec anything (verified: `failed to spawn sandbox entrypoint: Permission denied`). `auto` (default) adds them whenever the policy has no network rules,
    /// `always` adds them whenever a filesystem_policy is emitted, `never` leaves the policy exactly as annotated.
    #[serde(default = "auto")]
    pub system_paths: String,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        PolicyConfig {
            include_workdir: true,
            enforcement: "enforce".into(),
            system_paths: "auto".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProvidersConfig {
    /// Directory of provider-profile YAML files (e.g. `<OpenShell>/providers`).
    pub profiles_dir: Option<String>,
    /// Further profile directories (e.g. a repo-local `openshell/profiles` for custom profiles). Read after `profiles_dir`.
    #[serde(default)]
    pub profiles_dirs: Vec<String>,
    /// Env var -> profile id overrides.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Credentials the app reads through lookups no static scan can see (Ruby `ENV[key.to_s.upcase]`, viper
    /// `SetEnvPrefix` + `GetString`, a library choosing the variable by model name). Each one is treated exactly like an
    /// `@openshell provider ENV` directive written at `openshell-annotate.toml`: it is mapped to a profile, appears in the
    /// providers output, and counts as declared for `lint`.
    #[serde(default)]
    pub expect: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LintConfig {
    /// A directive covers call sites within this many lines below it.
    #[serde(default = "window")]
    pub window: usize,
    /// Extra path patterns excluded from lint call-site heuristics only (tests are excluded by default); see `path_matches`.
    /// Everything in `[scan].exclude` is already invisible to lint, so list a path here only to keep scanning its directives.
    /// Patterns are literal, not regexes.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Hosts (exact, or `*.example.com`) that are never reported as unannotated network destinations, e.g. documentation
    /// links in help text. Only for the tree-sitter engine. Use `@openshell ignore` for a single call site instead.
    #[serde(default)]
    pub ignore_hosts: Vec<String>,
    /// Call-site categories checked by the heuristics: any of "net", "fs", "env", "exec". Empty = all.
    /// Directive errors and policy sanity checks always run.
    #[serde(default)]
    pub categories: Vec<String>,
    /// `treesitter` (default; AST call sites, path-aware fs lint) or `regex` (the original line heuristics).
    #[serde(default = "engine")]
    pub engine: String,
    /// What to do with filesystem writes whose path is not a literal (so they cannot be checked statically):
    /// `summary` (default; one repo-level info), `info` (one per file), `warn` (one warning per file), `off`.
    /// With no `@openshell fs write` root declared anywhere these are always one warning per file.
    #[serde(default = "fs_dynamic")]
    pub fs_dynamic: String,
}

fn engine() -> String {
    "treesitter".into()
}
fn fs_dynamic() -> String {
    "summary".into()
}

impl Default for LintConfig {
    fn default() -> Self {
        LintConfig {
            window: 8,
            exclude: vec![],
            ignore_hosts: vec![],
            categories: vec![],
            engine: engine(),
            fs_dynamic: fs_dynamic(),
        }
    }
}

/// Exclude-pattern matching shared by `scan` and `lint`: `^prefix` anchors at the repo root,
/// anything else is a substring of `/<relative path>` (so `/ui/` matches any directory named ui).
pub fn path_matches(rel: &str, pat: &str) -> bool {
    if let Some(prefix) = pat.strip_prefix('^') {
        rel.starts_with(prefix)
    } else {
        format!("/{rel}").contains(pat)
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ScanConfig {
    /// Path patterns (see `path_matches`) never scanned, by `scan`, `generate`, `drift` and `lint` alike (in addition to
    /// .gitignore, node_modules, dist, build, target). Patterns are literal, not regexes: `^prefix/` anchors at the
    /// repo root, anything else is a substring of `/<relative path>`. A pattern that matches no file is reported.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// File extension (no dot) -> comment style for languages without first-class support:
    /// `hash` (`#`), `slash` (`//` and `/* */`) or `none` (do not scan). Built-in defaults cover
    /// rb, rs, java, kt, cs, php, swift, scala and a few more; entries here override them.
    #[serde(default)]
    pub comment_leaders: BTreeMap<String, String>,
}

fn auto() -> String {
    "auto".into()
}

fn yes() -> bool {
    true
}
fn enforce() -> String {
    "enforce".into()
}
fn window() -> usize {
    8
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let s =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let c: Config =
            toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))?;
        if !matches!(c.policy.enforcement.as_str(), "enforce" | "audit") {
            anyhow::bail!(
                "{}: policy.enforcement must be `enforce` or `audit`",
                path.display()
            );
        }
        for (ext, kind) in &c.scan.comment_leaders {
            if !matches!(kind.as_str(), "hash" | "slash" | "none") {
                anyhow::bail!(
                    "{}: scan.comment_leaders.{ext} must be `hash`, `slash` or `none`, not `{kind}`",
                    path.display()
                );
            }
        }
        for env in &c.providers.expect {
            let ok = env.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && env
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
            if !ok {
                anyhow::bail!(
                    "{}: providers.expect entry `{env}` must be an environment variable name like `GITHUB_PERSONAL_ACCESS_TOKEN`",
                    path.display()
                );
            }
        }
        Ok(c)
    }

    pub fn load_or_default(root: &Path, explicit: Option<&Path>) -> Result<Config> {
        match explicit {
            Some(p) => Config::load(p),
            None => {
                let p = root.join(CONFIG_FILE);
                if p.is_file() {
                    Config::load(&p)
                } else {
                    Ok(Config::default())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::path_matches;

    #[test]
    fn exclude_patterns() {
        assert!(path_matches("ui/src/a.ts", "^ui/"));
        assert!(!path_matches("server/src/ui/a.ts", "^ui/"));
        assert!(path_matches("server/src/ui/a.ts", "/ui/"));
        assert!(path_matches("ui/a.ts", "/ui/"));
        assert!(!path_matches("server/guide/a.ts", "/ui/"));
    }
}
