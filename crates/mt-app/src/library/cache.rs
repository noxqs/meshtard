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

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Default cache size limit, in megabytes.
pub(crate) const DEFAULT_LIMIT_MB: u64 = 1024;
/// Smallest limit the window accepts, in megabytes.
pub(crate) const MIN_LIMIT_MB: u64 = 50;
/// The note in a model's folder naming its main file (relative path).
const MAIN_NOTE: &str = ".main";

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
/// bytes, never `keep` (the model just downloaded). Returns the bytes left.
pub(crate) fn evict(cache: &Path, limit: u64, keep: Option<&Path>) -> u64 {
    let (mut e, fixed) = entries(cache);
    let mut total = fixed + e.iter().map(|e| e.size).sum::<u64>();
    // Oldest first; ties by path so the order is the same every time.
    e.sort_by(|a, b| a.used.cmp(&b.used).then_with(|| a.path.cmp(&b.path)));
    for x in e {
        if total <= limit {
            break;
        }
        if keep.is_some_and(|k| k == x.path) {
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
