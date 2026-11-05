//! Every committed example and golden policy must pass the OpenShell gateway validator.
use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            collect(&path, out);
        } else if matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some("policy.yaml" | "policy.expected.yaml")
        ) {
            out.push(path);
        }
    }
}

#[test]
fn example_and_golden_policies_pass_the_gateway_validator() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut policies = Vec::new();
    for dir in ["examples", "tests/fixtures", "tests/golden"] {
        let path = root.join(dir);
        if path.is_dir() {
            collect(&path, &mut policies);
        }
    }
    policies.sort();
    assert!(
        policies.len() >= 2,
        "expected the polyglot policy and the sample golden, found {policies:?}"
    );
    let mut failed = Vec::new();
    for path in &policies {
        let text = std::fs::read_to_string(path).unwrap();
        match openshell_annotate::validate::gateway_violations(&text) {
            Ok(vs) if vs.is_empty() => {}
            Ok(vs) => failed.push(format!(
                "{}: {}",
                path.strip_prefix(root).unwrap().display(),
                vs.join("; ")
            )),
            Err(e) => failed.push(format!(
                "{}: {e}",
                path.strip_prefix(root).unwrap().display()
            )),
        }
    }
    assert!(
        failed.is_empty(),
        "gateway validator rejected committed policies:\n{}",
        failed.join("\n")
    );
}
