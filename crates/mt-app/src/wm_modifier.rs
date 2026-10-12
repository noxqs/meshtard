//! Linux: lets SketchUp's Alt + drag tools (Orbit without gravity, Lasso, Eraser...)
//! reach Meshtard on desktops whose window manager moves windows with Alt + drag.
//! A Meshtard addition, not in SketchUp.
//!
//! While the preference is on and Meshtard's window has focus, the desktop's
//! window-drag modifier (`mouse-button-modifier`) is switched off through the
//! `gsettings` command; the user's own value is saved first to
//! `wm_modifier_backup.toml` in the config folder and put back (and the file
//! removed) when the window loses focus or Meshtard quits. A backup left by a crash
//! or kill is put back at the next start. A backup that exists is never
//! overwritten, so Meshtard's own "off" value is never taken for the user's.
//!
//! Supported: Cinnamon (Muffin), GNOME (Mutter) and MATE (Marco). The "off" value
//! is `''` for Cinnamon and GNOME and `'disabled'` for MATE (from memory of the
//! window managers' modifier parsers, unverified). KDE and Xfce keep the setting
//! elsewhere; the Preferences page only says where to change it.
//!
//! The first start on a supported desktop whose modifier is Alt asks once whether
//! to turn the preference on.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::App;

/// Longest a `gsettings` call may take before it is stopped.
const TIMEOUT: Duration = Duration::from_millis(1500);
const KEY: &str = "mouse-button-modifier";
const BACKUP_FILE: &str = "wm_modifier_backup.toml";

/// A desktop's window-drag modifier setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Target {
    pub desktop: &'static str,
    pub schema: &'static str,
    /// The value (GVariant text) that turns modified window drags off.
    pub off: &'static str,
}

const CINNAMON: Target = Target {
    desktop: "Cinnamon",
    schema: "org.cinnamon.desktop.wm.preferences",
    off: "''",
};
const GNOME: Target = Target {
    desktop: "GNOME",
    schema: "org.gnome.desktop.wm.preferences",
    off: "''",
};
const MATE: Target = Target {
    desktop: "MATE",
    schema: "org.mate.Marco.general",
    off: "'disabled'",
};
const TARGETS: [Target; 3] = [CINNAMON, MATE, GNOME];

const KDE_HOW: &str = "KDE Plasma: System Settings > Window Management > Window Behavior > \
     Window Actions > Modifier key (Meta instead of Alt), or in a terminal: \
     kwriteconfig6 --file kwinrc --group MouseBindings --key CommandAllKey Meta \
     (kwriteconfig5 on Plasma 5), then log out and back in.";
const XFCE_HOW: &str = "Xfce: Settings > Window Manager Tweaks > Accessibility > \
     Key used to grab and move windows (Super instead of Alt), or in a terminal: \
     xfconf-query -c xfwm4 -p /general/easy_click -s Super";

/// What this desktop allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Support {
    /// Meshtard can switch the setting.
    Auto(Target),
    /// The setting is elsewhere: where to change it by hand.
    Manual(&'static str),
    Unknown,
}

/// Picks the setting from `XDG_CURRENT_DESKTOP` (colon-separated names) and the
/// installed schemas (`gsettings list-schemas`; `None` when that failed). With no
/// desktop name, the first installed schema is used.
pub(crate) fn detect(desktop: &str, schemas: Option<&str>) -> Support {
    let names: Vec<String> = desktop
        .split(':')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let named = |n: &[&str]| names.iter().any(|d| n.iter().any(|x| d.contains(x)));
    if named(&["kde", "plasma"]) {
        return Support::Manual(KDE_HOW);
    }
    if named(&["xfce"]) {
        return Support::Manual(XFCE_HOW);
    }
    let installed = |t: &Target| schemas.is_some_and(|s| s.lines().any(|l| l.trim() == t.schema));
    let wanted = if named(&["cinnamon"]) {
        Some(CINNAMON)
    } else if named(&["mate"]) {
        Some(MATE)
    } else if named(&["gnome", "unity", "ubuntu", "budgie", "pantheon"]) {
        Some(GNOME)
    } else {
        None
    };
    match wanted {
        Some(t) if installed(&t) => Support::Auto(t),
        Some(_) => Support::Unknown,
        None if names.is_empty() => TARGETS
            .into_iter()
            .find(installed)
            .map_or(Support::Unknown, Support::Auto),
        None => Support::Unknown,
    }
}

/// Whether a modifier value (`gsettings get` output) is Alt.
fn is_alt(value: &str) -> bool {
    let v = value.to_ascii_lowercase();
    v.contains("alt") || v.contains("mod1")
}

/// Runs `gsettings` with `args`: its trimmed output, `None` on any failure.
pub(crate) trait Runner {
    fn run(&mut self, args: &[&str]) -> Option<String>;
}

/// The real `gsettings` command, stopped after [`TIMEOUT`].
struct Cli;

impl Runner for Cli {
    fn run(&mut self, args: &[&str]) -> Option<String> {
        let mut child = Command::new("gsettings")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut out = child.stdout.take()?;
        // Read on a thread so a long output (list-schemas) cannot fill the pipe.
        let reader = std::thread::spawn(move || {
            let mut s = String::new();
            out.read_to_string(&mut s).map(|_| s)
        });
        let deadline = Instant::now() + TIMEOUT;
        let ok = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.success(),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                _ => {
                    log::warn!("gsettings {} did not finish; stopped", args.join(" "));
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                }
            }
        };
        let text = reader.join().ok()?.ok()?;
        ok.then(|| text.trim().to_owned())
    }
}

/// The user's own value, kept while Meshtard has switched it off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Backup {
    pub schema: String,
    pub key: String,
    /// As `gsettings get` printed it (GVariant text, e.g. `'<Alt>'`).
    pub value: String,
}

/// Where the backup is kept.
pub(crate) trait Store {
    fn load(&self) -> Option<Backup>;
    fn save(&mut self, b: &Backup) -> bool;
    fn remove(&mut self);
}

/// `wm_modifier_backup.toml` in the config folder.
struct FileStore(Option<PathBuf>);

impl Store for FileStore {
    fn load(&self) -> Option<Backup> {
        let text = std::fs::read_to_string(self.0.as_ref()?).ok()?;
        match toml::from_str(&text) {
            Ok(b) => Some(b),
            Err(e) => {
                log::warn!("ignoring {BACKUP_FILE}: {e}");
                None
            }
        }
    }

    fn save(&mut self, b: &Backup) -> bool {
        let Some(path) = &self.0 else {
            return false;
        };
        let Ok(text) = toml::to_string(b) else {
            return false;
        };
        let tmp = path.with_extension("toml.tmp");
        let result = std::fs::create_dir_all(path.parent().expect("has a parent"))
            .and_then(|()| std::fs::write(&tmp, text))
            .and_then(|()| std::fs::rename(&tmp, path));
        if let Err(e) = &result {
            log::warn!("could not save {}: {e}", path.display());
        }
        result.is_ok()
    }

    fn remove(&mut self) {
        if let Some(p) = &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Switches the window-drag modifier off while wanted, and back.
pub(crate) struct AltDrag {
    runner: Box<dyn Runner>,
    store: Box<dyn Store>,
    desktop: String,
    support: Option<Support>,
    /// Whether the setting is wanted off (preference on and window focused).
    wanted: bool,
    /// The first-start question was considered this run.
    prompt_checked: bool,
}

impl std::fmt::Debug for AltDrag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AltDrag")
            .field("support", &self.support)
            .field("wanted", &self.wanted)
            .finish_non_exhaustive()
    }
}

impl Default for AltDrag {
    /// The real `gsettings` and backup file (nothing runs until used).
    fn default() -> Self {
        Self::new(
            Box::new(Cli),
            Box::new(FileStore(
                crate::settings::config_dir().map(|d| d.join(BACKUP_FILE)),
            )),
            std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
        )
    }
}

impl AltDrag {
    pub(crate) fn new(runner: Box<dyn Runner>, store: Box<dyn Store>, desktop: String) -> Self {
        Self {
            runner,
            store,
            desktop,
            support: None,
            wanted: false,
            prompt_checked: false,
        }
    }

    /// This desktop's support (detected once).
    pub(crate) fn support(&mut self) -> Support {
        if let Some(s) = self.support {
            return s;
        }
        let schemas = self.runner.run(&["list-schemas"]);
        let s = detect(&self.desktop, schemas.as_deref());
        self.support = Some(s);
        s
    }

    /// At start: puts back a value left by a run that did not quit normally.
    pub(crate) fn recover(&mut self) {
        if self.store.load().is_some() {
            log::info!("restoring the window-drag modifier left off by the last run");
            self.restore();
        }
    }

    /// Call every frame with whether the setting should be off now.
    pub(crate) fn update(&mut self, off: bool) {
        if off == self.wanted {
            return;
        }
        self.wanted = off;
        if off {
            self.engage();
        } else {
            self.restore();
        }
    }

    /// Normal exit: the user's value goes back.
    pub(crate) fn exit(&mut self) {
        self.wanted = false;
        self.restore();
    }

    /// Whether to ask the first-start question: once per run, on a supported desktop
    /// that moves windows with Alt.
    pub(crate) fn should_ask(&mut self) -> bool {
        if std::mem::replace(&mut self.prompt_checked, true) {
            return false;
        }
        let Support::Auto(t) = self.support() else {
            return false;
        };
        self.runner
            .run(&["get", t.schema, KEY])
            .is_some_and(|v| is_alt(&v))
    }

    fn engage(&mut self) {
        let Support::Auto(t) = self.support() else {
            return;
        };
        let Some(current) = self.runner.run(&["get", t.schema, KEY]) else {
            return;
        };
        if current == t.off {
            // Already off: by the user, or by Meshtard with the backup still kept.
            return;
        }
        // An existing backup is the user's value from before; never replace it.
        if self.store.load().is_none() {
            if !is_alt(&current) {
                // Another modifier (Super): Alt already reaches Meshtard.
                return;
            }
            let b = Backup {
                schema: t.schema.to_owned(),
                key: KEY.to_owned(),
                value: current,
            };
            if !self.store.save(&b) {
                // Never turn it off without a way back.
                return;
            }
        }
        if self.runner.run(&["set", t.schema, KEY, t.off]).is_none() {
            log::warn!("could not turn off {} {KEY}", t.schema);
        }
    }

    fn restore(&mut self) {
        let Some(b) = self.store.load() else {
            return;
        };
        let known = TARGETS.iter().any(|t| t.schema == b.schema) && b.key == KEY;
        if !known {
            log::warn!(
                "ignoring {BACKUP_FILE}: unknown setting {} {}",
                b.schema,
                b.key
            );
            self.store.remove();
            return;
        }
        if self
            .runner
            .run(&["set", &b.schema, &b.key, &b.value])
            .is_some()
        {
            self.store.remove();
        } else {
            // Kept for the next focus loss, exit or start.
            log::warn!("could not restore {} {}", b.schema, b.key);
        }
    }
}

/// The Linux group on Preferences > General.
pub(crate) fn prefs_section(ui: &mut egui::Ui, on: &mut bool, support: Support) {
    ui.add_space(6.0);
    ui.strong("Linux");
    match support {
        Support::Auto(t) => {
            ui.checkbox(
                on,
                "Let Meshtard use Alt + drag (temporarily turns off the desktop's Alt + \
                 drag window move while Meshtard is focused)",
            );
            let weak = ui.visuals().weak_text_color();
            ui.label(
                egui::RichText::new(format!(
                    "Changes the {} setting {} {KEY} while Meshtard is in front and puts \
                     your value back when you switch away or quit. If Meshtard is killed, \
                     it is put back at the next start; until then, windows do not move \
                     with Alt + drag.",
                    t.desktop, t.schema
                ))
                .color(weak),
            );
        }
        Support::Manual(how) => {
            ui.label("Alt + drag tools need the desktop to move windows with another key.");
            ui.label(egui::RichText::new(how).color(ui.visuals().weak_text_color()));
        }
        Support::Unknown => {
            ui.label(
                egui::RichText::new(
                    "If your desktop moves windows with Alt + drag, set it to use another \
                     key (such as Super) so SketchUp's Alt tools reach Meshtard.",
                )
                .color(ui.visuals().weak_text_color()),
            );
        }
    }
}

impl App {
    /// Every frame: the first-start question, and the setting following focus.
    pub(crate) fn alt_drag_tick(&mut self, ctx: &egui::Context) {
        if !self.prefs.general.linux_alt_drag_asked && self.splash.is_done() {
            if !self.prefs.alt_drag_prompt && self.prefs.alt_drag.should_ask() {
                self.prefs.alt_drag_prompt = true;
            }
            if self.prefs.alt_drag_prompt {
                let r = mt_ui::Dialog::new("alt_drag", "Alt + drag")
                    .width(380.0)
                    .show(ctx, |ui| {
                        mt_ui::dialog::message(
                            ui,
                            "Your desktop moves windows with Alt + drag, which blocks \
                         SketchUp's Alt tools. Let Meshtard turn it off while Meshtard is \
                         in front?",
                            Some(
                                "(Your setting comes back when you switch away or quit. \
                             Window > Preferences > General turns this off.)",
                            ),
                        );
                        mt_ui::dialog::buttons(ui, &["Allow", "Not now"], Some(0))
                    });
                let answer = match r.inner {
                    Some(0) => Some(true),
                    Some(_) => Some(false),
                    None if r.closed => Some(false),
                    None => None,
                };
                if let Some(allow) = answer {
                    self.prefs.alt_drag_prompt = false;
                    self.prefs.general.linux_alt_drag_asked = true;
                    self.prefs.general.linux_alt_drag |= allow;
                    self.save_preferences();
                }
            }
        }
        let focused = ctx.input(|i| i.focused);
        let off = self.prefs.general.linux_alt_drag && focused;
        self.prefs.alt_drag.update(off);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Desk {
        schemas: Vec<&'static str>,
        values: HashMap<String, String>,
        fail_set: bool,
        sets: Vec<String>,
        backup: Option<Backup>,
        fail_save: bool,
    }

    type Shared = Arc<Mutex<Desk>>;

    struct FakeRunner(Shared);
    impl Runner for FakeRunner {
        fn run(&mut self, args: &[&str]) -> Option<String> {
            let mut d = self.0.lock().unwrap();
            match args {
                ["list-schemas"] => Some(d.schemas.join("\n")),
                ["get", s, k] => d.values.get(&format!("{s} {k}")).cloned(),
                ["set", s, k, v] => {
                    if d.fail_set {
                        return None;
                    }
                    d.sets.push((*v).to_owned());
                    d.values.insert(format!("{s} {k}"), (*v).to_owned());
                    Some(String::new())
                }
                _ => None,
            }
        }
    }

    struct FakeStore(Shared);
    impl Store for FakeStore {
        fn load(&self) -> Option<Backup> {
            self.0.lock().unwrap().backup.clone()
        }
        fn save(&mut self, b: &Backup) -> bool {
            let mut d = self.0.lock().unwrap();
            if d.fail_save {
                return false;
            }
            d.backup = Some(b.clone());
            true
        }
        fn remove(&mut self) {
            self.0.lock().unwrap().backup = None;
        }
    }

    const CIN: &str = "org.cinnamon.desktop.wm.preferences mouse-button-modifier";

    fn cinnamon(value: &str) -> Shared {
        let mut d = Desk {
            schemas: vec!["org.gnome.desktop.wm.preferences", CINNAMON.schema],
            ..Default::default()
        };
        d.values.insert(CIN.into(), value.into());
        Arc::new(Mutex::new(d))
    }

    fn alt_drag(s: &Shared, desktop: &str) -> AltDrag {
        AltDrag::new(
            Box::new(FakeRunner(s.clone())),
            Box::new(FakeStore(s.clone())),
            desktop.into(),
        )
    }

    fn value(s: &Shared) -> String {
        s.lock().unwrap().values[CIN].clone()
    }

    #[test]
    fn detects_by_desktop_and_schema() {
        let all = "org.gnome.desktop.wm.preferences\norg.cinnamon.desktop.wm.preferences\n\
                   org.mate.Marco.general";
        assert_eq!(detect("X-Cinnamon", Some(all)), Support::Auto(CINNAMON));
        assert_eq!(detect("ubuntu:GNOME", Some(all)), Support::Auto(GNOME));
        assert_eq!(detect("MATE", Some(all)), Support::Auto(MATE));
        assert_eq!(detect("KDE", Some(all)), Support::Manual(KDE_HOW));
        assert_eq!(detect("XFCE", Some(all)), Support::Manual(XFCE_HOW));
        assert_eq!(detect("X-Cinnamon", None), Support::Unknown);
        assert_eq!(detect("i3", Some(all)), Support::Unknown);
        assert_eq!(detect("", Some(all)), Support::Auto(CINNAMON));
        assert_eq!(
            detect("", Some("org.gnome.desktop.wm.preferences")),
            Support::Auto(GNOME)
        );
    }

    #[test]
    fn focus_in_turns_off_and_focus_out_restores() {
        let s = cinnamon("'<Alt>'");
        let mut a = alt_drag(&s, "X-Cinnamon");
        a.update(true);
        assert_eq!(value(&s), "''");
        assert_eq!(s.lock().unwrap().backup.as_ref().unwrap().value, "'<Alt>'");
        // Staying focused does not run anything again.
        a.update(true);
        assert_eq!(s.lock().unwrap().sets.len(), 1);
        a.update(false);
        assert_eq!(value(&s), "'<Alt>'");
        assert!(s.lock().unwrap().backup.is_none());
        // Again on the next focus.
        a.update(true);
        assert_eq!(value(&s), "''");
        a.exit();
        assert_eq!(value(&s), "'<Alt>'");
        assert!(s.lock().unwrap().backup.is_none());
    }

    #[test]
    fn never_saves_own_value_as_the_original() {
        let s = cinnamon("'<Alt>'");
        let mut a = alt_drag(&s, "X-Cinnamon");
        a.update(true);
        // Restoring fails: the backup stays and the value stays off.
        s.lock().unwrap().fail_set = true;
        a.update(false);
        assert!(s.lock().unwrap().backup.is_some());
        s.lock().unwrap().fail_set = false;
        a.update(true);
        assert_eq!(s.lock().unwrap().backup.as_ref().unwrap().value, "'<Alt>'");
        // Another run finds the value off and a backup: keeps the backup.
        let mut b = alt_drag(&s, "X-Cinnamon");
        b.update(true);
        assert_eq!(s.lock().unwrap().backup.as_ref().unwrap().value, "'<Alt>'");
        b.update(false);
        assert_eq!(value(&s), "'<Alt>'");
    }

    #[test]
    fn existing_backup_is_not_overwritten() {
        let s = cinnamon("'<Super>'");
        s.lock().unwrap().backup = Some(Backup {
            schema: CINNAMON.schema.into(),
            key: KEY.into(),
            value: "'<Alt>'".into(),
        });
        let mut a = alt_drag(&s, "X-Cinnamon");
        a.update(true);
        assert_eq!(s.lock().unwrap().backup.as_ref().unwrap().value, "'<Alt>'");
        a.update(false);
        assert_eq!(value(&s), "'<Alt>'");
    }

    #[test]
    fn crash_recovery_at_start() {
        let s = cinnamon("''");
        s.lock().unwrap().backup = Some(Backup {
            schema: CINNAMON.schema.into(),
            key: KEY.into(),
            value: "'<Alt>'".into(),
        });
        let mut a = alt_drag(&s, "X-Cinnamon");
        a.recover();
        assert_eq!(value(&s), "'<Alt>'");
        assert!(s.lock().unwrap().backup.is_none());
        // Nothing to recover: nothing runs.
        a.recover();
        assert_eq!(s.lock().unwrap().sets.len(), 1);
    }

    #[test]
    fn unknown_backup_setting_is_dropped() {
        let s = cinnamon("'<Alt>'");
        s.lock().unwrap().backup = Some(Backup {
            schema: "org.example".into(),
            key: "other".into(),
            value: "'x'".into(),
        });
        let mut a = alt_drag(&s, "X-Cinnamon");
        a.recover();
        assert!(s.lock().unwrap().sets.is_empty());
        assert!(s.lock().unwrap().backup.is_none());
    }

    #[test]
    fn leaves_other_modifiers_and_off_values_alone() {
        for v in ["'<Super>'", "''"] {
            let s = cinnamon(v);
            let mut a = alt_drag(&s, "X-Cinnamon");
            a.update(true);
            a.update(false);
            assert_eq!(value(&s), v);
            assert!(s.lock().unwrap().sets.is_empty());
            assert!(s.lock().unwrap().backup.is_none());
        }
    }

    #[test]
    fn no_backup_no_change() {
        let s = cinnamon("'<Alt>'");
        s.lock().unwrap().fail_save = true;
        let mut a = alt_drag(&s, "X-Cinnamon");
        a.update(true);
        assert_eq!(value(&s), "'<Alt>'");
        assert!(s.lock().unwrap().sets.is_empty());
    }

    #[test]
    fn unsupported_desktop_does_nothing() {
        let s = cinnamon("'<Alt>'");
        let mut a = alt_drag(&s, "KDE");
        assert!(!a.should_ask());
        a.update(true);
        a.exit();
        assert!(s.lock().unwrap().sets.is_empty());
    }

    #[test]
    fn asks_once_when_alt_moves_windows() {
        let s = cinnamon("'<Alt>'");
        let mut a = alt_drag(&s, "X-Cinnamon");
        assert!(a.should_ask());
        assert!(!a.should_ask());
        let s = cinnamon("'<Super>'");
        assert!(!alt_drag(&s, "X-Cinnamon").should_ask());
    }

    #[test]
    fn file_store_round_trip() {
        let dir = std::env::temp_dir().join(format!("mt_wm_backup_{}", std::process::id()));
        let mut f = FileStore(Some(dir.join(BACKUP_FILE)));
        assert!(f.load().is_none());
        let b = Backup {
            schema: GNOME.schema.into(),
            key: KEY.into(),
            value: "'<Alt>'".into(),
        };
        assert!(f.save(&b));
        assert_eq!(f.load(), Some(b));
        f.remove();
        assert!(f.load().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
