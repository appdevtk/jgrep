#![cfg(all(unix, feature = "explore"))]

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

fn plugin_script(name: &str, state: &Path) -> Command {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/herdr-jgrep");
    let mut command = Command::new("sh");
    command
        .arg(root.join("scripts").join(name))
        .env("HERDR_PLUGIN_ROOT", &root)
        .env("HERDR_PLUGIN_STATE_DIR", state)
        .env("HERDR_PLUGIN_CONFIG_DIR", state.join("config"))
        .env("JGREP_BIN", assert_cmd::cargo::cargo_bin!("jgrep"))
        .env("JGREP_MAX_INPUT_BYTES", "52428800")
        .env("JGREP_MAX_SCHEMA_DOCUMENTS", "500")
        .env("JGREP_MAX_PREVIEW_RESULTS", "30")
        .env("JGREP_LAST_FILTER_FILE", state.join("last-filter.jq"))
        .env("JGREP_FILTER_HISTORY_FILE", state.join("history"))
        .env_remove("HERDR_PLUGIN_CONTEXT_JSON");
    command
}

#[test]
fn ignores_legacy_temporary_configuration_and_creates_private_defaults() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempdir().unwrap();
    let shared = tempdir().unwrap();
    let legacy = shared.path().join("jgrep-herdr/config");
    fs::create_dir_all(&legacy).unwrap();
    let marker = home.path().join("executed");
    let malicious = shared.path().join("fake-jgrep");
    fs::write(
        &malicious,
        format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&malicious, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        legacy.join("config.toml"),
        format!("jgrep_bin = \"{}\"\n", malicious.display()),
    )
    .unwrap();
    let input = home.path().join("safe.json");
    fs::write(&input, r#"{"name":"Alice"}"#).unwrap();
    plugin_script("open-explorer.sh", home.path())
        .env_remove("HERDR_PLUGIN_STATE_DIR")
        .env_remove("HERDR_PLUGIN_CONFIG_DIR")
        .env_remove("JGREP_BIN")
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env("HOME", home.path())
        .env("TMPDIR", shared.path())
        .env(
            "PATH",
            format!(
                "{}:{}",
                Path::new(assert_cmd::cargo::cargo_bin!("jgrep"))
                    .parent()
                    .unwrap()
                    .display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .arg(input)
        .assert()
        .success()
        .stdout(predicate::str::contains("Alice"));
    assert!(!marker.exists());
    assert_eq!(
        fs::metadata(home.path().join(".local/state/jgrep/herdr"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn rejects_symlink_state_and_writable_ancestors_before_launch() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let dir = tempdir().unwrap();
    let alias = dir.path().join("alias");
    symlink(dir.path(), &alias).unwrap();
    plugin_script("open-explorer.sh", &alias)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unsafe plugin path"));
    let shared = dir.path().join("shared");
    fs::create_dir(&shared).unwrap();
    fs::set_permissions(&shared, fs::Permissions::from_mode(0o777)).unwrap();
    plugin_script("filter-selection.sh", &shared.join("state"))
        .write_stdin("{}")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("writable ancestor"));
}

#[test]
fn configuration_authority_is_checked_in_every_config_consumer() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let dir = tempdir().unwrap();
    let config = dir.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o700)).unwrap();
    let file = config.join("config.toml");
    fs::write(
        &file,
        format!(
            "jgrep_bin = \"{}\"\n",
            assert_cmd::cargo::cargo_bin!("jgrep").display()
        ),
    )
    .unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o666)).unwrap();
    for script in [
        "open-explorer.sh",
        "open-logs.sh",
        "filter-selection.sh",
        "install-completion.sh",
    ] {
        plugin_script(script, dir.path())
            .env_remove("JGREP_BIN")
            .assert()
            .code(2)
            .stderr(predicate::str::contains(
                "untrusted owner/write permissions",
            ));
    }
    fs::remove_file(&file).unwrap();
    let target = dir.path().join("other.toml");
    fs::write(&target, "jgrep_bin = \"jgrep\"\n").unwrap();
    symlink(&target, &file).unwrap();
    plugin_script("install-completion.sh", dir.path())
        .env_remove("JGREP_BIN")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("symlink"));
    fs::remove_file(&file).unwrap();
    fs::write(
        &file,
        format!(
            "jgrep_bin = \"{}\"\n",
            assert_cmd::cargo::cargo_bin!("jgrep").display()
        ),
    )
    .unwrap();
    plugin_script("filter-selection.sh", dir.path())
        .env_remove("JGREP_BIN")
        .write_stdin(r#"{"name":"legitimate"}"#)
        .assert()
        .success()
        .stdout(predicate::str::contains("legitimate"));
}

#[test]
fn opens_explicit_file_and_filters_error_logs() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("logs.ndjson");
    fs::write(&input, "{\"message\":\"accepted\",\"log.level\":\"INFO\"}\n{\"message\":\"declined\",\"log.level\":\"ERROR\"}\n").unwrap();

    plugin_script("open-explorer.sh", dir.path())
        .arg(&input)
        .assert()
        .success()
        .stdout(predicate::str::contains("accepted"))
        .stdout(predicate::str::contains("declined"));

    plugin_script("open-logs.sh", dir.path())
        .arg(&input)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "preview:\n  {\"message\":\"declined\"",
        ));
}

#[test]
fn selection_from_stdin_is_private_and_empty_selection_is_rejected() {
    let dir = tempdir().unwrap();
    plugin_script("filter-selection.sh", dir.path())
        .write_stdin("{\"name\":\"Alice\"}\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Alice"));
    let selection = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("selection-")
        })
        .unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(selection).unwrap().permissions().mode() & 0o777,
        0o600
    );

    plugin_script("filter-selection.sh", dir.path())
        .write_stdin("")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("selected text is empty"));
}

#[test]
fn reads_selected_path_and_text_from_herdr_context() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("data.json");
    fs::write(&input, "{\"name\":\"Alice\"}\n").unwrap();
    plugin_script("open-explorer.sh", dir.path())
        .env(
            "HERDR_PLUGIN_CONTEXT_JSON",
            format!(r#"{{"pane":{{"selected_path":"{}"}}}}"#, input.display()),
        )
        .assert()
        .success()
        .stdout(predicate::str::contains("Alice"));
    plugin_script("filter-selection.sh", dir.path())
        .env(
            "HERDR_PLUGIN_CONTEXT_JSON",
            r#"{"pane":{"selected_text":"{\"name\":\"Bob\"}"}}"#,
        )
        .assert()
        .success()
        .stdout(predicate::str::contains("Bob"));
}

#[test]
fn reports_missing_binary_and_oversized_input() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("data.json");
    fs::write(&input, "{\"name\":\"Alice\"}\n").unwrap();
    plugin_script("open-explorer.sh", dir.path())
        .arg(&input)
        .env("JGREP_BIN", dir.path().join("missing-binary"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot find jgrep"));
    plugin_script("open-explorer.sh", dir.path())
        .arg(input)
        .env("JGREP_MAX_INPUT_BYTES", "4")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("larger than max_input_bytes"));
}

#[test]
fn prints_last_filter_and_reports_missing_filter() {
    let dir = tempdir().unwrap();
    plugin_script("print-last-filter.sh", dir.path())
        .assert()
        .code(1);
    fs::write(dir.path().join("last-filter.jq"), ".name\n").unwrap();
    plugin_script("print-last-filter.sh", dir.path())
        .assert()
        .success()
        .stdout(".name\n");
}

#[cfg(feature = "completion")]
#[test]
fn completion_installer_prints_commands() {
    let dir = tempdir().unwrap();
    plugin_script("install-completion.sh", dir.path())
        .env("JGREP_COMPLETION_INSTALL", "print")
        .assert()
        .success()
        .stdout(predicate::str::contains("completion bash"))
        .stdout(predicate::str::contains("completion zsh"))
        .stdout(predicate::str::contains("completion fish"));
}
