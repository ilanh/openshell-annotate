//! End-to-end tests: drive the real binary against the fixture repo in a temp dir.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_openshell-annotate"))
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let to = dst.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &to);
        } else {
            std::fs::copy(e.path(), to).unwrap();
        }
    }
}

fn fixture() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample"),
        t.path(),
    );
    // The golden policy names the tool version in its header; keep it independent of Cargo.toml.
    let golden = t.path().join("policy.expected.yaml");
    let text = std::fs::read_to_string(&golden).unwrap();
    std::fs::write(
        &golden,
        text.replace("@VERSION@", env!("CARGO_PKG_VERSION")),
    )
    .unwrap();
    t
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

#[test]
fn scan_lists_directives_with_locations_and_ignores_strings() {
    let t = fixture();
    let o = run(t.path(), &["scan", "--format", "compact"]);
    assert!(o.status.success(), "{}", err(&o));
    let s = out(&o);
    assert!(s.contains("src/llm.ts:1  egress api.openai.com:443"), "{s}");
    assert!(s.contains("src/llm.ts:2  provider OPENAI_API_KEY"), "{s}");
    assert!(
        s.contains("src/cache.py:2  fs write /workspace/data"),
        "{s}"
    );
    assert!(
        s.contains("cmd/main.go:3  egress api.github.com:443"),
        "{s}"
    );
    assert!(
        s.contains("scripts/run.sh:3  fs read /etc/ssl/certs"),
        "{s}"
    );
    assert!(
        !s.contains("not-a-directive.test"),
        "directive inside a string literal must be ignored: {s}"
    );
    assert_eq!(s.lines().count(), 8);
}

#[test]
fn scan_shows_process_identity_and_ignore_kind() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(
        t.path().join("a.js"),
        "// @openshell process user=sandbox group=sandbox\n\
         // @openshell process\n\
         // @openshell ignore kind=net reason=\"runtime host\" scope=file\n\
         fetch(x)\n",
    )
    .unwrap();
    let o = run(t.path(), &["scan", "--format", "compact"]);
    assert!(o.status.success(), "{}", err(&o));
    let lines: Vec<String> = out(&o).lines().map(str::to_string).collect();
    assert_eq!(
        lines,
        [
            "a.js:1  process user=sandbox group=sandbox",
            "a.js:2  process",
            "a.js:3  ignore kind=net scope=file  # runtime host",
        ],
        "{}",
        out(&o)
    );
    let o = run(t.path(), &["scan", "--color", "never"]);
    assert!(o.status.success(), "{}", err(&o));
    let s = out(&o);
    assert!(s.contains("process user=sandbox group=sandbox\n"), "{s}");
    assert!(
        s.contains("ignore kind=net scope=file  # runtime host"),
        "{s}"
    );
}

#[test]
fn scan_json() {
    let t = fixture();
    let o = run(t.path(), &["scan", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["directives"].as_array().unwrap().len(), 8);
    assert_eq!(v["directives"][0]["file"], "cmd/main.go");
    assert_eq!(v["directives"][0]["kind"], "egress");
}

#[test]
fn generate_matches_golden_and_validates_with_schema_crate() {
    let t = fixture();
    let o = run(
        t.path(),
        &[
            "generate",
            "-o",
            "policy.yaml",
            "--providers-out",
            "providers.json",
        ],
    );
    assert!(o.status.success(), "{}", err(&o));
    let got = std::fs::read_to_string(t.path().join("policy.yaml")).unwrap();
    let want = std::fs::read_to_string(t.path().join("policy.expected.yaml")).unwrap();
    assert_eq!(got, want);
    let doc =
        openshell_policy_schema::parse_policy(&got).expect("schema crate accepts generated policy");
    assert_eq!(doc.version, 1);
    assert_eq!(doc.network_policies.len(), 4);
    let gh = &doc.network_policies["api_github_com"];
    assert_eq!(gh.endpoints[0].host, "api.github.com");
    assert_eq!(gh.endpoints[0].access, "read-only");
    assert_eq!(gh.endpoints[0].enforcement, "enforce");
    assert_eq!(gh.binaries[0].path, "/opt/app/server");
    let fs = doc.filesystem_policy.unwrap();
    assert!(fs.include_workdir);
    assert_eq!(fs.read_write, vec!["/workspace/data"]);
    // Each endpoint carries a provenance comment with file:line.
    assert!(got.contains("# LLM calls (src/llm.ts:1)"));
    assert!(got.contains("# release lookup (second call site) (cmd/main.go:4)"));
    let prov = std::fs::read_to_string(t.path().join("providers.json")).unwrap();
    assert_eq!(
        prov,
        std::fs::read_to_string(t.path().join("providers.expected.json")).unwrap()
    );
}

#[test]
fn providers_script_has_no_secrets_and_uses_bare_credential_form() {
    let t = fixture();
    let o = run(
        t.path(),
        &[
            "generate",
            "-o",
            "p.yaml",
            "--providers-out",
            "providers.sh",
        ],
    );
    assert!(o.status.success(), "{}", err(&o));
    let s = std::fs::read_to_string(t.path().join("providers.sh")).unwrap();
    assert!(
        s.contains(
            "openshell provider create --name openai --type openai --credential OPENAI_API_KEY\n"
        ),
        "{s}"
    );
    assert!(!s.contains("OPENAI_API_KEY="), "{s}");
    assert!(
        s.contains("openshell sandbox create --provider openai --policy p.yaml"),
        "{s}"
    );
}

#[test]
fn drift_clean_then_detects_missing_extra_and_changed() {
    let t = fixture();
    assert!(run(t.path(), &["generate", "-o", "policy.yaml"])
        .status
        .success());
    let o = run(t.path(), &["drift", "--policy", "policy.yaml"]);
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    assert!(out(&o).contains("no drift"));

    // Comment/format-only edits are not drift.
    let y = std::fs::read_to_string(t.path().join("policy.yaml")).unwrap();
    std::fs::write(
        t.path().join("policy.yaml"),
        format!(
            "# hand-added comment\n{}",
            y.replace("\"api.github.com\"", "api.github.com")
        ),
    )
    .unwrap();
    assert!(run(t.path(), &["drift", "--policy", "policy.yaml"])
        .status
        .success());

    // A new directive that the committed policy lacks -> missing (live).
    std::fs::write(
        t.path().join("src/new.ts"),
        "// @openshell egress api.example.com:443 reason=\"new\"\n",
    )
    .unwrap();
    let o = run(t.path(), &["drift", "--policy", "policy.yaml"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        out(&o).contains("missing  [live] network_policies api_example_com"),
        "{}",
        out(&o)
    );
    std::fs::remove_file(t.path().join("src/new.ts")).unwrap();

    // A grant with no directive -> extra.
    let y = std::fs::read_to_string(t.path().join("policy.yaml")).unwrap();
    std::fs::write(t.path().join("policy.yaml"), format!("{y}  sneaky:\n    endpoints:\n      - host: evil.test\n        port: 443\n    binaries:\n      - path: /usr/bin/curl\n")).unwrap();
    let o = run(t.path(), &["drift", "--policy", "policy.yaml"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        out(&o).contains("extra    [live] network_policies sneaky"),
        "{}",
        out(&o)
    );

    // Changed binary -> changed.
    assert!(run(t.path(), &["generate", "-o", "policy.yaml"])
        .status
        .success());
    let y = std::fs::read_to_string(t.path().join("policy.yaml")).unwrap();
    std::fs::write(
        t.path().join("policy.yaml"),
        y.replace("/opt/app/server", "/opt/app/other"),
    )
    .unwrap();
    let o = run(t.path(), &["drift", "--policy", "policy.yaml"]);
    assert_eq!(o.status.code(), Some(1));
    let s = out(&o);
    assert!(
        s.contains("changed")
            && s.contains("binaries")
            && s.contains("/opt/app/server")
            && s.contains("/opt/app/other"),
        "{s}"
    );
}

#[test]
fn drift_labels_startup_sections() {
    let t = fixture();
    assert!(run(t.path(), &["generate", "-o", "policy.yaml"])
        .status
        .success());
    let y = std::fs::read_to_string(t.path().join("policy.yaml")).unwrap();
    std::fs::write(
        t.path().join("policy.yaml"),
        y.replace(
            "    - \"/workspace/data\"\n",
            "    - \"/workspace/data\"\n    - \"/var/extra\"\n",
        ),
    )
    .unwrap();
    let o = run(t.path(), &["drift", "--policy", "policy.yaml"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        out(&o).contains("[startup] filesystem_policy.read_write /var/extra"),
        "{}",
        out(&o)
    );
}

#[test]
fn drift_unmanaged_sections_noted_and_strict_flags_them() {
    let t = fixture();
    assert!(run(t.path(), &["generate", "-o", "policy.yaml"])
        .status
        .success());
    let y = std::fs::read_to_string(t.path().join("policy.yaml")).unwrap();
    std::fs::write(
        t.path().join("policy.yaml"),
        y.replace(
            "version: 1\n",
            "version: 1\nlandlock:\n  compatibility: hard_requirement\n",
        ),
    )
    .unwrap();
    let o = run(t.path(), &["drift", "--policy", "policy.yaml"]);
    assert!(o.status.success());
    assert!(out(&o).contains("landlock"));
    let o = run(t.path(), &["drift", "--policy", "policy.yaml", "--strict"]);
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn drift_strict_labels_network_middlewares_live_and_landlock_startup() {
    let t = fixture();
    assert!(run(t.path(), &["generate", "-o", "policy.yaml"])
        .status
        .success());
    let mut y = std::fs::read_to_string(t.path().join("policy.yaml"))
        .unwrap()
        .replace(
            "version: 1\n",
            "version: 1\nlandlock:\n  compatibility: hard_requirement\n",
        );
    y.push_str(
        "network_middlewares:\n  redact:\n    name: redact\n    middleware: openshell/regex\n    order: 10\n    \
         config: {}\n    on_error: fail_closed\n    endpoints:\n      include: [\"api.openai.com\"]\n",
    );
    std::fs::write(t.path().join("policy.yaml"), y).unwrap();
    let o = run(t.path(), &["drift", "--policy", "policy.yaml", "--strict"]);
    assert_eq!(o.status.code(), Some(1), "{}{}", out(&o), err(&o));
    let s = out(&o);
    let line = |section: &str| {
        s.lines()
            .find(|l| l.contains(&format!("] {section} ")))
            .unwrap_or_else(|| panic!("no finding for {section}: {s}"))
            .to_string()
    };
    assert!(line("landlock").contains("[startup]"), "{s}");
    assert!(line("network_middlewares").contains("[live]"), "{s}");
}

#[test]
fn generate_rejects_invalid_directives_with_locations() {
    let t = fixture();
    std::fs::write(
        t.path().join("src/bad.ts"),
        "// @openshell egress api.bad.test:443\n// @openshell egress localhost:8080 reason=x\n// @openshell fs write relative/path reason=x\n// @openshell egress *.com:443 reason=x\n// @openshell egress ok.test:443 reason=x access=full allow=\"GET:/x\"\n",
    )
    .unwrap();
    let o = run(t.path(), &["generate"]);
    assert_eq!(o.status.code(), Some(1));
    let e = err(&o);
    assert!(e.contains("src/bad.ts:1: egress requires reason"), "{e}");
    assert!(e.contains("src/bad.ts:2:") && e.contains("loopback"), "{e}");
    assert!(e.contains("src/bad.ts:3:") && e.contains("absolute"), "{e}");
    assert!(
        e.contains("src/bad.ts:4:") && e.contains("three DNS labels"),
        "{e}"
    );
    assert!(
        e.contains("src/bad.ts:5:") && e.contains("mutually exclusive"),
        "{e}"
    );
}

#[test]
fn generate_mcp_emits_allow_all_known_methods_instead_of_an_access_preset() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(
        t.path().join("mcp.js"),
        "// @openshell egress mcp.example.com:443 reason=\"sample MCP server\" protocol=mcp access=read-write bin=/usr/bin/java\n\
         // @openshell egress api.example.com:443 reason=\"rest still uses a preset\" access=read-only bin=/usr/bin/curl\n\
         // @openshell egress tools.example.com:443 reason=\"one MCP method\" protocol=mcp allow=\"POST:/rpc\" bin=/usr/bin/java\n",
    )
    .unwrap();
    let o = run(t.path(), &["generate", "-o", "p.yaml"]);
    assert!(o.status.success(), "{}", err(&o));
    let y = std::fs::read_to_string(t.path().join("p.yaml")).unwrap();
    let mcp = endpoint_block(&y, "mcp.example.com");
    assert!(mcp.contains("protocol: mcp"), "{mcp}");
    assert!(mcp.contains("allow_all_known_mcp_methods: true"), "{mcp}");
    assert!(!mcp.contains("access:"), "{mcp}");
    let rest = endpoint_block(&y, "api.example.com");
    assert!(rest.contains("access: read-only"), "{rest}");
    assert!(!rest.contains("allow_all_known_mcp_methods"), "{rest}");
    let ruled = endpoint_block(&y, "tools.example.com");
    assert!(ruled.contains("protocol: mcp"), "{ruled}");
    assert!(ruled.contains("method: \"POST\""), "{ruled}");
    assert!(!ruled.contains("access:"), "{ruled}");
    assert!(!ruled.contains("allow_all_known_mcp_methods"), "{ruled}");
    let doc = openshell_policy_schema::parse_policy(&y).unwrap();
    let ep = &doc.network_policies["mcp_example_com"].endpoints[0];
    assert!(ep.access.is_empty(), "{ep:?}");
    assert_eq!(
        ep.mcp.as_ref().and_then(|m| m.allow_all_known_mcp_methods),
        Some(true)
    );
}

#[test]
fn generate_rejects_mcp_with_neither_rules_nor_access() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(
        t.path().join("mcp.js"),
        "// @openshell egress bare.example.com:443 reason=\"mcp needs a grant\" protocol=mcp bin=/usr/bin/java\n",
    )
    .unwrap();
    let o = run(t.path(), &["generate"]);
    assert_eq!(o.status.code(), Some(1), "{}", out(&o));
    let e = err(&o);
    assert!(
        e.contains("mcp.js:1:") && e.contains("mcp.allow_all_known_mcp_methods"),
        "{e}"
    );
}

#[test]
fn generate_emits_operation_type_only_for_graphql() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(
        t.path().join("api.js"),
        "// @openshell egress repo.packagist.org:443 reason=\"package metadata\" protocol=graphql allow=\"query\" deny=\"mutation:DeletePackage\" bin=/usr/bin/php\n\
         // @openshell egress api.example.com:443 reason=\"rest\" allow=\"POST:/v1\" bin=/usr/bin/curl\n\
         // @openshell egress rpc.example.net:443 reason=\"rpc\" protocol=json-rpc allow=\"POST:/\" bin=/usr/bin/curl\n",
    )
    .unwrap();
    let o = run(t.path(), &["generate", "-o", "p.yaml"]);
    assert!(o.status.success(), "{}", err(&o));
    let y = std::fs::read_to_string(t.path().join("p.yaml")).unwrap();
    let gql = endpoint_block(&y, "repo.packagist.org");
    let rules = gql.split("binaries:").next().unwrap();
    assert!(rules.contains("protocol: graphql"), "{rules}");
    assert!(rules.contains("operation_type: \"query\""), "{rules}");
    assert!(rules.contains("operation_type: \"mutation\""), "{rules}");
    assert!(
        rules.contains("operation_name: \"DeletePackage\""),
        "{rules}"
    );
    assert!(!rules.contains("method:"), "{rules}");
    assert!(!rules.contains("path:"), "{rules}");
    for host in ["api.example.com", "rpc.example.net"] {
        let ep = endpoint_block(&y, host);
        assert!(ep.contains("method: \"POST\""), "{ep}");
        assert!(
            !ep.contains("operation_type"),
            "operation_type is GraphQL-only: {ep}"
        );
    }
}

#[test]
fn generate_rejects_http_method_as_graphql_operation_type() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(
        t.path().join("api.js"),
        "// @openshell egress repo.packagist.org:443 reason=\"package metadata\" protocol=graphql allow=\"POST:/graphql\" bin=/usr/bin/php\n",
    )
    .unwrap();
    let o = run(t.path(), &["generate"]);
    assert_eq!(o.status.code(), Some(1), "{}", out(&o));
    let e = err(&o);
    assert!(
        e.contains("api.js:1:")
            && e.contains("operation_type must be query, mutation, or subscription"),
        "{e}"
    );
}

#[test]
fn l7_protocol_without_rules_or_access_is_a_directive_error() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(
        t.path().join("stream.dart"),
        "// @openshell egress stream.example.com:443 reason=\"live event stream\" protocol=websocket\n",
    )
    .unwrap();
    std::fs::write(
        t.path().join("api.js"),
        "// @openshell egress api.example.com:443 reason=\"rest\" protocol=rest\n\
         // @openshell egress gql.example.com:443 reason=\"graphql\" protocol=graphql\n\
         // @openshell egress db.internal.example:5432 reason=\"tcp\" protocol=tcp bin=/usr/bin/psql\n\
         // @openshell egress ok.example.com:443 reason=\"websocket reads\" protocol=websocket access=read-only bin=/usr/bin/dart\n",
    )
    .unwrap();
    let o = run(t.path(), &["generate"]);
    assert_eq!(o.status.code(), Some(1), "{}", out(&o));
    let e = err(&o);
    for (loc, proto) in [
        ("stream.dart:1:", "websocket"),
        ("api.js:1:", "rest"),
        ("api.js:2:", "graphql"),
    ] {
        assert!(
            e.contains(loc) && e.contains(&format!("protocol {proto} requires allow=")),
            "{e}"
        );
    }
    assert!(
        !e.contains("protocol tcp requires"),
        "tcp does not need rules or access: {e}"
    );
    let lint = run(t.path(), &["lint", "--format", "compact"]);
    let s = out(&lint);
    assert!(
        s.contains("stream.dart:1: error[directive]: protocol websocket requires allow="),
        "{s}"
    );
    std::fs::write(
        t.path().join("stream.dart"),
        "// @openshell egress stream.example.com:443 reason=\"live event stream\" protocol=websocket access=read-only bin=/usr/bin/dart\n",
    )
    .unwrap();
    std::fs::write(
        t.path().join("api.js"),
        "// @openshell egress db.internal.example:5432 reason=\"tcp\" protocol=tcp bin=/usr/bin/psql\n",
    )
    .unwrap();
    let o = run(t.path(), &["generate", "-o", "p.yaml"]);
    assert!(o.status.success(), "{}", err(&o));
    let y = std::fs::read_to_string(t.path().join("p.yaml")).unwrap();
    let stream = endpoint_block(&y, "stream.example.com");
    assert!(stream.contains("protocol: websocket"), "{stream}");
    assert!(stream.contains("access: read-only"), "{stream}");
    let tcp = endpoint_block(&y, "db.internal.example");
    assert!(tcp.contains("protocol: tcp"), "{tcp}");
    assert!(!tcp.contains("access:"), "{tcp}");
    assert!(!tcp.contains("rules:"), "{tcp}");
}

#[test]
fn generate_rejects_a_policy_the_gateway_rejects() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(
        t.path().join("rpc.js"),
        "// @openshell egress rpc.example.net:443 reason=\"rpc\" protocol=json-rpc access=full bin=/usr/bin/curl\n",
    )
    .unwrap();
    let o = run(t.path(), &["generate"]);
    assert_eq!(o.status.code(), Some(1), "{}", out(&o));
    let e = err(&o);
    assert!(
        e.contains("gateway validation") && e.contains("does not support access presets"),
        "{e}"
    );
}

#[test]
fn lint_policy_reports_gateway_violations_schema_accepts() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("a.js"), "console.log(1)\n").unwrap();
    std::fs::write(
        t.path().join("bad.yaml"),
        "version: 1\nnetwork_policies:\n  mcp_example_com:\n    endpoints:\n      - host: mcp.example.com\n        port: 443\n        protocol: mcp\n        access: read-write\n    binaries:\n      - path: /usr/bin/java\n",
    )
    .unwrap();
    let o = run(
        t.path(),
        &["lint", "--format", "compact", "--policy", "bad.yaml"],
    );
    assert_eq!(o.status.code(), Some(1), "{}", out(&o));
    let s = out(&o);
    assert!(
        s.contains("does not support access presets"),
        "schema-only lint would accept this file: {s}"
    );
    assert!(openshell_policy_schema::parse_policy(
        &std::fs::read_to_string(t.path().join("bad.yaml")).unwrap()
    )
    .is_ok());
}

#[test]
fn drift_rejects_a_committed_policy_the_gateway_rejects() {
    let t = fixture();
    assert!(run(t.path(), &["generate", "-o", "policy.yaml"])
        .status
        .success());
    let y = std::fs::read_to_string(t.path().join("policy.yaml")).unwrap();
    let y = y.replace(
        "protocol: rest\n        enforcement: enforce\n        access: read-write\n",
        "protocol: mcp\n        enforcement: enforce\n        access: read-write\n",
    );
    std::fs::write(t.path().join("policy.yaml"), y).unwrap();
    let o = run(t.path(), &["drift", "--policy", "policy.yaml"]);
    assert_eq!(o.status.code(), Some(1), "{}{}", out(&o), err(&o));
    assert!(
        err(&o).contains("does not support access presets"),
        "{}",
        err(&o)
    );
}

fn endpoint_block(policy: &str, host: &str) -> String {
    let marker = format!("- host: \"{host}\"");
    let at = policy
        .find(&marker)
        .unwrap_or_else(|| panic!("no endpoint for {host}: {policy}"));
    let ep = &policy[at..];
    let next = ep[1..].find("\n      - host:").map_or(ep.len(), |i| i + 1);
    ep[..next].to_string()
}

#[test]
fn generate_requires_a_binary_when_unknown() {
    let t = fixture();
    std::fs::remove_file(t.path().join("openshell-annotate.toml")).unwrap();
    let o = run(t.path(), &["generate"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("[languages].javascript"), "{}", err(&o));
}

#[test]
fn overlapping_port_lists_for_one_host_become_one_endpoint() {
    let t = fixture();
    std::fs::write(
        t.path().join("src/nuget.ts"),
        "// @openshell egress api.nuget.test:443,8443 reason=\"NuGet v3\" access=read-only\n\
         // @openshell egress api.nuget.test:443 reason=\"NuGet again\" access=read-only\n",
    )
    .unwrap();
    let o = run(t.path(), &["generate", "-o", "p.yaml"]);
    assert!(o.status.success(), "{}", err(&o));
    let y = std::fs::read_to_string(t.path().join("p.yaml")).unwrap();
    assert_eq!(
        y.matches("- host: \"api.nuget.test\"").count(),
        1,
        "one endpoint for the host: {y}"
    );
    let at = y.find("- host: \"api.nuget.test\"").unwrap();
    let ep = &y[at..];
    let next = ep[1..].find("- host:").map_or(ep.len(), |i| i + 1);
    let ep = &ep[..next];
    assert!(ep.contains("ports: [443, 8443]"), "{ep}");
    assert!(!ep.contains("port: 443\n"), "{ep}");
    // both reasons stay above the merged endpoint
    let before = &y[..at];
    assert!(before.contains("NuGet v3 (src/nuget.ts:1)"), "{y}");
    assert!(before.contains("NuGet again (src/nuget.ts:2)"), "{y}");
}

#[test]
fn overlapping_port_lists_with_different_settings_conflict() {
    let t = fixture();
    std::fs::write(
        t.path().join("src/dup.ts"),
        "// @openshell egress dup.test:443,8443 reason=a access=read-only\n// @openshell egress dup.test:443 reason=b access=full\n",
    )
    .unwrap();
    let o = run(t.path(), &["generate"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        err(&o).contains("conflicting settings for dup.test:443 "),
        "{}",
        err(&o)
    );
}

#[test]
fn conflicting_settings_and_slug_collisions_are_errors() {
    let t = fixture();
    std::fs::write(t.path().join("src/c1.ts"), "// @openshell egress dup.test:443 reason=a access=read-only\n// @openshell egress dup.test:443 reason=b access=full\n").unwrap();
    let o = run(t.path(), &["generate"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        err(&o).contains("conflicting settings for dup.test:443"),
        "{}",
        err(&o)
    );
    std::fs::remove_file(t.path().join("src/c1.ts")).unwrap();
    std::fs::write(
        t.path().join("src/c2.ts"),
        "// @openshell egress a-b.test:443 reason=a\n// @openshell egress a.b.test:443 reason=b\n",
    )
    .unwrap();
    let o = run(t.path(), &["generate"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("rule=<name>"), "{}", err(&o));
}

#[test]
fn rule_attribute_groups_endpoints_and_unions_binaries() {
    let t = fixture();
    std::fs::write(
        t.path().join("src/g.ts"),
        "// @openshell egress a.example.com:443 reason=a rule=pair\n// @openshell egress b.example.com:443,8443 reason=b rule=pair bin=/usr/bin/curl\n",
    )
    .unwrap();
    let o = run(t.path(), &["generate", "-o", "p.yaml"]);
    assert!(o.status.success(), "{}", err(&o));
    let doc = openshell_policy_schema::parse_policy(
        &std::fs::read_to_string(t.path().join("p.yaml")).unwrap(),
    )
    .unwrap();
    let r = &doc.network_policies["pair"];
    assert_eq!(r.endpoints.len(), 2);
    assert_eq!(r.endpoints[1].ports, vec![443, 8443]);
    let bins: Vec<_> = r.binaries.iter().map(|b| b.path.as_str()).collect();
    assert_eq!(bins, vec!["/usr/bin/curl", "/usr/local/bin/node"]);
}

#[test]
fn lint_flags_unannotated_calls_and_honours_ignore() {
    let t = fixture();
    std::fs::write(
        t.path().join("src/raw.ts"),
        "import { writeFileSync } from \"node:fs\";\nexport async function f() {\n  const r = await fetch(\"https://api.stripe.com/v1/charges\");\n  writeFileSync(\"/srv/x\", \"a\");\n  const k = process.env.STRIPE_API_KEY;\n  const m = \"fetch(not code)\"; // fetch(also not code)\n  writeFileSync(\"/tmp/y\", \"baseline\");\n  obj.fetch(\"relative\");\n}\n",
    )
    .unwrap();
    let o = run(t.path(), &["lint", "--format", "compact"]);
    assert!(o.status.success());
    let s = out(&o);
    assert!(s.contains("src/raw.ts:3: warning[net]: fetch() without an @openshell directive (suggest: @openshell egress api.stripe.com:443"), "{s}");
    assert!(
        s.contains("src/raw.ts:4: warning[fs]: filesystem write"),
        "{s}"
    );
    assert!(
        s.contains("src/raw.ts:5: warning[env]: reads secret-like variable STRIPE_API_KEY"),
        "{s}"
    );
    assert!(
        !s.contains("src/raw.ts:6"),
        "strings/comments must not match: {s}"
    );
    assert!(
        !s.contains("src/raw.ts:7"),
        "/tmp is covered by the sandbox baseline: {s}"
    );
    assert!(
        !s.contains("src/raw.ts:8"),
        "obj.fetch is a method, not the global fetch: {s}"
    );
    assert!(
        s.contains("outside the sandbox baseline (/tmp) and every declared fs write root"),
        "{s}"
    );
    assert_eq!(
        run(t.path(), &["lint", "--deny-warnings"]).status.code(),
        Some(1)
    );

    // Annotate -> clean.
    std::fs::write(
        t.path().join("src/raw.ts"),
        "// @openshell ignore kind=fs reason=\"tmp scratch\" scope=file\n// @openshell egress api.stripe.com:443 reason=\"charges\"\n// @openshell provider STRIPE_API_KEY\nimport { writeFileSync } from \"node:fs\";\nexport async function f() {\n  const r = await fetch(\"https://api.stripe.com/v1/charges\");\n  writeFileSync(\"/srv/x\", \"a\");\n  const k = process.env.STRIPE_API_KEY;\n}\n",
    )
    .unwrap();
    let o = run(t.path(), &["lint", "--deny-warnings"]);
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
}

#[test]
fn lint_python_go_shell_call_sites_and_test_files_excluded() {
    let t = fixture();
    std::fs::write(t.path().join("src/x.py"), "import requests, subprocess\nrequests.get(\"https://api.acme-a.dev/a\")\nopen(\"/srv/out.txt\", \"w\").write(\"x\")\nopen(\"in.txt\").read()\nsubprocess.run([\"git\", \"pull\"])\n").unwrap();
    std::fs::write(t.path().join("cmd/x.go"), "package main\nfunc f() { http.Get(\"https://api.acme-b.dev/\"); os.WriteFile(\"/srv/a\", nil, 0o644); _ = os.Getenv(\"DB_PASSWORD\") }\n").unwrap();
    std::fs::write(
        t.path().join("scripts/x.sh"),
        "curl -fsSL https://dl.acme-c.dev/i.sh > /opt/i.sh\necho hi > /dev/null\nmkdir -p /opt/a\n",
    )
    .unwrap();
    std::fs::create_dir_all(t.path().join("src/__tests__")).unwrap();
    std::fs::write(
        t.path().join("src/__tests__/t.ts"),
        "fetch(\"https://ignored.test\")\n",
    )
    .unwrap();
    let s = out(&run(t.path(), &["lint", "--format", "compact"]));
    assert!(s.contains("src/x.py:2: warning[net]: requests"), "{s}");
    assert!(
        s.contains("src/x.py:3: warning[fs]: open() for writing"),
        "{s}"
    );
    assert!(
        !s.contains("src/x.py:4"),
        "read-mode open must not be flagged: {s}"
    );
    assert!(s.contains("src/x.py:5: info[exec]"), "{s}");
    assert!(
        s.contains("cmd/x.go:2: warning[net]")
            && s.contains("cmd/x.go:2: warning[fs]")
            && s.contains("DB_PASSWORD"),
        "{s}"
    );
    assert!(
        s.contains("scripts/x.sh:1: warning[net]: curl/wget")
            && s.contains("scripts/x.sh:1: warning[fs]"),
        "{s}"
    );
    assert!(
        !s.contains("scripts/x.sh:2"),
        "> /dev/null is not a write: {s}"
    );
    assert!(s.contains("scripts/x.sh:3: warning[fs]"), "{s}");
    assert!(!s.contains("__tests__"), "{s}");
}

#[test]
fn lint_checks_a_policy_file() {
    let t = fixture();
    std::fs::write(t.path().join("bad.yaml"), "version: 1\nprocess:\n  run_as_user: \"0\"\nnetwork_policies:\n  x:\n    endpoints:\n      - host: \"*.com\"\n        port: 443\n    binaries: []\nbogus: 1\n").unwrap();
    let o = run(
        t.path(),
        &["lint", "--format", "compact", "--policy", "bad.yaml"],
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(out(&o).contains("unknown field 'bogus'"), "{}", out(&o));
    std::fs::write(t.path().join("bad.yaml"), "version: 1\nprocess:\n  run_as_user: \"0\"\nnetwork_policies:\n  x:\n    endpoints:\n      - host: \"*.com\"\n        port: 443\n    binaries: []\n").unwrap();
    let s = out(&run(
        t.path(),
        &["lint", "--format", "compact", "--policy", "bad.yaml"],
    ));
    assert!(
        s.contains("run_as_user") && s.contains("root is rejected"),
        "{s}"
    );
    assert!(s.contains("three DNS labels"), "{s}");
    assert!(s.contains("allows nothing"), "{s}");
}

#[test]
fn schema_crate_rejects_what_we_never_emit() {
    // Guard rails: the pinned OpenShell schema really is strict about these.
    assert!(
        openshell_policy_schema::parse_policy("version: 1\nproviders: {}\n").is_err(),
        "policy.yaml has no providers section"
    );
    assert!(openshell_policy_schema::parse_policy("version: 2\n").is_err());
    assert!(openshell_policy_schema::parse_policy("version: 1\nnetwork_policies:\n  a:\n    endpoints:\n      - host: x.test\n        port: 443\n        nope: 1\n").is_err());
}

fn fs_only_repo(extra_toml: &str) -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join("src")).unwrap();
    std::fs::write(
        t.path().join("src/a.js"),
        "// @openshell fs write /data/out reason=\"results\"\n",
    )
    .unwrap();
    std::fs::write(t.path().join("openshell-annotate.toml"), extra_toml).unwrap();
    t
}

#[test]
fn system_paths_are_added_when_there_are_no_network_rules() {
    let t = fs_only_repo("");
    let o = run(t.path(), &["generate"]);
    assert!(o.status.success(), "{}", err(&o));
    let y = out(&o);
    for p in [
        "\"/usr\"",
        "\"/lib\"",
        "\"/etc\"",
        "\"/proc\"",
        "\"/dev/urandom\"",
        "\"/tmp\"",
        "\"/dev/null\"",
        "\"/data/out\"",
    ] {
        assert!(y.contains(p), "missing {p} in\n{y}");
    }
}

#[test]
fn system_paths_never_leaves_the_policy_as_annotated() {
    let t = fs_only_repo("[policy]\nsystem_paths = \"never\"\n");
    let y = out(&run(t.path(), &["generate"]));
    assert!(
        y.contains("\"/data/out\"") && !y.contains("\"/usr\""),
        "{y}"
    );
    let bad = fs_only_repo("[policy]\nsystem_paths = \"sometimes\"\n");
    let o = run(bad.path(), &["generate"]);
    assert!(
        !o.status.success() && err(&o).contains("system_paths"),
        "{}",
        err(&o)
    );
}

#[test]
fn process_only_policy_still_gets_a_runnable_filesystem() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(
        t.path().join("a.js"),
        "// @openshell process user=1000 group=1000 reason=\"non-root\"\n",
    )
    .unwrap();
    let y = out(&run(t.path(), &["generate"]));
    assert!(
        y.contains("run_as_user") && y.contains("\"/usr\"") && y.contains("\"/tmp\""),
        "{y}"
    );
}

// ---- diff-logs ------------------------------------------------------------------------------------------------------

fn logs_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/logs")
        .join(name)
}

fn diff_json(t: &Path, log: &str) -> (i32, serde_json::Value) {
    let o = run(
        t,
        &["diff-logs", logs_fixture(log).to_str().unwrap(), "--json"],
    );
    (
        o.status.code().unwrap(),
        serde_json::from_str(&out(&o)).unwrap_or_else(|e| panic!("{e}: {} {}", out(&o), err(&o))),
    )
}

fn kinds(v: &serde_json::Value) -> Vec<(String, String, String)> {
    v["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["kind"].as_str().unwrap().to_string(),
                s["host"].as_str().unwrap().to_string(),
                s["binary"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

#[test]
fn diff_logs_shorthand_classifies_denies() {
    let t = fixture();
    let (code, v) = diff_json(t.path(), "shorthand.log");
    assert_eq!(code, 1);
    assert_eq!(v["events"], 7); // NET:REFUSE lines carry no binary and are skipped
    assert_eq!(v["denied"], 6);
    let k = kinds(&v);
    assert!(
        k.contains(&(
            "binary_mismatch".into(),
            "registry.npmjs.org".into(),
            "/usr/bin/curl".into()
        )),
        "{k:?}"
    );
    assert!(
        k.contains(&(
            "missing_egress".into(),
            "api.newvendor.io".into(),
            "/usr/bin/python3.12".into()
        )),
        "{k:?}"
    );
    assert!(
        k.contains(&("l7_allow".into(), "huggingface.co".into(), "".into())),
        "{k:?}"
    );
    assert!(
        k.contains(&("manual".into(), "api.github.com".into(), "".into())),
        "{k:?}"
    );
    // the two curl denials are one suggestion with count 2
    let m = v["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"] == "binary_mismatch")
        .unwrap();
    assert_eq!(m["count"], 2);
    assert!(m["directive"]
        .as_str()
        .unwrap()
        .contains("bin=/usr/bin/curl"));
    assert!(m["directive"]
        .as_str()
        .unwrap()
        .contains("access=read-only"));
    // SSRF/metadata is never actionable
    assert!(v["non_actionable"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["host"] == "169.254.169.254"));
    assert!(!k.iter().any(|(_, h, _)| h == "169.254.169.254"));
    // openai never appears, so it is stale
    assert!(v["stale"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["target"].as_str().unwrap().starts_with("api.openai.com")));
}

#[test]
fn diff_logs_ocsf_json_matches_shorthand_semantics() {
    let t = fixture();
    let (code, v) = diff_json(t.path(), "ocsf.jsonl");
    assert_eq!(code, 1);
    // class 3002 is ignored: 4 events
    assert_eq!(v["events"], 4);
    let k = kinds(&v);
    assert!(
        k.contains(&(
            "missing_egress".into(),
            "cdn.other.dev".into(),
            "/usr/local/bin/node".into()
        )),
        "{k:?}"
    );
    assert!(k.contains(&(
        "missing_egress".into(),
        "api.newvendor.io".into(),
        "/usr/bin/python3.12".into()
    )));
    let l7 = v["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"] == "l7_allow")
        .unwrap();
    assert!(l7["directive"]
        .as_str()
        .unwrap()
        .contains("allow=\"POST:/api/upload\""));
    assert!(l7["directive"].as_str().unwrap().contains("bin="));
}

#[test]
fn diff_logs_clean_when_everything_is_covered() {
    let t = fixture();
    let log = t.path().join("ok.log");
    std::fs::write(&log, "[t] [sandbox] [OCSF ] [ocsf] NET:OPEN [INFO] ALLOWED /usr/local/bin/node(1) -> registry.npmjs.org:443 [policy:r engine:opa]\n").unwrap();
    let o = run(
        t.path(),
        &["diff-logs", log.to_str().unwrap(), "--format", "compact"],
    );
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    // not-seen directives are reported as stale, and --deny-stale turns that into a failure
    assert!(out(&o).contains("STALE"));
    let o = run(t.path(), &["diff-logs", log.to_str().unwrap()]);
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    assert!(
        out(&o).contains("every event is explained") && out(&o).contains("Stale directives"),
        "{}",
        out(&o)
    );
    let o = run(
        t.path(),
        &["diff-logs", log.to_str().unwrap(), "--deny-stale"],
    );
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn diff_logs_write_places_directives_and_is_idempotent() {
    let t = fixture();
    std::fs::write(
        t.path().join("src/extra.ts"),
        "export const u = 'https://api.newvendor.io/v1';\n",
    )
    .unwrap();
    let log = logs_fixture("ocsf.jsonl");
    let o = run(t.path(), &["diff-logs", log.to_str().unwrap(), "--write"]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    // host mentioned in source: directive inserted above that line, with an explicit bin
    let extra = std::fs::read_to_string(t.path().join("src/extra.ts")).unwrap();
    assert!(
        extra.starts_with("// @openshell egress api.newvendor.io:443"),
        "{extra}"
    );
    assert!(extra.contains("bin=/usr/bin/python3.12"));
    // host that no source mentions: parked in the observed file
    let obs = std::fs::read_to_string(t.path().join("openshell/observed-egress.sh")).unwrap();
    assert!(
        obs.contains("# @openshell egress cdn.other.dev:8443"),
        "{obs}"
    );
    // the L7 allow lands next to the directive it extends
    let hf = std::fs::read_to_string(t.path().join("src/cache.py")).unwrap();
    assert!(hf.contains("allow=\"POST:/api/upload\""), "{hf}");
    // the edited tree still scans and now explains the same logs: only manual/none left
    let o = run(t.path(), &["scan"]);
    assert!(o.status.success(), "{}", err(&o));
    let o = run(t.path(), &["diff-logs", log.to_str().unwrap(), "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert!(
        v["suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["kind"] == "manual"),
        "{}",
        out(&o)
    );
}

// ---- tree-sitter lint -----------------------------------------------------------------------------------------------

#[test]
fn lint_ast_classifies_fs_write_paths_and_summarises_dynamic_ones() {
    let t = fixture();
    // sample declares `fs write /workspace/data`; write under it, under /tmp, to a temp dir, outside, and dynamically
    std::fs::write(
        t.path().join("src/w.ts"),
        "import fs from \"node:fs\";\nimport path from \"node:path\";\nimport os from \"node:os\";\nexport function f(id: string, dir: string) {\n  fs.writeFileSync(\"/workspace/data/a\", \"x\");\n  fs.writeFileSync(path.join(os.tmpdir(), \"b\"), \"x\");\n  fs.writeFileSync(`/workspace/data/${id}`, \"x\");\n  fs.writeFileSync(`/var/lib/app/${id}`, \"x\");\n  fs.mkdirSync(path.join(\"/srv/app\", id));\n  fs.writeFileSync(path.join(dir, \"c\"), \"x\");\n  fs.writeFileSync(dir + \"/d\", \"x\");\n}\n",
    )
    .unwrap();
    let s = out(&run(t.path(), &["lint", "--format", "compact"]));
    assert!(
        !s.contains("src/w.ts:5") && !s.contains("src/w.ts:6") && !s.contains("src/w.ts:7"),
        "{s}"
    );
    assert!(
        s.contains("src/w.ts:8: warning[fs]") && s.contains("/var/lib/app/"),
        "{s}"
    );
    assert!(
        s.contains("src/w.ts:9: warning[fs]") && s.contains("/srv/app"),
        "{s}"
    );
    // the two dynamic writes are one repo-level info, not warnings
    assert!(
        !s.contains("src/w.ts:10") && !s.contains("src/w.ts:11"),
        "{s}"
    );
    assert!(
        s.contains("info[fs]") && s.contains("2 filesystem write(s)"),
        "{s}"
    );
    let o = run(t.path(), &["lint", "--json"]);
    assert!(out(&o).contains("\"file\": \".\""), "{}", out(&o));
}

#[test]
fn lint_ast_without_any_fs_root_warns_once_per_file() {
    let t = fixture();
    let toml = std::fs::read_to_string(t.path().join("openshell-annotate.toml")).unwrap();
    std::fs::write(t.path().join("openshell-annotate.toml"), toml).unwrap();
    // drop the fs directive from the sample so no root is declared
    let mut stripped = false;
    for e in walk(t.path()) {
        if let Ok(txt) = std::fs::read_to_string(&e) {
            if txt.contains("@openshell fs write") {
                let kept: Vec<&str> = txt
                    .lines()
                    .filter(|l| !l.contains("@openshell fs write"))
                    .collect();
                std::fs::write(&e, kept.join("\n") + "\n").unwrap();
                stripped = true;
            }
        }
    }
    assert!(stripped);
    std::fs::write(
        t.path().join("src/w.py"),
        "def f(p, q):\n    open(p, \"w\")\n    open(q, \"wb\")\n    open(p)\n",
    )
    .unwrap();
    let s = out(&run(t.path(), &["lint", "--format", "compact"]));
    let n = s
        .lines()
        .filter(|l| l.starts_with("src/w.py") && l.contains("warning[fs]"))
        .count();
    assert_eq!(n, 1, "{s}");
    assert!(
        s.contains("2 filesystem write(s) with non-literal paths"),
        "{s}"
    );
}

fn walk(d: &Path) -> Vec<PathBuf> {
    let mut v = vec![];
    for e in std::fs::read_dir(d).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            v.extend(walk(&p));
        } else {
            v.push(p);
        }
    }
    v
}

#[test]
fn lint_ast_resolves_urls_against_declared_egress_and_reads_python_go_shell_syntax() {
    let t = fixture();
    // huggingface.co is annotated in the sample: a call with that literal URL is covered even without a nearby directive
    std::fs::write(
        t.path().join("src/u.py"),
        "import requests, os\nrequests.get(\"https://huggingface.co/api/models\")\nrequests.get(\"https://api.acme-z.dev/x\")\nurl = \"https://cdn.acme-y.dev/f\"\nw = os.environ.get(\"MY_API_KEY\")\nopen(\"/tmp/f\", \"w\")\nopen(\"/tmp/f\", mode=\"rb\")\nx = \"requests.get('https://nope.test')\"\n",
    )
    .unwrap();
    let s = out(&run(t.path(), &["lint", "--format", "compact"]));
    assert!(!s.contains("src/u.py:2"), "{s}");
    assert!(
        s.contains("src/u.py:3: warning[net]") && s.contains("api.acme-z.dev:443"),
        "{s}"
    );
    assert!(
        s.contains("src/u.py:4: info[net]: URL literal to cdn.acme-y.dev:443"),
        "{s}"
    );
    assert!(
        s.contains("src/u.py:5: warning[env]") && s.contains("MY_API_KEY"),
        "{s}"
    );
    assert!(
        !s.contains("src/u.py:6") && !s.contains("src/u.py:7") && !s.contains("src/u.py:8"),
        "{s}"
    );
    // shell: expansions and redirections come from the AST
    std::fs::write(t.path().join("scripts/y.sh"), "echo \"$MY_SECRET_TOKEN\" > \"$TMPDIR/x\"\nwget -q https://dl.acme-q.dev/a.tgz -O /srv/a.tgz\nls 2>&1 >&2\n").unwrap();
    let s = out(&run(t.path(), &["lint", "--format", "compact"]));
    assert!(
        s.contains("scripts/y.sh:1: warning[env]") && s.contains("MY_SECRET_TOKEN"),
        "{s}"
    );
    assert!(
        !s.contains("scripts/y.sh:1: warning[fs]"),
        "TMPDIR target is a temp path: {s}"
    );
    assert!(s.contains("scripts/y.sh:2: warning[net]: curl/wget"), "{s}");
    assert!(!s.contains("scripts/y.sh:3"), "{s}");
}

#[test]
fn lint_regex_engine_is_still_available() {
    let t = fixture();
    let toml = std::fs::read_to_string(t.path().join("openshell-annotate.toml")).unwrap();
    std::fs::write(
        t.path().join("openshell-annotate.toml"),
        format!("{toml}\n[lint]\nengine = \"regex\"\n"),
    )
    .unwrap();
    std::fs::write(
        t.path().join("src/r.ts"),
        "export const a = fetch(\"https://api.acme-r.dev/x\");\n",
    )
    .unwrap();
    let s = out(&run(t.path(), &["lint", "--format", "compact"]));
    assert!(s.contains("src/r.ts:1: warning[net]: fetch()"), "{s}");
}

#[test]
fn lint_ast_follows_fs_imports_and_aliases() {
    let t = fixture();
    std::fs::write(
        t.path().join("src/imp.ts"),
        "import { mkdir as mk, readFile } from \"node:fs/promises\";\nconst { rename } = require(\"fs\");\nexport async function f() {\n  await mk(\"/srv/a\");\n  await readFile(\"/srv/b\");\n  rename(\"/srv/c\", \"/srv/d\");\n  const mkdir = () => 1;\n  mkdir();\n}\n",
    )
    .unwrap();
    std::fs::write(
        t.path().join("src/noimp.ts"),
        "function mkdir(p: string) {}\nmkdir(\"/srv/z\");\n",
    )
    .unwrap();
    let s = out(&run(t.path(), &["lint", "--format", "compact"]));
    assert!(
        s.contains("src/imp.ts:4: warning[fs]") && s.contains("(mkdir) to /srv/a"),
        "{s}"
    );
    assert!(!s.contains("src/imp.ts:5"), "{s}");
    assert!(
        s.contains("src/imp.ts:6: warning[fs]") && s.contains("to /srv/d"),
        "{s}"
    );
    assert!(
        !s.contains("src/noimp.ts"),
        "a local function named mkdir is not fs.mkdir: {s}"
    );
}

/// The error of a failed run must be exactly one error line naming `dir_name` and its source, then one hint line.
fn assert_missing_profiles_dir(o: &Output, dir_name: &str, source: &str, hint: &str) {
    assert!(!o.status.success(), "a missing profiles dir must fail");
    let s = err(o);
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines.len(), 2, "one error line plus one hint line: {s}");
    assert!(
        lines[0].starts_with("error: profiles directory ")
            && lines[0].contains(dir_name)
            && lines[0].ends_with(&format!(" ({source}) does not exist")),
        "error line must name the directory and its source: {s}"
    );
    assert_eq!(lines[1], format!("hint: {hint}"), "{s}");
    assert!(!s.contains("No such file"), "no raw OS error: {s}");
}

fn set_profiles_config(t: &Path, from: &str, to: &str) {
    let f = t.join("openshell-annotate.toml");
    let toml = std::fs::read_to_string(&f).unwrap();
    assert!(toml.contains(from), "fixture config changed: {toml}");
    std::fs::write(&f, toml.replace(from, to)).unwrap();
}

#[test]
fn missing_profiles_dir_from_config_names_the_key_and_file() {
    let t = fixture();
    set_profiles_config(
        t.path(),
        "profiles_dir = \"profiles\"",
        "profiles_dir = \"nope_dir\"",
    );
    let o = run(t.path(), &["generate", "-o", "policy.yaml"]);
    assert_missing_profiles_dir(
        &o,
        "nope_dir",
        "from `[providers] profiles_dir` in openshell-annotate.toml",
        "create it, fix the path in openshell-annotate.toml, or clone OpenShell",
    );
}

#[test]
fn missing_profiles_dirs_entry_from_config_names_the_key_and_file() {
    let t = fixture();
    set_profiles_config(
        t.path(),
        "profiles_dir = \"profiles\"",
        "profiles_dirs = [\"profiles\", \"../OpenShell/providers\"]",
    );
    let o = run(t.path(), &["generate", "-o", "policy.yaml"]);
    assert_missing_profiles_dir(
        &o,
        "OpenShell/providers",
        "from `[providers] profiles_dirs` in openshell-annotate.toml",
        "create it, fix the path in openshell-annotate.toml, or clone OpenShell",
    );
}

#[test]
fn missing_profiles_dir_from_the_flag_names_the_flag_not_the_config() {
    let t = fixture();
    let o = run(
        t.path(),
        &[
            "generate",
            "--profiles-dir",
            "nope_flag",
            "-o",
            "policy.yaml",
        ],
    );
    assert_missing_profiles_dir(
        &o,
        "nope_flag",
        "from --profiles-dir",
        "create it, fix the --profiles-dir path, or clone OpenShell",
    );
    assert!(
        !err(&o).contains("openshell-annotate.toml"),
        "the flag error must not blame the config file: {}",
        err(&o)
    );
}

#[test]
fn profiles_dir_that_is_not_a_directory_keeps_its_own_error() {
    let t = fixture();
    std::fs::write(t.path().join("a_file"), "x").unwrap();
    set_profiles_config(
        t.path(),
        "profiles_dir = \"profiles\"",
        "profiles_dir = \"a_file\"",
    );
    let o = run(t.path(), &["generate", "-o", "policy.yaml"]);
    assert!(!o.status.success());
    let s = err(&o);
    assert!(
        s.contains("reading profiles dir"),
        "existing context kept: {s}"
    );
    assert!(!s.contains("hint:") && !s.contains("does not exist"), "{s}");
}

#[test]
fn profiles_dirs_lists_several_directories_and_profiles_dir_still_works() {
    let t = fixture();
    let base = out(&run(
        t.path(),
        &["generate", "--providers-out", "p.json", "-o", "policy.yaml"],
    ));
    let want = std::fs::read_to_string(t.path().join("p.json")).unwrap();
    let _ = base;
    // move the profiles to two other directories and list both
    std::fs::create_dir_all(t.path().join("profiles_a")).unwrap();
    std::fs::create_dir_all(t.path().join("profiles_b")).unwrap();
    for (i, e) in std::fs::read_dir(t.path().join("profiles"))
        .unwrap()
        .enumerate()
    {
        let e = e.unwrap();
        let dir = if i % 2 == 0 {
            "profiles_a"
        } else {
            "profiles_b"
        };
        std::fs::create_dir_all(t.path().join(dir)).unwrap();
        std::fs::rename(e.path(), t.path().join(dir).join(e.file_name())).unwrap();
    }
    std::fs::remove_dir_all(t.path().join("profiles")).unwrap();
    let toml = std::fs::read_to_string(t.path().join("openshell-annotate.toml")).unwrap();
    std::fs::write(
        t.path().join("openshell-annotate.toml"),
        toml.replace(
            "profiles_dir = \"profiles\"",
            "profiles_dirs = [\"profiles_a\", \"profiles_b\"]",
        ),
    )
    .unwrap();
    let o = run(
        t.path(),
        &[
            "generate",
            "--providers-out",
            "p2.json",
            "-o",
            "policy2.yaml",
        ],
    );
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(
        std::fs::read_to_string(t.path().join("p2.json")).unwrap(),
        want
    );
}

#[test]
fn diff_logs_ignore_host_declines_a_denial_on_purpose() {
    let t = fixture();
    std::fs::write(
        t.path().join("src/decline.ts"),
        "// @openshell ignore kind=net host=*.newvendor.io reason=\"vendor telemetry is deliberately blocked\"\nexport {};\n",
    )
    .unwrap();
    let (code, v) = diff_json(t.path(), "ocsf.jsonl");
    assert_eq!(code, 1); // cdn.other.dev is still missing
    let k = kinds(&v);
    assert!(!k.iter().any(|(_, h, _)| h == "api.newvendor.io"), "{k:?}");
    let d = &v["declined"].as_array().unwrap()[0];
    assert_eq!(d["host"], "api.newvendor.io");
    assert!(d["reason"]
        .as_str()
        .unwrap()
        .contains("deliberately blocked"));
    // host= is only valid with kind=net
    std::fs::write(
        t.path().join("src/bad.ts"),
        "// @openshell ignore host=x.example.com reason=\"r\"\n",
    )
    .unwrap();
    let o = run(t.path(), &["lint", "--format", "compact"]);
    assert!(
        out(&o).contains("ignore host=... needs kind=net"),
        "{}{}",
        out(&o),
        err(&o)
    );
}

#[test]
fn js_directives_survive_regex_literals_that_confuse_a_lexer() {
    let t = fixture();
    // an unbalanced quote inside a regex literal would hide everything below it from a hand-written lexer
    std::fs::write(
        t.path().join("src/rx.ts"),
        "const re = /[\"'`]\\//g;\nexport function f(a: number, b: number) { return a / b / 2; }\n// @openshell egress ws.rx-test.example:443 reason=\"after a regex literal\"\nexport const x = 1;\n",
    )
    .unwrap();
    let o = run(t.path(), &["scan", "--format", "compact"]);
    assert!(
        out(&o).contains("src/rx.ts:3  egress"),
        "{}{}",
        out(&o),
        err(&o)
    );
}

// ---- human lint report ------------------------------------------------------------------------------------------------

/// Project with one finding of each kind the report formats: a directive error, net, fs, env and exec.
fn report_fixture() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join("src")).unwrap();
    std::fs::write(
        t.path().join("src/app.js"),
        "import { writeFileSync } from \"node:fs\";\nimport { spawn } from \"node:child_process\";\nexport async function f() {\n  const key = process.env.ACME_API_KEY;\n  const r = await fetch(\"https://api.acme.io/v1/items\");\n  writeFileSync(\"/srv/data/items.json\", \"[]\");\n  spawn(\"git\", [\"fetch\"]);\n}\n",
    )
    .unwrap();
    std::fs::write(
        t.path().join("src/job.py"),
        "import requests\n\ndef run(url):\n    return requests.get(url)\n",
    )
    .unwrap();
    std::fs::write(
        t.path().join("src/setup.sh"),
        "#!/bin/sh\n# @openshell frobnicate x\n",
    )
    .unwrap();
    t
}

/// Run with a controlled environment (the host's LANG/TERM/NO_COLOR must not leak into snapshots).
fn run_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(bin());
    c.current_dir(dir).args(args);
    for k in [
        "NO_COLOR",
        "CLICOLOR_FORCE",
        "TERM",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
    ] {
        c.env_remove(k);
    }
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().unwrap()
}

/// Compare with `tests/golden/<name>`; `UPDATE_GOLDEN=1 cargo test` rewrites the file (review the diff!).
fn golden(name: &str, actual: &str) {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&p, actual).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&p).unwrap_or_else(|_| panic!("missing golden {name}"));
    assert_eq!(
        want, actual,
        "golden {name} differs; rerun with UPDATE_GOLDEN=1 if intended"
    );
}

#[test]
fn lint_human_plain_snapshot() {
    let t = report_fixture();
    let o = run_env(t.path(), &["lint", "--color", "never", "--no-icons"], &[]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    let s = out(&o);
    assert!(
        !s.contains('\x1b') && s.is_ascii(),
        "plain output must be ASCII: {s}"
    );
    golden("lint_human_plain.txt", &s);
    // the human report carries its own summary; stderr stays quiet
    assert_eq!(err(&o), "");
}

#[test]
fn lint_human_icons_and_colour_snapshot() {
    let t = report_fixture();
    let utf8 = [("LANG", "en_US.UTF-8"), ("TERM", "xterm-256color")];
    let o = run_env(t.path(), &["lint", "--color", "always"], &utf8);
    let s = out(&o);
    assert!(
        s.contains("\u{26a0} \u{1f310} \x1b") || s.contains("\u{26a0} \u{1f310} warning[net]"),
        "{s}"
    );
    golden("lint_human_color.txt", &s);
    // icons without colour, then the same text with the icons removed matches the ASCII run
    let o = run_env(t.path(), &["lint", "--color", "never"], &utf8);
    let s = out(&o);
    for icon in [
        "\u{2716} \u{1f4dd} error[directive]",
        "\u{26a0} \u{1f310} warning[net]",
        "\u{26a0} \u{1f4c1} warning[fs]",
        "\u{26a0} \u{1f511} warning[env]",
        "\u{2139} \u{2699}\u{fe0f} info[exec]",
    ] {
        assert!(s.contains(icon), "missing `{icon}` in {s}");
    }
    assert!(
        s.contains("\u{1f4c4} src/app.js") && s.contains("by category:"),
        "{s}"
    );
}

#[test]
fn lint_human_shows_context_underline_and_a_copy_pasteable_directive() {
    let t = report_fixture();
    let s = out(&run_env(
        t.path(),
        &["lint", "--color", "never", "--no-icons"],
        &[],
    ));
    // location with line:col, gutter, the offending call underlined from its start
    assert!(s.contains(" --> src/app.js:5:19\n"), "{s}");
    assert!(
        s.contains("5 |   const r = await fetch(\"https://api.acme.io/v1/items\");\n"),
        "{s}"
    );
    assert!(s.contains("\n  |                   ^^^^^"), "{s}");
    // Python/shell use `#` comments, JS `//`
    assert!(
        s.contains("= help: declare the host this call connects to, on the line above:\n"),
        "{s}"
    );
    assert!(
        s.contains("# @openshell egress <host>:443 reason=\"...\""),
        "{s}"
    );
    assert!(
        s.contains("// @openshell fs write /srv/data reason=\"...\""),
        "{s}"
    );
    assert!(s.contains("// @openshell provider ACME_API_KEY"), "{s}");
    // findings are grouped per file, with a summary
    assert!(s.contains("src/app.js  (4 findings)"), "{s}");
    assert!(s.contains("src/job.py  (1 finding)"), "{s}");
    assert!(s.contains("1 error, 4 warnings, 1 info"), "{s}");
    assert!(s.contains("src/setup.sh  (1 finding)"), "{s}");
}

#[test]
fn lint_ascii_fallback_without_a_utf8_locale_or_on_a_dumb_terminal() {
    let t = report_fixture();
    for env in [
        vec![("LANG", "C")],
        vec![],
        vec![("LANG", "en_US.UTF-8"), ("TERM", "dumb")],
    ] {
        let s = out(&run_env(t.path(), &["lint", "--color", "never"], &env));
        assert!(s.is_ascii(), "{env:?}: {s}");
    }
    let s = out(&run_env(
        t.path(),
        &["lint", "--color", "never", "--ascii"],
        &[("LANG", "en_US.UTF-8")],
    ));
    assert!(s.is_ascii(), "--ascii: {s}");
}

#[test]
fn lint_colour_flag_and_environment() {
    let t = report_fixture();
    let has = |o: Output| out(&o).contains("\x1b[");
    // not a terminal: auto means no colour; the flag forces it either way
    assert!(!has(run_env(t.path(), &["lint"], &[])));
    assert!(has(run_env(t.path(), &["lint", "--color", "always"], &[])));
    assert!(!has(run_env(
        t.path(),
        &["lint", "--color", "never"],
        &[("CLICOLOR_FORCE", "1")]
    )));
    // CLICOLOR_FORCE colours a pipe; NO_COLOR wins over it in auto mode
    assert!(has(run_env(
        t.path(),
        &["lint"],
        &[("CLICOLOR_FORCE", "1")]
    )));
    assert!(!has(run_env(
        t.path(),
        &["lint"],
        &[("CLICOLOR_FORCE", "1"), ("NO_COLOR", "1")]
    )));
}

#[test]
fn lint_machine_formats_are_stable_and_exit_codes_do_not_depend_on_the_format() {
    let t = report_fixture();
    let compact = out(&run(t.path(), &["lint", "--format", "compact"]));
    assert!(
        compact.contains("src/app.js:5: warning[net]: fetch() without an @openshell directive"),
        "{compact}"
    );
    assert!(
        !compact.contains('\x1b') && !compact.contains("-->"),
        "{compact}"
    );
    // `--json` is an alias of `--format json`, and its shape is unchanged (no new keys)
    let a = out(&run(t.path(), &["lint", "--json"]));
    let b = out(&run(t.path(), &["lint", "--format", "json"]));
    assert_eq!(a, b);
    let v: serde_json::Value = serde_json::from_str(&a).unwrap();
    let first = v[0].as_object().unwrap();
    let mut keys: Vec<&str> = first.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["category", "file", "line", "message", "severity"]);
    // contradictory flags are a usage error
    assert_eq!(
        run(t.path(), &["lint", "--json", "--format", "compact"])
            .status
            .code(),
        Some(2)
    );
    // exit codes: errors fail every format; warnings only with --deny-warnings
    for fmt in ["human", "json", "compact"] {
        assert_eq!(
            run(t.path(), &["lint", "--format", fmt]).status.code(),
            Some(1),
            "{fmt}"
        );
    }
    std::fs::write(
        t.path().join("src/app.js"),
        "// @openshell ignore kind=net reason=\"x\" scope=file\nfetch(u);\n",
    )
    .unwrap();
    std::fs::remove_file(t.path().join("src/job.py")).unwrap();
    std::fs::remove_file(t.path().join("src/setup.sh")).unwrap();
    for fmt in ["human", "json", "compact"] {
        assert_eq!(
            run(t.path(), &["lint", "--format", fmt]).status.code(),
            Some(0),
            "{fmt}"
        );
    }
    let clean = out(&run_env(
        t.path(),
        &["lint", "--no-icons", "--color", "never"],
        &[],
    ));
    assert!(clean.starts_with("no findings (1 directive(s))"), "{clean}");
}

#[test]
fn lint_human_handles_policy_files_long_lines_tabs_and_project_wide_findings() {
    let t = report_fixture();
    std::fs::write(t.path().join("bad.yaml"), "version: 1\nnetwork_policies:\n  x:\n    endpoints:\n      - host: \"*.com\"\n        port: 443\n    binaries: []\n").unwrap();
    let pad = " ".repeat(200);
    std::fs::write(
        t.path().join("src/long.js"),
        format!("function g() {{\n\tconst x = \"é\"; {pad}fetch(\"https://long.acme.io/\");\n}}\n"),
    )
    .unwrap();
    let s = out(&run_env(
        t.path(),
        &[
            "lint",
            "--policy",
            "bad.yaml",
            "--color",
            "never",
            "--no-icons",
        ],
        &[],
    ));
    assert!(s.contains("bad.yaml  (3 findings)"), "{s}");
    assert!(
        s.contains("TLD wildcard") && s.contains("three DNS labels"),
        "{s}"
    );
    assert!(
        s.contains("warning[policy]: empty `binaries` matches no executable"),
        "{s}"
    );
    assert!(s.contains("= note: at `network_policies.x`"), "{s}");
    // the long line is windowed around the call; no source line exceeds the display cap
    assert!(s.contains("src/long.js:2:"), "{s}");
    assert!(s.lines().all(|l| l.chars().count() < 200), "{s}");
    assert!(
        s.contains("2 | ...") && s.contains("fetch(\"https://long.acme.io/\")"),
        "{s}"
    );
}

#[test]
fn lint_human_puts_a_long_usage_message_under_help() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("a.sh"), "#!/bin/sh\n# @openshell egress\n").unwrap();
    let s = out(&run_env(
        t.path(),
        &["lint", "--color", "never", "--no-icons"],
        &[],
    ));
    assert!(
        s.contains("error[directive]: malformed @openshell directive\n --> a.sh:2:3\n"),
        "{s}"
    );
    assert!(
        s.contains("= help: expected syntax: egress <host>:<port>"),
        "{s}"
    );
}

// ---- human scan / diff-logs / generate -------------------------------------------------------------------------------

const UTF8: [(&str, &str); 2] = [("LANG", "en_US.UTF-8"), ("TERM", "xterm-256color")];

/// The sample project plus a copy of the shorthand log (relative path, so the snapshot is machine independent).
fn diff_fixture() -> tempfile::TempDir {
    let t = fixture();
    std::fs::copy(logs_fixture("shorthand.log"), t.path().join("denied.log")).unwrap();
    t
}

#[test]
fn scan_human_plain_and_colour_snapshots() {
    let t = fixture();
    let o = run_env(t.path(), &["scan", "--color", "never", "--no-icons"], &[]);
    assert!(o.status.success(), "{}", err(&o));
    let s = out(&o);
    assert!(
        !s.contains('\x1b') && s.is_ascii(),
        "plain output must be ASCII: {s}"
    );
    golden("scan_human_plain.txt", &s);
    assert_eq!(err(&o), "", "the human report carries its own summary");
    let o = run_env(t.path(), &["scan", "--color", "always"], &UTF8);
    let s = out(&o);
    assert!(s.contains('\x1b'), "{s}");
    golden("scan_human_color.txt", &s);
    // icons without colour: one per category, grouped under the file, with the summary by category
    let o = run_env(t.path(), &["scan", "--color", "never"], &UTF8);
    let s = out(&o);
    for want in [
        "\u{1f4c4} cmd/main.go  (2 directives)",
        "\u{1f310} egress api.github.com:443",
        "\u{1f4c1} fs write /workspace/data",
        "\u{1f511} provider OPENAI_API_KEY",
        "--> src/llm.ts:2",
        "8 directives in 4 files",
        "by category: \u{1f511} env 1",
    ] {
        assert!(s.contains(want), "missing `{want}` in {s}");
    }
}

#[test]
fn scan_machine_formats_are_stable() {
    let t = fixture();
    let compact = run_env(t.path(), &["scan", "--format", "compact"], &UTF8);
    let c = out(&compact);
    assert_eq!(c.lines().count(), 8, "{c}");
    assert!(c.contains("src/llm.ts:2  provider OPENAI_API_KEY"), "{c}");
    assert!(err(&compact).contains("8 directive(s) in 4 file(s)"));
    let a = out(&run(t.path(), &["scan", "--json"]));
    let b = out(&run(t.path(), &["scan", "--format", "json"]));
    assert_eq!(a, b);
    assert!(a.trim_start().starts_with('{'));
    let o = run(t.path(), &["scan", "--json", "--format", "compact"]);
    assert_eq!(o.status.code(), Some(2), "--json conflicts with --format");
}

#[test]
fn scan_human_reports_nothing_found_and_keeps_parse_errors_on_stderr() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("a.sh"), "echo hi\n").unwrap();
    let o = run_env(t.path(), &["scan", "--color", "never", "--no-icons"], &[]);
    assert!(o.status.success());
    assert!(
        out(&o).contains("no @openshell directives found (1 file scanned)"),
        "{}",
        out(&o)
    );
    std::fs::write(t.path().join("b.sh"), "# @openshell frobnicate x\n").unwrap();
    let o = run_env(t.path(), &["scan", "--color", "never", "--no-icons"], &[]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        out(&o).contains("1 parse error (details on stderr)"),
        "{}",
        out(&o)
    );
    assert!(err(&o).contains("b.sh:1"), "{}", err(&o));
}

#[test]
fn diff_logs_human_plain_and_colour_snapshots() {
    let t = diff_fixture();
    let o = run_env(
        t.path(),
        &["diff-logs", "denied.log", "--color", "never", "--no-icons"],
        &[],
    );
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    let s = out(&o);
    assert!(
        !s.contains('\x1b') && s.is_ascii(),
        "plain output must be ASCII: {s}"
    );
    golden("diff_logs_human_plain.txt", &s);
    let o = run_env(
        t.path(),
        &["diff-logs", "denied.log", "--color", "always"],
        &UTF8,
    );
    assert_eq!(o.status.code(), Some(1));
    golden("diff_logs_human_color.txt", &out(&o));
    let o = run_env(
        t.path(),
        &["diff-logs", "denied.log", "--color", "never"],
        &UTF8,
    );
    let s = out(&o);
    for want in [
        "\u{2716} \u{1f310} missing egress: api.newvendor.io:443",
        "\u{26a0} \u{1f310} binary mismatch: registry.npmjs.org:443  \u{d7}2  /usr/bin/curl",
        "\u{26a0} \u{1f9f9} stale: api.openai.com:443",
        "\u{2139} \u{1f310} manual review: api.github.com:443",
        "\u{1f50d} diff-logs  7 events",
    ] {
        assert!(s.contains(want), "missing `{want}` in {s}");
    }
    // the fix is previewed as an inserted line (`+`) above the line it would go on, with the source's own comment leader
    assert!(
        s.contains("+ | # @openshell egress registry.npmjs.org:443")
            && s.contains("--> scripts/run.sh:2"),
        "{s}"
    );
    assert!(s.contains("3 directives"), "{s}");
}

#[test]
fn diff_logs_machine_formats_and_exit_codes_do_not_depend_on_the_format() {
    let t = diff_fixture();
    let human = run_env(t.path(), &["diff-logs", "denied.log"], &UTF8);
    let compact = run(
        t.path(),
        &["diff-logs", "denied.log", "--format", "compact"],
    );
    let json = run(t.path(), &["diff-logs", "denied.log", "--json"]);
    let json2 = run(t.path(), &["diff-logs", "denied.log", "--format", "json"]);
    for o in [&human, &compact, &json, &json2] {
        assert_eq!(o.status.code(), Some(1));
    }
    assert_eq!(out(&json), out(&json2));
    let c = out(&compact);
    assert!(
        c.starts_with(
            "diff-logs: 7 event(s) (6 denied, 1 allowed); 4 suggestion(s), 1 stale directive(s)\n"
        ),
        "{c}"
    );
    for want in [
        "MISSING_EGRESS   api.newvendor.io:443 x1 /usr/bin/python3.12\n",
        "  place:   scripts/run.sh:2\n",
        "STALE            src/llm.ts:1 api.openai.com:443 (never matched in these logs)\n",
        "NON-ACTIONABLE   169.254.169.254:80 x1 always-blocked destination (SSRF protection)\n",
    ] {
        assert!(c.contains(want), "missing `{want}` in {c}");
    }
    let v: serde_json::Value = serde_json::from_str(&out(&json)).unwrap();
    assert_eq!(v["suggestions"].as_array().unwrap().len(), 4);
    // --write still applies the same suggestions after a human report
    let o = run_env(
        t.path(),
        &["diff-logs", "denied.log", "--write", "--color", "never"],
        &UTF8,
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(
        err(&o).contains("wrote suggested directives to:"),
        "{}",
        err(&o)
    );
    let sh = std::fs::read_to_string(t.path().join("scripts/run.sh")).unwrap();
    assert!(sh.contains("bin=/usr/bin/curl"), "{sh}");
}

#[test]
fn generate_policy_is_byte_identical_for_every_format_and_the_summary_goes_to_stderr() {
    let t = fixture();
    let want = std::fs::read_to_string(t.path().join("policy.expected.yaml")).unwrap();
    for fmt in ["human", "compact", "json"] {
        let o = run_env(
            t.path(),
            &["generate", "--format", fmt, "--color", "always"],
            &UTF8,
        );
        assert!(o.status.success(), "{}", err(&o));
        assert_eq!(
            out(&o),
            want,
            "stdout must be the bare policy with --format {fmt}"
        );
        let o = run_env(
            t.path(),
            &[
                "generate", "--format", fmt, "-o", "out.yaml", "--color", "always",
            ],
            &UTF8,
        );
        assert!(o.status.success(), "{}", err(&o));
        assert_eq!(out(&o), "", "nothing on stdout when writing a file ({fmt})");
        assert_eq!(
            std::fs::read_to_string(t.path().join("out.yaml")).unwrap(),
            want
        );
    }
    // compact: exactly the earlier stderr
    let o = run_env(t.path(), &["generate", "--format", "compact"], &UTF8);
    let e = err(&o);
    assert!(
        e.starts_with("warning: egress api.openai.com:443 (rule `api_openai_com`) duplicates"),
        "{e}"
    );
    assert!(
        e.ends_with("generated: 8 directive(s) -> 4 network rule(s), 4 endpoint(s), 4 binary path(s), fs read 1 / write 1, 1 provider(s) (0 unmapped)\n"),
        "{e}"
    );
    assert_eq!(e.lines().count(), 2);
    // json: summary object on stderr
    let o = run_env(
        t.path(),
        &["generate", "--format", "json", "-o", "p.yaml"],
        &UTF8,
    );
    let v: serde_json::Value = serde_json::from_str(&err(&o)).unwrap();
    assert_eq!(v["stats"]["network_rules"], 4);
    assert_eq!(v["policy"], "p.yaml");
    assert_eq!(v["warnings"].as_array().unwrap().len(), 1);
}

#[test]
fn generate_human_summary_snapshots() {
    let t = fixture();
    let args = [
        "generate",
        "-o",
        "policy.yaml",
        "--providers-out",
        "providers.sh",
    ];
    let mut plain = args.to_vec();
    plain.extend(["--color", "never", "--no-icons"]);
    let o = run_env(t.path(), &plain, &[]);
    assert!(o.status.success(), "{}", err(&o));
    let e = err(&o);
    assert!(
        !e.contains('\x1b') && e.is_ascii(),
        "plain output must be ASCII: {e}"
    );
    golden("generate_human_plain.txt", &e);
    let mut colour = args.to_vec();
    colour.extend(["--color", "always"]);
    let o = run_env(t.path(), &colour, &UTF8);
    golden("generate_human_color.txt", &err(&o));
    let o = run_env(t.path(), &["generate", "--color", "never"], &UTF8);
    let e = err(&o);
    for want in [
        "\u{2714} generated a policy from 8 directives",
        "\u{1f4dc} policy",
        "\u{1f310} network",
        "\u{1f4c1} filesystem",
        "\u{1f511} providers",
        "\u{26a0} 1 warning",
        "from cmd/main.go:3, cmd/main.go:4",
    ] {
        assert!(e.contains(want), "missing `{want}` in {e}");
    }
    assert!(e.contains("\u{2192} stdout"), "{e}");
}

#[test]
fn colour_flag_and_environment_apply_to_scan_diff_logs_and_generate() {
    let t = diff_fixture();
    let cmds: [(&[&str], bool); 3] = [
        (&["scan"], false),
        (&["diff-logs", "denied.log"], false),
        (&["generate"], true), // summary is on stderr
    ];
    for (args, on_stderr) in cmds {
        let text = |o: &Output| if on_stderr { err(o) } else { out(o) };
        // not a terminal: plain by default
        let o = run_env(t.path(), args, &UTF8);
        assert!(!text(&o).contains('\x1b'), "{args:?}");
        // CLICOLOR_FORCE forces, NO_COLOR beats it, --color always beats NO_COLOR, --color never beats everything
        let o = run_env(
            t.path(),
            args,
            &[("LANG", "en_US.UTF-8"), ("CLICOLOR_FORCE", "1")],
        );
        assert!(text(&o).contains('\x1b'), "{args:?}");
        let o = run_env(
            t.path(),
            args,
            &[
                ("LANG", "en_US.UTF-8"),
                ("CLICOLOR_FORCE", "1"),
                ("NO_COLOR", "1"),
            ],
        );
        assert!(!text(&o).contains('\x1b'), "{args:?}");
        let mut a = args.to_vec();
        a.extend(["--color", "always"]);
        let o = run_env(t.path(), &a, &[("NO_COLOR", "1")]);
        assert!(text(&o).contains('\x1b'), "{args:?}");
        let mut a = args.to_vec();
        a.extend(["--color", "never"]);
        let o = run_env(t.path(), &a, &[("CLICOLOR_FORCE", "1")]);
        assert!(!text(&o).contains('\x1b'), "{args:?}");
        // ASCII fallbacks: --no-icons, --ascii, a non-UTF-8 locale and TERM=dumb all drop the emoji
        for extra in [vec!["--no-icons"], vec!["--ascii"], vec![]] {
            let mut a = args.to_vec();
            a.extend(extra.iter().copied());
            a.extend(["--color", "never"]);
            let env: &[(&str, &str)] = if a.contains(&"--no-icons") || a.contains(&"--ascii") {
                &UTF8
            } else {
                &[("LANG", "C")]
            };
            let o = run_env(t.path(), &a, env);
            assert!(text(&o).is_ascii(), "{a:?}: {}", text(&o));
        }
        let o = run_env(t.path(), args, &[("LANG", "en_US.UTF-8"), ("TERM", "dumb")]);
        assert!(text(&o).is_ascii(), "{args:?}");
    }
}

/// `examples/polyglot` ships its generated policy and providers file. Regenerating must give the same
/// output (the header carries `@VERSION@` instead of the tool version), and `drift` must be clean.
#[test]
fn polyglot_example_regenerates_identically_and_has_no_drift() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/polyglot");
    let t = tempfile::tempdir().unwrap();
    copy_dir(&src, t.path());
    let o = run(
        t.path(),
        &[
            "generate",
            "-o",
            "new-policy.yaml",
            "--providers-out",
            "new-providers.json",
        ],
    );
    assert!(o.status.success(), "{}", err(&o));
    let committed = committed_policy(&src);
    let fresh = std::fs::read_to_string(t.path().join("new-policy.yaml")).unwrap();
    assert_eq!(
        fresh, committed,
        "examples/polyglot/openshell/policy.yaml is stale; regenerate it (see examples/polyglot/README.md)"
    );
    let committed = std::fs::read_to_string(src.join("openshell/providers.json")).unwrap();
    let fresh = std::fs::read_to_string(t.path().join("new-providers.json")).unwrap();
    assert_eq!(
        fresh, committed,
        "examples/polyglot/openshell/providers.json is stale"
    );
    std::fs::write(
        t.path().join("openshell/policy.yaml"),
        committed_policy(&src),
    )
    .unwrap();
    let o = run(
        t.path(),
        &[
            "drift",
            "--policy",
            "openshell/policy.yaml",
            "--providers",
            "openshell/providers.json",
        ],
    );
    assert!(o.status.success(), "drift: {}{}", out(&o), err(&o));
    assert!(out(&o).contains("no drift"), "{}", out(&o));
}

fn committed_policy(example: &Path) -> String {
    std::fs::read_to_string(example.join("openshell/policy.yaml"))
        .unwrap()
        .replace("@VERSION@", env!("CARGO_PKG_VERSION"))
}
