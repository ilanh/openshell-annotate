//! `generate --scaffold-profiles` and the duplicate-endpoint warning.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_openshell-annotate"))
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(bin())
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

#[allow(dead_code)]
fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn write(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

fn project() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    write(
        t.path(),
        "src/a.ts",
        "// @openshell egress api.ctx.com:443 reason=\"docs\" bin=/usr/local/bin/node access=read-only\n// @openshell egress cdn.ctx.com:443 reason=\"assets\" bin=/usr/local/bin/node tls=skip\n// @openshell provider CTX_KEY\n",
    );
    write(
        t.path(),
        "src/b.ts",
        "// @openshell provider LONELY_TOKEN\n",
    );
    t
}

#[test]
fn scaffold_writes_a_skeleton_per_unmapped_credential_and_keeps_existing_files() {
    let t = project();
    let o = run(
        t.path(),
        &[
            "generate",
            "--scaffold-profiles",
            "prof",
            "--format",
            "compact",
        ],
    );
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        err(&o).contains("scaffolded profile prof/ctx-key.yaml (2 endpoint(s))"),
        "{}",
        err(&o)
    );
    let ps = openshell_annotate::providers::load_profiles(&t.path().join("prof")).unwrap();
    let ctx = ps.iter().find(|p| p.id == "ctx-key").expect("ctx-key");
    assert_eq!(ctx.env_vars, vec!["CTX_KEY"]);
    assert_eq!(ctx.endpoints, vec!["api.ctx.com:443", "cdn.ctx.com:443"]);
    assert_eq!(ctx.binaries, vec!["/usr/local/bin/node"]);
    let y = std::fs::read_to_string(t.path().join("prof/ctx-key.yaml")).unwrap();
    assert!(y.contains("access: read-only"), "{y}");
    assert!(
        y.contains("allow_uninspected_credentials: true"),
        "tls=skip endpoint needs it: {y}"
    );
    let lonely = std::fs::read_to_string(t.path().join("prof/lonely-token.yaml")).unwrap();
    assert!(
        lonely.contains("endpoints: []") && lonely.contains("# TODO"),
        "{lonely}"
    );
    assert!(!y.contains("CTX_KEY="), "no secret values");

    // a second run keeps what the user edited
    std::fs::write(t.path().join("prof/ctx-key.yaml"), "id: ctx-key\n# mine\n").unwrap();
    let o = run(
        t.path(),
        &[
            "generate",
            "--scaffold-profiles",
            "prof",
            "--format",
            "compact",
        ],
    );
    assert!(
        err(&o).contains("kept existing profile prof/ctx-key.yaml"),
        "{}",
        err(&o)
    );
    assert_eq!(
        std::fs::read_to_string(t.path().join("prof/ctx-key.yaml")).unwrap(),
        "id: ctx-key\n# mine\n"
    );
}

#[test]
fn mapped_credentials_are_not_scaffolded_and_nothing_is_written_without_the_flag() {
    let t = project();
    run(t.path(), &["generate", "--format", "compact"]);
    assert!(!t.path().join("prof").exists());
    write(
        t.path(),
        "openshell-annotate.toml",
        "[providers.env]\nCTX_KEY = \"ctx\"\nLONELY_TOKEN = \"lonely\"\n",
    );
    let o = run(
        t.path(),
        &[
            "generate",
            "--scaffold-profiles",
            "prof",
            "--format",
            "compact",
        ],
    );
    assert!(!err(&o).contains("scaffolded"), "{}", err(&o));
}

fn github_profile(t: &Path, binaries: &str) {
    write(
        t,
        "profiles/github.yaml",
        &format!("id: github\ncredentials:\n  - env_vars: [GITHUB_TOKEN]\nendpoints:\n  - host: api.github.com\n    port: 443\nbinaries: [{binaries}]\n"),
    );
}

fn dup_project(bin: &str, binaries: &str) -> (tempfile::TempDir, String) {
    let t = tempfile::tempdir().unwrap();
    write(
        t.path(),
        "src/a.ts",
        &format!("// @openshell egress api.github.com:443 reason=\"r\" bin={bin}\n// @openshell provider GITHUB_TOKEN\n"),
    );
    github_profile(t.path(), binaries);
    let o = run(
        t.path(),
        &[
            "generate",
            "--profiles-dir",
            "profiles",
            "--format",
            "compact",
        ],
    );
    assert!(o.status.success(), "{}", err(&o));
    let e = err(&o);
    (t, e)
}

#[test]
fn duplicate_endpoint_warning_only_when_the_profile_covers_the_same_binary() {
    let (_t, e) = dup_project("/usr/bin/gh", "/usr/bin/gh, /usr/bin/git");
    assert!(
        e.contains("duplicates an endpoint the `github` provider profile adds"),
        "{e}"
    );
    let (_t, e) = dup_project("/usr/local/bin/node", "/usr/bin/gh, /usr/bin/git");
    assert!(
        !e.contains("duplicates an endpoint"),
        "different binary: the rule is needed: {e}"
    );
}
