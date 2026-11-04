use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use openshell_annotate::config::{Config, CONFIG_FILE};
use openshell_annotate::directive::Directive;
use openshell_annotate::drift;
use openshell_annotate::generate::{self, miette_str};
use openshell_annotate::lint::{self, Severity};
use openshell_annotate::providers::{self, Profile};
use openshell_annotate::render;
use openshell_annotate::report::{self, ColorMode, Style};
use openshell_annotate::scan::{self, Scan};
use openshell_annotate::validate;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Policy-as-code for OpenShell: `@openshell` source directives -> policy.yaml.
#[derive(Parser)]
#[command(name = "openshell-annotate", version, about)]
struct Cli {
    /// Repository root: paths in output are relative to it; `openshell-annotate.toml` is read from it. `diff-logs` run
    /// without --root looks for that file in the parent directories, so it works from a subdirectory.
    #[arg(long, global = true, default_value = ".")]
    root: PathBuf,
    /// Config file (default: <root>/openshell-annotate.toml if present).
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

/// Output format (`scan`, `generate`, `diff-logs`, `lint`).
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    /// Multi-line report with source context, icons and colour (the default).
    Human,
    /// Machine-readable JSON (stable). For `generate` the policy stays on stdout and the JSON summary goes to stderr.
    Json,
    /// One line per item, the plain output of earlier versions (stable; for scripts and CI logs).
    Compact,
}

/// `--color` values.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ColorArg {
    /// Colour when the output stream is a terminal (stdout; stderr for the `generate` summary); honours NO_COLOR, CLICOLOR_FORCE and TERM=dumb.
    Auto,
    Always,
    Never,
}

/// Presentation flags shared by `scan`, `generate`, `diff-logs` and `lint`.
#[derive(clap::Args, Clone, Copy)]
struct OutputOpts {
    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Human)]
    format: Format,
    /// Colour for the human format.
    #[arg(long, value_enum, default_value_t = ColorArg::Auto)]
    color: ColorArg,
    /// Plain ASCII in the human format (no emoji or Unicode icons). Icons also turn off by themselves when the locale is
    /// not UTF-8 or TERM=dumb.
    #[arg(long, visible_alias = "ascii")]
    no_icons: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// List directives with file:line.
    Scan {
        /// Files or directories to process (default: the whole root).
        paths: Vec<PathBuf>,
        /// Alias for `--format json`.
        #[arg(long, conflicts_with = "format")]
        json: bool,
        #[command(flatten)]
        out: OutputOpts,
    },
    /// Generate policy.yaml (and a providers file) from directives.
    ///
    /// The policy YAML is the only thing on stdout (or in --output) whatever the format, so piping is safe. The summary
    /// (rules, files, sources, warnings) goes to stderr and follows --format, --color and --no-icons.
    Generate {
        /// Files or directories to process (default: the whole root).
        paths: Vec<PathBuf>,
        /// Write policy here (default: stdout).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Write providers here: `.sh` = setup script, anything else = JSON.
        #[arg(long)]
        providers_out: Option<PathBuf>,
        /// Write a provider-profile skeleton into DIR for every credential no profile declares. It is built from the
        /// egress directives in the file that declares the credential; existing files are kept. Review it, then check
        /// it with `openshell provider profile lint -f`.
        #[arg(long, value_name = "DIR")]
        scaffold_profiles: Option<PathBuf>,
        /// Provider-profile directory (overrides [providers].profiles_dir).
        #[arg(long)]
        profiles_dir: Option<PathBuf>,
        /// Container image the policy is for: inspect its WORKDIR, ENTRYPOINT and CMD (docker or podman) and propose
        /// `@openshell fs read` for application directories outside OpenShell's baseline read set. Nothing is added automatically.
        #[arg(long, value_name = "IMAGE")]
        image: Option<String>,
        /// Read `image inspect` JSON from this file instead of running a container tool (for offline CI).
        #[arg(long, value_name = "FILE", requires = "image")]
        image_inspect: Option<PathBuf>,
        #[command(flatten)]
        out: OutputOpts,
    },
    /// Validate directives, find un-annotated network/fs/secret call sites, optionally check a policy file.
    Lint {
        /// Files or directories to process (default: the whole root).
        paths: Vec<PathBuf>,
        /// Also parse and semantically check this policy.yaml.
        #[arg(long)]
        policy: Option<PathBuf>,
        /// Exit 1 on warnings too.
        #[arg(long)]
        deny_warnings: bool,
        /// Alias for `--format json`.
        #[arg(long, conflicts_with = "format")]
        json: bool,
        #[command(flatten)]
        out: OutputOpts,
    },
    /// Map sandbox allow/deny events (OCSF JSON lines or `openshell logs` shorthand) back to directives: suggest missing ones,
    /// flag stale ones. Exit 1 when directives are missing.
    DiffLogs {
        /// Log files (`-` = stdin). OCSF JSONL (`/var/log/openshell-ocsf.*.jsonl`) and `openshell logs` output both work.
        logs: Vec<PathBuf>,
        /// Apply the suggested directives to the sources (or to --observed-file when no source mentions the host).
        #[arg(long)]
        write: bool,
        /// File that receives suggestions no source line mentions (relative to --root).
        #[arg(long, default_value = "openshell/observed-egress.sh")]
        observed_file: String,
        /// Also exit 1 when stale directives are found.
        #[arg(long)]
        deny_stale: bool,
        /// Alias for `--format json`.
        #[arg(long, conflicts_with = "format")]
        json: bool,
        #[command(flatten)]
        out: OutputOpts,
    },
    /// Generate the reference docs (CLI.md, DIRECTIVES.md) from the code; `--check` fails if the committed ones are stale.
    Docs {
        /// Output directory.
        #[arg(long, default_value = "docs")]
        out: PathBuf,
        /// Do not write; exit 1 when the files in --out differ from what the code generates.
        #[arg(long)]
        check: bool,
    },
    /// Compare the policy generated from directives with a committed policy.yaml (CI gate).
    Drift {
        /// Files or directories to process (default: the whole root).
        paths: Vec<PathBuf>,
        #[arg(long)]
        policy: PathBuf,
        /// Also compare a committed providers JSON file.
        #[arg(long)]
        providers: Option<PathBuf>,
        #[arg(long)]
        profiles_dir: Option<PathBuf>,
        /// Flag committed sections directives cannot express (landlock, network_middlewares).
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        json: bool,
    },
}

fn expand(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(h) = std::env::var_os("HOME") {
            return PathBuf::from(h).join(rest);
        }
    }
    PathBuf::from(p)
}

fn load_profiles(cfg: &Config, cli_dir: Option<&Path>, root: &Path) -> Result<Vec<Profile>> {
    let resolve = |p: &str| {
        let p = expand(p);
        if p.is_relative() {
            root.join(p)
        } else {
            p
        }
    };
    // Each directory remembers where it was named, so a missing one can point at the right place.
    let mut dirs: Vec<(PathBuf, DirSource)> = Vec::new();
    match cli_dir {
        Some(d) => dirs.push((d.to_path_buf(), DirSource::Flag)),
        None => {
            if let Some(d) = cfg.providers.profiles_dir.as_deref() {
                dirs.push((resolve(d), DirSource::Config("profiles_dir")));
            }
        }
    }
    for d in &cfg.providers.profiles_dirs {
        dirs.push((resolve(d), DirSource::Config("profiles_dirs")));
    }
    let mut out = Vec::new();
    for (d, source) in dirs {
        // Only a missing directory gets the hint; every other failure keeps its own context.
        if let Err(e) = std::fs::metadata(&d) {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::bail!("{}", source.missing_message(&d));
            }
        }
        out.extend(providers::load_profiles(&d)?);
    }
    Ok(out)
}

/// Where a provider-profiles directory was named.
#[derive(Clone, Copy)]
enum DirSource {
    /// `--profiles-dir`.
    Flag,
    /// A `[providers]` key in `openshell-annotate.toml`.
    Config(&'static str),
}

impl DirSource {
    /// One error line naming the source, then one hint line.
    fn missing_message(self, dir: &Path) -> String {
        match self {
            DirSource::Flag => format!(
                "profiles directory {} (from --profiles-dir) does not exist\nhint: create it, fix the --profiles-dir path, or clone OpenShell",
                dir.display()
            ),
            DirSource::Config(key) => format!(
                "profiles directory {} (from `[providers] {key}` in {CONFIG_FILE}) does not exist\nhint: create it, fix the path in {CONFIG_FILE}, or clone OpenShell",
                dir.display()
            ),
        }
    }
}

fn do_scan(root: &Path, paths: &[PathBuf], cfg: &Config) -> Result<Scan> {
    let s = scan::scan(root, paths, cfg)?;
    for n in s.unscanned_notes() {
        eprintln!("note: {n}");
    }
    for d in &s.dead_excludes {
        eprintln!("warning: {}", d.note());
    }
    Ok(s)
}

fn style_for(o: &OutputOpts, stream_is_tty: bool) -> Style {
    let env = |k: &str| std::env::var(k).ok();
    Style {
        color: report::resolve_color(
            match o.color {
                ColorArg::Auto => ColorMode::Auto,
                ColorArg::Always => ColorMode::Always,
                ColorArg::Never => ColorMode::Never,
            },
            stream_is_tty,
            &env,
        ),
        icons: !o.no_icons && report::resolve_icons(&env),
    }
}

/// Source lines of a file: from the scan when it was scanned, else read from disk (policy files, observed-egress.sh).
fn lines_of<'a>(root: &'a Path, s: &'a Scan) -> impl Fn(&str) -> Option<Vec<String>> + 'a {
    move |file: &str| {
        if let Some(src) = s.sources.iter().find(|x| x.rel == file) {
            return Some(src.lexed.raw.clone());
        }
        let text = std::fs::read_to_string(root.join(file)).ok()?;
        Some(text.lines().map(str::to_string).collect())
    }
}

fn report_parse_errors(s: &Scan) -> bool {
    for e in &s.errors {
        eprintln!("error: {e}");
    }
    !s.errors.is_empty()
}

fn run(cli: Cli) -> Result<ExitCode> {
    if let Cmd::Docs { out, check } = &cli.cmd {
        let cmd = Cli::command();
        if *check {
            openshell_annotate::docs::check(&cmd, out)?;
            eprintln!("docs: {} is up to date", out.display());
        } else {
            for f in openshell_annotate::docs::write(&cmd, out)? {
                eprintln!("wrote {f}");
            }
        }
        return Ok(ExitCode::SUCCESS);
    }
    let root = cli
        .root
        .canonicalize()
        .with_context(|| format!("root {}", cli.root.display()))?;
    let cfg = Config::load_or_default(&root, cli.config.as_deref())?;
    match cli.cmd {
        Cmd::Scan { paths, json, out } => {
            use std::io::IsTerminal;
            let format = if json { Format::Json } else { out.format };
            let s = do_scan(&root, &paths, &cfg)?;
            match format {
                Format::Json => {
                    #[derive(serde::Serialize)]
                    struct Out<'a> {
                        directives: &'a [Directive],
                        errors: &'a [openshell_annotate::directive::ParseError],
                    }
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&Out {
                            directives: &s.directives,
                            errors: &s.errors
                        })?
                    );
                }
                Format::Compact => {
                    for d in &s.directives {
                        let detail = render::directive_detail(d);
                        let detail = if detail.is_empty() {
                            detail
                        } else {
                            format!(" {detail}")
                        };
                        let reason = d.reason().map(|r| format!("  # {r}")).unwrap_or_default();
                        println!("{}:{}  {}{detail}{reason}", d.file, d.line, d.kind.name());
                    }
                    eprintln!(
                        "{} directive(s) in {} file(s)",
                        s.directives.len(),
                        s.sources.len()
                    );
                }
                Format::Human => {
                    let style = style_for(&out, std::io::stdout().is_terminal());
                    print!("{}", render::scan_human(&s, &lines_of(&root, &s), style));
                }
            }
            let had = report_parse_errors(&s);
            Ok(if had {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
        Cmd::Generate {
            paths,
            output,
            providers_out,
            scaffold_profiles,
            profiles_dir,
            image,
            image_inspect,
            out,
        } => {
            use std::io::IsTerminal;
            let s = do_scan(&root, &paths, &cfg)?;
            if report_parse_errors(&s) {
                return Ok(ExitCode::from(1));
            }
            let profiles = load_profiles(&cfg, profiles_dir.as_deref(), &root)?;
            match generate::generate(&s.directives, &cfg, &profiles) {
                Ok(g) => {
                    if out.format == Format::Compact {
                        for w in &g.warnings {
                            eprintln!("warning: {w}");
                        }
                    }
                    // The policy goes to stdout / --output untouched, whatever the format: only stderr is decorated.
                    match &output {
                        Some(p) => {
                            if let Some(parent) = p.parent().filter(|x| !x.as_os_str().is_empty()) {
                                std::fs::create_dir_all(parent)?;
                            }
                            std::fs::write(p, &g.policy_yaml)?
                        }
                        None => print!("{}", g.policy_yaml),
                    }
                    if let Some(p) = &providers_out {
                        if let Some(parent) = p.parent().filter(|x| !x.as_os_str().is_empty()) {
                            std::fs::create_dir_all(parent)?;
                        }
                        let body = if p.extension().and_then(|e| e.to_str()) == Some("sh") {
                            g.providers.to_script(
                                &output
                                    .as_ref()
                                    .map(|o| o.display().to_string())
                                    .unwrap_or_else(|| "policy.yaml".into()),
                                &root,
                            )
                        } else {
                            g.providers.to_json()
                        };
                        std::fs::write(p, body)?;
                    }
                    if let Some(dir) = &scaffold_profiles {
                        for sk in openshell_annotate::scaffold::scaffold(
                            &g.providers.unmapped,
                            &s.directives,
                            &cfg,
                        ) {
                            let path = dir.join(&sk.file_name);
                            if path.exists() {
                                eprintln!("kept existing profile {}", path.display());
                                continue;
                            }
                            std::fs::create_dir_all(dir)?;
                            std::fs::write(&path, &sk.yaml)?;
                            eprintln!(
                                "scaffolded profile {} ({} endpoint(s)); review it, then run: openshell provider profile lint -f {}",
                                path.display(),
                                sk.endpoints,
                                path.display()
                            );
                        }
                    }
                    match out.format {
                        Format::Compact => eprintln!(
                            "generated: {} directive(s) -> {} network rule(s), {} endpoint(s), {} binary path(s), fs read {} / write {}, {} provider(s) ({} unmapped)",
                            g.stats.directives, g.stats.network_rules, g.stats.endpoints, g.stats.binaries, g.stats.fs_read, g.stats.fs_write, g.stats.providers, g.stats.unmapped_providers
                        ),
                        Format::Json => {
                            #[derive(serde::Serialize)]
                            struct Summary<'a> {
                                stats: &'a generate::Stats,
                                warnings: &'a [String],
                                policy: Option<String>,
                                providers: Option<String>,
                            }
                            eprintln!(
                                "{}",
                                serde_json::to_string_pretty(&Summary {
                                    stats: &g.stats,
                                    warnings: &g.warnings,
                                    policy: output.as_ref().map(|p| p.display().to_string()),
                                    providers: providers_out
                                        .as_ref()
                                        .map(|p| p.display().to_string()),
                                })?
                            );
                        }
                        Format::Human => {
                            let style = style_for(&out, std::io::stderr().is_terminal());
                            let dest = |p: &PathBuf| p.display().to_string();
                            eprint!(
                                "{}",
                                render::generate_human(
                                    &g,
                                    &s.directives,
                                    output.as_ref().map(dest).as_deref(),
                                    providers_out.as_ref().map(dest).as_deref(),
                                    style
                                )
                            );
                        }
                    }
                    if let Some(img) = &image {
                        let info = match &image_inspect {
                            Some(f) => openshell_annotate::image::from_file(f),
                            None => openshell_annotate::image::inspect(img),
                        };
                        match info {
                            Ok(info) => {
                                let declared: Vec<String> = s
                                    .directives
                                    .iter()
                                    .filter_map(|d| match &d.kind {
                                        openshell_annotate::directive::Kind::Fs {
                                            path, ..
                                        } => Some(path.clone()),
                                        _ => None,
                                    })
                                    .collect();
                                let mut props = openshell_annotate::image::propose(
                                    &info,
                                    &declared,
                                    cfg.policy.include_workdir,
                                );
                                if image_inspect.is_none() {
                                    match openshell_annotate::image::list_app_dirs(img) {
                                        Ok(dirs) => openshell_annotate::image::add_dirs(
                                            &mut props, dirs, &declared,
                                        ),
                                        Err(e) => eprintln!("note: could not list the image filesystem ({e:#}); using WORKDIR, ENTRYPOINT and CMD only"),
                                    }
                                }
                                if declared.is_empty() {
                                    eprintln!("image {img}: the policy has no filesystem section (no `fs` directive), so file access is not restricted and nothing needs declaring");
                                } else if props.is_empty() {
                                    eprintln!("image {img}: WORKDIR, ENTRYPOINT and CMD are all inside the baseline read set or declared fs paths");
                                } else {
                                    eprint!(
                                        "{}",
                                        openshell_annotate::image::render_note(img, &props)
                                    );
                                }
                            }
                            Err(e) => {
                                eprintln!("error: {e:#}");
                                return Ok(ExitCode::from(1));
                            }
                        }
                    }
                    Ok(ExitCode::SUCCESS)
                }
                Err(errs) => {
                    for e in errs {
                        eprintln!("error: {e}");
                    }
                    Ok(ExitCode::from(1))
                }
            }
        }
        Cmd::Lint {
            paths,
            policy,
            deny_warnings,
            json,
            out,
        } => {
            let format = if json { Format::Json } else { out.format };
            let s = do_scan(&root, &paths, &cfg)?;
            let mut findings = lint::lint(&s, &cfg);
            if let Some(p) = &policy {
                let text = std::fs::read_to_string(p)
                    .with_context(|| format!("reading {}", p.display()))?;
                match openshell_policy_schema::parse_policy(&text) {
                    Ok(doc) => {
                        let (errs, warns) = validate::check_document(&doc);
                        for (v, sev) in errs
                            .into_iter()
                            .map(|e| (e, Severity::Error))
                            .chain(warns.into_iter().map(|w| (w, Severity::Warning)))
                        {
                            let mut f =
                                lint::Finding::new(p.display().to_string(), 0, sev, "policy", v);
                            // "network_policies.api.endpoints[0]: reason" -> headline = reason, note = where.
                            if let Some((at, why)) = f.message.split_once(": ") {
                                if !at.contains(' ') {
                                    f.note = Some(format!("at `{at}`"));
                                    f.title = Some(why.to_string());
                                }
                            }
                            findings.push(f);
                        }
                    }
                    Err(e) => findings.push(lint::Finding::new(
                        p.display().to_string(),
                        0,
                        Severity::Error,
                        "policy",
                        miette_str(&e),
                    )),
                }
            }
            let (ne, nw, ni) = (
                findings
                    .iter()
                    .filter(|f| f.severity == Severity::Error)
                    .count(),
                findings
                    .iter()
                    .filter(|f| f.severity == Severity::Warning)
                    .count(),
                findings
                    .iter()
                    .filter(|f| f.severity == Severity::Info)
                    .count(),
            );
            match format {
                Format::Json => println!("{}", serde_json::to_string_pretty(&findings)?),
                Format::Compact => {
                    for f in &findings {
                        let sev = format!("{:?}", f.severity).to_lowercase();
                        println!(
                            "{}:{}: {sev}[{}]: {}",
                            f.file, f.line, f.category, f.message
                        );
                    }
                }
                Format::Human => {
                    use std::io::IsTerminal;
                    let style = style_for(&out, std::io::stdout().is_terminal());
                    let lines_of = lines_of(&root, &s);
                    print!(
                        "{}",
                        report::render(&findings, &lines_of, style, s.directives.len())
                    );
                }
            }
            if format != Format::Human {
                eprintln!(
                    "lint: {} directive(s); {ne} error(s), {nw} warning(s), {ni} info",
                    s.directives.len()
                );
            }
            Ok(if ne > 0 || (deny_warnings && nw > 0) {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
        Cmd::DiffLogs {
            logs,
            write,
            observed_file,
            deny_stale,
            json,
            out,
        } => {
            use openshell_annotate::diff_logs as dl;
            use std::io::IsTerminal;
            use std::io::Read;
            if logs.is_empty() {
                anyhow::bail!("give at least one log file (or `-` for stdin)");
            }
            let mut events = Vec::new();
            for l in &logs {
                let mut text = String::new();
                let name = if l.as_os_str() == "-" {
                    std::io::stdin().read_to_string(&mut text)?;
                    "<stdin>".to_string()
                } else {
                    text = std::fs::read_to_string(l)
                        .with_context(|| format!("reading {}", l.display()))?;
                    l.display().to_string()
                };
                events.extend(dl::parse_logs(&text, &name));
            }
            // Like git: when --root was not given, look for openshell-annotate.toml in the parent directories, so the
            // command works from a subdirectory (e.g. next to the sandbox log).
            let (root, mut cfg) = match (cli.root == Path::new("."), cli.config.is_none()) {
                (true, true) => match openshell_annotate::config::find_root(&root) {
                    Some(r) if r != root => {
                        eprintln!(
                            "note: using repository root {} (found {})",
                            r.display(),
                            openshell_annotate::config::CONFIG_FILE
                        );
                        let c = Config::load_or_default(&r, None)?;
                        (r, c)
                    }
                    _ => (root, cfg),
                },
                _ => (root, cfg),
            };
            // suggestions never go into the file diff-logs itself writes to
            cfg.diff_logs.exclude.push(format!("^{observed_file}"));
            let s = do_scan(&root, &[], &cfg)?;
            if report_parse_errors(&s) {
                return Ok(ExitCode::from(1));
            }
            let rep = dl::analyze(&events, &s, &cfg);
            match if json { Format::Json } else { out.format } {
                Format::Json => println!("{}", serde_json::to_string_pretty(&rep)?),
                Format::Compact => print!("{}", dl::render_text(&rep)),
                Format::Human => {
                    let style = style_for(&out, std::io::stdout().is_terminal());
                    let hint = logs
                        .iter()
                        .map(|l| l.display().to_string())
                        .collect::<Vec<_>>()
                        .join(" ");
                    print!(
                        "{}",
                        render::diff_logs_human(
                            &rep,
                            &s,
                            &lines_of(&root, &s),
                            &observed_file,
                            &hint,
                            style
                        )
                    );
                }
            }
            if write {
                let touched = dl::write_suggestions(&rep, &root, &s, &observed_file)?;
                eprintln!(
                    "wrote suggested directives to: {}",
                    if touched.is_empty() {
                        "(nothing)".into()
                    } else {
                        touched.join(", ")
                    }
                );
            }
            let bad = !rep.suggestions.is_empty() || (deny_stale && !rep.stale.is_empty());
            Ok(ExitCode::from(u8::from(bad)))
        }
        Cmd::Docs { .. } => unreachable!("handled above"),
        Cmd::Drift {
            paths,
            policy,
            providers: prov_file,
            profiles_dir,
            strict,
            json,
        } => {
            let s = do_scan(&root, &paths, &cfg)?;
            if report_parse_errors(&s) {
                return Ok(ExitCode::from(1));
            }
            let profiles = load_profiles(&cfg, profiles_dir.as_deref(), &root)?;
            let g = match generate::generate(&s.directives, &cfg, &profiles) {
                Ok(g) => g,
                Err(errs) => {
                    for e in errs {
                        eprintln!("error: {e}");
                    }
                    return Ok(ExitCode::from(1));
                }
            };
            let text = std::fs::read_to_string(&policy)
                .with_context(|| format!("reading {}", policy.display()))?;
            let committed = openshell_policy_schema::parse_policy(&text)
                .map_err(|e| anyhow::anyhow!("{}: {}", policy.display(), miette_str(&e)))?;
            let mut rep = drift::compare(&g.doc, &committed, strict);
            if let Some(pf) = prov_file {
                let v: serde_json::Value = serde_json::from_str(
                    &std::fs::read_to_string(&pf)
                        .with_context(|| format!("reading {}", pf.display()))?,
                )?;
                rep.findings
                    .extend(drift::compare_providers(&g.providers, &v));
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else if rep.findings.is_empty() {
                print!("{}", rep.render());
                println!(
                    "no drift: {} matches {} directive(s)",
                    policy.display(),
                    g.stats.directives
                );
            } else {
                print!("{}", rep.render());
                eprintln!(
                    "drift: {} finding(s); regenerate with `openshell-annotate generate -o {}`",
                    rep.findings.len(),
                    policy.display()
                );
            }
            Ok(if rep.findings.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}
