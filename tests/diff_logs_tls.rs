//! diff-logs: a connection that is ALLOWED and then fails (NET:FAIL) is reported as a TLS trust problem.
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

const LOG: &str = "\
[1793625398.702] [sandbox] [OCSF ] [ocsf] NET:OPEN [INFO] ALLOWED /usr/local/bin/anthropic-proxy(41) -> openrouter.ai:443 [policy:rule_openrouter_ai_443 engine:opa]
[1793625398.710] [sandbox] [OCSF ] [ocsf] NET:FAIL [LOW] 198.18.0.2:443
[1793625399.702] [sandbox] [OCSF ] [ocsf] NET:OPEN [INFO] ALLOWED /usr/local/bin/anthropic-proxy(41) -> openrouter.ai:443 [policy:rule_openrouter_ai_443 engine:opa]
[1793625399.710] [sandbox] [OCSF ] [ocsf] NET:FAIL [LOW] 198.18.0.2:443
[1793625400.000] [sandbox] [OCSF ] [ocsf] NET:OPEN [INFO] ALLOWED /usr/bin/curl(50) -> example.org:443 [policy:rule_example_org_443 engine:opa]
";

fn repo(tls_skip: bool) -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    write(
        t.path(),
        "src/up.ts",
        &format!(
            "// @openshell egress openrouter.ai:443 reason=\"upstream\" bin=/usr/local/bin/anthropic-proxy{}\n",
            if tls_skip { " tls=skip" } else { "" }
        ),
    );
    write(
        t.path(),
        "src/other.sh",
        "# @openshell egress example.org:443 reason=\"x\" bin=/usr/bin/curl\n",
    );
    write(t.path(), "sandbox.log", LOG);
    t
}

#[test]
fn allowed_then_failed_is_a_tls_failure_finding() {
    let t = repo(false);
    let o = run(t.path(), &["diff-logs", "sandbox.log", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["failed"], 2, "{v}");
    let s = v["suggestions"].as_array().unwrap();
    assert_eq!(s.len(), 1, "{v}");
    assert_eq!(s[0]["kind"], "tls_failure");
    assert_eq!(s[0]["host"], "openrouter.ai");
    assert_eq!(s[0]["count"], 2);
    assert_eq!(s[0]["binary"], "/usr/local/bin/anthropic-proxy");
    assert_eq!(s[0]["place"], "src/up.ts:1");
    assert!(s[0]["note"].as_str().unwrap().contains("tls=skip"));
    assert!(s[0]["directive"].is_null(), "nothing is auto-written");
    assert!(!o.status.success(), "a suggestion fails the run");

    let h = run(
        t.path(),
        &["diff-logs", "sandbox.log", "--color", "never", "--no-icons"],
    );
    let h = (out(&h) + &err(&h))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(h.contains("tls failure"), "{h}");
    assert!(
        h.contains("does not trust the OpenShell TLS-termination CA"),
        "{h}"
    );
    assert!(h.contains("src/up.ts:1"), "{h}");
}

#[test]
fn tls_skip_already_set_is_not_blamed_on_certificates() {
    let t = repo(true);
    let o = run(t.path(), &["diff-logs", "sandbox.log", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    let s = v["suggestions"].as_array().unwrap();
    assert_eq!(s.len(), 1, "{v}");
    assert_eq!(s[0]["kind"], "manual");
    assert!(s[0]["note"]
        .as_str()
        .unwrap()
        .contains("although tls=skip is set"));
}

#[test]
fn write_does_not_touch_files_for_tls_failures() {
    let t = repo(false);
    let before = std::fs::read_to_string(t.path().join("src/up.ts")).unwrap();
    run(t.path(), &["diff-logs", "sandbox.log", "--write"]);
    assert_eq!(
        std::fs::read_to_string(t.path().join("src/up.ts")).unwrap(),
        before
    );
}

#[test]
fn ocsf_json_fail_events_are_understood() {
    let t = tempfile::tempdir().unwrap();
    write(
        t.path(),
        "src/a.sh",
        "# @openshell egress api.x.com:443 reason=\"x\" bin=/usr/bin/curl\n",
    );
    write(
        t.path(),
        "l.jsonl",
        "{\"class_uid\":4001,\"action\":\"Allowed\",\"dst_endpoint\":{\"domain\":\"api.x.com\",\"port\":443},\"actor\":{\"process\":{\"name\":\"/usr/bin/curl\"}}}\n{\"class_uid\":4001,\"activity_id\":4,\"status_id\":2,\"dst_endpoint\":{\"ip\":\"198.18.0.3\",\"port\":443}}\n",
    );
    let o = run(t.path(), &["diff-logs", "l.jsonl", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["suggestions"][0]["kind"], "tls_failure", "{v}");
}
