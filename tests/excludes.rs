//! `exclude` semantics: shared by scan and lint, literal (not regex), dead patterns reported, `./` paths normalised.
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
        "packages/cli/a.ts",
        "fetch(\"https://a.example.com\");\n",
    );
    write(
        t.path(),
        "packages/admin/b.ts",
        "fetch(\"https://b.example.com\");\n",
    );
    write(t.path(), "src/c.ts", "export const x = 1;\n");
    write(t.path(), "openshell-annotate.toml", toml);
    t
}

#[test]
fn scan_exclude_applies_to_lint_and_anchors_work_with_dot_paths() {
    let t = project("[scan]\nexclude = [\"^packages/cli/\"]\n");
    for args in [
        vec!["lint", "--format", "compact"],
        vec!["lint", "--format", "compact", "."],
    ] {
        let o = run(t.path(), &args);
        let s = out(&o) + &err(&o);
        assert!(!s.contains("packages/cli"), "{args:?}: {s}");
        assert!(s.contains("packages/admin/b.ts"), "{args:?}: {s}");
        assert!(!s.contains("matches no file"), "{args:?}: {s}");
    }
}

#[test]
fn source_paths_have_no_dot_slash_prefix() {
    let t = project("");
    write(
        t.path(),
        "src/d.ts",
        "// @openshell egress api.x.com:443 reason=\"r\"\n",
    );
    let o = run(t.path(), &["scan", "--format", "compact", "."]);
    let s = out(&o);
    assert!(s.contains("src/d.ts:1"), "{s}");
    assert!(!s.contains("./src"), "{s}");
}

#[test]
fn regex_exclude_is_reported_as_matching_nothing() {
    let t = project("[scan]\nexclude = [\"^packages/(admin|cli)/\"]\n");
    let o = run(t.path(), &["lint", "--format", "compact"]);
    let e = err(&o);
    assert!(
        e.contains("[scan].exclude pattern `^packages/(admin|cli)/` matches no file"),
        "{e}"
    );
    assert!(e.contains("not regexes"), "{e}");
    // and the files are, as before, still scanned
    assert!(out(&o).contains("packages/cli/a.ts"), "{}", out(&o));
}

#[test]
fn dead_lint_exclude_and_typo_are_reported_but_not_for_partial_scans() {
    let t = project("[lint]\nexclude = [\"packages/clii/\"]\n");
    let o = run(t.path(), &["lint", "--format", "compact"]);
    let e = err(&o);
    assert!(
        e.contains("[lint].exclude pattern `packages/clii/` matches no file"),
        "{e}"
    );
    assert!(e.contains("check the spelling"), "{e}");
    // a partial scan cannot know, so it stays quiet
    let o = run(t.path(), &["lint", "--format", "compact", "src"]);
    assert!(!err(&o).contains("matches no file"), "{}", err(&o));
    // JSON stdout is untouched
    let o = run(t.path(), &["scan", "--json"]);
    serde_json::from_str::<serde_json::Value>(&out(&o)).unwrap();
}
