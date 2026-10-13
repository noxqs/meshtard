//! Fuzzes meshtard's file readers: `cargo run --release -p mt-fuzz -- [OPTIONS] [TARGET...]`.
//! See `spec/FUZZING.md`.
//!
//! - no options: every target, 20 000 mutated inputs each, findings in `fuzz-out/`.
//! - `--iters N`, `--seed N`, `--out DIR`, `--budget-ms N` (a run longer than this is
//!   a hang; default 5000).
//! - `--list`: the targets and their seed counts; `--dump-seeds DIR` writes them.
//! - `--one TARGET FILE`: runs one input (exit 0 fine, 101 panic, 3 hang; an
//!   oversized allocation or a stack overflow aborts).
//! - `--minimize TARGET FILE OUT`: shrinks a crashing input while it fails the same way.
//!
//! Each target runs in a child process, so inputs that abort it (an allocation over
//! the limits in [`mt_fuzz::alloc`], a stack overflow) or hang are recorded and the
//! campaign goes on. Panics are caught in process and grouped by source location.

use std::collections::BTreeMap;
use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mt_fuzz::{Mutator, Rng, Target, alloc};

#[global_allocator]
static ALLOC: alloc::Counting = alloc::Counting;

/// Exit code of a run that took longer than the budget.
const HANG: i32 = 3;
/// Exit code of a run that panicked.
const PANIC: i32 = 101;
/// Stack size of the thread running the readers: the default for spawned threads,
/// smaller than the main thread's, as for the app's loading threads.
const STACK: usize = 2 << 20;

struct Opts {
    iters: u64,
    seed: u64,
    out: PathBuf,
    budget: Duration,
}

/// The last panic's message and location, recorded by the panic hook.
static LAST_PANIC: Mutex<Option<(String, String)>> = Mutex::new(None);

fn quiet_panics() {
    std::panic::set_hook(Box::new(|info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let mut loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        // Panics inside std or a dependency: name the reader's function too.
        if !loc.starts_with("crates/") {
            let bt = std::backtrace::Backtrace::force_capture().to_string();
            if let Some(f) = bt
                .lines()
                .map(str::trim)
                .find(|l| l.contains("mt_kernel::") || l.contains("mt_skp::"))
            {
                loc = format!("{loc} in {}", f.split_once(": ").map_or(f, |x| x.1));
            }
        }
        if let Ok(mut p) = LAST_PANIC.lock() {
            *p = Some((loc, msg));
        }
    }));
}

/// Inputs the reader accepted (no error) in this process.
static ACCEPTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Runs `target` over `data`: `None` when fine, else the panic's location and message.
fn run_caught(target: &Target, data: &[u8]) -> Option<(String, String)> {
    match catch_unwind(AssertUnwindSafe(|| (target.run)(data))) {
        Ok(accepted) => {
            if accepted {
                ACCEPTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            None
        }
        Err(_) => Some(
            LAST_PANIC
                .lock()
                .ok()
                .and_then(|mut p| p.take())
                .unwrap_or_default(),
        ),
    }
}

/// Runs inputs on a worker thread with a time limit each.
struct Worker {
    tx: mpsc::Sender<Arc<Vec<u8>>>,
    rx: mpsc::Receiver<Option<(String, String)>>,
}

impl Worker {
    fn new(name: &'static str) -> Self {
        let (tx, jobs) = mpsc::channel::<Arc<Vec<u8>>>();
        let (done, rx) = mpsc::channel();
        std::thread::Builder::new()
            .stack_size(STACK)
            .spawn(move || {
                let target = mt_fuzz::target(name).expect("target");
                for data in jobs {
                    let r = run_caught(&target, &data);
                    if done.send(r).is_err() {
                        break;
                    }
                }
            })
            .expect("worker thread");
        Self { tx, rx }
    }

    /// `Err(())` when the run took longer than `budget`.
    fn run(&self, data: Arc<Vec<u8>>, budget: Duration) -> Result<Option<(String, String)>, ()> {
        self.tx.send(data).map_err(|_| ())?;
        self.rx.recv_timeout(budget).map_err(|_| ())
    }
}

fn file_key(loc: &str) -> String {
    loc.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// A child process fuzzing one target. Writes each input to `current-<target>.bin`
/// before running it (so an abort leaves it behind) and panicking inputs to
/// `crashes/`.
fn child(name: &str, o: &Opts) -> ExitCode {
    quiet_panics();
    let Some(target) = mt_fuzz::target(name) else {
        eprintln!("unknown target {name}");
        return ExitCode::from(2);
    };
    alloc::enforce(false);
    let seeds = mt_fuzz::seeds().remove(name).unwrap_or_default();
    alloc::enforce(true);
    if seeds.is_empty() {
        eprintln!("{name}: no seeds");
        return ExitCode::SUCCESS;
    }
    let crashes = o.out.join("crashes");
    let _ = std::fs::create_dir_all(&crashes);
    let current = o.out.join(format!("current-{name}.bin"));
    let worker = Worker::new(target.name);
    // The valid files first: they must be read without an error.
    ACCEPTED.store(0, std::sync::atomic::Ordering::Relaxed);
    for s in &seeds {
        match worker.run(Arc::new(s.clone()), o.budget * 4) {
            Ok(None) => {}
            Ok(Some((loc, msg))) => println!("{name}: SEED PANIC at {loc}: {msg}"),
            Err(()) => {
                println!("{name}: SEED HANG");
                return ExitCode::from(HANG as u8);
            }
        }
    }
    let accepted = ACCEPTED.load(std::sync::atomic::Ordering::Relaxed);
    if accepted != seeds.len() as u64 {
        println!(
            "{name}: only {accepted} of {} seeds read without an error",
            seeds.len()
        );
    }
    let mut m = Mutator {
        rng: Rng::new(o.seed),
        pool: &seeds,
        text: target.text,
        zip: target.zip,
    };
    let mut seen: BTreeMap<String, u64> = BTreeMap::new();
    ACCEPTED.store(0, std::sync::atomic::Ordering::Relaxed);
    let start = Instant::now();
    let mut slowest = Duration::ZERO;
    alloc::reset_peak();
    for i in 0..o.iters {
        let base = &seeds[m.rng.below(seeds.len())];
        let mut data = m.mutate(base);
        if m.rng.chance(4) {
            data = m.mutate(&data);
        }
        let _ = std::fs::write(&current, &data);
        let data = Arc::new(data);
        let t = Instant::now();
        match worker.run(data.clone(), o.budget) {
            Ok(None) => {}
            Ok(Some((loc, msg))) => {
                let n = seen.entry(loc.clone()).or_insert(0);
                *n += 1;
                if *n == 1 {
                    println!("{name}: PANIC at {loc}: {msg}");
                    let p = crashes.join(format!("{name}-panic-{}.bin", file_key(&loc)));
                    let _ = std::fs::write(p, data.as_slice());
                }
            }
            Err(()) => {
                println!("{name}: HANG (over {:?}) at iteration {i}", o.budget);
                return ExitCode::from(HANG as u8);
            }
        }
        slowest = slowest.max(t.elapsed());
        if (i + 1) % 5000 == 0 {
            println!(
                "{name}: {} runs, {:.0}/s, {} panic sites, peak {} MB",
                i + 1,
                (i + 1) as f64 / start.elapsed().as_secs_f64(),
                seen.len(),
                alloc::peak() >> 20
            );
        }
    }
    let _ = std::fs::remove_file(&current);
    println!(
        "{name}: done, {} runs in {:.1} s ({:.0}% read without error), slowest {:.0} ms, \
         {} panic sites, peak {} MB",
        o.iters,
        start.elapsed().as_secs_f64(),
        100.0 * ACCEPTED.load(std::sync::atomic::Ordering::Relaxed) as f64 / o.iters.max(1) as f64,
        slowest.as_secs_f64() * 1000.0,
        seen.len(),
        alloc::peak() >> 20
    );
    if seen.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(PANIC as u8)
    }
}

/// Runs one input: 0 fine, 101 panic (location printed), 3 hang.
fn one(name: &str, file: &Path, budget: Duration) -> ExitCode {
    quiet_panics();
    let Ok(data) = std::fs::read(file) else {
        eprintln!("cannot read {}", file.display());
        return ExitCode::from(2);
    };
    if mt_fuzz::target(name).is_none() {
        eprintln!("unknown target {name}");
        return ExitCode::from(2);
    }
    let worker = Worker::new(mt_fuzz::target(name).expect("target").name);
    alloc::reset_peak();
    let t = Instant::now();
    match worker.run(Arc::new(data), budget) {
        Ok(None) => {
            println!(
                "ok in {:.1} ms, peak {} MB",
                t.elapsed().as_secs_f64() * 1000.0,
                alloc::peak() >> 20
            );
            ExitCode::SUCCESS
        }
        Ok(Some((loc, msg))) => {
            println!("panic at {loc}: {msg}");
            ExitCode::from(PANIC as u8)
        }
        Err(()) => {
            println!("hang");
            ExitCode::from(HANG as u8)
        }
    }
}

fn exe() -> PathBuf {
    std::env::current_exe().expect("current exe")
}

/// How a child run of one input ended: exit code (or signal) and panic location.
fn outcome(name: &str, file: &Path, budget: Duration) -> String {
    let out = Command::new(exe())
        .arg("--budget-ms")
        .arg(budget.as_millis().to_string())
        .arg("--one")
        .arg(name)
        .arg(file)
        .stderr(Stdio::piped())
        .output()
        .expect("run child");
    let stdout = String::from_utf8_lossy(&out.stdout);
    match out.status.code() {
        Some(0) => "ok".into(),
        Some(c) if c == PANIC => stdout
            .split(": ")
            .next()
            .unwrap_or("panic")
            .trim()
            .to_owned(),
        Some(c) => format!("exit {c}"),
        None => "signal".into(),
    }
}

/// Removes chunks, then single bytes, of a failing input while it fails the same way.
fn minimize(name: &str, file: &Path, out: &Path, budget: Duration) -> ExitCode {
    let Ok(mut data) = std::fs::read(file) else {
        eprintln!("cannot read {}", file.display());
        return ExitCode::from(2);
    };
    let want = outcome(name, file, budget);
    if want == "ok" {
        eprintln!("the input does not fail");
        return ExitCode::from(2);
    }
    println!("minimizing {} bytes failing as {want}", data.len());
    let tmp = out.with_extension("try");
    let mut tries = 0;
    let mut chunk = (data.len() / 2).max(1);
    while chunk >= 1 && tries < 20_000 {
        let mut i = 0;
        let mut progress = false;
        while i < data.len() && tries < 20_000 {
            let end = (i + chunk).min(data.len());
            let mut cand = data.clone();
            cand.drain(i..end);
            std::fs::write(&tmp, &cand).expect("write candidate");
            tries += 1;
            if outcome(name, &tmp, budget) == want {
                data = cand;
                progress = true;
            } else {
                i += chunk;
            }
        }
        if !progress {
            if chunk == 1 {
                break;
            }
            chunk /= 2;
        }
    }
    let _ = std::fs::remove_file(&tmp);
    std::fs::write(out, &data).expect("write output");
    println!("{} bytes after {tries} runs: {}", data.len(), out.display());
    ExitCode::SUCCESS
}

/// Fuzzes each target in a child process, restarting it after an abort or hang.
fn campaign(names: &[String], o: &Opts) -> ExitCode {
    let crashes = o.out.join("crashes");
    std::fs::create_dir_all(&crashes).expect("output folder");
    let mut failed = false;
    for name in names {
        let mut seed = o.seed;
        let mut done_ok = false;
        for restart in 0..20 {
            let mut child = Command::new(exe())
                .arg("--child")
                .arg(name)
                .arg("--iters")
                .arg(o.iters.to_string())
                .arg("--seed")
                .arg(seed.to_string())
                .arg("--out")
                .arg(&o.out)
                .arg("--budget-ms")
                .arg(o.budget.as_millis().to_string())
                .stderr(Stdio::piped())
                .spawn()
                .expect("start child");
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = e.read_to_string(&mut err);
            }
            let status = child.wait().expect("child");
            match status.code() {
                Some(0) => {
                    done_ok = true;
                    break;
                }
                Some(c) if c == PANIC => {
                    failed = true;
                    done_ok = true;
                    break;
                }
                _ => {
                    failed = true;
                    let current = o.out.join(format!("current-{name}.bin"));
                    let p = crashes.join(format!("{name}-abort-{seed}.bin"));
                    let _ = std::fs::rename(&current, &p);
                    let tail: Vec<&str> = err.lines().rev().take(3).collect();
                    println!(
                        "{name}: ABORT ({status}) saved {}: {}",
                        p.display(),
                        tail.join(" | ")
                    );
                    seed = seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(restart + 1);
                }
            }
        }
        if !done_ok {
            println!("{name}: gave up after 20 restarts");
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Removes `flag` and its value from `args`, giving the value.
fn take(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let i = args.iter().position(|a| a == flag)?;
    args.remove(i);
    (i < args.len()).then(|| args.remove(i))
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut o = Opts {
        iters: 20_000,
        seed: 0x5eed,
        out: PathBuf::from("fuzz-out"),
        budget: Duration::from_millis(5000),
    };
    if let Some(v) = take(&mut args, "--iters") {
        o.iters = v.parse().expect("--iters N");
    }
    if let Some(v) = take(&mut args, "--seed") {
        o.seed = v.parse().expect("--seed N");
    }
    if let Some(v) = take(&mut args, "--out") {
        o.out = PathBuf::from(v);
    }
    if let Some(v) = take(&mut args, "--budget-ms") {
        o.budget = Duration::from_millis(v.parse().expect("--budget-ms N"));
    }
    if let Some(name) = take(&mut args, "--child") {
        return child(&name, &o);
    }
    if let Some(name) = take(&mut args, "--one") {
        return one(&name, Path::new(&args[0]), o.budget);
    }
    if let Some(name) = take(&mut args, "--minimize") {
        return minimize(&name, Path::new(&args[0]), Path::new(&args[1]), o.budget);
    }
    if let Some(dir) = take(&mut args, "--dump-seeds") {
        alloc::enforce(false);
        for (name, seeds) in mt_fuzz::seeds() {
            let d = Path::new(&dir).join(name);
            std::fs::create_dir_all(&d).expect("seed folder");
            for (i, s) in seeds.iter().enumerate() {
                std::fs::write(d.join(format!("{i}.bin")), s).expect("write seed");
            }
        }
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--list") {
        alloc::enforce(false);
        let seeds = mt_fuzz::seeds();
        for t in mt_fuzz::targets() {
            let s = seeds.get(t.name).map_or(&[][..], Vec::as_slice);
            let bytes: usize = s.iter().map(Vec::len).sum();
            println!("{:<12} {} seeds, {} bytes", t.name, s.len(), bytes);
        }
        return ExitCode::SUCCESS;
    }
    let names: Vec<String> = if args.is_empty() {
        mt_fuzz::targets()
            .iter()
            .map(|t| t.name.to_owned())
            .collect()
    } else {
        args
    };
    campaign(&names, &o)
}
