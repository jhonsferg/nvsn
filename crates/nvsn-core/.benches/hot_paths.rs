//! Local-only performance benchmarks for nvsn-core's hottest pure-computation
//! paths, picked from real call-graph fan-in rather than guessed:
//!
//! - [`nvsn_core::spec::parse_spec`] - the single highest fan-in function in
//!   the crate: every command that takes a version (install, uninstall, use,
//!   run, exec, which, version, version-remote, alias, default, local, path,
//!   prune) parses its argument through it.
//! - [`nvsn_core::spec::resolve`] - matches a parsed spec against the
//!   release index; runs right after `parse_spec` on the same call paths
//!   whenever the spec is not an exact tag.
//! - [`nvsn_core::nvmrc::find_version_file`] - the ascending directory walk
//!   that `install`/`use` run with no argument, and that the shell hook
//!   calls on every `cd` into a project (through `nvsn __hook`), so its cost
//!   is paid far more often than a single command invocation suggests.
//! - [`nvsn_core::checksum::parse_shasums`] and
//!   [`nvsn_core::checksum::verify_sha256`] - run once per install, after the
//!   archive download; `verify_sha256` is the one benchmark here with a real
//!   I/O component (it hashes a file written to a temp directory).
//! - [`nvsn_core::remote::parse_index`] - parses `nodejs.org`'s `index.json`
//!   on every cache-miss `install`, `list-remote`, `outdated` and `prune
//!   --scan-dir`; benchmarked at realistic sizes (the real index at
//!   `https://nodejs.org/dist/index.json` has 868 entries at the time of
//!   writing).
//!
//! Run locally with `cargo bench -p nvsn-core`. This target is intentionally
//! never invoked from a GitHub workflow; see the "Benchmarks" section of the
//! top-level README for the local-only policy.

use std::fs;
use std::hint::black_box;
use std::io::Write;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use nvsn_core::checksum::{parse_shasums, verify_sha256};
use nvsn_core::nvmrc::find_version_file;
use nvsn_core::remote::parse_index;
use nvsn_core::spec::{parse_spec, resolve};

fn bench_parse_spec(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_spec");
    for input in [
        "20", "v20.11.1", "20.11", "lts/*", "lts/iron", "lts/-1", "^20.0.0", ">=18 <21",
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(input), input, |b, input| {
            b.iter(|| parse_spec(black_box(input)));
        });
    }
    group.finish();
}

/// Builds a synthetic release list the same shape `parse_index` produces,
/// large enough to exercise `resolve`'s linear scans realistically.
fn synthetic_releases(count: usize) -> Vec<nvsn_core::remote::RemoteRelease> {
    let body = synthetic_index_json(count);
    parse_index(&body).expect("synthetic index parses")
}

fn bench_resolve(c: &mut Criterion) {
    let releases = synthetic_releases(868);
    let mut group = c.benchmark_group("resolve");
    for input in ["20", "v20.11.1", "lts/*", "lts/-1", "^20.0.0"] {
        let spec = parse_spec(input).expect("valid spec");
        group.bench_with_input(BenchmarkId::from_parameter(input), &spec, |b, spec| {
            b.iter(|| resolve(black_box(spec), black_box(&releases)));
        });
    }
    group.finish();
}

/// Creates `depth` nested directories under `root` and returns the deepest
/// one, mimicking a project with a monorepo-style directory structure.
fn nested_dirs(root: &std::path::Path, depth: usize) -> std::path::PathBuf {
    let mut dir = root.to_path_buf();
    for i in 0..depth {
        dir = dir.join(format!("level-{i}"));
    }
    fs::create_dir_all(&dir).expect("create nested dirs");
    dir
}

fn bench_find_version_file(c: &mut Criterion) {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(tmp.path().join(".nvmrc"), "20.11.1\n").expect("write .nvmrc");
    let mut group = c.benchmark_group("find_version_file");
    for depth in [1usize, 5, 20] {
        let leaf = nested_dirs(tmp.path(), depth);
        group.bench_with_input(BenchmarkId::from_parameter(depth), &leaf, |b, leaf| {
            b.iter(|| find_version_file(black_box(leaf), None));
        });
    }
    group.finish();
}

fn bench_parse_shasums(c: &mut Criterion) {
    let mut text = String::new();
    for i in 0..40 {
        text.push_str(&format!(
            "{:064x}  node-v20.{i}.0-linux-x64.tar.gz\n",
            i as u128
        ));
    }
    text.push_str(&format!(
        "{:064x}  node-v20.11.1-linux-x64.tar.gz\n",
        0xABCDu64
    ));
    c.bench_function("parse_shasums", |b| {
        b.iter(|| {
            parse_shasums(
                black_box(&text),
                black_box("node-v20.11.1-linux-x64.tar.gz"),
            )
        });
    });
}

fn bench_verify_sha256(c: &mut Criterion) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut group = c.benchmark_group("verify_sha256");
    for size_mb in [1u64, 16, 64] {
        let path = tmp.path().join(format!("archive-{size_mb}mb.tar.gz"));
        {
            let mut f = fs::File::create(&path).expect("create archive");
            let chunk = vec![0x42u8; 1024 * 1024];
            for _ in 0..size_mb {
                f.write_all(&chunk).expect("write chunk");
            }
        }
        // The expected digest does not matter for timing: verify_sha256 hashes
        // the whole file either way, then compares, so a mismatch is as
        // representative as a match for this benchmark.
        let expected = "0".repeat(64);
        group.throughput(Throughput::Bytes(size_mb * 1024 * 1024));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{size_mb}MiB")),
            &path,
            |b, path| {
                b.iter(|| verify_sha256(black_box(path), black_box(&expected)));
            },
        );
    }
    group.finish();
}

/// Builds a synthetic `index.json` body with `count` entries, in the same
/// shape `nodejs.org/dist/index.json` uses (one LTS every 6th entry, to
/// mirror the real cadence roughly).
fn synthetic_index_json(count: usize) -> String {
    let mut body = String::from("[");
    for i in 0..count {
        if i > 0 {
            body.push(',');
        }
        let minor = count - i;
        let lts = if minor.is_multiple_of(6) {
            "\"Iron\""
        } else {
            "false"
        };
        body.push_str(&format!(
            "{{\"version\":\"v20.{minor}.0\",\"date\":\"2024-01-01\",\
             \"files\":[\"linux-x64\",\"win-x64-zip\",\"osx-arm64-tar\"],\
             \"npm\":\"10.2.4\",\"lts\":{lts},\"security\":false}}"
        ));
    }
    body.push(']');
    body
}

fn bench_parse_index(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_index");
    for count in [50usize, 200, 868] {
        let body = synthetic_index_json(count);
        group.throughput(Throughput::Bytes(body.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &body, |b, body| {
            b.iter(|| parse_index(black_box(body)));
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_parse_spec,
    bench_resolve,
    bench_find_version_file,
    bench_parse_shasums,
    bench_verify_sha256,
    bench_parse_index,
);
criterion_main!(benches);
