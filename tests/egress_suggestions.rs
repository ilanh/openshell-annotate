//! Lint egress suggestions are valid syntax; the AI skill examples parse.
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
fn lint_suggests_one_directive_per_host_and_they_parse() {
    let t = tempfile::tempdir().unwrap();
    write(
        t.path(),
        "a.py",
        "import requests\nrequests.get(\"https://alpha.example.com/a?next=https://beta.example.com/b\")\n",
    );
    let c = run(t.path(), &["lint", "--format", "compact"]);
    let c = out(&c) + &err(&c);
    assert!(
        c.contains("(suggest: @openshell egress alpha.example.com:443 reason=\"...\"; @openshell egress beta.example.com:443 reason=\"...\")"),
        "{c}"
    );
    assert!(!c.contains("443, beta"), "{c}");
    let h = run(t.path(), &["lint", "--color", "never", "--no-icons"]);
    let h = out(&h) + &err(&h);
    assert!(h.contains("one directive per host"), "{h}");
    assert!(
        h.contains("# @openshell egress alpha.example.com:443 reason=\"...\"\n"),
        "{h}"
    );
    assert!(
        h.contains("# @openshell egress beta.example.com:443 reason=\"...\"\n"),
        "{h}"
    );

    // the suggested lines are valid directives
    write(
        t.path(),
        "b.py",
        "# @openshell egress alpha.example.com:443 reason=\"x\" bin=/usr/bin/python3\n# @openshell egress beta.example.com:443 reason=\"x\" bin=/usr/bin/python3\n",
    );
    let g = run(t.path(), &["scan", "--format", "compact", "b.py"]);
    assert!(g.status.success(), "{}", err(&g));
}

#[test]
fn every_directive_in_the_skill_examples_parses() {
    // Fenced ts/python/go/sh blocks in the AI skill must be valid directives, so the skill cannot drift from the grammar.
    let md = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/annotate-openshell-policy/SKILL.md"),
    )
    .unwrap();
    let t = tempfile::tempdir().unwrap();
    let mut n = 0;
    let mut cur: Option<(&str, String)> = None;
    for line in md.lines() {
        if let Some(lang) = line.strip_prefix("```") {
            match cur.take() {
                Some((ext, body)) => {
                    write(t.path(), &format!("ex{n}.{ext}"), &body);
                    n += 1;
                }
                None => {
                    let ext = match lang.trim() {
                        "ts" => "ts",
                        "python" => "py",
                        "go" => "go",
                        "sh" => "sh",
                        _ => "",
                    };
                    if !ext.is_empty() {
                        cur = Some((ext, String::new()));
                    } else {
                        cur = None;
                    }
                }
            }
            continue;
        }
        if let Some((_, body)) = cur.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    assert!(n >= 4, "expected the skill's example blocks, found {n}");
    let o = run(t.path(), &["scan", "--format", "compact"]);
    assert!(
        o.status.success(),
        "skill example does not parse: {}",
        err(&o)
    );
    assert!(
        out(&o).contains("files.pythonhosted.org:443"),
        "{}",
        out(&o)
    );
}
