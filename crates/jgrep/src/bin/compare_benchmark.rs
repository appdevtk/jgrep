use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn main() -> io::Result<()> {
    let root = workspace_root()?;
    let fixtures = root.join("target/benchmark-fixtures");
    fs::create_dir_all(&fixtures)?;
    write_fixtures(&fixtures)?;

    let rust_bin = rust_binary(&root);
    let java_jgrep = root.join("jgrep/target/quarkus-app/quarkus-run.jar");
    let java_ygrep = root.join("ygrep/target/quarkus-app/quarkus-run.jar");

    require_file(&rust_bin, "Rust jgrep binary")?;
    require_file(&java_jgrep, "Java jgrep jar")?;
    require_file(&java_ygrep, "Java ygrep jar")?;

    let iterations = env::var("JGREP_BENCH_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7);

    let cases = vec![
        BenchCase {
            name: "null input arithmetic",
            rust: command_spec(&rust_bin, ["-n", "1 + 1"]),
            java: java_spec(&java_jgrep, ["-n", "1 + 1"]),
        },
        BenchCase {
            name: "NDJSON count 10k",
            rust: command_spec(
                &rust_bin,
                [
                    "-c",
                    "select(.active == true)",
                    fixtures.join("events.ndjson").to_str().unwrap(),
                ],
            ),
            java: java_spec(
                &java_jgrep,
                [
                    "-c",
                    "select(.active == true)",
                    fixtures.join("events.ndjson").to_str().unwrap(),
                ],
            ),
        },
        BenchCase {
            name: "YAML multi-doc count 2k",
            rust: command_spec(
                &rust_bin,
                [
                    "-c",
                    "select(.active == true)",
                    fixtures.join("events.yaml").to_str().unwrap(),
                ],
            ),
            java: java_spec(
                &java_ygrep,
                [
                    "-c",
                    "select(.active == true)",
                    fixtures.join("events.yaml").to_str().unwrap(),
                ],
            ),
        },
        BenchCase {
            name: "JSON projection 10k",
            rust: command_spec(
                &rust_bin,
                [".id", fixtures.join("events.ndjson").to_str().unwrap()],
            ),
            java: java_spec(
                &java_jgrep,
                [".id", fixtures.join("events.ndjson").to_str().unwrap()],
            ),
        },
    ];

    let mut results = Vec::new();
    for case in cases {
        eprintln!("benchmarking {}...", case.name);
        let rust = measure(&case.rust, iterations)?;
        let java = measure(&case.java, iterations)?;
        results.push(BenchResult {
            name: case.name,
            rust,
            java,
        });
    }

    let html = render_html(&results, iterations, &rust_bin, &java_jgrep, &java_ygrep);
    let out = root.join("target/benchmark-comparison.html");
    fs::write(&out, html)?;
    println!("{}", out.display());
    Ok(())
}

struct BenchCase<'a> {
    name: &'a str,
    rust: CommandSpec,
    java: CommandSpec,
}

struct BenchResult<'a> {
    name: &'a str,
    rust: Measurement,
    java: Measurement,
}

#[derive(Clone)]
struct CommandSpec {
    program: PathBuf,
    args: Vec<String>,
}

#[derive(Clone, Copy)]
struct Measurement {
    min: Duration,
    mean: Duration,
}

fn command_spec<I, S>(program: &Path, args: I) -> CommandSpec
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    CommandSpec {
        program: program.to_path_buf(),
        args: args.into_iter().map(|s| s.as_ref().to_owned()).collect(),
    }
}

fn java_spec<I, S>(jar: &Path, args: I) -> CommandSpec
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut all_args = vec!["-jar".to_owned(), jar.display().to_string()];
    all_args.extend(args.into_iter().map(|s| s.as_ref().to_owned()));
    CommandSpec {
        program: PathBuf::from("java"),
        args: all_args,
    }
}

fn measure(spec: &CommandSpec, iterations: usize) -> io::Result<Measurement> {
    run_once(spec)?;
    let mut times = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        run_once(spec)?;
        times.push(start.elapsed());
    }
    let min = *times.iter().min().unwrap();
    let total = times.iter().fold(Duration::ZERO, |acc, t| acc + *t);
    Ok(Measurement {
        min,
        mean: total / times.len() as u32,
    })
}

fn run_once(spec: &CommandSpec) -> io::Result<()> {
    let output = Command::new(&spec.program).args(&spec.args).output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "command failed: {} {}\nstdout:\n{}\nstderr:\n{}",
            spec.program.display(),
            spec.args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

fn workspace_root() -> io::Result<PathBuf> {
    let mut dir = env::current_dir()?;
    loop {
        if dir.join("rust-rewrite-plan.md").exists() && dir.join("pom.xml").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "could not locate workspace root",
            ));
        }
    }
}

fn rust_binary(root: &Path) -> PathBuf {
    let release = root.join("target/release/jgrep");
    if release.exists() {
        release
    } else {
        root.join("target/debug/jgrep")
    }
}

fn require_file(path: &Path, label: &str) -> io::Result<()> {
    path.exists().then_some(()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{label} not found at {}", path.display()),
        )
    })
}

fn write_fixtures(dir: &Path) -> io::Result<()> {
    let mut ndjson = String::new();
    for i in 0..10_000 {
        ndjson.push_str(&format!(
            "{{\"id\":{i},\"active\":{},\"value\":\"item-{i}\"}}\n",
            i % 2 == 0
        ));
    }
    fs::write(dir.join("events.ndjson"), ndjson)?;

    let mut yaml = String::new();
    for i in 0..2_000 {
        yaml.push_str("---\n");
        yaml.push_str(&format!(
            "id: {i}\nactive: {}\nvalue: item-{i}\n",
            i % 2 == 0
        ));
    }
    fs::write(dir.join("events.yaml"), yaml)?;
    Ok(())
}

fn render_html(
    results: &[BenchResult<'_>],
    iterations: usize,
    rust_bin: &Path,
    java_jgrep: &Path,
    java_ygrep: &Path,
) -> String {
    let rows = results
        .iter()
        .map(|r| {
            let ratio = r.java.mean.as_secs_f64() / r.rust.mean.as_secs_f64();
            format!(
                "<tr><th>{}</th><td>{:.2}</td><td>{:.2}</td><td>{:.2}</td><td>{:.2}</td><td>{:.2}x</td></tr>",
                escape(r.name),
                ms(r.rust.min),
                ms(r.rust.mean),
                ms(r.java.min),
                ms(r.java.mean),
                ratio
            )
        })
        .collect::<String>();

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>jgrep Rust vs Java Benchmark</title>
  <style>
    body {{ font-family: system-ui, sans-serif; margin: 2rem; color: #1f2937; }}
    table {{ border-collapse: collapse; width: 100%; margin-top: 1rem; }}
    th, td {{ border: 1px solid #d1d5db; padding: .55rem .7rem; text-align: right; }}
    th:first-child, td:first-child {{ text-align: left; }}
    thead th {{ background: #f3f4f6; }}
    code {{ background: #f3f4f6; padding: .12rem .25rem; border-radius: 4px; }}
    .meta {{ color: #4b5563; line-height: 1.5; }}
  </style>
</head>
<body>
  <h1>jgrep Rust vs Java Benchmark</h1>
  <p class="meta">
    Iterations per command: <strong>{iterations}</strong><br>
    Rust binary: <code>{rust_bin}</code><br>
    Java JSON reference: <code>{java_jgrep}</code><br>
    Java YAML reference: <code>{java_ygrep}</code>
  </p>
  <table>
    <thead>
      <tr>
        <th>Case</th>
        <th>Rust min ms</th>
        <th>Rust mean ms</th>
        <th>Java min ms</th>
        <th>Java mean ms</th>
        <th>Java/Rust mean</th>
      </tr>
    </thead>
    <tbody>{rows}</tbody>
  </table>
</body>
</html>
"#,
        rust_bin = escape(&rust_bin.display().to_string()),
        java_jgrep = escape(&java_jgrep.display().to_string()),
        java_ygrep = escape(&java_ygrep.display().to_string()),
    )
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
