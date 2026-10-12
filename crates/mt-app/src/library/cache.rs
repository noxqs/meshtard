//! The Model Library's download cache (`<data>/library`): its size, the size limit
//! (Config `cache_limit_mb`, set in the window's Cache section) and eviction of the
//! least recently used entries, and the note of each downloaded model's main file so
//! a cached model is placed again without the network.
//!
//! Entries are a downloaded model's folder (`models/<key>`, removed whole) and each
//! thumbnail (`thumbs/<key>.png`). Poly Haven's model list (`polyhaven/assets.json`,
//! about 1 MB, refreshed daily) is never evicted but counts towards the size. An
//! entry's age is the newest modification time in it; placing a cached model again
//! rewrites its note, so it counts as used. SketchUp has no such setting (its 3D
//! Warehouse downloads go to a temporary folder); limit, default and wording are
//! meshtard's own.
//!
//! Two running meshtards share the cache. A model being downloaded or read holds a
//! lock file `locks/<key>@<pid>-<n>` ([`use_model`]), and eviction never removes a
//! model with a live lock. Both take the cache's own lock (`locks/.cache`, made with
//! `create_new`, so only one process holds it) for a moment, so a model cannot be
//! locked between eviction's check and its removal. A lock is stale, and removed,
//! when its process has ended (checked on Linux) or when it has not been refreshed
//! for [`STALE`] (its holder touches it every [`HEARTBEAT`]); so a crashed meshtard
//! never pins a model for good.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::time::{Duration, SystemTime};

/// Default cache size limit, in megabytes.
pub(crate) const DEFAULT_LIMIT_MB: u64 = 1024;
/// Smallest limit the window accepts, in megabytes.
pub(crate) const MIN_LIMIT_MB: u64 = 50;
/// The note in a model's folder naming its main file (relative path).
const MAIN_NOTE: &str = ".main";
/// Folder of the lock files, in the cache.
const LOCKS: &str = "locks";
/// The cache's own lock (held while evicting or taking a model lock).
const CACHE_LOCK: &str = ".cache";
/// How often a model lock is touched while held.
pub(crate) const HEARTBEAT: Duration = Duration::from_secs(20);
/// A model lock not touched for this long is stale.
pub(crate) const STALE: Duration = Duration::from_secs(120);
/// The cache's own lock is held for moments; older than this it is stale.
const CACHE_LOCK_STALE: Duration = Duration::from_secs(30);
/// How long to wait for the cache's own lock.
const CACHE_LOCK_WAIT: Duration = Duration::from_secs(5);

/// Whether process `pid` is running: known on Linux (`/proc`), `None` elsewhere
/// (the lock's age decides there).
fn pid_alive(pid: u32) -> Option<bool> {
    if cfg!(target_os = "linux") {
        Some(Path::new("/proc").join(pid.to_string()).exists())
    } else {
        None
    }
}

/// How long ago `path` was last written (zero for a time in the future).
fn age(path: &Path) -> Option<Duration> {
    let t = path.metadata().ok()?.modified().ok()?;
    Some(SystemTime::now().duration_since(t).unwrap_or_default())
}

/// Whether the lock at `path`, held by `pid`, is stale after `stale`.
fn is_stale(path: &Path, pid: Option<u32>, stale: Duration) -> bool {
    if pid.and_then(pid_alive) == Some(false) {
        return true;
    }
    age(path).is_none_or(|a| a > stale)
}

/// The cache's own lock, released on drop.
struct CacheLock(PathBuf);

impl CacheLock {
    /// Waits up to `wait` for the lock; `None` when another process keeps it.
    fn take(cache: &Path, wait: Duration) -> Option<Self> {
        let dir = cache.join(LOCKS);
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join(CACHE_LOCK);
        let start = std::time::Instant::now();
        loop {
            match std::fs::File::options()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    use std::io::Write;
                    let _ = write!(f, "{}", std::process::id());
                    return Some(Self(path));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let pid = std::fs::read_to_string(&path)
                        .ok()
                        .and_then(|t| t.trim().parse().ok());
                    if is_stale(&path, pid, CACHE_LOCK_STALE) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                }
                Err(e) => {
                    log::warn!("cache lock {}: {e}", path.display());
                    return None;
                }
            }
            if start.elapsed() >= wait {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A model in use by this process: its lock file, touched every [`HEARTBEAT`] and
/// removed on drop.
pub(crate) struct InUse {
    path: PathBuf,
    /// Dropping it stops the heartbeat thread.
    _stop: Sender<()>,
}

impl Drop for InUse {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Locks the cached model `key` against eviction by any meshtard while the result
/// lives. `None` when the lock file cannot be written (the model is then not
/// protected; downloads still work).
pub(crate) fn use_model(cache: &Path, key: &str) -> Option<InUse> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    // Without the cache lock (another process stuck on it) the lock is still made.
    let _guard = CacheLock::take(cache, CACHE_LOCK_WAIT);
    let path = cache
        .join(LOCKS)
        .join(format!("{key}@{}-{n}", std::process::id()));
    std::fs::create_dir_all(path.parent()?).ok()?;
    if let Err(e) = std::fs::write(&path, std::process::id().to_string()) {
        log::warn!("could not write {}: {e}", path.display());
        return None;
    }
    let (tx, rx) = channel::<()>();
    let beat = path.clone();
    let _ = std::thread::Builder::new()
        .name("library-lock".into())
        .spawn(move || {
            while let Err(RecvTimeoutError::Timeout) = rx.recv_timeout(HEARTBEAT) {
                let touched = std::fs::File::options()
                    .write(true)
                    .open(&beat)
                    .and_then(|f| f.set_modified(SystemTime::now()));
                if touched.is_err() {
                    break;
                }
            }
        });
    Some(InUse { path, _stop: tx })
}

/// Keys of the models with a live lock in `cache`; stale lock files are removed.
fn locked_keys(cache: &Path) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    let Ok(rd) = std::fs::read_dir(cache.join(LOCKS)) else {
        return out;
    };
    for e in rd.filter_map(Result::ok) {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some((key, owner)) = name.rsplit_once('@') else {
            continue;
        };
        let pid = owner.split('-').next().and_then(|p| p.parse().ok());
        if is_stale(&e.path(), pid, STALE) {
            let _ = std::fs::remove_file(e.path());
        } else {
            out.insert(key.to_owned());
        }
    }
    out
}

/// One evictable entry of the cache.
#[derive(Debug)]
struct Entry {
    path: PathBuf,
    size: u64,
    used: SystemTime,
    dir: bool,
}

/// Total size and newest modification time of the files under `dir` (folders' own
/// times are not looked at; symbolic links are not followed).
fn dir_stats(dir: &Path) -> (u64, SystemTime) {
    let mut size = 0;
    let mut newest = SystemTime::UNIX_EPOCH;
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.filter_map(Result::ok) {
            let Ok(m) = e.path().symlink_metadata() else {
                continue;
            };
            if m.is_dir() {
                stack.push(e.path());
                continue;
            }
            size += m.len();
            if let Ok(t) = m.modified() {
                newest = newest.max(t);
            }
        }
    }
    (size, newest)
}

/// The evictable entries and the size of everything else in `cache`.
fn entries(cache: &Path) -> (Vec<Entry>, u64) {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(cache.join("models")) {
        for e in rd.filter_map(Result::ok) {
            let path = e.path();
            if path.symlink_metadata().is_ok_and(|m| m.is_dir()) {
                let (size, used) = dir_stats(&path);
                out.push(Entry {
                    path,
                    size,
                    used,
                    dir: true,
                });
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir(cache.join("thumbs")) {
        for e in rd.filter_map(Result::ok) {
            let Ok(m) = e.path().symlink_metadata() else {
                continue;
            };
            if m.is_file() {
                out.push(Entry {
                    path: e.path(),
                    size: m.len(),
                    used: m.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    dir: false,
                });
            }
        }
    }
    let fixed = dir_stats(&cache.join("polyhaven")).0;
    (out, fixed)
}

/// Bytes the cache takes.
#[cfg(test)]
fn usage(cache: &Path) -> u64 {
    let (e, fixed) = entries(cache);
    fixed + e.iter().map(|e| e.size).sum::<u64>()
}

/// Removes the least recently used entries until the cache takes at most `limit`
/// bytes, never `keep` (the model just downloaded) nor a model locked by any
/// meshtard ([`use_model`]). Returns the bytes left.
pub(crate) fn evict(cache: &Path, limit: u64, keep: Option<&Path>) -> u64 {
    let (mut e, fixed) = entries(cache);
    let mut total = fixed + e.iter().map(|e| e.size).sum::<u64>();
    if total <= limit {
        return total;
    }
    // Nothing is removed without the cache lock: another process may be locking a
    // model at this moment.
    let Some(_guard) = CacheLock::take(cache, CACHE_LOCK_WAIT) else {
        log::warn!("library cache is locked by another process; not evicting");
        return total;
    };
    let locked = locked_keys(cache);
    let in_use = |x: &Entry| {
        x.dir
            && x.path
                .file_name()
                .is_some_and(|n| locked.contains(n.to_string_lossy().as_ref()))
    };
    // Oldest first; ties by path so the order is the same every time.
    e.sort_by(|a, b| a.used.cmp(&b.used).then_with(|| a.path.cmp(&b.path)));
    for x in e {
        if total <= limit {
            break;
        }
        if keep.is_some_and(|k| k == x.path) || in_use(&x) {
            continue;
        }
        let r = if x.dir {
            std::fs::remove_dir_all(&x.path)
        } else {
            std::fs::remove_file(&x.path)
        };
        match r {
            Ok(()) => total = total.saturating_sub(x.size),
            Err(err) => log::warn!("could not remove {}: {err}", x.path.display()),
        }
    }
    total
}

/// Notes `main` (relative to `dir`) as the downloaded model's main file. Also marks
/// the model as just used.
pub(crate) fn note_main(dir: &Path, main: &str) {
    if let Err(e) = std::fs::write(dir.join(MAIN_NOTE), main) {
        log::warn!("could not write {}: {e}", dir.display());
    }
}

/// The main file of a model downloaded completely into `dir`, if it is still there.
pub(crate) fn cached_main(dir: &Path) -> Option<PathBuf> {
    let main = std::fs::read_to_string(dir.join(MAIN_NOTE)).ok()?;
    let main = main.trim();
    if !super::safe_relative(main) {
        return None;
    }
    let p = dir.join(main);
    p.is_file().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mt_library_cache_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Writes `len` bytes at `path` and dates it `age_s` seconds ago.
    fn file(path: &Path, len: usize, age_s: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![0u8; len]).unwrap();
        let t = SystemTime::now() - Duration::from_secs(age_s);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn evicts_the_oldest_entries_down_to_the_limit() {
        let c = temp_dir("evict");
        file(&c.join("polyhaven/assets.json"), 100, 10);
        file(&c.join("models/old/a.glb"), 1000, 500);
        file(&c.join("models/old/tex/b.jpg"), 500, 400);
        file(&c.join("models/mid/a.glb"), 1000, 300);
        file(&c.join("models/new/a.glb"), 1000, 100);
        file(&c.join("thumbs/x.png"), 50, 450);
        file(&c.join("thumbs/y.png"), 50, 50);
        assert_eq!(usage(&c), 100 + 1500 + 1000 + 1000 + 100);
        // Under the limit: nothing goes.
        assert_eq!(evict(&c, 10_000, None), 3700);
        assert!(c.join("models/old").exists());
        // The newest file in "old" is 400 s old, so the thumbnail x (450 s) goes
        // first, then "old"; that is enough for 2200 bytes.
        let left = evict(&c, 2200, None);
        assert_eq!(left, 3700 - 50 - 1500);
        assert!(!c.join("thumbs/x.png").exists());
        assert!(!c.join("models/old").exists());
        assert!(c.join("models/mid").exists() && c.join("thumbs/y.png").exists());
        // The kept entry stays even when over the limit; the list never goes.
        let keep = c.join("models/mid");
        let left = evict(&c, 0, Some(&keep));
        assert_eq!(left, 100 + 1000);
        assert!(keep.exists() && c.join("polyhaven/assets.json").exists());
        assert!(!c.join("models/new").exists() && !c.join("thumbs/y.png").exists());
        let _ = std::fs::remove_dir_all(&c);
    }

    /// Writes a lock file for `key` held by `pid`, last touched `age_s` ago.
    fn lock(c: &Path, key: &str, pid: u32, age_s: u64) -> PathBuf {
        let p = c.join(LOCKS).join(format!("{key}@{pid}-99"));
        file(&p, 4, age_s);
        p
    }

    #[test]
    fn eviction_skips_models_another_instance_reads() {
        let c = temp_dir("locked");
        file(&c.join("models/a/a.glb"), 1000, 500);
        file(&c.join("models/b.v2/b.glb"), 1000, 400);
        file(&c.join("models/c/c.glb"), 1000, 300);
        // "a" is read by this process, "b.v2" by another live one (a fresh lock with
        // this process's id stands for it on every system).
        let a = use_model(&c, "a").unwrap();
        let b = lock(&c, "b.v2", std::process::id(), 1);
        assert_eq!(evict(&c, 0, None), 2000);
        assert!(c.join("models/a").exists() && c.join("models/b.v2").exists());
        assert!(!c.join("models/c").exists());
        assert!(b.exists(), "a live lock stays");
        // Released: the next eviction may remove them.
        drop(a);
        std::fs::remove_file(&b).unwrap();
        assert_eq!(evict(&c, 0, None), 0);
        assert!(!c.join("models/a").exists() && !c.join("models/b.v2").exists());
        // The cache's own lock is not left behind.
        assert!(!c.join(LOCKS).join(CACHE_LOCK).exists());
        let _ = std::fs::remove_dir_all(&c);
    }

    #[test]
    fn stale_locks_are_dropped() {
        let c = temp_dir("stale");
        file(&c.join("models/a/a.glb"), 1000, 500);
        // Not touched for longer than STALE: its meshtard has gone (crash).
        let old = lock(&c, "a", std::process::id(), STALE.as_secs() + 60);
        assert_eq!(evict(&c, 0, None), 0);
        assert!(!old.exists() && !c.join("models/a").exists());
        // A process that has ended (Linux checks the id).
        if cfg!(target_os = "linux") {
            file(&c.join("models/b/b.glb"), 1000, 500);
            let dead = lock(&c, "b", u32::MAX - 1, 1);
            assert_eq!(evict(&c, 0, None), 0);
            assert!(!dead.exists());
        }
        // A stale cache lock (an evicting meshtard that crashed) is taken over.
        file(&c.join("models/d/d.glb"), 1000, 500);
        file(&c.join(LOCKS).join(CACHE_LOCK), 4, 3600);
        assert_eq!(evict(&c, 0, None), 0);
        let _ = std::fs::remove_dir_all(&c);
    }

    #[test]
    fn a_held_cache_lock_stops_eviction() {
        let c = temp_dir("busy");
        file(&c.join("models/a/a.glb"), 1000, 500);
        let held = CacheLock::take(&c, Duration::ZERO).unwrap();
        assert!(CacheLock::take(&c, Duration::from_millis(50)).is_none());
        // `evict` waits CACHE_LOCK_WAIT, then leaves everything.
        assert_eq!(evict(&c, 0, None), 1000);
        assert!(c.join("models/a").exists());
        drop(held);
        assert_eq!(evict(&c, 0, None), 0);
        let _ = std::fs::remove_dir_all(&c);
    }

    #[test]
    fn model_locks_are_removed_on_drop() {
        let c = temp_dir("inuse");
        let a = use_model(&c, "sketchfab-x").unwrap();
        let path = a.path.clone();
        assert!(path.exists());
        assert_eq!(
            locked_keys(&c).into_iter().collect::<Vec<_>>(),
            ["sketchfab-x"]
        );
        drop(a);
        assert!(!path.exists());
        assert!(locked_keys(&c).is_empty());
        let _ = std::fs::remove_dir_all(&c);
    }

    #[test]
    fn main_note_round_trips_and_refuses_unsafe_paths() {
        let d = temp_dir("main");
        assert!(cached_main(&d).is_none());
        note_main(&d, "scene.gltf");
        assert!(cached_main(&d).is_none(), "the file itself is missing");
        std::fs::write(d.join("scene.gltf"), b"{}").unwrap();
        assert_eq!(cached_main(&d), Some(d.join("scene.gltf")));
        note_main(&d, "../scene.gltf");
        assert!(cached_main(&d).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
