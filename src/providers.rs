//! Provider resolution: `@openshell provider ENV` -> provider profile -> setup commands.
//!
//! OpenShell has no `providers` section in policy.yaml. Providers are gateway objects created from
//! imported *profiles* and attached to sandboxes (`openshell sandbox create --provider NAME`).

use crate::config::Config;
use crate::directive::{Directive, Kind};
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Profile {
    pub id: String,
    pub env_vars: Vec<String>,
    /// `host:port` entries the profile contributes to the effective policy.
    pub endpoints: Vec<String>,
    pub file: PathBuf,
}

pub fn load_profiles(dir: &Path) -> Result<Vec<Profile>> {
    let mut out = Vec::new();
    let rd = std::fs::read_dir(dir)
        .with_context(|| format!("reading profiles dir {}", dir.display()))?;
    let mut paths: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("yaml" | "yml")))
        .collect();
    paths.sort();
    for p in paths {
        let s = std::fs::read_to_string(&p)?;
        let v: serde_json::Value = match serde_yml::from_str(&s) {
            Ok(v) => v,
            Err(_) => continue, // not a profile; `openshell profile lint` is the authority
        };
        let Some(id) = v.get("id").and_then(|x| x.as_str()) else {
            continue;
        };
        let mut env_vars = Vec::new();
        for c in v
            .get("credentials")
            .and_then(|c| c.as_array())
            .into_iter()
            .flatten()
        {
            for e in c
                .get("env_vars")
                .and_then(|e| e.as_array())
                .into_iter()
                .flatten()
            {
                if let Some(e) = e.as_str() {
                    env_vars.push(e.to_string());
                }
            }
        }
        let mut endpoints = Vec::new();
        for e in v
            .get("endpoints")
            .and_then(|c| c.as_array())
            .into_iter()
            .flatten()
        {
            let host = e.get("host").and_then(|h| h.as_str()).unwrap_or("");
            let mut ports: Vec<u64> = e
                .get("ports")
                .and_then(|p| p.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_u64()).collect())
                .unwrap_or_default();
            if let Some(p) = e.get("port").and_then(|p| p.as_u64()) {
                ports.push(p);
            }
            for p in ports {
                let ep = format!("{host}:{p}");
                if !endpoints.contains(&ep) {
                    endpoints.push(ep);
                }
            }
        }
        out.push(Profile {
            id: id.to_string(),
            env_vars,
            endpoints,
            file: p,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderEntry {
    /// Provider instance name to create (`openshell provider create --name`).
    pub instance: String,
    /// Profile id (`--type`), if known.
    pub profile: Option<String>,
    pub credentials: Vec<String>,
    pub sources: Vec<String>,
    /// Endpoints the profile already contributes to the effective policy.
    pub profile_endpoints: Vec<String>,
    #[serde(skip)]
    pub profile_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Unmapped {
    pub env: String,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct ProvidersOut {
    pub version: u32,
    pub providers: Vec<ProviderEntry>,
    pub unmapped: Vec<Unmapped>,
}

pub fn resolve(dirs: &[Directive], cfg: &Config, profiles: &[Profile]) -> ProvidersOut {
    // env -> (explicit profile/name, sources)
    type EnvEntry = (Option<String>, Option<String>, Vec<String>);
    let mut envs: BTreeMap<String, EnvEntry> = BTreeMap::new();
    for d in dirs {
        if let Kind::Provider { env } = &d.kind {
            let e = envs.entry(env.clone()).or_default();
            if let Some(p) = d.attr("profile") {
                e.0 = Some(p.to_string());
            }
            if let Some(n) = d.attr("name") {
                e.1 = Some(n.to_string());
            }
            e.2.push(d.loc());
        }
    }
    for env in &cfg.providers.expect {
        let e = envs.entry(env.clone()).or_default();
        e.2.push(format!(
            "{}: [providers].expect",
            crate::config::CONFIG_FILE
        ));
    }
    let mut by_instance: BTreeMap<String, ProviderEntry> = BTreeMap::new();
    let mut unmapped = Vec::new();
    for (env, (prof, name, sources)) in envs {
        let profile = prof
            .or_else(|| cfg.providers.env.get(&env).cloned())
            .or_else(|| {
                profiles
                    .iter()
                    .find(|p| p.env_vars.contains(&env))
                    .map(|p| p.id.clone())
            });
        match profile {
            Some(pid) => {
                let instance = name.unwrap_or_else(|| pid.clone());
                let known = profiles.iter().find(|p| p.id == pid);
                let e = by_instance
                    .entry(instance.clone())
                    .or_insert_with(|| ProviderEntry {
                        instance,
                        profile: Some(pid.clone()),
                        credentials: vec![],
                        sources: vec![],
                        profile_endpoints: known.map(|k| k.endpoints.clone()).unwrap_or_default(),
                        profile_file: known.map(|k| k.file.clone()),
                    });
                e.credentials.push(env);
                e.sources.extend(sources);
            }
            None => unmapped.push(Unmapped { env, sources }),
        }
    }
    let mut providers: Vec<ProviderEntry> = by_instance.into_values().collect();
    for p in &mut providers {
        p.credentials.sort();
        p.credentials.dedup();
        p.sources.sort();
        p.sources.dedup();
    }
    ProvidersOut {
        version: 1,
        providers,
        unmapped,
    }
}

impl ProvidersOut {
    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).unwrap();
        s.push('\n');
        s
    }

    /// Shell script with the `openshell` commands. Never contains credential values:
    /// `--credential KEY` (bare form) reads the value from the caller's environment.
    pub fn to_script(&self, policy_path: &str, root: &Path) -> String {
        let mut s = String::new();
        s.push_str("#!/usr/bin/env bash\n");
        s.push_str("# Generated by openshell-annotate from @openshell provider directives. Do not edit; regenerate.\n");
        s.push_str("# No secret values here: `--credential KEY` reads KEY from your current environment.\n");
        s.push_str("set -euo pipefail\n");
        s.push_str("ROOT=\"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\"\n\n");
        let mut attach = String::new();
        for p in &self.providers {
            s.push_str(&format!("# {} <- {}\n", p.instance, p.sources.join(", ")));
            if let Some(f) = &p.profile_file {
                let id = p.profile.as_deref().unwrap_or("");
                s.push_str(&format!(
                    "openshell profile describe {id} >/dev/null 2>&1 || openshell profile import -f {} --global\n",
                    root_relative(f, root)
                ));
            }
            let ty = p.profile.as_deref().unwrap_or("");
            let creds: String = p
                .credentials
                .iter()
                .map(|c| format!(" --credential {c}"))
                .collect();
            s.push_str(&format!(
                "openshell provider create --name {} --type {ty}{creds}\n",
                p.instance
            ));
            if !p.profile_endpoints.is_empty() {
                s.push_str(&format!(
                    "# profile contributes endpoints: {}\n",
                    p.profile_endpoints.join(", ")
                ));
            }
            s.push('\n');
            attach.push_str(&format!(" --provider {}", p.instance));
        }
        for u in &self.unmapped {
            s.push_str(&format!(
                "# UNMAPPED: no provider profile declares {} (used at {}).\n#   Write a custom profile (`openshell profile lint -f`), import it, or set [providers.env] in openshell-annotate.toml.\n\n",
                u.env,
                u.sources.join(", ")
            ));
        }
        s.push_str("# Attach at sandbox creation:\n");
        s.push_str(&format!(
            "#   openshell sandbox create{attach} --policy {} -- <command>\n",
            sh_quote(policy_path)
        ));
        s
    }
}

fn sh_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '~' | ':'))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// `"$ROOT/<path relative to the repo root>"` so the committed script has no machine-specific paths.
fn root_relative(f: &Path, root: &Path) -> String {
    let full = f.display().to_string();
    let base = root.display().to_string();
    match full.strip_prefix(&base) {
        Some(rest) if rest.starts_with('/') => {
            let esc: String = rest
                .chars()
                .flat_map(|c| match c {
                    '"' | '$' | '`' | '\\' => vec!['\\', c],
                    _ => vec![c],
                })
                .collect();
            format!("\"$ROOT{esc}\"")
        }
        _ => sh_quote(&full),
    }
}
