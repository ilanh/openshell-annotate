//! `generate --image IMAGE`: propose `fs read` directives for application paths in a container image.
//!
//! The OpenShell supervisor only grants read access to a fixed baseline (`/usr /lib /etc /app /var/log /proc
//! /dev/urandom`, plus `/tmp` and `/dev/null` for writing, verified in v0.1.6 `PROXY_BASELINE_*`). An app that lives
//! elsewhere (`/opt/context7`) fails at startup with EACCES, and OpenShell 0.1.6 does not log filesystem denials
//! (upstream #4270), so neither the logs nor `diff-logs` can point at it. The image says where the app lives: its
//! WORKDIR, ENTRYPOINT and CMD.

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

/// Paths the supervisor makes readable (or writable) without being asked.
pub const BASELINE: &[&str] = &[
    "/usr", "/lib", "/lib64", "/bin", "/sbin", "/etc", "/app", "/var/log", "/proc", "/dev", "/tmp",
    "/sandbox",
];

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImageInfo {
    pub workdir: Option<String>,
    pub entrypoint: Vec<String>,
    pub cmd: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    /// Directory to make readable.
    pub path: String,
    /// Where in the image it came from.
    pub why: String,
}

/// Parse the output of `docker image inspect` (a JSON array with one object).
pub fn parse_inspect(json: &str) -> Result<ImageInfo> {
    let v: serde_json::Value =
        serde_json::from_str(json).context("parsing `image inspect` output")?;
    let cfg = v
        .get(0)
        .unwrap_or(&v)
        .get("Config")
        .context("`image inspect` output has no Config")?;
    let strings = |k: &str| -> Vec<String> {
        cfg.get(k)
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok(ImageInfo {
        workdir: cfg
            .get("WorkingDir")
            .and_then(|w| w.as_str())
            .filter(|w| !w.is_empty())
            .map(str::to_string),
        entrypoint: strings("Entrypoint"),
        cmd: strings("Cmd"),
    })
}

/// `docker image inspect` (or `podman image inspect` when docker is not installed).
pub fn inspect(image: &str) -> Result<ImageInfo> {
    let mut last = String::new();
    for tool in ["docker", "podman"] {
        match Command::new(tool)
            .args(["image", "inspect", image])
            .output()
        {
            Ok(o) if o.status.success() => {
                return parse_inspect(&String::from_utf8_lossy(&o.stdout));
            }
            Ok(o) => {
                bail!(
                    "`{tool} image inspect {image}` failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                );
            }
            Err(e) => last = format!("{tool}: {e}"),
        }
    }
    bail!("cannot inspect image {image}: no container tool found ({last})")
}

fn under(path: &str, root: &str) -> bool {
    let root = root.trim_end_matches('/');
    path == root || path.starts_with(&format!("{root}/"))
}

/// The directory that holds an application: `/opt/app` for `/opt/app/dist/index.js`, else the parent directory.
fn app_root(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    match parts.first() {
        Some(&("opt" | "srv" | "home" | "var" | "mnt" | "data")) if parts.len() >= 2 => {
            format!("/{}/{}", parts[0], parts[1])
        }
        _ if parts.len() >= 2 => format!("/{}", parts[..parts.len() - 1].join("/")),
        _ => path.to_string(),
    }
}

/// Directories to make readable: image paths outside the baseline and outside the `declared` `fs read|write` roots.
/// The image WORKDIR is the sandbox workspace, which `[policy].include_workdir` already grants.
pub fn propose(info: &ImageInfo, declared: &[String], include_workdir: bool) -> Vec<Proposal> {
    let mut out: Vec<Proposal> = Vec::new();
    let mut add = |path: String, why: String| {
        let covered =
            BASELINE.iter().any(|b| under(&path, b)) || declared.iter().any(|d| under(&path, d));
        if covered {
            return;
        }
        if let Some(p) = out.iter_mut().find(|p| p.path == path) {
            if !p.why.contains(&why) {
                p.why.push_str(&format!(", {why}"));
            }
        } else {
            out.push(Proposal { path, why });
        }
    };
    if let (Some(w), false) = (&info.workdir, include_workdir) {
        add(w.trim_end_matches('/').to_string(), "WORKDIR".into());
    }
    for (kind, args) in [("ENTRYPOINT", &info.entrypoint), ("CMD", &info.cmd)] {
        for a in args
            .iter()
            .filter(|a| a.starts_with('/') && !a.contains(' '))
        {
            add(app_root(a), format!("{kind} {a}"));
        }
    }
    out
}

/// Top-level layout of the image filesystem that holds application files: `/opt/<x>`, `/srv/<x>`, `/mnt/<x>`, `/data/<x>` and
/// any unusual top-level directory. Input: `tar -t` lines of `docker export`.
pub fn app_dirs_from_listing(listing: &str) -> Vec<String> {
    const STANDARD: &[&str] = &[
        "bin",
        "boot",
        "dev",
        "etc",
        "home",
        "lib",
        "lib32",
        "lib64",
        "libx32",
        "media",
        "proc",
        "root",
        "run",
        "sbin",
        "sys",
        "tmp",
        "usr",
        "var",
        "app",
        "sandbox",
        "workspace",
        "opt",
        "srv",
        "mnt",
        "data",
    ];
    let mut out: Vec<String> = Vec::new();
    for line in listing.lines() {
        let parts: Vec<&str> = line
            .trim_start_matches("./")
            .split('/')
            .filter(|p| !p.is_empty())
            .collect();
        let found = match parts.as_slice() {
            [top, second, ..] if matches!(*top, "opt" | "srv" | "mnt" | "data") => {
                Some(format!("/{top}/{second}"))
            }
            [top] | [top, ..] if !STANDARD.contains(top) && !top.starts_with('.') => {
                Some(format!("/{top}"))
            }
            _ => None,
        };
        if let Some(f) = found {
            if !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out.sort();
    out
}

/// List the application directories of an image by exporting a throw-away container filesystem (`docker create` +
/// `docker export | tar -t`); nothing is started. Errors are returned so the caller can fall back to the metadata only.
pub fn list_app_dirs(image: &str) -> Result<Vec<String>> {
    let tool = ["docker", "podman"]
        .into_iter()
        .find(|t| Command::new(t).arg("--version").output().is_ok())
        .context("no container tool found")?;
    let created = Command::new(tool).args(["create", image]).output()?;
    if !created.status.success() {
        bail!(
            "`{tool} create {image}` failed: {}",
            String::from_utf8_lossy(&created.stderr).trim()
        );
    }
    let id = String::from_utf8_lossy(&created.stdout).trim().to_string();
    let listing = Command::new("sh")
        .arg("-c")
        .arg(format!("{tool} export {id} | tar -t"))
        .output();
    let _ = Command::new(tool).args(["rm", "-f", &id]).output();
    let listing = listing?;
    if !listing.status.success() {
        bail!("`{tool} export` failed");
    }
    Ok(app_dirs_from_listing(&String::from_utf8_lossy(
        &listing.stdout,
    )))
}

/// Add listed image directories that are neither in the baseline nor under a declared path.
pub fn add_dirs(props: &mut Vec<Proposal>, dirs: Vec<String>, declared: &[String]) {
    for d in dirs {
        let covered =
            BASELINE.iter().any(|b| under(&d, b)) || declared.iter().any(|x| under(&d, x));
        if !covered && !props.iter().any(|p| p.path == d) {
            props.push(Proposal {
                path: d,
                why: "found in the image filesystem".into(),
            });
        }
    }
}

pub fn render_note(image: &str, props: &[Proposal]) -> String {
    let mut s = format!(
        "note: image {image} keeps files outside OpenShell's baseline read set ({}); the sandbox cannot read them, and OpenShell 0.1.6 does not log that denial (#4270):\n",
        ["/usr", "/lib", "/etc", "/app", "/var/log", "/proc", "/dev/urandom"].join(" ")
    );
    for p in props {
        s.push_str(&format!("  {}  ({})\n", p.path, p.why));
    }
    s.push_str("  add, next to the code that needs them (or in a sidecar file):\n");
    for p in props {
        s.push_str(&format!(
            "    # @openshell fs read {} reason=\"application files in the image ({})\"\n",
            p.path, p.why
        ));
    }
    s
}

/// Load the image info from a file instead of a container tool (used by tests and offline CI).
pub fn from_file(path: &Path) -> Result<ImageInfo> {
    parse_inspect(
        &std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_roots() {
        assert_eq!(
            app_root("/opt/context7/packages/mcp/dist/index.js"),
            "/opt/context7"
        );
        assert_eq!(app_root("/srv/app"), "/srv/app");
        assert_eq!(app_root("/usr/local/bin/node"), "/usr/local/bin");
        assert_eq!(app_root("/entry.sh"), "/entry.sh");
    }

    #[test]
    fn listing_gives_app_dirs_outside_the_standard_layout() {
        let l = "etc/\nopt/\nopt/context7/\nopt/context7/packages/mcp/dist/index.js\nopt/other/x\nusr/local/bin/node\nworkspace/\nsrv/site/index.html\nfoo/bar\n.dockerenv\n";
        assert_eq!(
            app_dirs_from_listing(l),
            vec!["/foo", "/opt/context7", "/opt/other", "/srv/site"]
        );
    }

    #[test]
    fn baseline_and_declared_paths_are_not_proposed() {
        let info = ImageInfo {
            workdir: Some("/opt/context7".into()),
            entrypoint: vec!["/usr/local/bin/node".into()],
            cmd: vec![
                "/opt/context7/packages/mcp/dist/index.js".into(),
                "--transport".into(),
            ],
        };
        let p = propose(&info, &[], false);
        assert_eq!(p.len(), 1, "{p:?}");
        assert_eq!(p[0].path, "/opt/context7");
        assert!(
            p[0].why.contains("WORKDIR") && p[0].why.contains("CMD"),
            "{p:?}"
        );
        assert!(propose(&info, &["/opt/context7".into()], false).is_empty());
        assert!(propose(&info, &["/opt".into()], false).is_empty());
        // the workspace is covered by include_workdir; the script path still needs a declaration
        let p = propose(&info, &[], true);
        assert_eq!(p.len(), 1);
        assert!(!p[0].why.contains("WORKDIR"), "{p:?}");
    }
}
