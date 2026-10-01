use std::fs;
use std::process::{Command, Stdio};

use criterion::{criterion_group, criterion_main, Criterion};
use tempfile::tempdir;

fn ndjson_count(c: &mut Criterion) {
    let dir = tempdir().unwrap();
    let file = dir.path().join("events.ndjson");
    let mut data = String::new();
    for i in 0..10_000 {
        data.push_str(&format!(
            "{{\"id\":{i},\"active\":{},\"value\":\"item-{i}\"}}\n",
            i % 2 == 0
        ));
    }
    fs::write(&file, data).unwrap();
    let bin = env!("CARGO_BIN_EXE_jgrep");

    c.bench_function("rust jgrep ndjson count 10k", |b| {
        b.iter(|| {
            let status = Command::new(bin)
                .args(["-c", "select(.active == true)", file.to_str().unwrap()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
            assert!(status.success());
        })
    });
}

criterion_group!(benches, ndjson_count);
criterion_main!(benches);
