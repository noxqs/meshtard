//! Linux: installing this binary for the current user (a Meshtard addition, not in
//! SketchUp). At start, when the running binary is not `~/.local/bin/meshtard` and
//! nothing is installed there or the installed version is older, a dialog offers to
//! install it; Help > Install for This User does the same on request and
//! Preferences > General removes it again.
//!
//! An install writes the binary (atomically: a temporary file in the same folder,
//! fsync, mode 755, rename), the hicolor icons, a desktop entry, a shared-mime-info
//! package for `*.skp` (unless the system already knows the type) and a sidecar,
//! `$XDG_DATA_HOME/meshtard/installed.toml` (version, SHA-256 of the binary, date,
//! files written). Uninstall removes only the files the sidecar lists that are also
//! files an install writes, and the binary only while it is unchanged. Other files
//! already in place are left alone. The installed version comes from the sidecar,
//! else from `meshtard --version` (run without a display, 2 s at most).

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::App;

/// This build's version.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The MIME type of .skp files (as SketchUp registers it on Windows and macOS).
const SKP_MIME: &str = "application/vnd.sketchup.skp";
/// How long `meshtard --version` of the installed binary may take.
const VERSION_TIMEOUT: Duration = Duration::from_secs(2);

/// The app icons from `packaging/linux` (made by `tools/make_icons.py`).
macro_rules! icon {
    ($size:literal) => {
        (
            $size,
            include_bytes!(concat!(
                "../../../packaging/linux/icons/hicolor/",
                $size,
                "/apps/meshtard.png"
            )),
        )
    };
}
const ICONS: [(&str, &[u8]); 8] = [
    icon!("16x16"),
    icon!("24x24"),
    icon!("32x32"),
    icon!("48x48"),
    icon!("64x64"),
    icon!("128x128"),
    icon!("256x256"),
    icon!("512x512"),
];

/// A semantic version: `MAJOR.MINOR.PATCH[-PRE][+BUILD]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Version {
    core: [u64; 3],
    /// Pre-release identifiers (empty: a release).
    pre: Vec<String>,
}

impl Version {
    pub(crate) fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s.strip_prefix('v').unwrap_or(s);
        let s = s.split_once('+').map_or(s, |(v, _)| v);
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (s, None),
        };
        let mut parts = core.split('.');
        let mut n = [0u64; 3];
        for slot in &mut n {
            let p = parts.next()?;
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            *slot = p.parse().ok()?;
        }
        if parts.next().is_some() {
            return None;
        }
        let pre = match pre {
            Some(p) => {
                let ids: Vec<String> = p.split('.').map(str::to_owned).collect();
                if ids.iter().any(String::is_empty) {
                    return None;
                }
                ids
            }
            None => Vec::new(),
        };
        Some(Self { core: n, pre })
    }

    pub(crate) fn current() -> Self {
        Self::parse(VERSION).expect("CARGO_PKG_VERSION is semver")
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        self.core.cmp(&other.core).then_with(|| {
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => return Equal,
                (true, false) => return Greater,
                (false, true) => return Less,
                _ => {}
            }
            for (a, b) in self.pre.iter().zip(&other.pre) {
                let o = match (a.parse::<u64>(), b.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Ok(_), Err(_)) => Less,
                    (Err(_), Ok(_)) => Greater,
                    (Err(_), Err(_)) => a.cmp(b),
                };
                if o != Equal {
                    return o;
                }
            }
            self.pre.len().cmp(&other.pre.len())
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [a, b, c] = self.core;
        write!(f, "{a}.{b}.{c}")?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        Ok(())
    }
}

/// Where an install goes.
#[derive(Debug, Clone)]
pub(crate) struct Paths {
    pub home: PathBuf,
    pub bin_dir: PathBuf,
    /// `$XDG_DATA_HOME` (`~/.local/share`).
    pub data: PathBuf,
    /// `$XDG_DATA_DIRS`: where the system's MIME types are.
    pub system_data: Vec<PathBuf>,
}

impl Paths {
    /// From `$HOME`, `$XDG_DATA_HOME` and `$XDG_DATA_DIRS`.
    pub(crate) fn from_env() -> Option<Self> {
        let home = PathBuf::from(std::env::var_os("HOME").filter(|h| !h.is_empty())?);
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|d| d.is_absolute())
            .unwrap_or_else(|| home.join(".local/share"));
        let dirs = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        Some(Self {
            bin_dir: home.join(".local/bin"),
            data,
            system_data: std::env::split_paths(&dirs).collect(),
            home,
        })
    }

    /// Everything under `home` (tests).
    #[cfg(test)]
    fn under(home: &Path) -> Self {
        Self {
            home: home.to_owned(),
            bin_dir: home.join(".local/bin"),
            data: home.join(".local/share"),
            system_data: vec![home.join("usr/share")],
        }
    }

    pub(crate) fn binary(&self) -> PathBuf {
        self.bin_dir.join("meshtard")
    }

    pub(crate) fn sidecar(&self) -> PathBuf {
        self.data.join("meshtard/installed.toml")
    }

    fn desktop(&self) -> PathBuf {
        self.data.join("applications/meshtard.desktop")
    }

    fn mime(&self) -> PathBuf {
        self.data.join("mime/packages/meshtard.xml")
    }

    fn icon(&self, size: &str) -> PathBuf {
        self.data
            .join(format!("icons/hicolor/{size}/apps/meshtard.png"))
    }

    /// Every file an install may write (uninstall removes nothing else).
    fn ours(&self) -> BTreeSet<PathBuf> {
        let mut out: BTreeSet<PathBuf> = [self.binary(), self.desktop(), self.mime()].into();
        out.extend(ICONS.iter().map(|(s, _)| self.icon(s)));
        out
    }

    /// `p` with the home folder as `~`.
    pub(crate) fn tilde(&self, p: &Path) -> String {
        match p.strip_prefix(&self.home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => p.display().to_string(),
        }
    }
}

/// `installed.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Sidecar {
    pub version: String,
    /// SHA-256 of the installed binary, hex.
    pub sha256: String,
    /// Install date, `YYYY-MM-DD` (UTC).
    pub installed: String,
    /// Every file the install wrote.
    pub files: Vec<PathBuf>,
}

pub(crate) fn read_sidecar(paths: &Paths) -> Option<Sidecar> {
    toml::from_str(&std::fs::read_to_string(paths.sidecar()).ok()?).ok()
}

/// What is at `~/.local/bin/meshtard`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Installed {
    Nothing,
    Version(Version),
    /// A file whose version could not be read.
    Unknown,
}

pub(crate) fn installed(paths: &Paths) -> Installed {
    let bin = paths.binary();
    if !bin.is_file() {
        return Installed::Nothing;
    }
    if let Some(v) = read_sidecar(paths).and_then(|s| Version::parse(&s.version)) {
        return Installed::Version(v);
    }
    query_version(&bin, VERSION_TIMEOUT).map_or(Installed::Unknown, Installed::Version)
}

/// Runs `bin --version` without a display (an old build without the flag cannot
/// open a window) and reads `meshtard <version>`; killed after `timeout`.
pub(crate) fn query_version(bin: &Path, timeout: Duration) -> Option<Version> {
    let spawn = || {
        Command::new(bin)
            .arg("--version")
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
    };
    // ETXTBSY: a file just written may still be open in a forking process for a
    // moment.
    let mut child = spawn();
    for _ in 0..5 {
        match &child {
            Err(e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(Duration::from_millis(50));
                child = spawn();
            }
            _ => break,
        }
    }
    let mut child = child.ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let mut words = out.lines().next()?.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("meshtard") {
        return None;
    }
    Version::parse(words.next()?)
}

/// What to do at start.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Decision {
    /// Nothing installed, or an older version: offer to install.
    Offer,
    /// The running binary is the installed one.
    IsInstalled,
    /// The same version is installed.
    Current,
    /// A newer version is installed: no question (a status-bar note).
    Newer(Version),
    /// A file of unknown version is in the way: no question.
    Unknown,
}

pub(crate) fn decide(
    running: &Path,
    installed_path: &Path,
    running_version: &Version,
    installed: &Installed,
) -> Decision {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
    if canon(running) == canon(installed_path) {
        return Decision::IsInstalled;
    }
    match installed {
        Installed::Nothing => Decision::Offer,
        Installed::Unknown => Decision::Unknown,
        Installed::Version(v) if v < running_version => Decision::Offer,
        Installed::Version(v) if v == running_version => Decision::Current,
        Installed::Version(v) => Decision::Newer(v.clone()),
    }
}

/// `p` as an Exec= argument: quoted when it has reserved characters, then with the
/// desktop entry string escapes (Desktop Entry Specification, "The Exec key").
fn exec_arg(p: &Path) -> String {
    let s = p.to_string_lossy().replace('%', "%%");
    let reserved = " \t\n\"'\\><~|&;$*?#()`";
    let arg = if s.chars().any(|c| reserved.contains(c)) {
        let mut q = String::from("\"");
        for c in s.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                q.push('\\');
            }
            q.push(c);
        }
        q.push('"');
        q
    } else {
        s
    };
    arg.replace('\\', "\\\\")
}

/// The desktop entry for the binary at `exec`.
pub(crate) fn desktop_entry(exec: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Meshtard\n\
         GenericName=3D Modeler\n\
         Comment=3D modeling for SketchUp files\n\
         Exec={} %F\n\
         Icon=meshtard\n\
         Terminal=false\n\
         Categories=Graphics;3DGraphics;Engineering;\n\
         MimeType={SKP_MIME};\n\
         StartupWMClass=meshtard\n",
        exec_arg(exec)
    )
}

/// The shared-mime-info package for `*.skp`.
pub(crate) fn mime_package() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n  \
         <mime-type type=\"{SKP_MIME}\">\n    \
         <comment>SketchUp model</comment>\n    \
         <glob pattern=\"*.skp\"/>\n  \
         </mime-type>\n\
         </mime-info>\n"
    )
}

/// The system, or another package of the user's, already defines the .skp type.
fn mime_known(paths: &Paths) -> bool {
    let compiled = format!("mime/{SKP_MIME}.xml");
    if paths
        .system_data
        .iter()
        .any(|d| d.join(&compiled).is_file())
    {
        return true;
    }
    let ours = paths.mime();
    std::fs::read_dir(paths.data.join("mime/packages"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| *p != ours)
        .any(|p| std::fs::read_to_string(p).is_ok_and(|t| t.contains(SKP_MIME)))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    crate::extensions::catalog::sha256_hex(bytes)
}

/// `YYYY-MM-DD` of a time (UTC).
fn date(t: SystemTime) -> String {
    let days = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400) as i64;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Writes `bytes` to `path` through a temporary file in the same folder: fsync,
/// `mode`, rename.
fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    let dir = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().ok_or(std::io::ErrorKind::InvalidInput)?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    fn write_then_rename(tmp: &Path, path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
        let mut f = std::fs::File::create(tmp)?;
        f.write_all(bytes)?;
        f.set_permissions(std::fs::Permissions::from_mode(mode))?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(tmp, path)
    }
    let result = write_then_rename(&tmp, path, bytes, mode);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    } else if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    result
}

/// What an install did.
#[derive(Debug, Clone)]
pub(crate) struct Report {
    pub written: Vec<PathBuf>,
    /// Files already there that Meshtard did not write: left alone.
    pub kept: Vec<PathBuf>,
    pub bin_on_path: bool,
}

/// Installs the binary at `exe` with its icons, desktop entry and MIME type. The
/// caller has decided to replace whatever binary is installed.
pub(crate) fn install(paths: &Paths, exe: &Path) -> Result<Report, String> {
    let bytes = std::fs::read(exe)
        .or_else(|_| std::fs::read("/proc/self/exe"))
        .map_err(|e| format!("Could not read {}: {e}", exe.display()))?;
    let recorded: BTreeSet<PathBuf> =
        read_sidecar(paths).map_or_else(BTreeSet::new, |s| s.files.into_iter().collect());
    let bin = paths.binary();
    let mut files: Vec<(PathBuf, Vec<u8>, u32)> = vec![(bin.clone(), bytes, 0o755)];
    for (size, png) in ICONS {
        files.push((paths.icon(size), png.to_vec(), 0o644));
    }
    files.push((paths.desktop(), desktop_entry(&bin).into_bytes(), 0o644));
    if recorded.contains(&paths.mime()) || !mime_known(paths) {
        files.push((paths.mime(), mime_package().into_bytes(), 0o644));
    }
    let sha256 = sha256_hex(&files[0].1);
    let mut report = Report {
        written: Vec::new(),
        kept: Vec::new(),
        bin_on_path: on_path(&paths.bin_dir),
    };
    let mut error = None;
    for (path, contents, mode) in &files {
        // A file someone else put there stays, unless it is what we would write.
        let mine = *path == bin
            || recorded.contains(path)
            || std::fs::symlink_metadata(path).is_err()
            || std::fs::read(path).is_ok_and(|c| c == *contents);
        if !mine {
            report.kept.push(path.clone());
            continue;
        }
        match write_atomic(path, contents, *mode) {
            Ok(()) => report.written.push(path.clone()),
            Err(e) => {
                error = Some(format!("Could not write {}: {e}", paths.tilde(path)));
                break;
            }
        }
    }
    // What was written is recorded even after a failure, so uninstall finds it.
    if report.written.first() == Some(&bin) {
        let sidecar = Sidecar {
            version: VERSION.into(),
            sha256,
            installed: date(SystemTime::now()),
            files: report.written.clone(),
        };
        let text = toml::to_string(&sidecar).map_err(|e| e.to_string())?;
        let text = format!("# Written by Meshtard's Install for This User.\n{text}");
        if let Err(e) = write_atomic(&paths.sidecar(), text.as_bytes(), 0o644) {
            error.get_or_insert(format!(
                "Could not write {}: {e}",
                paths.tilde(&paths.sidecar())
            ));
        }
    }
    match error {
        Some(e) => Err(e),
        None => Ok(report),
    }
}

/// What an uninstall did.
#[derive(Debug, Clone)]
pub(crate) struct Removed {
    pub removed: Vec<PathBuf>,
    /// Listed files left in place (changed since install, or not ours).
    pub kept: Vec<PathBuf>,
}

/// Removes the files the sidecar lists (only those an install writes; the binary
/// only while unchanged), then the sidecar.
pub(crate) fn uninstall(paths: &Paths) -> Result<Removed, String> {
    let sidecar = read_sidecar(paths).ok_or("Meshtard did not install anything here.")?;
    let ours = paths.ours();
    let bin = paths.binary();
    let mut out = Removed {
        removed: Vec::new(),
        kept: Vec::new(),
    };
    for f in &sidecar.files {
        let changed = *f == bin
            && std::fs::read(f)
                .is_ok_and(|b| !sidecar.sha256.eq_ignore_ascii_case(&sha256_hex(&b)));
        if !ours.contains(f) || changed {
            out.kept.push(f.clone());
            continue;
        }
        match std::fs::remove_file(f) {
            Ok(()) => out.removed.push(f.clone()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Could not remove {}: {e}", paths.tilde(f))),
        }
    }
    std::fs::remove_file(paths.sidecar()).map_err(|e| e.to_string())?;
    if let Some(dir) = paths.sidecar().parent() {
        let _ = std::fs::remove_dir(dir);
    }
    Ok(out)
}

/// Updates the desktop's MIME, desktop entry and icon caches (each tool only if
/// present; failures ignored).
pub(crate) fn refresh_caches(paths: &Paths) {
    let mime = paths.data.join("mime");
    let apps = paths.data.join("applications");
    let icons = paths.data.join("icons/hicolor");
    let runs: [(&str, Vec<&std::ffi::OsStr>); 3] = [
        ("update-mime-database", vec![mime.as_os_str()]),
        (
            "update-desktop-database",
            vec!["-q".as_ref(), apps.as_os_str()],
        ),
        (
            "gtk-update-icon-cache",
            vec![
                "-q".as_ref(),
                "-t".as_ref(),
                "-f".as_ref(),
                icons.as_os_str(),
            ],
        ),
    ];
    for (tool, args) in runs {
        if args.last().is_some_and(|d| Path::new(d).is_dir()) {
            let _ = Command::new(tool)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

/// `dir` is on `$PATH`.
fn on_path(dir: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
    let dir = canon(dir);
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| canon(&d) == dir))
}

// ---------------------------------------------------------------------------------
// The dialogs.

/// The startup check's result.
#[derive(Debug)]
pub(crate) struct Check {
    decision: Decision,
    installed: Installed,
}

#[derive(Debug)]
enum Msg {
    Checked { check: Check, from_menu: bool },
    Installed(Result<Report, String>),
    Uninstalled(Result<Removed, String>),
}

#[derive(Debug)]
enum Dlg {
    /// Install? `startup`: the start-up question (with Don't ask again).
    Offer {
        startup: bool,
        check: Check,
    },
    ConfirmUninstall {
        version: String,
    },
    Info {
        title: &'static str,
        heading: String,
        detail: String,
    },
}

#[derive(Debug, Default)]
pub(crate) struct State {
    started: bool,
    job: Option<Receiver<Msg>>,
    dialog: Option<Dlg>,
    /// The start-up question, waiting for other start-up dialogs to close.
    queued: Option<Dlg>,
}

impl State {
    fn run(&mut self, ctx: &egui::Context, f: impl FnOnce() -> Msg + Send + 'static) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f());
            ctx.request_repaint();
        });
        self.job = Some(rx);
    }

    fn check(&mut self, ctx: &egui::Context, from_menu: bool) {
        self.run(ctx, move || {
            let check = match Paths::from_env() {
                Some(paths) => {
                    let installed = installed(&paths);
                    let exe = std::env::current_exe().unwrap_or_default();
                    Check {
                        decision: decide(&exe, &paths.binary(), &Version::current(), &installed),
                        installed,
                    }
                }
                None => Check {
                    decision: Decision::Unknown,
                    installed: Installed::Unknown,
                },
            };
            Msg::Checked { check, from_menu }
        });
    }
}

/// Preferences > General's Linux lines: the start-up question and Uninstall.
pub(crate) fn prefs_section(ui: &mut egui::Ui, ask: &mut bool, state: &mut State) {
    ui.checkbox(
        ask,
        "Offer to install this version for this user (in ~/.local/bin) at start",
    );
    let sidecar = Paths::from_env().and_then(|p| read_sidecar(&p).map(|s| (p, s)));
    if let Some((paths, s)) = sidecar {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!(
                    "Meshtard {} is installed in {}.",
                    s.version,
                    paths.tilde(&paths.bin_dir)
                ))
                .color(ui.visuals().weak_text_color()),
            );
            if ui.button("Uninstall...").clicked() {
                state.dialog = Some(Dlg::ConfirmUninstall { version: s.version });
            }
        });
    }
}

impl App {
    /// Help > Install for This User.
    pub(crate) fn open_install(&mut self, ctx: &egui::Context) {
        self.prefs.install.check(ctx, true);
    }

    /// Per frame, after the other start-up questions: the start-up check, its
    /// question once no other dialog is up, and the install dialogs.
    pub(crate) fn self_install_ui(&mut self, ctx: &egui::Context) {
        let st = &mut self.prefs.install;
        if !st.started && self.splash.is_done() {
            st.started = true;
            if self.prefs.general.linux_install_ask
                && std::env::var_os("MESHTARD_NO_INSTALL_PROMPT").is_none()
            {
                st.check(ctx, false);
            }
        }
        if let Some(msg) = st.job.as_ref().and_then(|rx| rx.try_recv().ok()) {
            st.job = None;
            self.install_message(msg);
        }
        let st = &mut self.prefs.install;
        let others_open = self.prefs.welcome.is_some() || self.prefs.alt_drag_prompt;
        if st.dialog.is_none() && !others_open {
            st.dialog = st.queued.take();
        }
        let Some(dlg) = st.dialog.take() else {
            return;
        };
        let current = Version::current();
        let paths = Paths::from_env();
        let bin = paths
            .as_ref()
            .map_or("~/.local/bin".into(), |p| p.tilde(&p.bin_dir));
        // (title, heading, detail, buttons, default)
        let (title, heading, detail, buttons, default): (&str, String, String, Vec<&str>, _) =
            match &dlg {
                Dlg::Offer { startup, check } => {
                    let plain = format!(
                        "Install Meshtard {current} for this user? It goes to {bin} and \
                         appears in your applications menu."
                    );
                    match (&check.decision, &check.installed) {
                        (Decision::Newer(v), _) => (
                            "Install for This User",
                            format!(
                                "Meshtard {v}, newer than this one, is installed in {bin}. \
                                 Replace it with {current}?"
                            ),
                            String::new(),
                            vec!["Replace", "Cancel"],
                            Some(1),
                        ),
                        (Decision::Current, _) => (
                            "Install for This User",
                            format!("Meshtard {current} is already installed in {bin}."),
                            "Install it again (binary, icons and menu entry)?".into(),
                            vec!["Reinstall", "Cancel"],
                            Some(1),
                        ),
                        (Decision::Unknown, _) => (
                            "Install for This User",
                            format!("{bin}/meshtard is there, but its version could not be read."),
                            format!("Replace it with Meshtard {current}?"),
                            vec!["Replace", "Cancel"],
                            Some(1),
                        ),
                        (Decision::Offer, Installed::Version(v)) => (
                            "Install for This User",
                            plain,
                            format!("(Replaces Meshtard {v}.)"),
                            if *startup {
                                vec!["Install", "Not now", "Don't ask again"]
                            } else {
                                vec!["Install", "Cancel"]
                            },
                            Some(0),
                        ),
                        (Decision::Offer, _) => (
                            "Install for This User",
                            plain,
                            String::new(),
                            if *startup {
                                vec!["Install", "Not now", "Don't ask again"]
                            } else {
                                vec!["Install", "Cancel"]
                            },
                            Some(0),
                        ),
                        (Decision::IsInstalled, _) => (
                            "Install for This User",
                            format!(
                                "This copy of Meshtard {current} is the installed one ({bin})."
                            ),
                            "Window > Preferences > General uninstalls it.".into(),
                            vec!["OK"],
                            Some(0),
                        ),
                    }
                }
                Dlg::ConfirmUninstall { version } => (
                    "Uninstall",
                    format!("Remove Meshtard {version} from {bin} and the applications menu?"),
                    "Only the files Meshtard installed are removed; your settings stay.".into(),
                    vec!["Uninstall", "Cancel"],
                    Some(1),
                ),
                Dlg::Info {
                    title,
                    heading,
                    detail,
                } => (title, heading.clone(), detail.clone(), vec!["OK"], Some(0)),
            };
        let r = mt_ui::Dialog::new("self_install", title)
            .width(420.0)
            .show(ctx, |ui| {
                crate::export::dialog_visuals(ui);
                mt_ui::dialog::message(ui, &heading, (!detail.is_empty()).then_some(&*detail));
                mt_ui::dialog::buttons(ui, &buttons, default)
            });
        let answer = match r.inner {
            Some(i) => buttons[i],
            None if r.closed => "Cancel",
            None => {
                self.prefs.install.dialog = Some(dlg);
                return;
            }
        };
        mt_ui::dialog::reset(ctx, "self_install");
        let st = &mut self.prefs.install;
        match answer {
            "Install" | "Replace" | "Reinstall" => {
                let Some(paths) = paths else {
                    st.dialog = Some(Dlg::Info {
                        title: "Install for This User",
                        heading: "Could not install: $HOME is not set.".into(),
                        detail: String::new(),
                    });
                    return;
                };
                self.message = Some(format!("Installing Meshtard {current}..."));
                st.run(ctx, move || {
                    let exe = std::env::current_exe().unwrap_or_else(|_| "/proc/self/exe".into());
                    let r = install(&paths, &exe);
                    refresh_caches(&paths);
                    Msg::Installed(r)
                });
            }
            "Uninstall" => {
                if let Some(paths) = paths {
                    st.run(ctx, move || {
                        let r = uninstall(&paths);
                        refresh_caches(&paths);
                        Msg::Uninstalled(r)
                    });
                }
            }
            "Don't ask again" => {
                self.prefs.general.linux_install_ask = false;
                self.save_preferences();
            }
            _ => {}
        }
    }

    fn install_message(&mut self, msg: Msg) {
        let paths = Paths::from_env();
        let tilde = |p: &Path| {
            paths
                .as_ref()
                .map_or(p.display().to_string(), |x| x.tilde(p))
        };
        let st = &mut self.prefs.install;
        match msg {
            Msg::Checked { check, from_menu } => {
                if from_menu {
                    st.dialog = Some(Dlg::Offer {
                        startup: false,
                        check,
                    });
                } else if check.decision == Decision::Offer {
                    st.queued = Some(Dlg::Offer {
                        startup: true,
                        check,
                    });
                } else if let Decision::Newer(v) = &check.decision {
                    self.message = Some(format!(
                        "Meshtard {v} (newer than this {VERSION}) is installed in ~/.local/bin."
                    ));
                }
            }
            Msg::Installed(r) => {
                self.message = None;
                st.dialog = Some(match r {
                    Ok(rep) => {
                        let bin = paths
                            .as_ref()
                            .map_or("~/.local/bin".into(), |p| p.tilde(&p.bin_dir));
                        let mut detail = String::from(
                            "It is in your applications menu (some desktops show it after \
                             logging in again).",
                        );
                        if !rep.bin_on_path {
                            detail.push_str(&format!(
                                "\n\n{bin} is not on your PATH, so typing meshtard in a \
                                 terminal does not find it; the menu entry still works."
                            ));
                        }
                        if !rep.kept.is_empty() {
                            detail.push_str("\n\nLeft alone (not installed by Meshtard):");
                            for k in &rep.kept {
                                detail.push_str(&format!("\n{}", tilde(k)));
                            }
                        }
                        Dlg::Info {
                            title: "Install for This User",
                            heading: format!("Meshtard {VERSION} is installed in {bin}."),
                            detail,
                        }
                    }
                    Err(e) => Dlg::Info {
                        title: "Install for This User",
                        heading: "Meshtard could not be installed.".into(),
                        detail: e,
                    },
                });
            }
            Msg::Uninstalled(r) => {
                st.dialog = Some(match r {
                    Ok(rem) => {
                        let mut detail = format!("{} files removed.", rem.removed.len());
                        if !rem.kept.is_empty() {
                            detail.push_str("\nLeft in place (changed since install):");
                            for k in &rem.kept {
                                detail.push_str(&format!("\n{}", tilde(k)));
                            }
                        }
                        Dlg::Info {
                            title: "Uninstall",
                            heading: "Meshtard is uninstalled for this user.".into(),
                            detail,
                        }
                    }
                    Err(e) => Dlg::Info {
                        title: "Uninstall",
                        heading: "Meshtard could not be uninstalled.".into(),
                        detail: e,
                    },
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    /// A fresh scratch folder standing in for `$HOME`.
    fn temp_home(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mt-self-install-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fake_exe(dir: &Path, script: &str) -> PathBuf {
        let p = dir.join("meshtard-build");
        std::fs::write(&p, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn versions_compare_as_semver() {
        assert!(v("0.1.10") > v("0.1.9"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(v("0.2.0") > v("0.2.0-rc.1"));
        assert!(v("0.2.0-rc.2") > v("0.2.0-rc.1"));
        assert!(v("0.2.0-rc.10") > v("0.2.0-rc.9"));
        assert!(v("0.2.0-beta") > v("0.2.0-alpha.5"));
        assert!(v("0.2.0-alpha.1") > v("0.2.0-alpha"));
        assert!(v("0.2.0-alpha") > v("0.2.0-1"));
        assert_eq!(v("v0.1.2+build.7"), v("0.1.2"));
        assert_eq!(v("0.1.2-rc.1").to_string(), "0.1.2-rc.1");
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "1.x.3",
            "1.2.3-",
            "1.2.3-a..b",
            "-1.2.3",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert_eq!(Version::current().to_string(), VERSION);
    }

    #[test]
    fn decisions() {
        let home = temp_home("decide");
        let paths = Paths::under(&home);
        let running = fake_exe(&home, "true");
        let cur = v("0.2.0");
        let d = |i: &Installed| decide(&running, &paths.binary(), &cur, i);
        assert_eq!(d(&Installed::Nothing), Decision::Offer);
        assert_eq!(d(&Installed::Version(v("0.1.9"))), Decision::Offer);
        assert_eq!(d(&Installed::Version(v("0.2.0-rc.1"))), Decision::Offer);
        assert_eq!(d(&Installed::Version(v("0.2.0"))), Decision::Current);
        assert_eq!(
            d(&Installed::Version(v("0.3.0"))),
            Decision::Newer(v("0.3.0"))
        );
        assert_eq!(d(&Installed::Unknown), Decision::Unknown);
        // Running from the installed path, directly or through a symlink.
        std::fs::create_dir_all(&paths.bin_dir).unwrap();
        std::fs::copy(&running, paths.binary()).unwrap();
        let installed = Installed::Version(v("0.1.0"));
        assert_eq!(
            decide(&paths.binary(), &paths.binary(), &cur, &installed),
            Decision::IsInstalled
        );
        let link = home.join("link");
        std::os::unix::fs::symlink(paths.binary(), &link).unwrap();
        assert_eq!(
            decide(&link, &paths.binary(), &cur, &installed),
            Decision::IsInstalled
        );
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn installed_version_from_sidecar_or_the_binary() {
        let home = temp_home("query");
        let paths = Paths::under(&home);
        assert_eq!(installed(&paths), Installed::Nothing);
        std::fs::create_dir_all(&paths.bin_dir).unwrap();
        let exe = fake_exe(&home, "echo 'meshtard 0.1.1'");
        std::fs::copy(&exe, paths.binary()).unwrap();
        assert_eq!(installed(&paths), Installed::Version(v("0.1.1")));
        // Not a meshtard, or never answering: unknown (killed after the timeout).
        let other = fake_exe(&home, "echo 'other 9.9.9'");
        assert_eq!(query_version(&other, VERSION_TIMEOUT), None);
        let slow = fake_exe(&home, "exec sleep 10");
        let t = Instant::now();
        assert_eq!(query_version(&slow, Duration::from_millis(300)), None);
        assert!(t.elapsed() < Duration::from_secs(5));
        // The sidecar wins.
        let s = Sidecar {
            version: "0.1.5".into(),
            sha256: String::new(),
            installed: "2026-10-13".into(),
            files: vec![],
        };
        std::fs::create_dir_all(paths.sidecar().parent().unwrap()).unwrap();
        std::fs::write(paths.sidecar(), toml::to_string(&s).unwrap()).unwrap();
        assert_eq!(installed(&paths), Installed::Version(v("0.1.5")));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn desktop_entry_content() {
        let e = desktop_entry(Path::new("/home/ann/.local/bin/meshtard"));
        assert!(e.starts_with("[Desktop Entry]\n"));
        for line in [
            "Type=Application",
            "Name=Meshtard",
            "Exec=/home/ann/.local/bin/meshtard %F",
            "Icon=meshtard",
            "Categories=Graphics;3DGraphics;Engineering;",
            "MimeType=application/vnd.sketchup.skp;",
            "Terminal=false",
        ] {
            assert!(e.lines().any(|l| l == line), "{line}\n{e}");
        }
        // Reserved characters: quoted, then string-escaped.
        let e = desktop_entry(Path::new("/home/a b/50%$/meshtard"));
        assert!(
            e.contains("Exec=\"/home/a b/50%%\\\\$/meshtard\" %F\n"),
            "{e}"
        );
        assert!(mime_package().contains("<glob pattern=\"*.skp\"/>"));
    }

    #[test]
    fn install_and_uninstall_touch_only_our_files() {
        let home = temp_home("install");
        let paths = Paths::under(&home);
        let exe = fake_exe(&home, "echo 'meshtard 0.1.2'");
        // Someone else's 48 px icon and files of their own next to ours.
        let foreign_icon = paths.icon("48x48");
        std::fs::create_dir_all(foreign_icon.parent().unwrap()).unwrap();
        std::fs::write(&foreign_icon, b"not ours").unwrap();
        let neighbour = paths.bin_dir.join("other-tool");
        std::fs::create_dir_all(&paths.bin_dir).unwrap();
        std::fs::write(&neighbour, b"x").unwrap();

        let rep = install(&paths, &exe).unwrap();
        assert_eq!(rep.kept, std::slice::from_ref(&foreign_icon));
        let bin = paths.binary();
        assert_eq!(std::fs::read(&bin).unwrap(), std::fs::read(&exe).unwrap());
        let mode = std::fs::metadata(&bin).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        assert!(paths.icon("256x256").is_file());
        let desktop = std::fs::read_to_string(paths.desktop()).unwrap();
        assert!(desktop.contains(&format!("Exec={} %F", bin.display())));
        assert!(paths.mime().is_file());
        assert_eq!(std::fs::read(&foreign_icon).unwrap(), b"not ours");
        // No temporary files left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&paths.bin_dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty());

        let side = read_sidecar(&paths).unwrap();
        assert_eq!(side.version, VERSION);
        assert_eq!(side.sha256, sha256_hex(&std::fs::read(&exe).unwrap()));
        assert_eq!(side.installed.len(), 10);
        assert!(!side.files.contains(&foreign_icon));
        assert_eq!(side.files.len(), 1 + 7 + 1 + 1);
        assert_eq!(installed(&paths), Installed::Version(Version::current()));

        // A sidecar edited to list foreign files does not get them removed.
        let mut tampered = side.clone();
        tampered.files.push(neighbour.clone());
        std::fs::write(paths.sidecar(), toml::to_string(&tampered).unwrap()).unwrap();

        let rem = uninstall(&paths).unwrap();
        assert_eq!(rem.removed.len(), side.files.len());
        for f in &side.files {
            assert!(!f.exists(), "{}", f.display());
        }
        assert!(neighbour.is_file() && foreign_icon.is_file());
        assert!(!paths.sidecar().exists());
        assert!(uninstall(&paths).is_err());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn uninstall_keeps_a_replaced_binary_and_mime_known_is_respected() {
        let home = temp_home("replaced");
        let paths = Paths::under(&home);
        // The system knows .skp already: no MIME package.
        let known = paths.system_data[0].join(format!("mime/{SKP_MIME}.xml"));
        std::fs::create_dir_all(known.parent().unwrap()).unwrap();
        std::fs::write(&known, "").unwrap();
        let exe = fake_exe(&home, "true");
        install(&paths, &exe).unwrap();
        assert!(!paths.mime().exists());
        // The user put another binary there since.
        std::fs::write(paths.binary(), b"#!/bin/sh\n").unwrap();
        let rem = uninstall(&paths).unwrap();
        assert_eq!(rem.kept, [paths.binary()]);
        assert!(paths.binary().is_file());
        assert!(!paths.desktop().exists());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn dates() {
        let t = |s: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(s);
        assert_eq!(date(t(0)), "1970-01-01");
        assert_eq!(date(t(951_782_400)), "2000-02-29");
        assert_eq!(date(t(1_791_849_600)), "2026-10-13");
    }

    #[test]
    fn install_item_is_linux_only_in_the_menus() {
        let spec = mt_spec::Spec::builtin().unwrap();
        assert!(spec.commands().contains_key("help.install_user"));
        let other = spec.without_commands(&["help.install_user"]);
        assert!(!other.commands().contains_key("help.install_user"));
    }
}
