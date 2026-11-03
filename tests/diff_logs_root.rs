//! diff-logs: finds the repo root from a subdirectory and keeps probe scripts out of suggestion placement.
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

const LOG: &str = "[t] [sandbox] [OCSF ] [ocsf] NET:OPEN [MED] DENIED /usr/bin/node(7) -> api.new.io:443 [policy:- engine:opa] [reason:transparent_tcp_policy_denied]\n";

fn project(toml: &str) -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    write(t.path(), "openshell-annotate.toml", toml);
    write(
        t.path(),
        "src/app.ts",
        "// the app talks to api.new.io\nfetch(\"https://api.new.io/x\");\n",
    );
    write(
        t.path(),
        "openshell/exercise.sh",
        "#!/bin/sh\ncurl https://api.new.io/x\n",
    );
    write(t.path(), "openshell/sandbox.log", LOG);
    t
}

fn json(o: &std::process::Output) -> serde_json::Value {
    serde_json::from_str(&out(o)).unwrap_or_else(|e| panic!("{e}: {} {}", out(o), err(o)))
}

#[test]
fn runs_from_a_subdirectory_using_the_repo_root() {
    let t = project("");
    let sub = t.path().join("openshell");
    let o = run(&sub, &["diff-logs", "sandbox.log", "--json"]);
    let v = json(&o);
    assert_eq!(v["suggestions"][0]["kind"], "missing_egress", "{v}");
    assert!(
        err(&o).contains("note: using repository root"),
        "{}",
        err(&o)
    );
    // an explicit --root other than `.` is taken literally and not second-guessed
    let o = run(
        &sub,
        &[
            "--root",
            sub.to_str().unwrap(),
            "diff-logs",
            "sandbox.log",
            "--json",
        ],
    );
    assert!(!err(&o).contains("using repository root"), "{}", err(&o));
}

#[test]
fn write_from_a_subdirectory_edits_the_real_sources() {
    let t = project("[diff-logs]\nexclude = [\"^openshell/exercise.sh\"]\n");
    let sub = t.path().join("openshell");
    run(&sub, &["diff-logs", "sandbox.log", "--write"]);
    let app = std::fs::read_to_string(t.path().join("src/app.ts")).unwrap();
    assert!(
        app.contains("// @openshell egress api.new.io:443"),
        "directive inserted into the source file: {app}"
    );
    assert!(
        !sub.join("openshell").exists(),
        "no stray openshell/ dir under the subdirectory"
    );
    let probe = std::fs::read_to_string(sub.join("exercise.sh")).unwrap();
    assert!(!probe.contains("@openshell"), "{probe}");
}

#[test]
fn probe_script_is_a_placement_target_only_until_excluded() {
    let t = project("");
    let o = run(t.path(), &["diff-logs", "openshell/sandbox.log", "--json"]);
    assert_eq!(
        json(&o)["suggestions"][0]["place"],
        "openshell/exercise.sh:2",
        "documents the problem the exclude solves"
    );
    let t = project("[diff-logs]\nexclude = [\"^openshell/\"]\n");
    let o = run(t.path(), &["diff-logs", "openshell/sandbox.log", "--json"]);
    assert_eq!(json(&o)["suggestions"][0]["place"], "src/app.ts:1");
}

#[test]
fn observed_file_is_never_a_placement_target() {
    let t = tempfile::tempdir().unwrap();
    write(t.path(), "openshell-annotate.toml", "");
    write(t.path(), "src/app.ts", "export const x = 1;\n");
    write(
        t.path(),
        "openshell/observed-egress.sh",
        "# notes: api.new.io was seen earlier\n",
    );
    write(t.path(), "openshell/sandbox.log", LOG);
    let o = run(t.path(), &["diff-logs", "openshell/sandbox.log", "--json"]);
    let v = json(&o);
    assert_eq!(v["suggestions"][0]["kind"], "missing_egress", "{v}");
    assert!(v["suggestions"][0]["place"].is_null(), "{v}");
}
