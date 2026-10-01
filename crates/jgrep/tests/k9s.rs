#![cfg(all(feature = "explore", unix))]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use tempfile::{tempdir, TempDir};

struct Fixture {
    directory: TempDir,
    kubectl: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempdir().unwrap();
        let kubectl = directory.path().join("kubectl stub");
        executable(
            &kubectl,
            r#"#!/bin/sh
printf '%s\n' "$@" > "$TEST_ARGS"
case "$TEST_MODE" in
 error) printf 'Forbidden: test RBAC denial\n' >&2; exit 7;;
 resource) printf '{"metadata":{"name":"checkout-pod"}}\n';;
 idle) printf '%s' "$$" > "$TEST_PID"; printf '{"message":"first"}\n'; exec sleep 60;;
 big) exec cat "$TEST_BIG";;
 *) printf '{"level":"ERROR","message":"structured"}\nplain log line\n';;
esac
"#,
        );
        Self { directory, kubectl }
    }

    fn command(&self, mode: &str, action: &str) -> Command {
        let mut command = Command::new(assert_cmd::cargo::cargo_bin!("jgrep"));
        command
            .args([
                "k9s",
                action,
                "--context=test-context",
                "--namespace=shop",
                "--name=checkout-pod",
            ])
            .arg(format!("--kubectl={}", self.kubectl.display()))
            .arg(format!(
                "--state-dir={}",
                self.directory.path().join("state").display()
            ))
            .env("TEST_ARGS", self.directory.path().join("args"))
            .env("TEST_PID", self.directory.path().join("pid"))
            .env("TEST_MODE", mode);
        command
    }
}

fn executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

// Bound process tests so a regression cannot hang CI indefinitely.
fn output(mut command: Command) -> Output {
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!("bridge timed out: {:?}", output);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

#[test]
fn real_explorer_accepts_mixed_logs_and_resource_json() {
    let fixture = Fixture::new();
    for (mode, action, expected) in [
        ("logs", "logs", "plain log line"),
        ("resource", "resource", "metadata.name"),
    ] {
        let result = output(fixture.command(mode, action));
        assert!(result.status.success(), "{:?}", result);
        assert!(String::from_utf8_lossy(&result.stdout).contains(expected));
        let routing = fs::read_to_string(fixture.directory.path().join("args")).unwrap();
        assert!(routing.contains("--context=test-context\n--namespace=shop\n"));
    }
}

#[test]
fn producer_failure_is_not_hidden_by_viewer_success() {
    let fixture = Fixture::new();
    let result = output(fixture.command("error", "logs"));
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("Forbidden"));
}

#[test]
fn resource_and_log_limits_are_errors() {
    let fixture = Fixture::new();
    for action in ["logs", "resource"] {
        let mut command = fixture.command(action, action);
        command.arg("--max-input-bytes=4");
        let result = output(command);
        assert_eq!(result.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&result.stderr).contains("limit"));
    }
}

#[test]
fn oversized_log_line_is_rejected() {
    let fixture = Fixture::new();
    let file = fixture.directory.path().join("large-line");
    fs::write(&file, vec![b'x'; 1024 * 1024 + 1]).unwrap();
    let mut command = fixture.command("big", "logs");
    command.env("TEST_BIG", file);
    let result = output(command);
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("log line exceeds 1 MiB"));
}

#[test]
fn closing_viewer_kills_and_reaps_idle_producer() {
    let fixture = Fixture::new();
    let viewer = fixture.directory.path().join("viewer stub");
    executable(&viewer, "#!/bin/sh\nIFS= read -r line\n");
    let mut command = fixture.command("idle", "logs");
    command.arg(format!("--jgrep={}", viewer.display()));
    let started = Instant::now();
    let result = output(command);
    assert!(result.status.success(), "{:?}", result);
    assert!(started.elapsed() < Duration::from_secs(5));
    let pid = fs::read_to_string(fixture.directory.path().join("pid")).unwrap();
    assert!(!Command::new("kill")
        .args(["-0", pid.trim()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success());
}

#[test]
fn native_installer_preserves_config_and_refuses_overwrite() {
    let fixture = Fixture::new();
    let config = fixture.directory.path().join("config with spaces");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("plugins.yaml"), "plugins: {}\n").unwrap();
    let install = || {
        let mut command = Command::new(assert_cmd::cargo::cargo_bin!("jgrep"));
        command
            .args(["k9s", "install", "--config-dir"])
            .arg(&config)
            .arg("--kubectl")
            .arg(&fixture.kubectl);
        output(command)
    };
    let result = install();
    assert!(result.status.success(), "{:?}", result);
    let target = config.join("plugins/jgrep");
    assert_eq!(
        fs::read_to_string(config.join("plugins.yaml")).unwrap(),
        "plugins: {}\n"
    );
    let yaml = fs::read_to_string(target.join("plugins.yaml")).unwrap();
    assert!(!yaml.contains("{{") && !yaml.contains("python"));
    assert!(yaml.contains("jgrep-container-logs"));
    let shortcuts = yaml
        .lines()
        .filter_map(|line| line.trim().strip_prefix("shortCut: "))
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        shortcuts.len(),
        3,
        "each managed action should have a distinct shortcut"
    );
    assert!(yaml.contains("override: true"));
    #[cfg(feature = "yaml")]
    {
        let mut command = Command::new(target.join("jgrep"));
        command
            .arg(".plugins | keys")
            .arg(target.join("plugins.yaml"));
        let result = output(command);
        assert!(result.status.success(), "{:?}", result);
        assert!(String::from_utf8_lossy(&result.stdout).contains("jgrep-resource"));
    }
    assert_eq!(
        fs::metadata(target.join("plugins.yaml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(target.join("state"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let result = install();
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("refusing to overwrite"));
    let result = output({
        let mut command = Command::new(target.join("jgrep"));
        command.args(["k9s", "--help"]);
        command
    });
    assert!(result.status.success());
}

#[test]
fn explicit_update_backs_up_snapshot_and_preserves_private_state() {
    let fixture = Fixture::new();
    let config = fixture.directory.path().join("config");
    let target = config.join("plugins/jgrep");
    fs::create_dir_all(target.join("state")).unwrap();
    fs::write(target.join("jgrep"), "old binary").unwrap();
    fs::write(target.join("plugins.yaml"), "old plugin").unwrap();
    fs::write(target.join("state/saved-filter.jq"), ".message\n").unwrap();
    fs::write(config.join("plugins.yaml"), "plugins: {}\n").unwrap();
    let result = output({
        let mut command = Command::new(assert_cmd::cargo::cargo_bin!("jgrep"));
        command
            .args(["k9s", "install", "--update", "--config-dir"])
            .arg(&config)
            .arg("--kubectl")
            .arg(&fixture.kubectl);
        command
    });
    assert!(result.status.success(), "{result:?}");
    let backups = fs::read_dir(config.join("jgrep-backups"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read(backups[0].join("jgrep")).unwrap(), b"old binary");
    assert_eq!(
        fs::read_to_string(target.join("state/saved-filter.jq")).unwrap(),
        ".message\n"
    );
    assert_eq!(
        fs::metadata(target.join("state/saved-filter.jq"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::read_to_string(config.join("plugins.yaml")).unwrap(),
        "plugins: {}\n"
    );
    assert_eq!(fs::read_dir(config.join("plugins")).unwrap().count(), 1);
}

#[test]
fn update_preflight_failure_does_not_touch_existing_snapshot() {
    let fixture = Fixture::new();
    let config = fixture.directory.path().join("config");
    let target = config.join("plugins/jgrep");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("jgrep"), "old binary").unwrap();
    let invalid_binary = fixture.directory.path().join("invalid");
    executable(&invalid_binary, "#!/bin/sh\nexit 1\n");
    let result = output({
        let mut command = Command::new(assert_cmd::cargo::cargo_bin!("jgrep"));
        command
            .args(["k9s", "install", "--update", "--config-dir"])
            .arg(&config)
            .arg("--jgrep-bin")
            .arg(&invalid_binary)
            .arg("--kubectl")
            .arg(&fixture.kubectl);
        command
    });
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(fs::read(target.join("jgrep")).unwrap(), b"old binary");
    assert!(!config.join("jgrep-backups").exists());
}

#[test]
fn update_refuses_symlinked_plugin_or_state_without_replacing_it() {
    let fixture = Fixture::new();
    let config = fixture.directory.path().join("config");
    let target = config.join("plugins/jgrep");
    let outside = fixture.directory.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("sentinel"), "preserve").unwrap();
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&outside, &target).unwrap();
    let update = || {
        let mut command = Command::new(assert_cmd::cargo::cargo_bin!("jgrep"));
        command
            .args(["k9s", "install", "--update", "--config-dir"])
            .arg(&config)
            .arg("--kubectl")
            .arg(&fixture.kubectl);
        output(command)
    };
    assert_eq!(update().status.code(), Some(2));
    assert!(target.symlink_metadata().unwrap().file_type().is_symlink());
    fs::remove_file(&target).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(target.join("jgrep"), "old binary").unwrap();
    std::os::unix::fs::symlink(&outside, target.join("state")).unwrap();
    assert_eq!(update().status.code(), Some(2));
    assert_eq!(fs::read(target.join("jgrep")).unwrap(), b"old binary");
    assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
}

#[test]
fn macos_build_helper_routes_sdk_and_installs_without_host_rust() {
    let fixture = Fixture::new();
    let root = fixture.directory.path();
    let tools = root.join("tools");
    fs::create_dir(&tools).unwrap();
    fs::create_dir(root.join("scripts")).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/build-macos.sh");
    fs::copy(script, root.join("scripts/build-macos.sh")).unwrap();
    let sdk = root.join("sdk with spaces");
    fs::create_dir_all(sdk.join("usr/lib")).unwrap();
    fs::write(sdk.join("usr/lib/libSystem.tbd"), "synthetic").unwrap();
    executable(
        &tools.join("uname"),
        "#!/bin/sh\ncase \"$1\" in -s) echo Darwin;; -m) echo arm64;; esac\n",
    );
    executable(&tools.join("docker"), "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$TEST_ARGS\"\nmkdir -p \"$TEST_ROOT/dist/macos-arm64\"\ncp \"$TEST_BINARY\" \"$TEST_ROOT/dist/macos-arm64/jgrep\"\n");
    for name in ["k9s", "kubectl"] {
        executable(&tools.join(name), "#!/bin/sh\nexit 0\n");
    }
    // Any accidental use of host Rust or Python is a test failure.
    for name in ["cargo", "rustc", "python3", "brew"] {
        executable(&tools.join(name), "#!/bin/sh\nexit 99\n");
    }
    let config = root.join("config with spaces");
    let mut command = Command::new("bash");
    command
        .arg(root.join("scripts/build-macos.sh"))
        .args(["--install", "--config-dir"])
        .arg(&config)
        .env("PATH", format!("{}:/usr/bin:/bin", tools.display()))
        .env("JGREP_MACOS_SDK", &sdk)
        .env("TEST_ROOT", root)
        .env("TEST_ARGS", root.join("args"))
        .env("TEST_BINARY", assert_cmd::cargo::cargo_bin!("jgrep"));
    let result = output(command);
    assert!(result.status.success(), "{result:?}");
    let args = fs::read_to_string(root.join("args")).unwrap();
    assert!(args.contains("MACOS_TARGET=aarch64-apple-darwin\n"));
    assert!(args.contains(&format!("macos-sdk={}\n", sdk.display())));
    assert!(config.join("plugins/jgrep/jgrep").is_file());
}
