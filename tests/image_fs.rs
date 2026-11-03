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

#[allow(dead_code)]
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

const INSPECT: &str = r#"[{"Config":{"WorkingDir":"/workspace","Entrypoint":["docker-entrypoint.sh"],"Cmd":["/opt/context7/packages/mcp/dist/index.js"]}}]"#;

fn project(dir: &Path, fs_path: &str) {
    write(
        dir,
        "src/a.ts",
        &format!(
            "// @openshell egress demo-site.io:443 reason=\"api\" bin=/usr/local/bin/node\n// @openshell fs write {fs_path} reason=\"scratch\"\n"
        ),
    );
    write(dir, "inspect.json", INSPECT);
}

#[test]
fn proposes_image_paths_outside_the_baseline() {
    let t = tempfile::tempdir().unwrap();
    project(t.path(), "/tmp/x");
    let o = run(
        t.path(),
        &[
            "generate",
            "--image",
            "ctx:local",
            "--image-inspect",
            "inspect.json",
        ],
    );
    assert!(o.status.success(), "{}", err(&o));
    let e = err(&o);
    assert!(e.contains("# @openshell fs read /opt/context7"), "{e}");
    assert!(e.contains("#4270"), "{e}");
    // nothing is added to the policy by itself
    assert!(!out(&o).contains("/opt/context7"), "{}", out(&o));
}

#[test]
fn declared_fs_root_silences_the_proposal() {
    let t = tempfile::tempdir().unwrap();
    project(t.path(), "/opt/context7");
    let o = run(
        t.path(),
        &[
            "generate",
            "--image",
            "ctx:local",
            "--image-inspect",
            "inspect.json",
        ],
    );
    assert!(o.status.success(), "{}", err(&o));
    assert!(!err(&o).contains("@openshell fs read"), "{}", err(&o));
}

#[test]
fn no_image_flag_no_note() {
    let t = tempfile::tempdir().unwrap();
    project(t.path(), "/tmp/x");
    let o = run(t.path(), &["generate"]);
    assert!(!err(&o).contains("image"), "{}", err(&o));
}
