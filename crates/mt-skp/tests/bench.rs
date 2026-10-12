//! `.skp` read and write benchmark (spec/PERFORMANCE.md), for profiling `mt-skp` on
//! its own. Ignored by default; run in release mode:
//!
//! ```text
//! cargo test --release -p mt-skp --test bench -- --ignored --nocapture
//! ```
//!
//! It writes a waved `n` x `n` grid of triangles at the root (`MESHTARD_SKP_BENCH_GRID`,
//! default 577: a million edges), parses it back, and parses the files listed in
//! `MESHTARD_SKP_BENCH_FILES` (`:`-separated) and the larger corpus files. Each step
//! is timed over `MESHTARD_SKP_BENCH_RUNS` runs (default 3; the best is printed).
//! Each result prints a checksum of the file written or of the model read (FNV-1a of
//! its `Debug` text), so an optimization can be checked to change neither.

use std::path::{Path, PathBuf};
use std::time::Instant;

use mt_skp::write::{Face, SkpBuilder};

/// FNV-1a of a value's `Debug` text, streamed (the text of a big model is huge).
/// `MESHTARD_SKP_BENCH_CHECKSUM=0` skips it (0) when profiling.
fn checksum(v: &impl std::fmt::Debug) -> u64 {
    if std::env::var("MESHTARD_SKP_BENCH_CHECKSUM").is_ok_and(|v| v == "0") {
        return 0;
    }
    struct Fnv(u64);
    impl std::fmt::Write for Fnv {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            for b in s.bytes() {
                self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
            }
            Ok(())
        }
    }
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    std::fmt::write(&mut h, format_args!("{v:?}")).unwrap();
    h.0
}

fn best_ms<T>(runs: usize, mut f: impl FnMut() -> T) -> (f64, T) {
    let mut best = f64::INFINITY;
    let mut out = None;
    for _ in 0..runs {
        let start = Instant::now();
        let r = f();
        best = best.min(start.elapsed().as_secs_f64() * 1e3);
        out = Some(r);
    }
    (best, out.expect("at least one run"))
}

fn grid(n: usize) -> Vec<u8> {
    let mut b = SkpBuilder::deterministic(1, 1_700_000_000);
    let p = |i: usize, j: usize| {
        let z = ((i as f64) * 0.3).sin() * ((j as f64) * 0.2).cos() * 2.0;
        [i as f64 * 10.0, j as f64 * 10.0, z]
    };
    for j in 0..n {
        for i in 0..n {
            b.add_face(&Face::new([p(i, j), p(i + 1, j), p(i + 1, j + 1)]))
                .unwrap();
            b.add_face(&Face::new([p(i, j), p(i + 1, j + 1), p(i, j + 1)]))
                .unwrap();
        }
    }
    b.to_bytes().unwrap()
}

#[test]
#[ignore = "benchmark: run with --release -- --ignored --nocapture"]
fn bench_skp() {
    let env = |k: &str| std::env::var(k).ok().and_then(|v| v.parse().ok());
    let runs: usize = env("MESHTARD_SKP_BENCH_RUNS").unwrap_or(3).max(1);
    let n: usize = env("MESHTARD_SKP_BENCH_GRID").unwrap_or(577);

    let (ms, bytes) = best_ms(runs, || grid(n));
    eprintln!(
        "bench: grid {n}: write {ms:>10.1} ms ({} bytes, checksum {:016x})",
        bytes.len(),
        checksum(&bytes)
    );
    let (ms, model) = best_ms(runs, || mt_skp::parse(&bytes).unwrap());
    eprintln!(
        "bench: grid {n}: parse {ms:>10.1} ms ({} edges, {} faces, checksum {:016x})",
        model.root.edges.len(),
        model.root.faces.len(),
        checksum(&model)
    );

    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/openskp");
    let mut files: Vec<PathBuf> = [
        "coedge_orientation_regression.skp",
        "gondola_v20.skp",
        "capilla_quiroz_v17.skp",
    ]
    .iter()
    .map(|f| corpus.join(f))
    .collect();
    if let Ok(extra) = std::env::var("MESHTARD_SKP_BENCH_FILES") {
        files.extend(
            extra
                .split(':')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
        );
    }
    for path in files {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let (ms, model) = best_ms(runs, || mt_skp::parse(&bytes).unwrap());
        eprintln!(
            "bench: {name}: parse {ms:>10.1} ms (checksum {:016x})",
            checksum(&model)
        );
    }
}
