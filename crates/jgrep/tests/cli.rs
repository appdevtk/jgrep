use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

fn jgrep() -> Command {
    assert_cmd::cargo::cargo_bin_cmd!("jgrep")
}

#[test]
fn seeded_where_output_count_and_slurp_matrix_for_files_and_stdin() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("random.ndjson");
    let mut seed = 0x4a67726570_u64;
    for _case in 0..48 {
        let mut random = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            seed >> 32
        };
        let count = random() % 60 + 1;
        let threshold = random() % 100;
        let mut data = String::new();
        let mut expected = Vec::new();
        for index in 0..count {
            let score = random() % 100;
            data.push_str(&format!("{{\"score\":{score},\"message\":\"m{index}\"}}\n"));
            if score >= threshold {
                expected.push(format!("m{index}"));
            }
        }
        fs::write(&path, &data).unwrap();
        for stdin in [false, true] {
            for mode in ["plain", "count", "slurp"] {
                let mut command = jgrep();
                command.args([
                    "--no-color",
                    "-w",
                    &format!("score>={threshold}"),
                    "-p",
                    "message",
                ]);
                let output = match mode {
                    "count" => {
                        command.arg("-c");
                        format!("{}\n", expected.len())
                    }
                    "slurp" => {
                        command.arg("-s");
                        format!(
                            "[{}]\n",
                            expected
                                .iter()
                                .map(|s| format!("\"{s}\""))
                                .collect::<Vec<_>>()
                                .join(",")
                        )
                    }
                    _ => expected.iter().map(|s| format!("{s}\n")).collect(),
                };
                if stdin {
                    command.write_stdin(data.clone());
                } else {
                    command.arg(&path);
                }
                command
                    .assert()
                    .code(if expected.is_empty() { 1 } else { 0 })
                    .stdout(output);
            }
        }
    }
}

#[test]
fn where_values_containing_operators_and_non_json_numbers_are_strings() {
    for value in ["a!=b", "age>=18", "NaN", "inf", "+1", "01"] {
        jgrep()
            .args(["-w", &format!("message={value}"), "-p", "message"])
            .write_stdin(format!(
                "{{\"message\":\"{value}\"}}\n{{\"message\":\"other\"}}\n"
            ))
            .assert()
            .success()
            .stdout(format!("{value}\n"));
    }
    jgrep()
        .args(["-w", "level==ERROR", "-p", "message"])
        .write_stdin("{\"level\":\"ERROR\",\"message\":\"found\"}\n")
        .assert()
        .success()
        .stdout("found\n");
}

#[test]
fn cli_mixed_type_errors_preserve_valid_output_count_and_slurp() {
    for (flag, expected) in [
        (None, "hello\nworld\n"),
        (Some("-c"), "2\n"),
        (Some("-s"), "[\"hello\",\"world\"]\n"),
    ] {
        let mut command = jgrep();
        command.args(["--filter", ".message | ascii_downcase", "--no-color"]);
        if let Some(flag) = flag {
            command.arg(flag);
        }
        command
            .write_stdin("{\"message\":\"HELLO\"}\n{\"message\":[]}\n{\"message\":\"WORLD\"}\n")
            .assert()
            .code(2)
            .stdout(expected)
            .stderr(predicate::str::contains("filter error"));
    }
}

#[test]
fn numeric_json_keys_are_literal_and_array_indexes_still_work() {
    for key in ["1.2", "1", "true", "null"] {
        jgrep()
            .args(["-p", key])
            .write_stdin(format!("{{\"{key}\":\"literal\"}}\n"))
            .assert()
            .success()
            .stdout("literal\n");
    }
    jgrep()
        .args(["-p", "1.2"])
        .write_stdin("{\"1\":{\"2\":\"nested\"}}\n")
        .assert()
        .success()
        .stdout("nested\n");
    jgrep()
        .args(["-p", "0"])
        .write_stdin("[\"array\"]\n")
        .assert()
        .success()
        .stdout("array\n");
    #[cfg(feature = "yaml")]
    jgrep()
        .args(["-p", "1.2"])
        .write_stdin("1.2: typed-yaml\n")
        .assert()
        .success()
        .stdout("typed-yaml\n");
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
fn deeply_nested_json_returns_controlled_errors_for_files_and_stdin() {
    let deep = format!("{}0{}", "[".repeat(20000), "]".repeat(20000));
    let dir = tempdir().unwrap();
    let file = dir.path().join("deep.json");
    fs::write(&file, &deep).unwrap();
    jgrep()
        .timeout(std::time::Duration::from_secs(5))
        .arg(&file)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("JSON nesting exceeds 128"));
    for input in [
        deep.clone(),
        format!("{{\"a\":{deep}}}\n"),
        format!("{{\"ok\":1}}\n{{\"a\":{deep}}}\n"),
    ] {
        jgrep()
            .timeout(std::time::Duration::from_secs(5))
            .write_stdin(input)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("JSON nesting exceeds 128"));
    }
    #[cfg(feature = "explore")]
    jgrep()
        .timeout(std::time::Duration::from_secs(5))
        .args(["explore", "--print"])
        .arg(file)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("JSON nesting exceeds 128"));
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

#[cfg(feature = "yaml")]
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
    fs::write(&json, r#"{"name":"Alice"}"#).unwrap();

    jgrep()
        .arg(json.to_str().unwrap())
        .assert()
        .success()
        .stdout("{\"name\":\"Alice\"}\n");

    #[cfg(feature = "yaml")]
    {
        let yaml = dir.path().join("data.yaml");
        fs::write(&yaml, "name: Bob\n").unwrap();
        jgrep()
            .arg(yaml.to_str().unwrap())
            .assert()
            .success()
            .stdout("{\"name\":\"Bob\"}\n");
    }
}

#[test]
fn defaults_to_identity_filter_for_stdin() {
    jgrep()
        .write_stdin("{\"name\":\"Alice\"}\n")
        .assert()
        .success()
        .stdout("{\"name\":\"Alice\"}\n");
}

#[cfg(feature = "yaml")]
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

#[cfg(feature = "yaml")]
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

#[cfg(feature = "yaml")]
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

#[cfg(feature = "yaml")]
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

#[cfg(feature = "completion")]
#[test]
fn completion_outputs_supported_shells() {
    for (shell, marker) in [
        ("bash", "complete"),
        ("zsh", "compdef"),
        ("fish", "complete"),
        ("powershell", "Register-ArgumentCompleter"),
        ("pwsh", "Register-ArgumentCompleter"),
    ] {
        jgrep()
            .args(["completion", shell])
            .assert()
            .success()
            .stdout(predicate::str::contains(marker));
    }

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

#[cfg(feature = "explore")]
#[test]
fn explore_invalid_utf8_keys_do_not_panic_and_manual_projection_still_works() {
    let input = b"{\"\xff\":1,\"name\":\"Alice\"}";
    jgrep()
        .args(["explore", "--print"])
        .write_stdin(input.as_slice())
        .assert()
        .code(2)
        .stdout(predicate::str::contains("schema completion unavailable"))
        .stderr(predicate::str::contains("panicked").not());
    jgrep()
        .args(["explore", "--print", "--filter", ".name"])
        .write_stdin(input.as_slice())
        .assert()
        .success()
        .stdout(predicate::str::contains("schema completion unavailable"))
        .stdout(predicate::str::contains("Alice"));
}

#[cfg(feature = "explore")]
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

#[cfg(feature = "explore")]
#[test]
fn explicit_stream_mode_rejects_buffered_inputs_and_keeps_non_tty_snapshots() {
    for extra in ["--print", "file.json"] {
        jgrep()
            .args(["explore", "--stream-json", extra])
            .assert()
            .code(2)
            .stderr(predicate::str::contains("cannot be used with"));
    }
    jgrep()
        .args(["explore", "--stream-json", "-p", "message"])
        .write_stdin("{\"message\":\"finite\"}\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("finite"));
}

#[cfg(feature = "explore")]
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

#[cfg(feature = "explore")]
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

#[cfg(all(feature = "explore", feature = "yaml"))]
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

#[cfg(feature = "explore")]
#[test]
fn explore_print_returns_error_for_invalid_filter() {
    jgrep()
        .args(["explore", "--print", "--filter", ".["])
        .write_stdin("{\"name\":\"Alice\"}\n")
        .assert()
        .code(2)
        .stdout(predicate::str::contains("error:"));
}

#[cfg(feature = "explore")]
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

#[test]
fn recursive_search_includes_ndjson_and_jsonl() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("a.ndjson"), "{\"v\":1}\n{\"v\":2}\n").unwrap();
    fs::write(dir.path().join("b.jsonl"), "{\"v\":3}\n").unwrap();
    fs::write(dir.path().join("ignored.txt"), "{\"v\":4}\n").unwrap();

    jgrep()
        .args(["-rc", ".v", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(format!(
            "{}:2\n{}:1\n",
            dir.path().join("a.ndjson").display(),
            dir.path().join("b.jsonl").display()
        ));
}

#[cfg(feature = "yaml")]
#[test]
fn detects_yaml_keys_with_json_scalar_prefixes() {
    for key in ["true_value", "false_value", "null_value", "123"] {
        jgrep()
            .args(["-p", key])
            .write_stdin(format!("{key}: accepted\n"))
            .assert()
            .success()
            .stdout("accepted\n");
    }
}

#[cfg(all(feature = "explore", feature = "yaml"))]
#[test]
fn explore_detects_yaml_keys_with_json_scalar_prefixes() {
    jgrep()
        .args(["explore", "--print", "-p", "true_value"])
        .write_stdin("true_value: accepted\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("  accepted\n"));
}

#[test]
fn accepts_json_scalars_and_empty_input() {
    jgrep()
        .write_stdin("true\n42\n\"text\"\n")
        .assert()
        .success()
        .stdout("true\n42\ntext\n");
    jgrep().write_stdin("").assert().code(1).stdout("");
    jgrep()
        .arg("-c")
        .write_stdin("")
        .assert()
        .code(1)
        .stdout("0\n");
}

#[cfg(feature = "explore")]
#[test]
fn explore_rejects_invalid_where_even_with_filter() {
    for args in [
        vec!["explore", "--print", "-w", "invalid"],
        vec!["explore", "--print", "-f", ".", "-w", "invalid"],
    ] {
        jgrep()
            .args(args)
            .write_stdin("{\"name\":\"Alice\"}\n")
            .assert()
            .code(2)
            .stdout("")
            .stderr(predicate::str::contains("invalid where shortcut"));
    }
}

#[cfg(feature = "explore")]
#[test]
fn explore_combines_explicit_filter_with_where() {
    jgrep()
        .args([
            "explore",
            "--print",
            "-f",
            "age>=18",
            "-w",
            "status=active",
            "-p",
            "name",
        ])
        .write_stdin(
            "{\"name\":\"Alice\",\"age\":30,\"status\":\"active\"}\n\
             {\"name\":\"Bob\",\"age\":15,\"status\":\"active\"}\n\
             {\"name\":\"Carla\",\"age\":40,\"status\":\"pending\"}\n",
        )
        .assert()
        .success()
        .stdout(predicate::str::contains("preview:\n  Alice\n"))
        .stdout(predicate::str::contains("  Bob\n").not())
        .stdout(predicate::str::contains("  Carla\n").not());
}

#[cfg(not(feature = "completion"))]
#[test]
fn disabled_completion_reports_error() {
    jgrep()
        .args(["completion", "bash"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "completion support is not enabled",
        ));
}

#[cfg(not(feature = "explore"))]
#[test]
fn disabled_explore_reports_error() {
    jgrep()
        .args(["explore", "--print"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("explore support is not enabled"));
    let config = tempdir().unwrap();
    jgrep()
        .args(["k9s", "install", "--config-dir"])
        .arg(config.path())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("k9s requires explore support"));
    assert!(!config.path().join("plugins").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn output_write_failures_return_error() {
    for args in [vec!["-n", "42"], vec!["-nc", "42"], vec!["-ns", "42"]] {
        let mut command = std::process::Command::new(assert_cmd::cargo::cargo_bin!("jgrep"));
        command.args(args).stdout(
            fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .unwrap(),
        );
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8(output.stderr).unwrap().contains("output"));
    }
}

#[cfg(all(target_os = "linux", feature = "explore"))]
#[test]
fn explore_snapshot_write_failures_return_error() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("data.json");
    fs::write(&file, "{\"name\":\"Alice\"}\n").unwrap();
    let mut command = std::process::Command::new(assert_cmd::cargo::cargo_bin!("jgrep"));
    command.args(["explore", "--print"]).arg(&file).stdout(
        fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap(),
    );
    assert_eq!(command.output().unwrap().status.code(), Some(2));
}

#[cfg(feature = "explore")]
#[test]
fn explore_accepts_maximum_input_limit_without_overflow() {
    jgrep()
        .args([
            "explore",
            "--print",
            "--max-input-bytes",
            &usize::MAX.to_string(),
        ])
        .write_stdin("{\"name\":\"Alice\"}\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Alice"));
}

#[test]
fn explicit_identity_and_recursive_descent_are_filters() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("data.json");
    fs::write(&input, "{\"name\":\"Alice\"}\n").unwrap();
    jgrep()
        .args([".", input.to_str().unwrap()])
        .assert()
        .success()
        .stdout("{\"name\":\"Alice\"}\n");
    jgrep()
        .args(["..", input.to_str().unwrap()])
        .assert()
        .success()
        .stdout("{\"name\":\"Alice\"}\nAlice\n");
    jgrep()
        .arg(".")
        .write_stdin("{\"name\":\"Alice\"}\n")
        .assert()
        .success()
        .stdout("{\"name\":\"Alice\"}\n");
}

#[test]
fn shortcut_missing_file_is_not_ignored_in_favor_of_stdin() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("missing.data");
    jgrep()
        .args(["-p", "name", missing.to_str().unwrap()])
        .write_stdin("{\"name\":\"must not be read\"}\n")
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("missing.data"));
}

#[test]
fn missing_implicit_input_gets_a_file_error() {
    let dir = tempdir().unwrap();
    jgrep()
        .current_dir(dir.path())
        .arg("missing.json")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("missing.json"))
        .stderr(predicate::str::contains("invalid filter").not());
}

#[test]
fn named_filter_is_unambiguous_and_can_use_missing_input_paths() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join(".name"), "not an input").unwrap();
    fs::write(dir.path().join("data.json"), "{\"name\":\"Alice\"}\n").unwrap();
    jgrep()
        .current_dir(dir.path())
        .args(["--filter", ".name", "data.json"])
        .assert()
        .success()
        .stdout("Alice\n");
    jgrep()
        .current_dir(dir.path())
        .args(["--filter", ".name", "missing.data"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("missing.data"));
}

#[test]
fn current_directory_shortcut_still_works_and_explicit_filter_can_recurse() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("data.json"), "{\"name\":\"Alice\"}\n").unwrap();
    for args in [vec!["-r", "."], vec!["--filter", ".", "-r", "."]] {
        jgrep()
            .current_dir(dir.path())
            .args(args)
            .assert()
            .success()
            .stdout("{\"name\":\"Alice\"}\n");
    }
}

#[test]
fn conflicting_filter_sources_are_rejected() {
    for args in [
        vec!["--filter", ".", "-p", "name"],
        vec!["--filter", ".", "-f", "filter.jq"],
        vec!["-p", "name", "-f", "filter.jq"],
    ] {
        jgrep()
            .args(args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("cannot be used with"));
    }
}

#[test]
fn help_contains_working_examples_and_recovery_guidance() {
    jgrep()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("[FILTER]"))
        .stdout(predicate::str::contains("--filter EXPR"))
        .stdout(predicate::str::contains("Examples:"))
        .stdout(predicate::str::contains("-w 'age>=18'"))
        .stdout(predicate::str::contains("Exit codes:"));
    let dir = tempdir().unwrap();
    jgrep()
        .arg(dir.path())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Use --recursive (-r)"));
}

#[test]
fn path_like_jq_expressions_keep_their_meaning() {
    jgrep()
        .arg("./length")
        .write_stdin("6\n")
        .assert()
        .success()
        .stdout("1.0\n");
    jgrep()
        .arg("length/.json")
        .write_stdin("{\"json\":2}\n")
        .assert()
        .success()
        .stdout("0.5\n");
}
