//! providers.sh: profile files outside the repo are located through OPENSHELL_PROFILES_DIR instead of a fixed relative path.
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

const PROFILE: &str = "id: acme\ncredentials:\n  - env_vars: [ACME_KEY]\nendpoints:\n  - host: api.acme.test\n    port: 443\n";

fn setup(profiles_dir: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let repo = t.path().join("repo");
    write(&repo, "src/a.ts", "// @openshell egress api.acme.test:443 reason=\"r\" bin=/usr/bin/node\n// @openshell provider ACME_KEY\n");
    write(&t.path().join("ext"), "acme.yaml", PROFILE);
    write(&repo, "profiles/acme.yaml", PROFILE);
    write(
        &repo,
        "openshell-annotate.toml",
        &format!("[providers]\nprofiles_dir = \"{profiles_dir}\"\n"),
    );
    (t, repo)
}

fn script(repo: &Path) -> String {
    let o = run(
        repo,
        &[
            "generate",
            "-o",
            "p.yaml",
            "--providers-out",
            "providers.sh",
            "--format",
            "compact",
        ],
    );
    assert!(o.status.success(), "{}", err(&o));
    std::fs::read_to_string(repo.join("providers.sh")).unwrap()
}

#[test]
fn profiles_outside_the_repo_can_be_relocated_with_an_env_var() {
    let (_t, repo) = setup("../ext");
    let s = script(&repo);
    assert!(
        s.contains("openshell profile import -f \"${OPENSHELL_PROFILES_DIR:-$ROOT/../ext}/acme.yaml\" --global"),
        "{s}"
    );
    assert!(
        s.contains("# Provider profiles outside this repo are read from $OPENSHELL_PROFILES_DIR"),
        "{s}"
    );
    // the script has no machine-specific absolute path of the temp dir
    assert!(!s.contains(repo.parent().unwrap().to_str().unwrap()), "{s}");
}

#[test]
fn absolute_profile_dirs_get_the_same_override() {
    let (t, repo) = setup("unused");
    let abs = t.path().join("ext");
    std::fs::write(
        repo.join("openshell-annotate.toml"),
        format!("[providers]\nprofiles_dir = \"{}\"\n", abs.display()),
    )
    .unwrap();
    let s = script(&repo);
    assert!(
        s.contains(&format!(
            "\"${{OPENSHELL_PROFILES_DIR:-{}}}/acme.yaml\"",
            abs.display()
        )),
        "{s}"
    );
}

#[test]
fn profiles_inside_the_repo_keep_the_root_relative_form() {
    let (_t, repo) = setup("profiles");
    let s = script(&repo);
    assert!(
        s.contains("openshell profile import -f \"$ROOT/profiles/acme.yaml\" --global"),
        "{s}"
    );
    assert!(!s.contains("OPENSHELL_PROFILES_DIR"), "{s}");
}
