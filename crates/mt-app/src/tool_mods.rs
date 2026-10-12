//! Modifier keys inside tools, per platform (FEATURES.md 76, spec/parity/MAC.md).
//!
//! Tools ask for a role ("copy", "alternate"), not a key. SketchUp for Mac uses
//! Option where Windows uses Ctrl and, for most tools, Command where Windows uses Alt;
//! a few tools whose Windows key is Alt take Option on the Mac instead
//! ([`Role::AltOption`]). The mapping is from the 2026 Quick Reference Cards, Mac and
//! Windows [doc], and the help page "Introducing Drawing Basics and Concepts" (Line
//! inference cycle, Rotate / Protractor free center) [doc]; nothing was checked in
//! SketchUp for Mac. Windows and Linux keep Ctrl and Alt.
//!
//! [`Platform::current`] picks the mapping; tests pass a platform to check both on
//! every system.

use eframe::egui;

use crate::cursor::Mods;
use crate::nav::Tool;

/// Whose modifier keys apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    /// Windows and Linux: Ctrl and Alt.
    Pc,
    /// macOS: Option and Command.
    Mac,
}

impl Platform {
    /// The system this build runs on.
    pub(crate) const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Pc
        }
    }
}

/// The physical modifier keys held (egui's names: on the Mac `alt` is Option).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Keys {
    pub ctrl: bool,
    /// Alt; Option on the Mac.
    pub alt: bool,
    /// The Mac's Command key (never set on Windows and Linux).
    pub cmd: bool,
    pub shift: bool,
}

impl Keys {
    pub(crate) fn from_egui(m: egui::Modifiers) -> Self {
        Self {
            ctrl: m.ctrl,
            alt: m.alt,
            cmd: m.mac_cmd,
            shift: m.shift,
        }
    }
}

/// What a modifier does in a tool, independent of the key that does it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// Ctrl / Option: Move, Rotate, Push/Pull and Flip copy; Scale about the center;
    /// Paint Bucket fill; Eraser soften; Select add; Tape Measure and Protractor
    /// guide toggle; Tag tool all instances; Walk run.
    Copy,
    /// Alt / Command: Move auto-fold; Offset overlap; Follow Me face perimeter; Paint
    /// Bucket and Tag sample; Rotated Rectangle plane; Line inference cycle;
    /// Rotate and Protractor free center; Shift+Alt plane lock; grip cycling.
    Alternate,
    /// Alt / Option: Orbit without gravity; Lasso add; Rectangle from the center;
    /// arcs' segment count with + and -.
    AltOption,
}

/// A modifier key's name as SketchUp writes it on each system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Key {
    Ctrl,
    Alt,
    Option,
    Command,
}

impl Key {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Ctrl => "Ctrl",
            Self::Alt => "Alt",
            Self::Option => "Option",
            Self::Command => "Command",
        }
    }

    fn held(self, k: Keys) -> bool {
        match self {
            Self::Ctrl => k.ctrl,
            Self::Alt | Self::Option => k.alt,
            Self::Command => k.cmd,
        }
    }
}

/// Tools whose Windows Alt is Option on the Mac (Quick Reference Card 2026, Mac).
pub(crate) fn alt_is_option(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::Orbit
            | Tool::Lasso
            | Tool::Draw("tool.rectangle" | "tool.arc" | "tool.arc_2pt" | "tool.arc_3pt")
    )
}

impl Platform {
    /// The key that plays `role`.
    pub(crate) fn key(self, role: Role) -> Key {
        match (self, role) {
            (Self::Pc, Role::Copy) => Key::Ctrl,
            (Self::Pc, Role::Alternate | Role::AltOption) => Key::Alt,
            (Self::Mac, Role::Copy | Role::AltOption) => Key::Option,
            (Self::Mac, Role::Alternate) => Key::Command,
        }
    }

    /// Whether the key of `role` is held.
    pub(crate) fn held(self, keys: Keys, role: Role) -> bool {
        self.key(role).held(keys)
    }

    /// The flags tools read, in Windows names: `ctrl` is [`Role::Copy`], `alt` is
    /// [`Role::Alternate`], or [`Role::AltOption`] in the tools of
    /// [`alt_is_option`].
    pub(crate) fn mods(self, keys: Keys, tool: Tool) -> Mods {
        let alt = if alt_is_option(tool) {
            Role::AltOption
        } else {
            Role::Alternate
        };
        Mods {
            ctrl: self.held(keys, Role::Copy),
            shift: keys.shift,
            alt: self.held(keys, alt),
        }
    }

    /// A status bar or Instructor text written with Windows keys, in this
    /// platform's names ("Ctrl" and "Alt" as whole words).
    pub(crate) fn hint(self, text: &str, tool: Tool) -> String {
        if self == Self::Pc {
            return text.to_owned();
        }
        let alt = if alt_is_option(tool) {
            Role::AltOption
        } else {
            Role::Alternate
        };
        let mut out = String::with_capacity(text.len() + 8);
        let mut word = String::new();
        let flush = |word: &mut String, out: &mut String| {
            out.push_str(match word.as_str() {
                "Ctrl" => self.key(Role::Copy).name(),
                "Alt" => self.key(alt).name(),
                w => w,
            });
            word.clear();
        };
        for c in text.chars() {
            if c.is_alphanumeric() {
                word.push(c);
            } else {
                flush(&mut word, &mut out);
                out.push(c);
            }
        }
        flush(&mut word, &mut out);
        out
    }
}

/// The current platform's tool flags for egui's modifiers.
pub(crate) fn mods(m: egui::Modifiers, tool: Tool) -> Mods {
    Platform::current().mods(Keys::from_egui(m), tool)
}

/// Whether the current platform's key of `role` is held.
pub(crate) fn held(m: egui::Modifiers, role: Role) -> bool {
    Platform::current().held(Keys::from_egui(m), role)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: Keys = Keys {
        ctrl: true,
        alt: false,
        cmd: false,
        shift: false,
    };
    const ALT: Keys = Keys {
        ctrl: false,
        alt: true,
        cmd: false,
        shift: false,
    };
    const CMD: Keys = Keys {
        ctrl: false,
        alt: false,
        cmd: true,
        shift: false,
    };

    fn m(ctrl: bool, alt: bool) -> Mods {
        Mods {
            ctrl,
            shift: false,
            alt,
        }
    }

    #[test]
    fn windows_and_linux_keep_ctrl_and_alt() {
        let p = Platform::Pc;
        let mv = Tool::Modify("tool.move");
        assert_eq!(p.mods(CTRL, mv), m(true, false));
        assert_eq!(p.mods(ALT, mv), m(false, true));
        assert_eq!(p.mods(CMD, mv), m(false, false));
        // Alt is Alt in the Option tools too.
        assert_eq!(p.mods(ALT, Tool::Orbit), m(false, true));
        assert_eq!(p.mods(ALT, Tool::Lasso), m(false, true));
        assert!(p.held(CTRL, Role::Copy) && !p.held(ALT, Role::Copy));
        assert!(p.held(ALT, Role::Alternate) && p.held(ALT, Role::AltOption));
    }

    #[test]
    fn mac_copy_is_option_and_alternate_is_command() {
        let p = Platform::Mac;
        // Move: Option copies, Command auto-folds (QRC 2026 Mac).
        let mv = Tool::Modify("tool.move");
        assert_eq!(p.mods(ALT, mv), m(true, false));
        assert_eq!(p.mods(CMD, mv), m(false, true));
        // The Control key does nothing in tools on the Mac.
        assert_eq!(p.mods(CTRL, mv), m(false, false));
        // Paint Bucket: Command samples, Option fills.
        let paint = Tool::Modify("tool.paint_bucket");
        assert_eq!(p.mods(CMD, paint), m(false, true));
        assert_eq!(p.mods(ALT, paint), m(true, false));
        // Line: Command cycles the inferences (drawing-basics page).
        assert_eq!(p.mods(CMD, Tool::Draw("tool.line")), m(false, true));
    }

    #[test]
    fn mac_option_tools() {
        let p = Platform::Mac;
        // Orbit, Lasso, Rectangle and the arcs take Option where Windows has Alt.
        for tool in [
            Tool::Orbit,
            Tool::Lasso,
            Tool::Draw("tool.rectangle"),
            Tool::Draw("tool.arc_3pt"),
        ] {
            assert!(p.mods(ALT, tool).alt, "{tool:?}");
            assert!(!p.mods(CMD, tool).alt, "{tool:?}");
        }
        // Lasso: Option adds, Shift+Option subtracts.
        use crate::selection::Mode;
        let k = p.mods(ALT, Tool::Lasso);
        assert_eq!(Mode::for_lasso(k.ctrl, k.shift, k.alt), Mode::Add);
        let k = p.mods(Keys { shift: true, ..ALT }, Tool::Lasso);
        assert_eq!(Mode::for_lasso(k.ctrl, k.shift, k.alt), Mode::Remove);
        // Eraser: Option softens, Option+Shift unsoftens.
        let e = Tool::Eraser;
        let k = p.mods(ALT, e);
        assert_eq!(
            crate::cursor::eraser_mode(k.ctrl, k.shift, k.alt),
            (false, true)
        );
        let k = p.mods(Keys { shift: true, ..ALT }, e);
        assert_eq!(
            crate::cursor::eraser_mode(k.ctrl, k.shift, k.alt),
            (true, true)
        );
        assert_eq!(p.key(Role::AltOption), Key::Option);
    }

    #[test]
    fn hints_use_the_platform_names() {
        let mv = Tool::Modify("tool.move");
        let text = "Ctrl = toggle copy. Alt = auto-fold. Ctrl+Shift, Altitude.";
        assert_eq!(Platform::Pc.hint(text, mv), text);
        assert_eq!(
            Platform::Mac.hint(text, mv),
            "Option = toggle copy. Command = auto-fold. Option+Shift, Altitude."
        );
        assert_eq!(
            Platform::Mac.hint("Alt = draw from center.", Tool::Draw("tool.rectangle")),
            "Option = draw from center."
        );
    }
}
