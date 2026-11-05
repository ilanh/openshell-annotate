//! Single source of truth for the `@openshell` directive grammar.
//!
//! The parser (`directive.rs`) takes its allowed attributes and usage strings from [`VERBS`], and
//! `openshell-annotate docs` renders `docs/DIRECTIVES.md` from the same table, so the documentation
//! cannot drift from what the parser accepts. Semantic checks (value ranges, mutual exclusion) live in
//! `validate.rs`; their user-facing summary is in each attribute's `help`.

use crate::validate::PROTOCOLS;

/// EBNF of a directive. A directive must be the first thing in a comment line (after the comment
/// leader), so prose that merely mentions `@openshell` is ignored.
pub const GRAMMAR: &str = r#"directive = "@openshell" , ws , body ;
body      = egress | provider | fs | bin | process | ignore ;
egress    = "egress" , ws , target , { ws , attr } ;
target    = host , ":" , ports ;            (* port is mandatory *)
ports     = port , { "," , port } ;
provider  = "provider" , ws , env-name , { ws , attr } ;
fs        = "fs" , ws , ( "read" | "write" ) , ws , path , { ws , attr } ;
bin       = "bin" , ws , path , { ws , attr } ;
process   = "process" , { ws , attr } ;
ignore    = "ignore" , { ws , attr } ;
attr      = key , "=" , value ;
value     = bare | '"' , { any - '"' | '\"' } , '"' ;"#;

/// One attribute a directive accepts.
pub struct AttrSpec {
    /// Key as written before `=`.
    pub name: &'static str,
    /// Repeatable attribute (`allow`, `deny`); otherwise a duplicate is an error.
    pub multi: bool,
    /// Human-readable description of the accepted values, shown in the docs table.
    pub values: &'static str,
    /// Meaning of the attribute, shown in the docs table.
    pub help: &'static str,
}

/// One directive verb: its usage, arguments, attributes and documentation.
pub struct VerbSpec {
    /// Keyword after `@openshell`.
    pub verb: &'static str,
    /// Shown in `usage:` parse errors.
    pub usage: &'static str,
    /// One-sentence description for the docs.
    pub summary: &'static str,
    /// Positional arguments: (name, description).
    pub args: &'static [(&'static str, &'static str)],
    /// Verb-specific attributes; [`COMMON_ATTRS`] are accepted in addition.
    pub attrs: &'static [AttrSpec],
    /// Whether `reason="..."` is mandatory.
    pub reason_required: bool,
    /// Lint call-site categories this directive covers (see `ignore kind=`).
    pub covers: &'static str,
    /// What the directive becomes in the generated output.
    pub emits: &'static str,
}

/// Attributes accepted by every verb.
pub const COMMON_ATTRS: &[AttrSpec] = &[
    AttrSpec { name: "reason", multi: false, values: "text", help: "Why this is needed. Becomes the comment above the generated entry; required where marked." },
    AttrSpec { name: "scope", multi: false, values: "file | next", help: "`file` makes the directive cover the whole file for lint; the default covers the call sites within `[lint] window` lines below it." },
];

/// Values of `ignore kind=`, as listed in the docs.
pub const IGNORE_KINDS: &[&str] = &["net", "fs", "env", "exec", "all"];
/// Values of `egress access=`, as listed in the docs.
pub const ACCESS_PRESETS: &[&str] = &["read-only", "read-write", "full"];

/// Every directive verb, in documentation order.
pub const VERBS: &[VerbSpec] = &[
    VerbSpec {
        verb: "egress",
        usage: "egress <host>:<port>[,<port>...] reason=\"...\" [access=read-only|read-write|full] [protocol=rest|...] [allow=\"METHOD:/path\"]... [bin=/path] [rule=name] [enforce=enforce|audit] [tls=skip]",
        summary: "Allow outbound network access to a host and port(s).",
        args: &[("host:port[,port...]", "Destination. A port is mandatory (OpenShell rejects endpoints without one); the host is lower-cased.")],
        attrs: &[
            AttrSpec { name: "access", multi: false, values: "read-only | read-write | full", help: "OpenShell access preset; turns on request inspection (protocol defaults to `rest`). Mutually exclusive with `allow`. For `protocol=mcp` the preset is not written out (MCP rejects access presets); it is emitted as `mcp.allow_all_known_mcp_methods: true`." },
            AttrSpec { name: "protocol", multi: false, values: "rest | websocket | graphql | mcp | json-rpc | tcp", help: "L7 protocol. `tcp` is uninspected L4 and cannot be combined with `access`/`allow`/`deny`. `rest`, `websocket`, and `graphql` need `access` or `allow` (a protocol alone is rejected). `mcp` never emits `access`; an `access` value becomes `mcp.allow_all_known_mcp_methods: true`." },
            AttrSpec { name: "enforce", multi: false, values: "enforce | audit", help: "Enforcement mode for inspected endpoints (default from `[policy] enforcement`, itself defaulting to `enforce`)." },
            AttrSpec { name: "allow", multi: true, values: "METHOD:/path | query | mutation | subscription", help: "Explicit allowed request, e.g. `GET:/v1/**`. Repeatable. Implies inspection. For `protocol=graphql` the value is `query`, `mutation`, or `subscription`, optionally `:operationName`. That is emitted as `rules[].allow.operation_type` and only for GraphQL." },
            AttrSpec { name: "deny", multi: true, values: "METHOD:/path | query | mutation | subscription", help: "Request subtracted from what is allowed. Repeatable; needs `allow` or `access`. For `protocol=graphql` the value is an operation type (`query`, `mutation`, or `subscription`), emitted as `deny_rules[].operation_type` and only for GraphQL." },
            AttrSpec { name: "bin", multi: false, values: "/absolute/real/path", help: "Binary that opens the connection. Overrides the nearest `@openshell bin` and `[languages]` in the config." },
            AttrSpec { name: "rule", multi: false, values: "[A-Za-z0-9_-]+", help: "Name of the `network_policies` entry (default: slug of the host). Must not start with `_provider_`." },
            AttrSpec { name: "tls", multi: false, values: "skip", help: "Relay without TLS termination. Cannot be combined with request inspection. Use it for clients that do not trust the OpenShell CA. `openshell-prover check` reported such policies as `unsupported` with OpenShell 0.1.6 (tracked in #19)." },
        ],
        reason_required: true,
        covers: "net",
        emits: "`network_policies.<rule>.endpoints[]` (`host`, `port`/`ports`, `protocol`, `enforcement`, `access` or `rules`/`deny_rules`, or `mcp.allow_all_known_mcp_methods` for MCP) plus `binaries[]`.",
    },
    VerbSpec {
        verb: "provider",
        usage: "provider <ENV_VAR> [profile=<id>] [name=<instance>]",
        summary: "Declare that an environment variable is a credential supplied by an OpenShell provider.",
        args: &[("ENV_VAR", "Environment variable name, `[A-Z_][A-Z0-9_]*`.")],
        attrs: &[
            AttrSpec { name: "profile", multi: false, values: "profile id", help: "Provider profile to use instead of the one resolved from the profile YAML's `credentials[].env_vars`." },
            AttrSpec { name: "name", multi: false, values: "instance name", help: "Name of the provider instance to create." },
        ],
        reason_required: false,
        covers: "env",
        emits: "Not part of `policy.yaml`: a provider setup in the `providers.json` / `providers.sh` output (`openshell provider create ... --credential ENV`).",
    },
    VerbSpec {
        verb: "fs",
        usage: "fs read|write <absolute-path> reason=\"...\"",
        summary: "Grant filesystem access inside the sandbox.",
        args: &[("read|write", "Access mode."), ("path", "Absolute in-sandbox path.")],
        attrs: &[],
        reason_required: true,
        covers: "fs (write only)",
        emits: "`filesystem_policy.read_only` or `read_write` (plus `include_workdir`). Startup-only in OpenShell: it does not reload live.",
    },
    VerbSpec {
        verb: "bin",
        usage: "bin <absolute-path-of-interpreter-or-binary>",
        summary: "Set the binary allowed to use the egress rules in this file.",
        args: &[("path", "Absolute real path of the interpreter or binary inside the sandbox image (OpenShell does not follow symlinks).")],
        attrs: &[],
        reason_required: false,
        covers: "exec",
        emits: "`binaries[].path` of egress rules in the same file that have no `bin=`.",
    },
    VerbSpec {
        verb: "process",
        usage: "process [user=<sandbox|uid>] [group=<sandbox|gid>]",
        summary: "Choose the identity the sandboxed process runs as.",
        args: &[],
        attrs: &[
            AttrSpec { name: "user", multi: false, values: "sandbox | uid", help: "`process.run_as_user`." },
            AttrSpec { name: "group", multi: false, values: "sandbox | gid", help: "`process.run_as_group`. Conflicting values across directives are an error." },
        ],
        reason_required: false,
        covers: "-",
        emits: "`process.run_as_user` / `process.run_as_group`. Startup-only in OpenShell.",
    },
    VerbSpec {
        verb: "ignore",
        usage: "ignore kind=net|fs|env|exec|all reason=\"...\" [scope=file]",
        summary: "Tell lint that a call site needs no policy entry (or that a denial is declined on purpose).",
        args: &[],
        attrs: &[
            AttrSpec { name: "kind", multi: false, values: "net | fs | env | exec | all", help: "Lint category to silence (default `all`)." },
            AttrSpec { name: "host", multi: false, values: "hostname", help: "With `kind=net`: also mark denials of this host in `diff-logs` as declined." },
        ],
        reason_required: true,
        covers: "the category named by `kind`",
        emits: "Nothing; affects `lint` and `diff-logs` only.",
    },
];

/// Returns the spec of the verb `name`, if it exists.
pub fn verb(name: &str) -> Option<&'static VerbSpec> {
    VERBS.iter().find(|v| v.verb == name)
}

/// Returns all verb keywords joined with `, `, for error messages.
pub fn verb_names() -> String {
    VERBS.iter().map(|v| v.verb).collect::<Vec<_>>().join(", ")
}

/// Render `docs/DIRECTIVES.md`.
pub fn directives_md() -> String {
    let mut s = String::new();
    s.push_str("# `@openshell` directives\n\n");
    s.push_str("<!-- Generated by `openshell-annotate docs` from src/spec.rs. Do not edit; run `scripts/gen-docs.sh`. -->\n\n");
    s.push_str("A directive is a source comment whose text starts with `@openshell`. It must be the first thing in the comment line (after the comment leader), so prose that merely mentions `@openshell` is ignored. Unknown directives and attributes are errors, mirroring OpenShell's strict schema.\n\n");
    s.push_str("## Grammar\n\n```ebnf\n");
    s.push_str(GRAMMAR);
    s.push_str("\n```\n\n");
    s.push_str("Values containing spaces must be quoted: `reason=\"LLM calls\"`. Absolute paths may contain `=`.\n\n");
    s.push_str(
        "## Directives\n\n| Directive | Summary | `reason` | Lint category |\n|---|---|---|---|\n",
    );
    for v in VERBS {
        s.push_str(&format!(
            "| [`{0}`](#{0}) | {1} | {2} | {3} |\n",
            v.verb,
            v.summary,
            if v.reason_required {
                "required"
            } else {
                "optional"
            },
            v.covers
        ));
    }
    s.push_str("\n## Attributes accepted by every directive\n\n");
    attr_table(&mut s, COMMON_ATTRS);
    for v in VERBS {
        s.push_str(&format!("\n## {}\n\n{}\n\n", v.verb, v.summary));
        s.push_str(&format!("```\n@openshell {}\n```\n\n", v.usage));
        if !v.args.is_empty() {
            s.push_str("Arguments:\n\n");
            for (n, d) in v.args {
                s.push_str(&format!("- `{n}`: {d}\n"));
            }
            s.push('\n');
        }
        if !v.attrs.is_empty() {
            s.push_str("Attributes:\n\n");
            attr_table(&mut s, v.attrs);
            s.push('\n');
        }
        s.push_str(&format!(
            "`reason` is {}.\n\n**Becomes:** {}\n",
            if v.reason_required {
                "required"
            } else {
                "optional"
            },
            v.emits
        ));
    }
    s.push_str(&format!(
        "\n## Value sets\n\n- `protocol`: {}\n- `access`: {}\n- `ignore kind`: {}\n",
        PROTOCOLS.join(", "),
        ACCESS_PRESETS.join(", "),
        IGNORE_KINDS.join(", ")
    ));
    s
}

/// Appends a Markdown table of `attrs` to `s`, escaping `|` in the values column.
fn attr_table(s: &mut String, attrs: &[AttrSpec]) {
    s.push_str("| Attribute | Values | Repeatable | Meaning |\n|---|---|---|---|\n");
    for a in attrs {
        s.push_str(&format!(
            "| `{}` | `{}` | {} | {} |\n",
            a.name,
            a.values.replace('|', "\\|"),
            if a.multi { "yes" } else { "no" },
            a.help
        ));
    }
}
