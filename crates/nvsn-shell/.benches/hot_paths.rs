//! Local-only performance benchmark for nvsn-shell's hottest pure-computation
//! path, picked from real call-graph fan-in rather than guessed:
//!
//! - [`nvsn_shell::ShellProfile::parse`] - re-parses the user's shell profile
//!   on every `nvsn init`/`nvsn self-uninstall`/`nvsn implode` run, and is
//!   the direct analog of gvsn's own highest-emphasis benchmark (same
//!   function name, same job: find nvsn-managed blocks among years of
//!   accumulated unrelated profile content without disturbing it).
//!
//! Run locally with `cargo bench -p nvsn-shell`. This target is intentionally
//! never invoked from a GitHub workflow; see the "Benchmarks" section of the
//! top-level README for the local-only policy.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use nvsn_shell::ShellProfile;

/// Builds a synthetic shell profile with `blocks` nvsn-managed blocks
/// interleaved with unrelated user lines, roughly matching what a
/// long-lived `.bashrc`/`.zshrc`/`.profile` looks like after years of
/// accumulated aliases and exports.
fn synthetic_profile(blocks: usize) -> String {
    let mut s = String::new();
    for i in 0..blocks {
        s.push_str(&format!("export MY_VAR_{i}=value_{i}\n"));
        s.push_str("alias ll='ls -la'\n\n");
    }
    s.push_str("# nvsn init\n");
    s.push_str("eval \"$(nvsn env --shell bash)\"\n\n");
    s.push_str("# nvsn wrapper\n");
    s.push_str("nvsn() { command nvsn \"$@\"; }\n\n");
    s.push_str("# nvsn path\n");
    s.push_str("export PATH=\"$HOME/.nvsn/current/bin:$PATH\"\n\n");
    for i in 0..blocks {
        s.push_str(&format!(
            "# custom function {i}\nmy_func_{i}() {{ echo {i}; }}\n\n"
        ));
    }
    s
}

fn bench_profile_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("ShellProfile::parse");
    for blocks in [10usize, 100, 1000] {
        let content = synthetic_profile(blocks);
        group.throughput(Throughput::Bytes(content.len() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(blocks),
            &content,
            |b, content| {
                b.iter(|| ShellProfile::parse(black_box(content)));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_profile_parse);
criterion_main!(benches);
