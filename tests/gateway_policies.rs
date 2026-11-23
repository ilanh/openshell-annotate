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

/// OpenShell v0.1.8 (ai-openshell/openshell#5336): `websocket_credential_rewrite: true` needs `protocol: websocket`.
/// `lint --policy`, `drift` and `generate` run the gateway validator, so they report it with the upstream hint.
#[test]
fn websocket_credential_rewrite_needs_the_websocket_protocol() {
    let policy = |protocol: &str| {
        format!(
            "version: 1\nnetwork_policies:\n  gw:\n    binaries:\n      - path: /usr/bin/node\n    endpoints:\n      - host: gateway.example.com\n        port: 443\n        protocol: {protocol}\n        access: full\n        websocket_credential_rewrite: true\n"
        )
    };
    let rest = openshell_annotate::validate::gateway_violations(&policy("rest")).unwrap();
    assert!(
        rest.iter().any(|v| v.contains("use protocol: websocket")),
        "{rest:?}"
    );
    let ws = openshell_annotate::validate::gateway_violations(&policy("websocket")).unwrap();
    assert!(ws.is_empty(), "{ws:?}");
}

/// OpenShell v0.1.9 (ai-openshell/openshell#4359): middleware endpoint selectors match hosts on every port, and a
/// selector with a port now fails validation. annotate does not generate `network_middlewares`, but `lint --policy`,
/// `drift` and `generate` run the gateway validator on committed policies, so they report it.
#[test]
fn middleware_selectors_cannot_include_a_port() {
    let policy = |include: &str| {
        format!(
            "version: 1\nnetwork_middlewares:\n  redact:\n    middleware: openshell/regex\n    order: 10\n    config:\n      mode: redact\n    on_error: fail_closed\n    endpoints:\n      include: [\"{include}\"]\nnetwork_policies:\n  api:\n    binaries:\n      - path: /usr/bin/curl\n    endpoints:\n      - host: api.example.com\n        port: 443\n"
        )
    };
    let with_port =
        openshell_annotate::validate::gateway_violations(&policy("api.example.com:443")).unwrap();
    assert!(
        with_port.iter().any(|v| v.contains("cannot include one")),
        "{with_port:?}"
    );
    let without =
        openshell_annotate::validate::gateway_violations(&policy("api.example.com")).unwrap();
    assert!(
        !without.iter().any(|v| v.contains("cannot include one")),
        "{without:?}"
    );
}
