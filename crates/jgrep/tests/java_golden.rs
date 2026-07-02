use assert_cmd::Command;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use tempfile::tempdir;

fn rust_jgrep() -> Command {
    Command::cargo_bin("jgrep").expect("jgrep binary")
}

fn java_runner(module: &str) -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)?
        .join(module)
        .join("target/quarkus-app/quarkus-run.jar");
    path.exists().then_some(path)
}

fn run_java(module: &str, args: &[&str]) -> Option<(i32, String, String)> {
    let jar = java_runner(module)?;
    let output = StdCommand::new("java")
        .arg("-jar")
        .arg(jar)
        .args(args)
        .output()
        .ok()?;
    Some((
        output.status.code().unwrap_or(255),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

fn run_rust(args: &[&str]) -> (i32, String, String) {
    let output = rust_jgrep().args(args).output().expect("rust output");
    (
        output.status.code().unwrap_or(255),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
#[ignore = "requires Maven-built Java quarkus-run.jar artifacts"]
fn json_behavior_matches_java_jgrep() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("data.json");
    fs::write(
        &file,
        "{\"name\":\"Alice\",\"age\":30}\n{\"name\":\"Bob\",\"age\":15}\n",
    )
    .unwrap();

    for args in [
        vec![".name", file.to_str().unwrap()],
        vec!["select(.age >= 18) | .name", file.to_str().unwrap()],
        vec!["-s", ".age", file.to_str().unwrap()],
        vec!["-c", "select(.age >= 18)", file.to_str().unwrap()],
    ] {
        let java = run_java("jgrep", &args).expect("java jgrep artifact");
        let rust = run_rust(&args);
        assert_eq!(rust.0, java.0, "exit code for {args:?}");
        assert_eq!(rust.1, java.1, "stdout for {args:?}");
    }
}

#[test]
#[ignore = "requires Maven-built Java quarkus-run.jar artifacts"]
fn yaml_behavior_matches_java_ygrep() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("data.yaml");
    fs::write(
        &file,
        "---\nname: Alice\nage: 30\n---\nname: Bob\nage: 15\n",
    )
    .unwrap();

    for args in [
        vec![".name", file.to_str().unwrap()],
        vec!["select(.age >= 18) | .name", file.to_str().unwrap()],
        vec!["-s", ".age", file.to_str().unwrap()],
        vec!["-c", "select(.age >= 18)", file.to_str().unwrap()],
    ] {
        let java = run_java("ygrep", &args).expect("java ygrep artifact");
        let rust = run_rust(&args);
        assert_eq!(rust.0, java.0, "exit code for {args:?}");
        assert_eq!(rust.1, java.1, "stdout for {args:?}");
    }
}
