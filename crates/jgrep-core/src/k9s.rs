//! Read-only process adapter. No shell, implicit context, or cluster writes.

use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use jaq_json::Val;

use crate::cli::{K9sAction, K9sArgs, K9sSelection};

const MAX_LINE: u64 = 1024 * 1024;
const TEMPLATE: &str = include_str!("../../../plugins/k9s-jgrep/plugins.yaml");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Logs,
    Resource,
}

pub fn run(args: K9sArgs) -> i32 {
    let result = match args.action {
        K9sAction::Logs(selection) => bridge(Action::Logs, selection),
        K9sAction::Resource(selection) => bridge(Action::Resource, selection),
        K9sAction::Install {
            config_dir,
            jgrep_bin,
            kubectl,
            update,
        } => install(&config_dir, jgrep_bin.as_deref(), &kubectl, update).map(|target| {
            println!("Installed jgrep k9s plugin: {}", target.display());
            0
        }),
    };
    result.unwrap_or_else(|error| {
        eprintln!("jgrep k9s: {error}");
        2
    })
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

fn kubectl_command(action: Action, selection: &K9sSelection) -> io::Result<Command> {
    if selection.context.is_empty() || selection.context.starts_with('$') {
        return Err(invalid(
            "missing explicit k9s context, refusing the default context",
        ));
    }
    if !valid_name(&selection.namespace) || !valid_name(&selection.name) {
        return Err(invalid("invalid or missing namespace/name from k9s"));
    }
    let mut command = Command::new(&selection.kubectl);
    command
        .arg(format!("--context={}", selection.context))
        .arg(format!("--namespace={}", selection.namespace));
    if !selection.kubeconfig.is_empty() && !selection.kubeconfig.starts_with('$') {
        command.env("KUBECONFIG", &selection.kubeconfig);
        if std::env::split_paths(&selection.kubeconfig).count() == 1 {
            command.arg(format!("--kubeconfig={}", selection.kubeconfig));
        }
    }
    match action {
        Action::Logs => {
            command.args(["logs", &selection.name, "--follow=true", "--tail=1000"]);
            if selection.container.is_empty() {
                command.arg("--all-containers=true");
            } else if valid_name(&selection.container) {
                command.arg(format!("--container={}", selection.container));
            } else {
                return Err(invalid("invalid container from k9s"));
            }
        }
        Action::Resource => {
            if ![
                "pods",
                "deployments",
                "statefulsets",
                "daemonsets",
                "services",
                "jobs",
                "cronjobs",
            ]
            .contains(&selection.resource.as_str())
            {
                return Err(invalid(
                    "unsupported resource, secrets are intentionally excluded",
                ));
            }
            if !selection.group.is_empty() && !valid_name(&selection.group) {
                return Err(invalid("invalid API group from k9s"));
            }
            let resource = if selection.group.is_empty() {
                selection.resource.clone()
            } else {
                format!("{}.{}", selection.resource, selection.group)
            };
            command.args([
                "--request-timeout=15s",
                "get",
                &resource,
                &selection.name,
                "-o",
                "json",
            ]);
        }
    }
    Ok(command)
}

fn normalize_log(line: &[u8]) -> io::Result<Vec<u8>> {
    let text = String::from_utf8_lossy(line);
    let text = text.trim_end_matches(['\r', '\n']);
    let value = if safe_json_depth(text.as_bytes()) {
        jaq_json::read::parse_single(text.as_bytes())
            .ok()
            .filter(finite_json)
    } else {
        None
    }
    .unwrap_or_else(|| Val::from(text.to_owned()));
    let value = if matches!(value, Val::Obj(_)) {
        value
    } else {
        Val::obj(
            [(Val::from("message".to_owned()), value)]
                .into_iter()
                .collect(),
        )
    };
    let mut data = crate::output::format_value(&value, false)?.into_bytes();
    data.push(b'\n');
    Ok(data)
}

// The jq parser accepts non-JSON numbers and recursive structures. Keep hostile
// log lines from overflowing its stack, and preserve invalid JSON as plain text.
fn safe_json_depth(bytes: &[u8]) -> bool {
    crate::input::check_json_depth(bytes).is_ok()
}

fn finite_json(value: &Val) -> bool {
    match value {
        Val::Num(jaq_json::Num::Float(number)) => number.is_finite(),
        Val::Arr(values) => values.iter().all(finite_json),
        Val::Obj(values) => values.values().all(finite_json),
        _ => true,
    }
}

/// Own child lifetime even when spawning the viewer or transferring data fails.
struct Process(Child);

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn transfer(action: Action, source: impl Read, mut sink: impl Write, limit: u64) -> io::Result<()> {
    let mut source = BufReader::new(source);
    if action == Action::Resource {
        let mut bytes = Vec::new();
        source
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            return Err(invalid("resource exceeds the configured input limit"));
        }
        sink.write_all(&bytes)?;
    } else {
        let mut total = 0u64;
        loop {
            let mut line = Vec::new();
            if source
                .by_ref()
                .take(MAX_LINE + 1)
                .read_until(b'\n', &mut line)?
                == 0
            {
                break;
            }
            if line.len() as u64 > MAX_LINE {
                return Err(invalid("log line exceeds 1 MiB"));
            }
            let data = normalize_log(&line)?;
            total = total.saturating_add(data.len() as u64);
            if total > limit {
                return Err(invalid(
                    "log session reached its input limit, reopen to continue",
                ));
            }
            sink.write_all(&data)?;
            sink.flush()?;
        }
    }
    Ok(())
}

fn bridge(action: Action, selection: K9sSelection) -> io::Result<i32> {
    let mut command = kubectl_command(action, &selection)?;
    let current = std::env::current_exe()?;
    let state = selection
        .state_dir
        .unwrap_or_else(|| current.parent().unwrap_or(Path::new(".")).join("state"));
    private_dir(&state)?;
    let mut producer = Process(
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?,
    );
    let source = producer.0.stdout.take().expect("piped producer stdout");
    let diagnostics = producer.0.stderr.take().expect("piped producer stderr");
    let (diagnostics_sender, diagnostics_receiver) = mpsc::channel();
    let errors = thread::spawn(move || {
        let result = (|| {
            let mut source = diagnostics;
            let mut captured = Vec::new();
            source.by_ref().take(8192).read_to_end(&mut captured)?;
            io::copy(&mut source, &mut io::sink())?;
            Ok::<_, io::Error>(captured)
        })();
        let _ = diagnostics_sender.send(result);
    });
    let mut viewer_command = Command::new(selection.jgrep.as_deref().unwrap_or(&current));
    viewer_command.args([
        "explore",
        "--max-input-bytes",
        &selection.max_input_bytes.to_string(),
        "--color-level",
    ]);
    if action == Action::Logs {
        viewer_command.arg("--stream-json");
    }
    let mut viewer = Process(
        viewer_command
            .env("JGREP_LAST_FILTER_FILE", state.join("last-filter.jq"))
            .env("JGREP_FILTER_HISTORY_FILE", state.join("history"))
            .env("JGREP_FILTER_SAVE_PATH", state.join("saved-filter.jq"))
            .stdin(Stdio::piped())
            .spawn()?,
    );
    let sink = viewer.0.stdin.take().expect("piped viewer stdin");
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let result = transfer(action, source, sink, selection.max_input_bytes);
        let _ = sender.send(result);
    });
    let mut completed = None;
    let status = loop {
        if completed.is_none() {
            if let Ok(result) = receiver.try_recv() {
                if result.is_err() {
                    let _ = producer.0.kill();
                }
                completed = Some(result);
            }
        }
        if let Some(status) = viewer.0.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(10));
    };
    if completed.is_none() {
        completed = receiver.try_recv().ok();
    }
    // A finite source may have just closed its pipe before its process has exited.
    let deadline = Instant::now() + Duration::from_secs(3);
    let producer_status = loop {
        if let Some(status) = producer.0.try_wait()? {
            break Some(status);
        }
        if completed.as_ref().is_some_and(Result::is_ok) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        } else {
            break None;
        }
    };
    let _ = producer.0.kill();
    let _ = producer.0.wait();
    let result = match completed {
        Some(result) => result,
        None => receiver
            .recv_timeout(Duration::from_secs(4))
            .map_err(|_| invalid("log reader did not finish after kubectl stopped"))?,
    };
    reader.join().map_err(|_| invalid("log reader failed"))?;
    let diagnostics = diagnostics_receiver
        .recv_timeout(Duration::from_secs(4))
        .map_err(|_| invalid("diagnostics reader did not finish after kubectl stopped"))??;
    errors
        .join()
        .map_err(|_| invalid("diagnostics reader failed"))?;
    if let Err(error) = result {
        if error.kind() != io::ErrorKind::BrokenPipe {
            return Err(error);
        }
    }
    if producer_status.is_some_and(|status| !status.success()) {
        let detail = String::from_utf8_lossy(&diagnostics);
        return Err(io::Error::other(if detail.trim().is_empty() {
            "kubectl failed"
        } else {
            detail.trim()
        }));
    }
    Ok(status.code().unwrap_or(130))
}

fn private_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)
    }
}

fn executable(path: &Path) -> io::Result<PathBuf> {
    let found = if path.components().count() > 1 || path.is_absolute() {
        Some(path.to_owned())
    } else {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(path))
                .find(|file| file.is_file())
        })
    };
    let found = found.ok_or_else(|| invalid("cannot find executable"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(&found)?.permissions().mode() & 0o111 == 0 {
            return Err(invalid("file is not executable"));
        }
    }
    fs::canonicalize(found)
}

fn install(
    config: &Path,
    binary: Option<&Path>,
    kubectl: &Path,
    update: bool,
) -> io::Result<PathBuf> {
    fs::create_dir_all(config)?;
    let target = fs::canonicalize(config)?.join("plugins/jgrep");
    let existing = target.symlink_metadata().ok();
    if existing.is_some() && !update {
        return Err(invalid("refusing to overwrite existing plugin directory"));
    }
    if existing
        .as_ref()
        .is_some_and(|metadata| !metadata.is_dir() || metadata.file_type().is_symlink())
    {
        return Err(invalid(
            "existing plugin must be a real directory, not a symlink",
        ));
    }
    let binary = executable(binary.unwrap_or(&std::env::current_exe()?))?;
    let kubectl = executable(kubectl)?;
    preflight(&binary, &["k9s", "--help"])?;
    preflight(&binary, &["explore", "--print"])?;
    let mut yaml = TEMPLATE.to_owned();
    for (name, value) in [
        ("JGREP", target.join("jgrep").to_string_lossy().into_owned()),
        ("KUBECTL_ARG", format!("--kubectl={}", kubectl.display())),
    ] {
        yaml = yaml.replace(
            &format!("{{{{{name}}}}}"),
            &crate::output::format_value(&Val::from(value), false)?,
        );
    }
    fs::create_dir_all(target.parent().expect("plugin parent"))?;
    // Stage all fallible writes before touching the installed snapshot. Backups
    // stay outside plugins/ so k9s never discovers duplicate shortcuts.
    let staging = config.join("jgrep-backups");
    private_dir(&staging)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let prepared = staging.join(format!("prepared-{stamp}"));
    private_dir(&prepared)?;
    fs::copy(&binary, prepared.join("jgrep"))?;
    fs::write(prepared.join("plugins.yaml"), yaml)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            prepared.join("plugins.yaml"),
            fs::Permissions::from_mode(0o600),
        )?;
    }
    private_dir(&prepared.join("state"))?;
    if existing.is_some() {
        let state = target.join("state");
        if state.symlink_metadata().is_ok() {
            copy_private_state(&state, &prepared.join("state"))?;
        }
        let backup = staging.join(format!("jgrep-{stamp}"));
        fs::rename(&target, &backup)?;
        if let Err(error) = fs::rename(&prepared, &target) {
            fs::rename(&backup, &target)?;
            return Err(error);
        }
        println!("Previous plugin snapshot backed up: {}", backup.display());
    } else {
        // Preserve the no-overwrite contract, including concurrent installers.
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new().mode(0o700).create(&target)?;
        }
        #[cfg(not(unix))]
        {
            fs::create_dir(&target)?;
        }
        fs::rename(prepared.join("jgrep"), target.join("jgrep"))?;
        fs::rename(prepared.join("plugins.yaml"), target.join("plugins.yaml"))?;
        fs::rename(prepared.join("state"), target.join("state"))?;
        fs::remove_dir(&prepared)?;
    }
    Ok(target)
}

fn copy_private_state(source: &Path, destination: &Path) -> io::Result<()> {
    if !source.symlink_metadata()?.is_dir() {
        return Err(invalid("plugin state must be a real directory"));
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(invalid("plugin state must contain only regular files"));
        }
        let copied = destination.join(entry.file_name());
        fs::copy(entry.path(), &copied)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(copied, fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(())
}

fn preflight(binary: &Path, args: &[&str]) -> io::Result<()> {
    let mut check = Process(
        Command::new(binary)
            .args(args)
            .stdin(if args.first() == Some(&"explore") {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    );
    if let Some(mut input) = check.0.stdin.take() {
        input.write_all(b"{}\n")?;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = check.0.try_wait()? {
            if !status.success() {
                return Err(invalid("jgrep binary must support native k9s and explore"));
            }
            break;
        }
        if Instant::now() >= deadline {
            return Err(invalid("jgrep preflight timed out"));
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection() -> K9sSelection {
        K9sSelection {
            context: "test-context".into(),
            namespace: "shop".into(),
            name: "checkout-pod".into(),
            container: String::new(),
            resource: "pods".into(),
            group: String::new(),
            kubeconfig: String::new(),
            kubectl: "kubectl".into(),
            jgrep: None,
            state_dir: None,
            max_input_bytes: 10 * 1024 * 1024,
        }
    }

    fn args(action: Action, selection: &K9sSelection) -> Vec<String> {
        kubectl_command(action, selection)
            .unwrap()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn logs_preserve_routing_and_container_selection() {
        let mut selected = selection();
        let values = args(Action::Logs, &selected);
        assert_eq!(
            &values[..2],
            &["--context=test-context", "--namespace=shop"]
        );
        assert!(values.contains(&"--all-containers=true".into()));
        assert!(values.contains(&"--tail=1000".into()));
        selected.container = "sidecar".into();
        let values = args(Action::Logs, &selected);
        assert!(values.contains(&"checkout-pod".into()));
        assert!(values.contains(&"--container=sidecar".into()));
        assert!(!values.contains(&"--all-containers=true".into()));
    }

    #[test]
    fn resources_preserve_group_timeout_and_kubeconfig() {
        let mut selected = selection();
        selected.resource = "deployments".into();
        selected.group = "apps".into();
        selected.kubeconfig = "/tmp/config with spaces".into();
        let values = args(Action::Resource, &selected);
        for expected in [
            "deployments.apps",
            "--request-timeout=15s",
            "--kubeconfig=/tmp/config with spaces",
        ] {
            assert!(values.contains(&expected.to_owned()));
        }
    }

    #[test]
    fn merged_kubeconfig_uses_environment_not_single_file_flag() {
        let mut selected = selection();
        selected.kubeconfig = std::env::join_paths(["/tmp/one", "/tmp/two"])
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let command = kubectl_command(Action::Logs, &selected).unwrap();
        assert!(!command
            .get_args()
            .any(|arg| arg.to_string_lossy().starts_with("--kubeconfig=")));
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "KUBECONFIG"
                    && value == Some(selected.kubeconfig.as_ref()))
        );
    }

    #[test]
    fn invalid_selections_and_secrets_are_rejected() {
        let mut selected = selection();
        selected.context.clear();
        assert!(kubectl_command(Action::Logs, &selected).is_err());
        selected = selection();
        selected.namespace.clear();
        assert!(kubectl_command(Action::Logs, &selected).is_err());
        selected = selection();
        selected.name = "pod; touch marker".into();
        assert!(kubectl_command(Action::Logs, &selected).is_err());
        selected = selection();
        selected.resource = "secrets".into();
        assert!(kubectl_command(Action::Resource, &selected).is_err());
        selected = selection();
        selected.container = "bad container".into();
        assert!(kubectl_command(Action::Logs, &selected).is_err());
        selected = selection();
        selected.group = "bad group".into();
        assert!(kubectl_command(Action::Resource, &selected).is_err());
    }

    #[test]
    fn context_is_an_argument_not_shell_code() {
        let mut selected = selection();
        selected.context = "cluster; $(touch /tmp/not-executed)".into();
        assert_eq!(
            args(Action::Logs, &selected)[0],
            format!("--context={}", selected.context)
        );
    }

    #[test]
    fn mixed_logs_become_json_objects() {
        for (input, expected) in [
            (
                b"{\"level\":\"ERROR\"}\n".as_slice(),
                "{\"level\":\"ERROR\"}",
            ),
            (b"plain line\r\n", "{\"message\":\"plain line\"}"),
            (b"\"scalar\"\n", "{\"message\":\"scalar\"}"),
            (b"[1,true]\n", "{\"message\":[1,true]}"),
            (b"NaN\n", "{\"message\":\"NaN\"}"),
            (b"{} {}\n", "{\"message\":\"{} {}\"}"),
        ] {
            assert_eq!(
                normalize_log(input).unwrap(),
                format!("{expected}\n").as_bytes()
            );
        }
        for input in [b"invalid \xff".as_slice(), br#"{"message":"\ud800"}"#] {
            let result = normalize_log(input).unwrap();
            assert!(jaq_json::read::parse_many(&result).all(|value| value.is_ok()));
        }
    }

    #[test]
    fn transfer_limits_and_resource_identity() {
        let mut output = Vec::new();
        transfer(Action::Resource, b"{\"a\":1}".as_slice(), &mut output, 7).unwrap();
        assert_eq!(output, b"{\"a\":1}");
        assert!(transfer(Action::Resource, b"12345".as_slice(), io::sink(), 4).is_err());
        assert!(transfer(Action::Logs, b"plain".as_slice(), io::sink(), 4).is_err());
        assert!(transfer(
            Action::Logs,
            vec![b'x'; MAX_LINE as usize + 1].as_slice(),
            io::sink(),
            MAX_LINE * 2
        )
        .is_err());
    }

    #[test]
    fn deeply_nested_or_nonfinite_logs_are_preserved_as_text() {
        let deep = format!("{}0{}", "[".repeat(300), "]".repeat(300));
        for text in [deep, "{\"n\":NaN}".into(), "[Infinity]".into()] {
            let normalized = normalize_log(text.as_bytes()).unwrap();
            let parsed = jaq_json::read::parse_single(&normalized).unwrap();
            let Val::Obj(object) = parsed else {
                panic!("expected object")
            };
            assert_eq!(
                object.get(&Val::from("message".to_owned())),
                Some(&Val::from(text))
            );
        }
        assert!(safe_json_depth(br#"{"message":"escaped \" [[["}"#));
    }
}
