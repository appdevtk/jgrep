use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

fn jgrep() -> Command {
    Command::cargo_bin("jgrep").expect("jgrep binary")
}

#[test]
fn extracts_json_field() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("data.json");
    fs::write(&file, r#"{"name":"Alice","age":30}"#).unwrap();

    jgrep()
        .args([".name", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Alice"));
}

#[test]
fn filters_ndjson_documents() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("data.json");
    fs::write(
        &file,
        r#"{"name":"Alice","age":30}
{"name":"Bob","age":15}
"#,
    )
    .unwrap();

    jgrep()
        .args(["select(.age >= 18) | .name", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Alice"))
        .stdout(predicate::str::contains("Bob").not());
}

#[test]
fn streams_ndjson_from_stdin_line_by_line() {
    jgrep()
        .args(["-p", "message"])
        .write_stdin(
            "{\"message\":\"accepted\",\"log\":{\"level\":\"INFO\"}}\n\
             {\"message\":\"payment declined\",\"log\":{\"level\":\"ERROR\"}}\n",
        )
        .assert()
        .success()
        .stdout("accepted\npayment declined\n");
}

#[test]
fn streaming_stdin_count_and_parse_error_after_match() {
    jgrep()
        .args(["-c", "-w", "log.level=ERROR"])
        .write_stdin(
            "{\"message\":\"accepted\",\"log\":{\"level\":\"INFO\"}}\n\
             {\"message\":\"payment declined\",\"log\":{\"level\":\"ERROR\"}}\n",
        )
        .assert()
        .success()
        .stdout("1\n");

    jgrep()
        .args(["-p", "message"])
        .write_stdin("{\"message\":\"accepted\"}\n{\"message\":\n")
        .assert()
        .code(2)
        .stdout("accepted\n")
        .stderr(predicate::str::contains("parse error"));
}

#[test]
fn reads_yaml_with_same_command() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("data.yaml");
    fs::write(&file, "name: Alice\nage: 30\n").unwrap();

    jgrep()
        .args([".name", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout("Alice\n");
}

#[test]
fn defaults_to_identity_filter_for_existing_json_or_yaml_path() {
    let dir = tempdir().unwrap();
    let json = dir.path().join("data.json");
    let yaml = dir.path().join("data.yaml");
    fs::write(&json, r#"{"name":"Alice"}"#).unwrap();
    fs::write(&yaml, "name: Bob\n").unwrap();

    jgrep()
        .arg(json.to_str().unwrap())
        .assert()
        .success()
        .stdout("{\"name\":\"Alice\"}\n");

    jgrep()
        .arg(yaml.to_str().unwrap())
        .assert()
        .success()
        .stdout("{\"name\":\"Bob\"}\n");
}

#[test]
fn defaults_to_identity_filter_for_stdin() {
    jgrep()
        .write_stdin("name: Alice\n")
        .assert()
        .success()
        .stdout("{\"name\":\"Alice\"}\n");
}

#[test]
fn reads_yaml_multi_documents() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("data.yml");
    fs::write(
        &file,
        "---\nname: Alice\nage: 30\n---\nname: Bob\nage: 15\n",
    )
    .unwrap();

    jgrep()
        .args(["select(.age >= 18) | .name", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout("Alice\n");
}

#[test]
fn detects_yaml_from_stdin() {
    jgrep()
        .args([".name"])
        .write_stdin("name: Alice\nage: 30\n")
        .assert()
        .success()
        .stdout("Alice\n");
}

#[test]
fn null_and_false_are_not_matches() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("data.json");
    fs::write(&file, r#"{"name":"Alice"}"#).unwrap();

    jgrep()
        .args([".missing", file.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout("");

    jgrep()
        .args(["false", file.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout("");
}

#[test]
fn zero_empty_string_array_and_object_are_matches() {
    jgrep().args(["-n", "0"]).assert().success().stdout("0\n");
    jgrep()
        .args(["-n", r#""""#])
        .assert()
        .success()
        .stdout("\n");
    jgrep().args(["-n", "[]"]).assert().success().stdout("[]\n");
    jgrep().args(["-n", "{}"]).assert().success().stdout("{}\n");
}

#[test]
fn count_and_files_with_matches() {
    let dir = tempdir().unwrap();
    let a = dir.path().join("a.json");
    let b = dir.path().join("b.json");
    fs::write(&a, "{\"active\":true}\n{\"active\":false}\n").unwrap();
    fs::write(&b, "{\"active\":false}\n").unwrap();

    jgrep()
        .args(["-c", "select(.active == true)", a.to_str().unwrap()])
        .assert()
        .success()
        .stdout("1\n");

    jgrep()
        .args([
            "-l",
            "select(.active == true)",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("a.json"))
        .stdout(predicate::str::contains("b.json").not());
}

#[test]
fn count_and_files_with_matches_cannot_be_combined() {
    jgrep()
        .args(["-cl", "."])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be used together"));
}

#[test]
fn slurp_collects_matches_and_empty_slurp_returns_one() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("data.json");
    fs::write(&file, "{\"v\":1}\n{\"v\":2}\n").unwrap();

    jgrep()
        .args(["-s", ".v", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout("[1,2]\n");

    jgrep()
        .args(["-s", "select(.v > 99)", file.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout("[]\n");
}

#[test]
fn from_file_and_null_input() {
    let dir = tempdir().unwrap();
    let data = dir.path().join("data.json");
    let filter = dir.path().join("filter.jq");
    fs::write(&data, r#"{"name":"Alice","age":30}"#).unwrap();
    fs::write(&filter, ".name").unwrap();

    jgrep()
        .args(["-f", filter.to_str().unwrap(), data.to_str().unwrap()])
        .assert()
        .success()
        .stdout("Alice\n");

    jgrep()
        .args(["-n", "1 + 1"])
        .assert()
        .success()
        .stdout("2\n");
}

#[test]
fn recursive_search_includes_json_and_yaml() {
    let dir = tempdir().unwrap();
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    fs::write(dir.path().join("a.json"), r#"{"v":1}"#).unwrap();
    fs::write(sub.join("b.yaml"), "v: 2\n").unwrap();
    fs::write(sub.join("ignored.txt"), "v: 3\n").unwrap();

    jgrep()
        .args(["-r", ".v", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("a.json:1"))
        .stdout(predicate::str::contains("b.yaml:2"))
        .stdout(predicate::str::contains("ignored").not());
}

#[test]
fn recursive_search_can_use_default_filter() {
    let dir = tempdir().unwrap();
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    fs::write(dir.path().join("a.json"), r#"{"v":1}"#).unwrap();
    fs::write(sub.join("b.yaml"), "v: 2\n").unwrap();

    jgrep()
        .args(["-r", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("a.json:{\"v\":1}"))
        .stdout(predicate::str::contains("b.yaml:{\"v\":2}"));
}

#[test]
fn directory_without_recursive_is_error() {
    let dir = tempdir().unwrap();

    jgrep()
        .args([".name", dir.path().to_str().unwrap()])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Is a directory"));
}

#[test]
fn parse_error_after_match_keeps_output_and_returns_two() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("broken.json");
    fs::write(&file, "{\"name\":\"Alice\"}\n{\"name\":\n").unwrap();

    jgrep()
        .args([".name", file.to_str().unwrap()])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("Alice"))
        .stderr(predicate::str::contains("parse error"));
}

#[test]
fn completion_outputs_supported_shells() {
    jgrep()
        .args(["completion", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete"));

    jgrep()
        .args(["completion", "tcsh"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unsupported shell"));
}

#[test]
fn color_level_can_use_nested_field_and_no_color_disables_it() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("logs.json");
    fs::write(&file, r#"{"app":{"level":"WARN"},"message":"slow"}"#).unwrap();

    jgrep()
        .args([
            "--color-level",
            "--color-level-field",
            "app.level",
            r#""[\(.app.level)] \(.message)""#,
            file.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("[WARN] slow"));

    jgrep()
        .args([
            "--color-level",
            "--no-color",
            r#""[\(.app.level)] \(.message)""#,
            file.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\u{1b}[33m").not());
}

#[test]
fn color_level_short_flag_works_with_default_filter() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("logs.json");
    fs::write(&file, r#"{"level":"ERROR","message":"boom"}"#).unwrap();

    jgrep()
        .args(["-C", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""message":"boom""#));
}

#[test]
fn path_shortcut_extracts_fields_without_jq_syntax() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("data.json");
    fs::write(&file, r#"{"user":{"name":"Alice"},"user.name":"Direct"}"#).unwrap();

    jgrep()
        .args(["-p", "user.name", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout("Direct\n");

    let nested = dir.path().join("nested.json");
    fs::write(&nested, r#"{"user":{"name":"Alice"}}"#).unwrap();
    jgrep()
        .args(["--path", "user.name", nested.to_str().unwrap()])
        .assert()
        .success()
        .stdout("Alice\n");
}

#[test]
fn where_shortcut_filters_without_jq_syntax() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("users.json");
    fs::write(
        &file,
        "{\"name\":\"Alice\",\"age\":30,\"active\":true}\n{\"name\":\"Bob\",\"age\":15,\"active\":false}\n",
    )
    .unwrap();

    jgrep()
        .args([
            "-w",
            "active=true",
            "-w",
            "age>=18",
            "-p",
            "name",
            file.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout("Alice\n");
}

#[test]
fn where_shortcut_supports_dotted_log_keys() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("logs.json");
    fs::write(
        &file,
        "{\"log.level\":\"ERROR\",\"message\":\"boom\"}\n{\"log.level\":\"INFO\",\"message\":\"ok\"}\n",
    )
    .unwrap();

    jgrep()
        .args([
            "-w",
            "log.level=ERROR",
            "-p",
            "message",
            file.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout("boom\n");
}

#[test]
fn explore_prints_schema_and_preview_for_file() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("users.json");
    fs::write(
        &file,
        "{\"name\":\"Alice\",\"status\":\"active\"}\n{\"name\":\"Bob\",\"status\":\"blocked\"}\n",
    )
    .unwrap();

    jgrep()
        .args([
            "explore",
            "--print",
            "--filter",
            "status=active",
            file.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "generated jq: select(.status == \"active\") | .",
        ))
        .stdout(predicate::str::contains("name  string  (2, docs=2"))
        .stdout(predicate::str::contains(
            r#"{"name":"Alice","status":"active"}"#,
        ));
}

#[test]
fn explore_can_combine_filter_output_and_color_options() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("logs.json");
    fs::write(
        &file,
        "{\"log\":{\"level\":\"INFO\"},\"message\":\"ok\"}\n{\"log\":{\"level\":\"ERROR\"},\"message\":\"boom\"}\n",
    )
    .unwrap();

    jgrep()
        .env_remove("NO_COLOR")
        .args([
            "explore",
            "--print",
            "-w",
            "log.level=ERROR",
            "-p",
            "message",
            "-C",
            file.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("output: message"))
        .stdout(predicate::str::contains("generated jq: select("))
        .stdout(predicate::str::contains("preview:\n  \u{1b}[31mboom"))
        .stdout(predicate::str::contains("\u{1b}[31m"))
        .stdout(predicate::str::contains("preview:\n  ok").not());
}

#[test]
fn explore_output_accepts_multi_field_jq_expression() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("logs.json");
    fs::write(
        &file,
        "{\"timestamp\":\"t1\",\"log\":{\"level\":\"ERROR\"},\"message\":\"boom\",\"trace_id\":\"trc-1\"}\n",
    )
    .unwrap();

    jgrep()
        .args([
            "explore",
            "--print",
            "--pretty",
            "-w",
            "log.level=ERROR",
            "-p",
            "{time: .timestamp, level: .log.level, message, trace_id}",
            file.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "output: {time: .timestamp, level: .log.level, message, trace_id}",
        ))
        .stdout(predicate::str::contains("  \"time\": \"t1\""))
        .stdout(predicate::str::contains("  \"trace_id\": \"trc-1\""));
}

#[test]
fn explore_reads_yaml_from_stdin() {
    jgrep()
        .args(["explore", "--print", "--filter", "name"])
        .write_stdin("name: Alice\nage: 30\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("generated jq: .name"))
        .stdout(predicate::str::contains("age  number  (1, docs=1"))
        .stdout(predicate::str::contains("Alice"));
}

#[test]
fn explore_print_returns_error_for_invalid_filter() {
    jgrep()
        .args(["explore", "--print", "--filter", ".["])
        .write_stdin("{\"name\":\"Alice\"}\n")
        .assert()
        .code(2)
        .stdout(predicate::str::contains("error:"));
}

#[test]
fn explore_refuses_input_above_configured_limit() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("data.json");
    fs::write(&file, "{\"name\":\"Alice\"}\n").unwrap();

    jgrep()
        .args([
            "explore",
            "--print",
            "--max-input-bytes",
            "4",
            file.to_str().unwrap(),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--max-input-bytes"));
}
