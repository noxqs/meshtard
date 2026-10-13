//! Replays the inputs that crashed, hung or over-allocated in a reader (minimized,
//! `regressions/<target>/*`) and the valid seed files: each must now finish within
//! the fuzzer's limits without a panic. An allocation over the limits aborts the test
//! binary (see `mt_fuzz::alloc`).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

#[global_allocator]
static ALLOC: mt_fuzz::alloc::Counting = mt_fuzz::alloc::Counting;

/// Generous: debug builds of the readers are slow.
const BUDGET: Duration = Duration::from_secs(20);

/// Runs one input: whether the reader accepted it.
fn run(target: &mt_fuzz::Target, name: &str, data: &[u8]) -> bool {
    let start = Instant::now();
    let r = catch_unwind(AssertUnwindSafe(|| (target.run)(data)));
    assert!(r.is_ok(), "{}: {name} panics", target.name);
    assert!(
        start.elapsed() < BUDGET,
        "{}: {name} took {:?}",
        target.name,
        start.elapsed()
    );
    r.unwrap_or(false)
}

#[test]
fn crashing_inputs_are_handled() {
    let dir = mt_fuzz::regression_dir();
    let mut n = 0;
    for t in mt_fuzz::targets() {
        let Ok(entries) = std::fs::read_dir(dir.join(t.name)) else {
            continue;
        };
        let mut files: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        files.sort();
        for f in files {
            let data = std::fs::read(&f).expect("read regression input");
            let name = f.file_name().unwrap().to_string_lossy().into_owned();
            run(&t, &name, &data);
            n += 1;
        }
    }
    assert!(n > 0, "no regression inputs found in {}", dir.display());
}

/// The seed files read without an error, and a quick fuzzing run of each target (200
/// mutated inputs) without a panic, so `cargo test` keeps a little fuzzing.
#[test]
fn short_fuzz_run() {
    mt_fuzz::alloc::enforce(false);
    let seeds = mt_fuzz::seeds();
    mt_fuzz::alloc::enforce(true);
    for t in mt_fuzz::targets() {
        let Some(pool) = seeds.get(t.name) else {
            continue;
        };
        // Debug builds: only the small seeds.
        let pool: Vec<Vec<u8>> = pool.iter().filter(|s| s.len() < 40_000).cloned().collect();
        if pool.is_empty() {
            continue;
        }
        for (i, s) in pool.iter().enumerate() {
            assert!(
                run(&t, &format!("seed {i}"), s),
                "{}: seed {i} is refused",
                t.name
            );
        }
        let mut m = mt_fuzz::Mutator {
            rng: mt_fuzz::Rng::new(0x5eed ^ t.name.len() as u64),
            pool: &pool,
            text: t.text,
            zip: t.zip,
        };
        for i in 0..200 {
            let base = &pool[m.rng.below(pool.len())];
            let data = m.mutate(base);
            run(&t, &format!("mutation {i}"), &data);
        }
    }
}
