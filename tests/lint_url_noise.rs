//! lint: URL literals in docstrings, help text, log lines and error messages are not network destinations.
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

fn lint_compact(t: &Path) -> String {
    let o = run(t, &["lint", "--format", "compact"]);
    out(&o) + &err(&o)
}

#[test]
fn python_text_for_people_is_skipped_but_real_destinations_are_not() {
    let t = tempfile::tempdir().unwrap();
    write(
        t.path(),
        "app.py",
        r#""""Module docstring, see https://docs.module.demo-site.io/guide."""
import argparse, logging, requests

_HELP_TEXT = """Usage: see https://help.var.demo-site.io/"""
logger = logging.getLogger(__name__)


def f(url):
    """Docstring with https://docs.func.demo-site.io/x"""
    print("migrate: https://print.demo-site.io/migrate")
    logger.warning("see https://log.demo-site.io/x")
    p = argparse.ArgumentParser(description="more at https://desc.demo-site.io/")
    p.add_argument("--x", help="docs https://arg.demo-site.io/help")
    msg = (
        "Check https://msg.demo-site.io/costs "
        "for details"
    )
    if not url:
        raise ValueError("see https://raise.demo-site.io/err")
    s = Client("https://api.sdk.demo-site.io/v1")
    return requests.get("https://api.real.demo-site.io/data"), s
"#,
    );
    let s = lint_compact(t.path());
    for noise in [
        "docs.module",
        "help.var",
        "docs.func",
        "print.demo-site",
        "log.demo-site",
        "desc.demo-site",
        "arg.demo-site",
        "msg.demo-site",
        "raise.demo-site",
    ] {
        assert!(!s.contains(noise), "{noise} should be skipped:\n{s}");
    }
    assert!(
        s.contains("api.real.demo-site.io"),
        "real call still flagged:\n{s}"
    );
    assert!(
        s.contains("api.sdk.demo-site.io"),
        "SDK constructor URL still reported:\n{s}"
    );
}

#[test]
fn typescript_and_go_messages_are_skipped() {
    let t = tempfile::tempdir().unwrap();
    write(
        t.path(),
        "a.ts",
        "console.log(\"docs https://console.msg.demo-site.io/\");\nfunction f() { throw new Error(\"see https://throw.msg.demo-site.io/\"); }\nconst usage = \"https://usage.msg.demo-site.io\";\nconst c = new Client(\"https://api.ts.demo-site.io\");\n",
    );
    write(
        t.path(),
        "m.go",
        "package main\n\nimport (\n\t\"errors\"\n\t\"fmt\"\n)\n\nfunc f() error {\n\tfmt.Println(\"https://go.print.demo-site.io/\")\n\t_ = errors.New(\"see https://go.errors.demo-site.io/\")\n\treturn fmt.Errorf(\"see https://go.errorf.demo-site.io/: %w\", nil)\n}\n",
    );
    let s = lint_compact(t.path());
    for noise in [
        "console.msg",
        "throw.msg",
        "usage.msg",
        "go.print",
        "go.errors",
        "go.errorf",
    ] {
        assert!(!s.contains(noise), "{noise} should be skipped:\n{s}");
    }
    assert!(s.contains("api.ts.demo-site.io"), "{s}");
}

#[test]
fn ignore_hosts_silences_listed_hosts_including_wildcards() {
    let t = tempfile::tempdir().unwrap();
    write(
        t.path(),
        "a.ts",
        "const a = new Client(\"https://one.docs.io/x\");\nconst b = new Client(\"https://keep.me.io/x\");\nconst c = new Client(\"https://exact.host.io/x\");\n",
    );
    let before = lint_compact(t.path());
    assert!(
        before.contains("one.docs.io") && before.contains("exact.host.io"),
        "{before}"
    );
    write(
        t.path(),
        "openshell-annotate.toml",
        "[lint]\nignore_hosts = [\"*.docs.io\", \"exact.host.io\"]\n",
    );
    let after = lint_compact(t.path());
    assert!(
        !after.contains("one.docs.io") && !after.contains("exact.host.io"),
        "{after}"
    );
    assert!(after.contains("keep.me.io"), "{after}");
}
