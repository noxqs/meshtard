//! Fonts of dimension and text labels (Model Info > Dimensions and Text, LEFTOVERS
//! item 11). A style names a font by its full name, as the 3D Text dialog lists them
//! (the two bundled fonts and the TrueType / OpenType fonts of the system font
//! folders); the first time a label needs it, its file is loaded into egui as a font
//! family of its own, with the application font behind it for missing glyphs.
//!
//! SketchUp opens the system font dialog (family, style and size); here a list of the
//! fonts and the size choice stand in for it (unverified). Fonts are model-only: they
//! are not saved in the .skp file. A model whose font is not installed shows its
//! labels in the application font.

use std::collections::HashSet;
use std::sync::Arc;

use eframe::egui;
use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};

/// egui's family name of the label font `font`.
pub(crate) fn family_name(font: &str) -> String {
    format!("mt-label:{font}")
}

/// Label fonts asked for and the ones that could not be found.
#[derive(Debug, Default)]
pub(crate) struct LabelFonts {
    requested: HashSet<String>,
    missing: HashSet<String>,
}

impl LabelFonts {
    /// The egui family to draw a label in `font` with: its own once loaded, else the
    /// proportional (application) font. The first call for a font starts loading it;
    /// egui takes it from the next frame on.
    pub(crate) fn family(&mut self, ctx: &egui::Context, font: Option<&str>) -> egui::FontFamily {
        let Some(font) = font.filter(|f| !f.is_empty()) else {
            return egui::FontFamily::Proportional;
        };
        let name = family_name(font);
        let family = egui::FontFamily::Name(name.as_str().into());
        if ctx.fonts(|f| f.definitions().families.contains_key(&family)) {
            return family;
        }
        if self.missing.contains(font) || !self.requested.insert(font.to_owned()) {
            return egui::FontFamily::Proportional;
        }
        match crate::text3d::font_data(font) {
            Some((data, index)) => {
                let mut data = egui::FontData::from_owned(data);
                data.index = index;
                ctx.add_font(FontInsert::new(
                    &name,
                    data,
                    vec![InsertFontFamily {
                        family: family.clone(),
                        priority: FontPriority::Highest,
                    }],
                ));
                // Glyphs the font lacks come from the application font.
                ctx.add_font(FontInsert::new(
                    &format!("{name} fallback"),
                    egui::FontData::from_static(epaint_default_fonts::UBUNTU_LIGHT),
                    vec![InsertFontFamily {
                        family,
                        priority: FontPriority::Lowest,
                    }],
                ));
                ctx.request_repaint();
            }
            None => {
                log::warn!("label font {font:?} not found; using the application font");
                self.missing.insert(font.to_owned());
            }
        }
        egui::FontFamily::Proportional
    }
}

/// The font names offered, read once when a font list first opens.
pub(crate) fn names() -> Arc<Vec<String>> {
    static NAMES: std::sync::OnceLock<Arc<Vec<String>>> = std::sync::OnceLock::new();
    NAMES
        .get_or_init(|| Arc::new(crate::text3d::font_names()))
        .clone()
}

/// The text shown for a style's font.
pub(crate) fn label(font: Option<&str>) -> &str {
    font.unwrap_or("Default")
}

/// A font choice: "Default" (the application font) and the fonts of [`names`];
/// changes `font` and returns whether it changed. The list is read only when the
/// combo box opens (scanning the system fonts takes a moment the first time).
pub(crate) fn combo(ui: &mut egui::Ui, id: &str, font: &mut Option<String>) -> bool {
    let before = font.clone();
    egui::ComboBox::from_id_salt(id)
        .selected_text(label(font.as_deref()))
        .width(180.0)
        .height(300.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(font, None, "Default");
            for n in names().iter() {
                if ui
                    .selectable_label(font.as_deref() == Some(n.as_str()), n)
                    .clicked()
                {
                    *font = Some(n.clone());
                }
            }
        });
    *font != before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_fonts_load_as_their_own_family() {
        let ctx = egui::Context::default();
        let mut fonts = LabelFonts::default();
        let mut families = Vec::new();
        // Frame 1 asks for the font, frame 2 draws with it.
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                families.push(fonts.family(ui.ctx(), Some("Hack")));
                let none = fonts.family(ui.ctx(), None);
                assert_eq!(none, egui::FontFamily::Proportional);
                let missing = fonts.family(ui.ctx(), Some("No Such Font 123"));
                assert_eq!(missing, egui::FontFamily::Proportional);
            });
            out.textures_delta.clear();
        }
        assert_eq!(families[0], egui::FontFamily::Proportional);
        assert_eq!(
            families[1],
            egui::FontFamily::Name(family_name("Hack").into())
        );
        // Text lays out in it, glyphs it lacks from the fallback.
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let id = egui::FontId::new(13.0, families[1].clone());
            let g = ui
                .painter()
                .layout_no_wrap("10' 6\" ²".into(), id, egui::Color32::BLACK);
            assert!(g.size().x > 10.0);
        });
        out.textures_delta.clear();
        assert!(fonts.missing.contains("No Such Font 123"));
    }

    #[test]
    fn names_start_with_the_bundled_fonts() {
        let n = names();
        assert_eq!(&n[..2], ["Ubuntu Light", "Hack"]);
        assert_eq!(label(None), "Default");
    }
}
