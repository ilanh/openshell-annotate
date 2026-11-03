//! Generic `#` and `//` comment leaders for languages without first-class support.
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

#[test]
fn ruby_and_rust_inline_directives_are_scanned_by_default() {
    let t = tempfile::tempdir().unwrap();
    write(
        t.path(),
        "lib/client.rb",
        "# @openshell egress api.x.com:443 reason=\"ruby http\" bin=/usr/bin/ruby\nNet::HTTP.get(URI(\"https://api.x.com\"))\n",
    );
    write(
        t.path(),
        "src/main.rs",
        "// @openshell egress api.y.com:443 reason=\"rust http\" bin=/usr/local/bin/proxy\nfn main() { let s = \"// @openshell egress not.a.directive:443\"; }\n",
    );
    let o = run(t.path(), &["scan", "--format", "compact"]);
    assert!(o.status.success(), "{}", err(&o));
    let s = out(&o);
    assert!(s.contains("lib/client.rb:1  egress api.x.com:443"), "{s}");
    assert!(s.contains("src/main.rs:1  egress api.y.com:443"), "{s}");
    assert!(!s.contains("not.a.directive"), "{s}");
    let g = run(t.path(), &["generate", "."]);
    assert!(g.status.success(), "{}", err(&g));
    let p = out(&g);
    assert!(
        p.contains("api.x.com") && p.contains("/usr/bin/ruby"),
        "{p}"
    );
    assert!(p.contains("api.y.com"), "{p}");
}

#[test]
fn comment_leaders_config_adds_and_disables_extensions() {
    let t = tempfile::tempdir().unwrap();
    write(
        t.path(),
        "a.lua",
        "-- not a leader we know\n# @openshell egress api.lua.com:443 reason=\"r\"\n",
    );
    write(
        t.path(),
        "b.rb",
        "# @openshell egress api.rb.com:443 reason=\"r\"\n",
    );
    write(
        t.path(),
        "openshell-annotate.toml",
        "[scan.comment_leaders]\nlua = \"hash\"\nrb = \"none\"\n",
    );
    let o = run(t.path(), &["scan", "--format", "compact"]);
    assert!(o.status.success(), "{}", err(&o));
    let s = out(&o);
    assert!(s.contains("a.lua:2  egress api.lua.com:443"), "{s}");
    assert!(!s.contains("api.rb.com"), "rb switched off: {s}");
    assert!(
        err(&o).contains("1 .rb file was not scanned"),
        "{}",
        err(&o)
    );
}

#[test]
fn unscanned_languages_are_reported_and_bad_leader_is_an_error() {
    let t = tempfile::tempdir().unwrap();
    for i in 0..3 {
        write(t.path(), &format!("src/m{i}.lua"), "print('hi')\n");
    }
    let o = run(t.path(), &["scan", "--format", "compact"]);
    assert!(o.status.success(), "{}", err(&o));
    let e = err(&o);
    assert!(e.contains("note: 3 .lua files were not scanned"), "{e}");
    assert!(e.contains("[scan.comment_leaders] lua"), "{e}");
    // stdout stays machine-readable
    let j = run(t.path(), &["scan", "--json"]);
    serde_json::from_str::<serde_json::Value>(&out(&j)).unwrap();

    write(
        t.path(),
        "openshell-annotate.toml",
        "[scan.comment_leaders]\nlua = \"semicolon\"\n",
    );
    let o = run(t.path(), &["scan"]);
    assert!(!o.status.success());
    assert!(
        err(&o).contains("must be `hash`, `slash` or `none`"),
        "{}",
        err(&o)
    );
}
