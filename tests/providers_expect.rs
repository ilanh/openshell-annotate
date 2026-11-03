//! `[providers] expect`: credentials read through dynamic lookups are declared once in the config.
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

fn project(toml: &str) -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    write(
        t.path(),
        "src/a.ts",
        "const t = process.env.GITHUB_PERSONAL_ACCESS_TOKEN;\n// @openshell egress api.github.com:443 reason=\"r\" bin=/usr/bin/node\nfetch(\"https://api.github.com\");\n",
    );
    write(t.path(), "openshell-annotate.toml", toml);
    t
}

#[test]
fn lint_flags_the_credential_until_it_is_expected() {
    let t = project("");
    let o = run(t.path(), &["lint", "--format", "compact"]);
    assert!(
        out(&o).contains("warning[env]: reads secret-like variable GITHUB_PERSONAL_ACCESS_TOKEN"),
        "{}",
        out(&o)
    );
    let t = project("[providers]\nexpect = [\"GITHUB_PERSONAL_ACCESS_TOKEN\"]\n");
    let o = run(t.path(), &["lint", "--format", "compact"]);
    assert!(!out(&o).contains("warning[env]"), "{}", out(&o));
}

#[test]
fn expected_credentials_reach_the_providers_output() {
    let t = project(
        "[providers]\nexpect = [\"CONTEXT7_API_KEY\", \"GITHUB_PERSONAL_ACCESS_TOKEN\"]\n[providers.env]\nGITHUB_PERSONAL_ACCESS_TOKEN = \"github\"\n",
    );
    let o = run(
        t.path(),
        &[
            "generate",
            "--providers-out",
            "p.json",
            "--format",
            "compact",
        ],
    );
    assert!(o.status.success(), "{}", err(&o));
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(t.path().join("p.json")).unwrap()).unwrap();
    assert_eq!(
        v["providers"][0]["credentials"][0], "GITHUB_PERSONAL_ACCESS_TOKEN",
        "{v}"
    );
    assert_eq!(v["unmapped"][0]["env"], "CONTEXT7_API_KEY", "{v}");
    assert!(
        v["unmapped"][0]["sources"][0]
            .as_str()
            .unwrap()
            .contains("[providers].expect"),
        "{v}"
    );
}

#[test]
fn expect_entries_must_look_like_environment_variables() {
    let t = project("[providers]\nexpect = [\"github token\"]\n");
    let o = run(t.path(), &["scan"]);
    assert!(!o.status.success());
    assert!(
        err(&o).contains("providers.expect entry `github token`"),
        "{}",
        err(&o)
    );
}
