//! Tools > 3D Text (FEATURES.md item 31): a dialog for the text, font, alignment,
//! height, Filled and Extruded; the glyph outlines (read with `ttf-parser`) become
//! faces with holes, pushed up with Push/Pull, in a new component that the Place
//! Component tool then places (glued to faces, like SketchUp's 3D text; unverified).
//!
//! Fonts: two bundled with the app through `epaint_default_fonts` (egui's own fonts,
//! already in the binary): Ubuntu Light (Ubuntu Font Licence 1.0) and Hack (MIT /
//! Bitstream Vera licence); plus the TrueType/OpenType fonts found in the usual system
//! font folders.
//!
//! Drafts from memory of SketchUp (unverified): the height is the capital letters'
//! height; the text lies in the component's XY plane reading along red, extruded up
//! along blue; the component's origin is the lower left corner of the text's extent.
//! Curves are flattened with a fixed number of segments; overlapping contours (some
//! variable or decorative fonts) give overlapping faces; there is no kerning.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use eframe::egui;
use glam::DVec2;
use mt_infer::units::{LengthUnit, parse_length};

use crate::App;

/// Segments per quadratic and cubic outline curve (unverified).
const QUAD_SEGMENTS: usize = 4;
const CUBIC_SEGMENTS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone)]
pub struct FontEntry {
    pub name: String,
    source: FontSource,
}

#[derive(Debug, Clone)]
enum FontSource {
    Bundled(&'static [u8]),
    File(PathBuf, u32),
}

impl FontEntry {
    fn data(&self) -> Result<std::borrow::Cow<'static, [u8]>, String> {
        match &self.source {
            FontSource::Bundled(d) => Ok((*d).into()),
            FontSource::File(p, _) => std::fs::read(p)
                .map(Into::into)
                .map_err(|e| format!("{}: {e}", p.display())),
        }
    }

    fn index(&self) -> u32 {
        match self.source {
            FontSource::Bundled(_) => 0,
            FontSource::File(_, i) => i,
        }
    }
}

/// The bundled fonts.
pub fn bundled() -> Vec<FontEntry> {
    vec![
        FontEntry {
            name: "Ubuntu Light".into(),
            source: FontSource::Bundled(epaint_default_fonts::UBUNTU_LIGHT),
        },
        FontEntry {
            name: "Hack".into(),
            source: FontSource::Bundled(epaint_default_fonts::HACK_REGULAR),
        },
    ]
}

/// Name of face `index` of a font file.
fn font_name(data: &[u8], index: u32) -> Option<String> {
    let face = ttf_parser::Face::parse(data, index).ok()?;
    let names = face.names();
    [ttf_parser::name_id::FULL_NAME, ttf_parser::name_id::FAMILY]
        .into_iter()
        .find_map(|id| {
            names
                .into_iter()
                .filter(|n| n.name_id == id && n.is_unicode())
                .find_map(|n| n.to_string())
        })
}

/// Fonts in the system font folders, by name (scanned once).
fn system_fonts() -> &'static [FontEntry] {
    static FONTS: OnceLock<Vec<FontEntry>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut dirs: Vec<PathBuf> = [
            "/usr/share/fonts",
            "/usr/local/share/fonts",
            "/System/Library/Fonts",
            "/Library/Fonts",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        if let Some(h) = &home {
            dirs.extend([".fonts", ".local/share/fonts", "Library/Fonts"].map(|d| h.join(d)));
        }
        if let Some(w) = std::env::var_os("WINDIR") {
            dirs.push(PathBuf::from(w).join("Fonts"));
        }
        let mut files = Vec::new();
        for d in dirs {
            collect(&d, 0, &mut files);
        }
        let mut out: Vec<FontEntry> = Vec::new();
        for f in files {
            let Ok(data) = std::fs::read(&f) else {
                continue;
            };
            let count = ttf_parser::fonts_in_collection(&data).unwrap_or(1).min(16);
            for i in 0..count {
                if let Some(name) = font_name(&data, i)
                    && !out.iter().any(|e| e.name == name)
                {
                    out.push(FontEntry {
                        name,
                        source: FontSource::File(f.clone(), i),
                    });
                }
            }
        }
        out.sort_by_key(|e| e.name.to_lowercase());
        out
    })
}

/// Font files under `dir` (a few levels deep, a few thousand at most).
fn collect(dir: &std::path::Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 4 || out.len() > 3000 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, depth + 1, out);
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| matches!(x.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc"))
        {
            out.push(p);
        }
    }
}

/// Collects a glyph's closed contours as polylines.
struct Outline {
    contours: Vec<Vec<DVec2>>,
    current: Vec<DVec2>,
    scale: f64,
    offset: DVec2,
}

impl Outline {
    fn pt(&self, x: f32, y: f32) -> DVec2 {
        DVec2::new(f64::from(x), f64::from(y)) * self.scale + self.offset
    }
    fn last(&self) -> DVec2 {
        self.current.last().copied().unwrap_or(self.offset)
    }
}

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        let p = self.pt(x, y);
        self.current.push(p);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.pt(x, y);
        self.current.push(p);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (p0, p1, p2) = (self.last(), self.pt(x1, y1), self.pt(x, y));
        for k in 1..=QUAD_SEGMENTS {
            let t = k as f64 / QUAD_SEGMENTS as f64;
            let s = 1.0 - t;
            self.current
                .push(p0 * s * s + p1 * 2.0 * s * t + p2 * t * t);
        }
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (p0, p1, p2, p3) = (self.last(), self.pt(x1, y1), self.pt(x2, y2), self.pt(x, y));
        for k in 1..=CUBIC_SEGMENTS {
            let t = k as f64 / CUBIC_SEGMENTS as f64;
            let s = 1.0 - t;
            self.current.push(
                p0 * s * s * s + p1 * 3.0 * s * s * t + p2 * 3.0 * s * t * t + p3 * t * t * t,
            );
        }
    }
    fn close(&mut self) {
        if self.current.len() >= 3 {
            self.contours.push(std::mem::take(&mut self.current));
        }
        self.current.clear();
    }
}

/// Flattened outlines of `text` (lines split at newlines) in inches, capital letters
/// `height` high, lower left corner of the extent at the origin.
pub fn outlines(
    data: &[u8],
    index: u32,
    text: &str,
    height: f64,
    align: Align,
) -> Result<Vec<Vec<DVec2>>, String> {
    let face = ttf_parser::Face::parse(data, index).map_err(|e| e.to_string())?;
    let cap = face
        .capital_height()
        .filter(|c| *c > 0)
        .unwrap_or_else(|| face.ascender().max(1));
    let scale = height / f64::from(cap);
    let line = f64::from(face.ascender() - face.descender() + face.line_gap()) * scale;
    let mut all = Vec::new();
    for (row, text) in text.lines().enumerate() {
        let glyphs: Vec<_> = text
            .chars()
            .map(|c| face.glyph_index(c).unwrap_or(ttf_parser::GlyphId(0)))
            .collect();
        let width: f64 = glyphs
            .iter()
            .map(|g| f64::from(face.glyph_hor_advance(*g).unwrap_or(0)) * scale)
            .sum();
        let mut x = match align {
            Align::Left => 0.0,
            Align::Center => -width / 2.0,
            Align::Right => -width,
        };
        let y = -(row as f64) * line;
        for g in glyphs {
            let mut o = Outline {
                contours: Vec::new(),
                current: Vec::new(),
                scale,
                offset: DVec2::new(x, y),
            };
            face.outline_glyph(g, &mut o);
            ttf_parser::OutlineBuilder::close(&mut o);
            all.extend(o.contours);
            x += f64::from(face.glyph_hor_advance(g).unwrap_or(0)) * scale;
        }
    }
    let min = all
        .iter()
        .flatten()
        .copied()
        .reduce(DVec2::min)
        .unwrap_or(DVec2::ZERO);
    for c in &mut all {
        for p in c {
            *p -= min;
        }
    }
    Ok(all)
}

/// The 3D Text dialog while open (kept in egui's memory, as is the last one used).
#[derive(Debug, Clone)]
struct Dialog {
    text: String,
    fonts: Arc<Vec<FontEntry>>,
    font: usize,
    align: Align,
    height: String,
    filled: bool,
    extruded: bool,
    depth: String,
    error: Option<String>,
}

fn dialog_id() -> egui::Id {
    egui::Id::new("mt_text3d")
}

fn last_id() -> egui::Id {
    egui::Id::new("mt_text3d_last")
}

/// Tools > 3D Text: opens the dialog.
pub fn open(ctx: &egui::Context) {
    let last: Option<Dialog> = ctx.data(|d| d.get_temp(last_id()));
    let dialog = last.unwrap_or_else(|| {
        let mut fonts = bundled();
        fonts.extend(system_fonts().iter().cloned());
        Dialog {
            // SketchUp's default text (unverified).
            text: "Enter text".into(),
            fonts: Arc::new(fonts),
            font: 0,
            align: Align::Left,
            height: String::new(),
            filled: true,
            extruded: true,
            depth: String::new(),
            error: None,
        }
    });
    ctx.data_mut(|d| d.insert_temp(dialog_id(), dialog));
}

impl App {
    pub(crate) fn text3d_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(mut d) = ctx.data(|m| m.get_temp::<Dialog>(dialog_id())) else {
            return;
        };
        let units = self.document.as_ref().map_or(LengthUnit::Inch, |d| d.units);
        // Defaults: 1' high, extruded 3" (unverified), in the model's units.
        if d.height.is_empty() {
            d.height = units.format(12.0);
        }
        if d.depth.is_empty() {
            d.depth = units.format(3.0);
        }
        let mut done = None;
        egui::Modal::new(egui::Id::new("mt_text3d_modal")).show(ctx, |ui| {
            ui.set_width(340.0);
            crate::export::dialog_visuals(ui);
            ui.strong("Place 3D Text");
            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::multiline(&mut d.text)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            egui::Grid::new("mt_text3d_grid")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Font:");
                    let name = d.fonts.get(d.font).map_or("", |f| f.name.as_str());
                    egui::ComboBox::from_id_salt("mt_text3d_font")
                        .selected_text(name)
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            for (i, f) in d.fonts.iter().enumerate() {
                                ui.selectable_value(&mut d.font, i, &f.name);
                            }
                        });
                    ui.end_row();
                    ui.label("Align:");
                    ui.horizontal(|ui| {
                        for (a, n) in [
                            (Align::Left, "Left"),
                            (Align::Center, "Center"),
                            (Align::Right, "Right"),
                        ] {
                            ui.radio_value(&mut d.align, a, n);
                        }
                    });
                    ui.end_row();
                    ui.label("Height:");
                    ui.text_edit_singleline(&mut d.height);
                    ui.end_row();
                    ui.label("Form:");
                    ui.checkbox(&mut d.filled, "Filled");
                    ui.end_row();
                    ui.add_enabled(d.filled, egui::Checkbox::new(&mut d.extruded, "Extruded:"));
                    ui.add_enabled(
                        d.filled && d.extruded,
                        egui::TextEdit::singleline(&mut d.depth),
                    );
                    ui.end_row();
                });
            if let Some(e) = &d.error {
                ui.colored_label(egui::Color32::from_rgb(180, 0, 0), e);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Place").clicked() {
                    done = Some(true);
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    done = Some(false);
                }
            });
        });
        match done {
            None => ctx.data_mut(|m| {
                m.insert_temp(dialog_id(), d);
            }),
            Some(false) => ctx.data_mut(|m| m.remove::<Dialog>(dialog_id())),
            Some(true) => match self.place_text(&d, units, frame) {
                Ok(()) => ctx.data_mut(|m| {
                    m.remove::<Dialog>(dialog_id());
                    d.error = None;
                    m.insert_temp(last_id(), d);
                }),
                Err(e) => {
                    d.error = Some(e);
                    ctx.data_mut(|m| {
                        m.insert_temp(dialog_id(), d);
                    });
                }
            },
        }
    }

    fn place_text(
        &mut self,
        d: &Dialog,
        units: LengthUnit,
        frame: &eframe::Frame,
    ) -> Result<(), String> {
        let height = parse_length(&d.height, units)
            .filter(|h| *h > 0.0)
            .ok_or("Enter a height.")?;
        let depth = if d.filled && d.extruded {
            parse_length(&d.depth, units).ok_or("Enter an extrusion.")?
        } else {
            0.0
        };
        let font = d.fonts.get(d.font).ok_or("Choose a font.")?;
        let data = font.data()?;
        let outlines = outlines(&data, font.index(), &d.text, height, d.align)?;
        if outlines.is_empty() {
            return Err("The text has no letters to draw.".into());
        }
        // Extruding down when the value is negative (unverified).
        let (mut ents, _) = mt_kernel::text3d::from_outlines(&outlines, d.filled, depth.abs());
        if depth < 0.0 {
            let flip = glam::DMat4::from_translation(glam::DVec3::new(0.0, 0.0, depth));
            let all: Vec<_> = ents.vertices.keys().collect();
            mt_kernel::transform_vertices(&mut ents, &all, flip);
        }
        let Some(doc) = &mut self.document else {
            return Err("No model is open.".into());
        };
        let first = d.text.lines().next().unwrap_or("").trim();
        let mut def = None;
        doc.edit_model(|m, _| {
            let name = m.unique_definition_name(if first.is_empty() { "Text" } else { first });
            def = Some(m.definitions.insert(mt_kernel::Definition {
                name,
                entities: ents,
                glue: mt_kernel::GlueTo::Any,
                ..Default::default()
            }));
            true
        });
        let Some(definition) = def else {
            return Ok(());
        };
        if let Some(t) = self.draw.as_mut() {
            t.cancel();
        }
        self.tool = crate::nav::Tool::Modify("tool.place_component");
        self.draw = None;
        self.modify = Some(Box::new(crate::modify::PlaceComponent {
            definition,
            glue: mt_kernel::GlueTo::Any,
        }));
        self.selection.clear();
        if let Some(rs) = frame.wgpu_render_state() {
            let device = rs.device.clone();
            self.rebuild(&device);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_kernel::{Checks, validate};

    fn faces(text: &str, depth: f64) -> mt_kernel::Entities {
        let o = outlines(
            epaint_default_fonts::UBUNTU_LIGHT,
            0,
            text,
            10.0,
            Align::Left,
        )
        .unwrap();
        mt_kernel::text3d::from_outlines(&o, true, depth).0
    }

    #[test]
    fn letters_with_holes() {
        // "O": one face with one hole; "B": two holes; "i": two faces; "-": one.
        for (text, nfaces, nholes) in [("O", 1, 1), ("B", 1, 2), ("i", 2, 0), ("-", 1, 0)] {
            let e = faces(text, 0.0);
            assert_eq!(e.faces.len(), nfaces, "{text}");
            let holes: usize = e.faces.values().map(|f| f.holes.len()).sum();
            assert_eq!(holes, nholes, "{text}");
            assert_eq!(validate(&e, Checks::ALL), Vec::<String>::new(), "{text}");
        }
    }

    #[test]
    fn height_alignment_and_extrusion() {
        let o = outlines(
            epaint_default_fonts::UBUNTU_LIGHT,
            0,
            "H",
            10.0,
            Align::Left,
        )
        .unwrap();
        let (lo, hi) = o
            .iter()
            .flatten()
            .fold((DVec2::MAX, DVec2::MIN), |(a, b), p| (a.min(*p), b.max(*p)));
        assert_eq!(lo, DVec2::ZERO);
        assert!((hi.y - 10.0).abs() < 0.05, "cap height {}", hi.y);
        // Two lines centered: the short one sits in the middle of the long one.
        let o = outlines(
            epaint_default_fonts::HACK_REGULAR,
            0,
            "II\nI",
            10.0,
            Align::Center,
        )
        .unwrap();
        assert_eq!(o.len(), 3);

        let e = faces("O", 2.0);
        // Top and bottom, plus one side face per outline segment.
        let o = outlines(
            epaint_default_fonts::UBUNTU_LIGHT,
            0,
            "O",
            10.0,
            Align::Left,
        )
        .unwrap();
        let segments: usize = o.iter().map(|c| c.len()).sum();
        assert!(e.faces.len() >= 2 + segments - 4 && e.faces.len() <= 2 + segments);
        assert!(e.edges.values().all(|x| x.faces.len() == 2), "closed solid");
        // The round sides are smooth, the top and bottom outlines hard.
        assert!(e.edges.values().any(|x| x.attrs.soft && x.attrs.smooth));
        // (An "O" has no corners: every vertical edge is smooth.)
        assert!(
            e.edges
                .values()
                .filter(|x| !x.attrs.soft)
                .all(|x| (e.pos(x.start).z - e.pos(x.end).z).abs() < 1e-9)
        );
        assert_eq!(validate(&e, Checks::ALL), Vec::<String>::new());
    }

    #[test]
    fn bundled_fonts_have_names() {
        assert_eq!(
            font_name(epaint_default_fonts::UBUNTU_LIGHT, 0).as_deref(),
            Some("Ubuntu Light")
        );
        assert!(font_name(epaint_default_fonts::HACK_REGULAR, 0).is_some());
    }
}
