//! Compare a policy generated from directives with a committed policy.yaml.
//!
//! Both sides go through `openshell_policy_schema::parse_policy` and are compared as typed
//! documents, so comments, key order and quoting never count as drift.

use crate::providers::ProvidersOut;
use openshell_policy_schema::{NetworkPolicyRule, PolicyDocument};
use serde::Serialize;
use std::collections::BTreeSet;

/// Kind of difference between the generated and the committed policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    /// Directive exists in source but the committed policy lacks it (policy needs regenerating).
    Missing,
    /// Committed policy grants something no directive declares.
    Extra,
    /// Present on both sides with different content.
    Changed,
}

/// One drift finding.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Finding {
    pub change: Change,
    /// Policy section, e.g. `network_policies` or `filesystem_policy.read_only`.
    pub section: String,
    /// Rule key, path or field within the section.
    pub item: String,
    /// Human-readable description of the difference.
    pub detail: String,
    /// `startup` sections need a sandbox recreate to change; `live` can be updated on a running sandbox.
    pub applies: &'static str,
}

/// Result of [`compare`] (plus any [`compare_providers`] findings the caller adds).
#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub findings: Vec<Finding>,
    /// Sections present in the committed policy that directives cannot express (ignored unless `strict`).
    pub unmanaged: Vec<String>,
}

/// Returns the endpoints of a rule as `host:[ports]` strings.
fn eps(r: &NetworkPolicyRule) -> BTreeSet<String> {
    r.endpoints
        .iter()
        .map(|e| format!("{}:{:?}", e.host, e.effective_ports()))
        .collect()
}
/// Returns the binary paths of a rule.
fn bins(r: &NetworkPolicyRule) -> BTreeSet<String> {
    r.binaries.iter().map(|b| b.path.clone()).collect()
}
/// Describes what is only in `a` (declared) and only in `b` (committed); empty when the sets are equal.
fn diff_sets(a: &BTreeSet<String>, b: &BTreeSet<String>) -> String {
    let add: Vec<_> = a.difference(b).cloned().collect();
    let del: Vec<_> = b.difference(a).cloned().collect();
    let mut s = Vec::new();
    if !add.is_empty() {
        s.push(format!("declared but not committed: {}", add.join(", ")));
    }
    if !del.is_empty() {
        s.push(format!("committed but not declared: {}", del.join(", ")));
    }
    s.join("; ")
}

/// Compares the network rules (by key), the effective filesystem policy and the process identity of two policies.
///
/// Committed `landlock` (when the generated policy has none) and `network_middlewares` go to
/// [`Report::unmanaged`]; with `strict` they become [`Change::Extra`] findings instead.
pub fn compare(generated: &PolicyDocument, committed: &PolicyDocument, strict: bool) -> Report {
    let mut r = Report::default();
    // network_policies (live)
    let keys: BTreeSet<&String> = generated
        .network_policies
        .keys()
        .chain(committed.network_policies.keys())
        .collect();
    for k in keys {
        match (
            generated.network_policies.get(k),
            committed.network_policies.get(k),
        ) {
            (Some(g), None) => r.findings.push(Finding {
                change: Change::Missing,
                section: "network_policies".into(),
                item: k.clone(),
                detail: format!("endpoints {:?}", eps(g)),
                applies: "live",
            }),
            (None, Some(c)) => r.findings.push(Finding {
                change: Change::Extra,
                section: "network_policies".into(),
                item: k.clone(),
                detail: format!(
                    "endpoints {:?}, no @openshell egress directive declares it",
                    eps(c)
                ),
                applies: "live",
            }),
            (Some(g), Some(c)) if g != c => {
                let mut parts = Vec::new();
                let (ge, ce, gb, cb) = (eps(g), eps(c), bins(g), bins(c));
                if ge != ce {
                    parts.push(format!("endpoints: {}", diff_sets(&ge, &ce)));
                }
                if gb != cb {
                    parts.push(format!("binaries: {}", diff_sets(&gb, &cb)));
                }
                if parts.is_empty() {
                    parts.push(
                        "endpoint settings (protocol/access/rules/enforcement/tls) differ".into(),
                    );
                }
                r.findings.push(Finding {
                    change: Change::Changed,
                    section: "network_policies".into(),
                    item: k.clone(),
                    detail: parts.join("; "),
                    applies: "live",
                });
            }
            _ => {}
        }
    }
    // filesystem (startup)
    let (g, c) = (
        generated.effective_filesystem_policy(),
        committed.effective_filesystem_policy(),
    );
    if g.include_workdir != c.include_workdir {
        r.findings.push(Finding {
            change: Change::Changed,
            section: "filesystem_policy".into(),
            item: "include_workdir".into(),
            detail: format!(
                "declared {} committed {}",
                g.include_workdir, c.include_workdir
            ),
            applies: "startup",
        });
    }
    for (name, gl, cl) in [
        ("read_only", &g.read_only, &c.read_only),
        ("read_write", &g.read_write, &c.read_write),
    ] {
        let (gs, cs): (BTreeSet<_>, BTreeSet<_>) =
            (gl.iter().cloned().collect(), cl.iter().cloned().collect());
        for p in gs.difference(&cs) {
            r.findings.push(Finding {
                change: Change::Missing,
                section: format!("filesystem_policy.{name}"),
                item: p.clone(),
                detail: "declared by @openshell fs but not committed".into(),
                applies: "startup",
            });
        }
        for p in cs.difference(&gs) {
            r.findings.push(Finding {
                change: Change::Extra,
                section: format!("filesystem_policy.{name}"),
                item: p.clone(),
                detail: "committed but no @openshell fs directive declares it".into(),
                applies: "startup",
            });
        }
    }
    // process (startup)
    let gp = generated
        .process
        .as_ref()
        .map(|p| (p.run_as_user.clone(), p.run_as_group.clone()))
        .unwrap_or_default();
    let cp = committed
        .process
        .as_ref()
        .map(|p| (p.run_as_user.clone(), p.run_as_group.clone()))
        .unwrap_or_default();
    if gp != cp {
        r.findings.push(Finding {
            change: Change::Changed,
            section: "process".into(),
            item: "run_as_user/run_as_group".into(),
            detail: format!("declared {gp:?} committed {cp:?}"),
            applies: "startup",
        });
    }
    // sections directives cannot express
    if committed.landlock.is_some() && generated.landlock.is_none() {
        r.unmanaged.push("landlock".into());
    }
    if !committed.network_middlewares.is_empty() {
        r.unmanaged.push("network_middlewares".into());
    }
    if strict {
        for s in std::mem::take(&mut r.unmanaged) {
            r.findings.push(Finding {
                change: Change::Extra,
                section: s.clone(),
                item: s.clone(),
                detail: "present in committed policy; directives cannot express it (--strict)"
                    .into(),
                applies: unmanaged_applies(&s),
            });
        }
    }
    r
}

/// When a change to an unmanaged section takes effect: `network_middlewares` is a dynamic field (updated on a running
/// sandbox like `network_policies`); `landlock` and anything else is fixed at sandbox creation.
fn unmanaged_applies(section: &str) -> &'static str {
    match section {
        "network_middlewares" => "live",
        _ => "startup",
    }
}

/// Compares the generated providers output with a committed providers JSON file.
///
/// Providers are matched by instance name and compared as whole JSON objects; the `unmapped` lists are compared as
/// sets.
///
/// # Panics
///
/// Panics if `generated` cannot be converted to a JSON value, which its plain fields cannot cause.
pub fn compare_providers(generated: &ProvidersOut, committed: &serde_json::Value) -> Vec<Finding> {
    let mut out = Vec::new();
    let gen = serde_json::to_value(generated).unwrap();
    let list = |v: &serde_json::Value, key: &str| -> BTreeSet<String> {
        v.get(key)
            .and_then(|x| x.as_array())
            .map(|a| a.iter().map(|e| e.to_string()).collect())
            .unwrap_or_default()
    };
    // Compare by instance name for readable output.
    let names = |v: &serde_json::Value| -> std::collections::BTreeMap<String, String> {
        v.get("providers")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .map(|e| {
                        (
                            e.get("instance")
                                .and_then(|i| i.as_str())
                                .unwrap_or("?")
                                .to_string(),
                            e.to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let (g, c) = (names(&gen), names(committed));
    for (k, v) in &g {
        match c.get(k) {
            None => out.push(Finding {
                change: Change::Missing,
                section: "providers".into(),
                item: k.clone(),
                detail: "declared by @openshell provider but absent from committed providers file"
                    .into(),
                applies: "live",
            }),
            Some(cv) if cv != v => out.push(Finding {
                change: Change::Changed,
                section: "providers".into(),
                item: k.clone(),
                detail: "credentials/profile/sources differ".into(),
                applies: "live",
            }),
            _ => {}
        }
    }
    for k in c.keys() {
        if !g.contains_key(k) {
            out.push(Finding {
                change: Change::Extra,
                section: "providers".into(),
                item: k.clone(),
                detail: "in committed providers file but no @openshell provider directive".into(),
                applies: "live",
            });
        }
    }
    if list(&gen, "unmapped") != list(committed, "unmapped") {
        out.push(Finding {
            change: Change::Changed,
            section: "providers".into(),
            item: "unmapped".into(),
            detail: "unmapped credential list differs".into(),
            applies: "live",
        });
    }
    out
}

impl Report {
    /// Renders one line per finding, then one note per unmanaged section.
    pub fn render(&self) -> String {
        let mut s = String::new();
        for f in &self.findings {
            s.push_str(&format!(
                "{:<8} [{}] {} {}: {}\n",
                format!("{:?}", f.change).to_lowercase(),
                f.applies,
                f.section,
                f.item,
                f.detail
            ));
        }
        for u in &self.unmanaged {
            s.push_str(&format!("note     committed policy has `{u}`, which directives cannot express; not compared (use --strict to flag)\n"));
        }
        s
    }
}
