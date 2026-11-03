//! Long `reason` text is kept in full in the generated policy comments.
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
fn long_reasons_are_wrapped_not_truncated() {
    let reason = "reqwest with rustls bundles its own root certificates and so rejects the OpenShell TLS termination CA, therefore the upstream is relayed without inspection and L7 rules cannot be used for this host until the client trusts the sandbox CA bundle";
    assert!(reason.len() > 200);
    let t = tempfile::tempdir().unwrap();
    write(
        t.path(),
        "src/up.ts",
        &format!("// @openshell egress openrouter.ai:443 reason=\"{reason}\" bin=/usr/bin/node tls=skip\n"),
    );
    let o = run(t.path(), &["generate"]);
    assert!(o.status.success(), "{}", err(&o));
    let y = out(&o);
    assert!(!y.contains("..."), "no truncation marker: {y}");
    let body = &y[y.find("network_policies:").expect("network_policies")..];
    let comment: Vec<&str> = body
        .lines()
        .filter(|l| l.trim_start().starts_with("# "))
        .collect();
    let joined = comment
        .iter()
        .map(|l| l.trim_start().trim_start_matches("# "))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(joined.contains(reason), "full reason present: {joined}");
    assert!(joined.contains("(src/up.ts:1)"), "{joined}");
    for l in &comment {
        assert!(l.len() <= 110, "line too long ({}): {l}", l.len());
    }
    // still a valid policy for the schema crate
    let p = t.path().join("p.yaml");
    std::fs::write(&p, &y).unwrap();
    let l = run(
        t.path(),
        &["lint", "--policy", "p.yaml", "--format", "compact"],
    );
    assert!(err(&l).contains("0 error(s)"), "{}", err(&l));
}
