//! File > Print Setup, Print Preview and Print.
//!
//! There is no OS print dialog: Print renders the current view offscreen, writes a
//! one-page PDF (a JPEG image on the page; the PDF is written here directly, no PDF
//! library) where the Save dialog says, and opens it with the system's default PDF
//! viewer (`xdg-open`, `open` or `start`) to print from there. With the
//! `MESHTARD_TEST_DIALOG` file-dialog hook set (GUI checks) the viewer is not opened.
//!
//! Print Setup: paper (A4 or Letter), orientation, "Fit to page" or a scale, and the
//! print quality (resolution). The scale ("1 : N", paper length to model length) needs
//! a parallel projection camera, as in SketchUp (unverified); with a perspective camera
//! the view is fitted. Fit to page keeps the view's aspect and centers it; the scaled
//! print keeps the view's center and fills the printable area. Margins are 0.5 inch.
//! SketchUp's tiled sheets, "Use model extents", and print size fields are not done.
//! Print Preview shows the page (paper, margins and the image) in a window.

use eframe::egui;

use crate::App;

/// PDF points per inch.
const PT_PER_IN: f64 = 72.0;
/// Page margin, inches.
const MARGIN_IN: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Paper {
    A4,
    Letter,
}

impl Paper {
    /// Portrait size in points.
    fn size_pt(self) -> (f64, f64) {
        match self {
            Self::A4 => (595.276, 841.89),
            Self::Letter => (612.0, 792.0),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::A4 => "A4 (210 x 297 mm)",
            Self::Letter => "Letter (8.5 x 11 in)",
        }
    }
}

/// Print Setup settings (kept for the session).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PrintSetup {
    pub paper: Paper,
    pub landscape: bool,
    pub fit: bool,
    /// Model lengths per paper length (the N of 1 : N).
    pub scale: f64,
    /// Image resolution, dots per inch.
    pub dpi: u32,
}

impl Default for PrintSetup {
    fn default() -> Self {
        Self {
            paper: Paper::A4,
            landscape: true,
            fit: true,
            scale: 100.0,
            dpi: 150,
        }
    }
}

impl PrintSetup {
    /// Page size in points with the orientation applied.
    pub(crate) fn page_pt(&self) -> (f64, f64) {
        let (w, h) = self.paper.size_pt();
        if self.landscape { (h, w) } else { (w, h) }
    }

    /// The printable area: x, y (from the bottom left), width, height in points.
    fn printable_pt(&self) -> [f64; 4] {
        let (w, h) = self.page_pt();
        let m = MARGIN_IN * PT_PER_IN;
        [m, m, w - 2.0 * m, h - 2.0 * m]
    }

    /// Where the image goes on the page (points) for a view of `aspect` (width /
    /// height): centered and fitted, or the whole printable area when `fill`.
    pub(crate) fn placement(&self, aspect: f64, fill: bool) -> [f64; 4] {
        let [x, y, w, h] = self.printable_pt();
        if fill || aspect <= 0.0 {
            return [x, y, w, h];
        }
        let (iw, ih) = if w / h > aspect {
            (h * aspect, h)
        } else {
            (w, w / aspect)
        };
        [x + (w - iw) / 2.0, y + (h - ih) / 2.0, iw, ih]
    }
}

/// Print Preview: the page image while the window is open.
pub(crate) struct Preview {
    texture: egui::TextureHandle,
    size: [usize; 2],
    note: String,
}

/// A one-page PDF of `page_pt` (width, height in points) showing the JPEG `jpeg`
/// (`px` pixels) at `rect` (x, y from the bottom left, width, height, in points).
pub(crate) fn pdf_with_jpeg(
    page_pt: (f64, f64),
    rect: [f64; 4],
    jpeg: &[u8],
    px: [u32; 2],
    title: &str,
) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    let f = |x: f64| format!("{x:.3}");
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    obj(
        &mut out,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /XObject << /Im0 4 0 R >> /ProcSet [/PDF /ImageC] >> /Contents 5 0 R >>",
            f(page_pt.0),
            f(page_pt.1)
        )
        .as_bytes(),
    );
    let mut image = format!(
        "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
        px[0],
        px[1],
        jpeg.len()
    )
    .into_bytes();
    image.extend_from_slice(jpeg);
    image.extend_from_slice(b"\nendstream");
    obj(&mut out, &image);
    let content = format!(
        "q {} 0 0 {} {} {} cm /Im0 Do Q",
        f(rect[2]),
        f(rect[3]),
        f(rect[0]),
        f(rect[1])
    );
    obj(
        &mut out,
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        )
        .as_bytes(),
    );
    // PDF string literal: escape backslash and parentheses; keep it ASCII.
    let title: String = title
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() {
                c
            } else {
                '?'
            }
        })
        .collect::<String>()
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)");
    obj(
        &mut out,
        format!("<< /Title ({title}) /Producer (meshtard) >>").as_bytes(),
    );
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
    );
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info {} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len() + 1,
            offsets.len()
        )
        .as_bytes(),
    );
    out
}

/// The page's image and where it goes: rendered image, placement in points, and a
/// note when the scale could not be used.
struct PageImage {
    image: image::RgbaImage,
    rect: [f64; 4],
    note: Option<String>,
}

impl App {
    /// Renders the view for the page at `dpi`.
    fn page_image(
        &mut self,
        frame: &eframe::Frame,
        setup: &PrintSetup,
        dpi: u32,
    ) -> Option<PageImage> {
        let view_aspect = self.viewport_px.x.max(1.0) / self.viewport_px.y.max(1.0);
        let mut camera = self.camera.clone();
        let scaled = !setup.fit && !camera.perspective && setup.scale > 0.0;
        let note = (!setup.fit && camera.perspective).then(|| {
            "Scaled printing needs Camera > Parallel Projection; fitted to the page.".to_owned()
        });
        let rect = setup.placement(view_aspect, scaled);
        if scaled {
            // The printable height shows `scale` times its length of the model.
            camera.set_view_height(rect[3] / PT_PER_IN * setup.scale);
        }
        let max = 8192.0;
        let px = |pt: f64| ((pt / PT_PER_IN * f64::from(dpi)).round()).clamp(16.0, max) as u32;
        let size = [px(rect[2]), px(rect[3])];
        // Lines a little wider than on a 96 dpi screen, as on paper.
        let ppp = (dpi as f32 / 96.0).max(1.0);
        let image = self.render_view(frame, &camera, size, ppp)?;
        Some(PageImage { image, rect, note })
    }

    pub(crate) fn open_print_preview(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let setup = self.present.print_setup.clone();
        // Screen resolution for the preview.
        let Some(page) = self.page_image(frame, &setup, 72) else {
            return;
        };
        let (w, h) = setup.page_pt();
        let mut sheet = image::RgbaImage::from_pixel(
            w.round() as u32,
            h.round() as u32,
            image::Rgba([255, 255, 255, 255]),
        );
        let [x, y, rw, rh] = page.rect;
        let fitted = image::imageops::resize(
            &page.image,
            rw.round().max(1.0) as u32,
            rh.round().max(1.0) as u32,
            image::imageops::FilterType::Triangle,
        );
        // PDF y goes up; image rows go down.
        let top = (h - y - rh).round() as i64;
        image::imageops::overlay(&mut sheet, &fitted, x.round() as i64, top);
        let size = [sheet.width() as usize, sheet.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, sheet.as_raw());
        let texture = ctx.load_texture("mt_print_preview", color, egui::TextureOptions::LINEAR);
        self.present.print_preview = Some(Preview {
            texture,
            size,
            note: page.note.unwrap_or_default(),
        });
    }

    /// File > Print: the PDF, then the system viewer.
    pub(crate) fn print(&mut self, _ctx: &egui::Context, frame: &eframe::Frame) {
        let setup = self.present.print_setup.clone();
        let name = self.export_file_name("pdf");
        let Some(path) = crate::filedialog::save(&[("PDF Document", &["pdf"])], &name) else {
            return;
        };
        let Some(page) = self.page_image(frame, &setup, setup.dpi) else {
            return;
        };
        let rgb = image::DynamicImage::ImageRgba8(page.image).to_rgb8();
        let mut jpeg = std::io::Cursor::new(Vec::new());
        if let Err(e) =
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 92).encode_image(&rgb)
        {
            self.message = Some(format!("Could not print: {e}"));
            return;
        }
        let pdf = pdf_with_jpeg(
            setup.page_pt(),
            page.rect,
            jpeg.get_ref(),
            [rgb.width(), rgb.height()],
            &self.model_stem(),
        );
        if let Err(e) = std::fs::write(&path, pdf) {
            self.message = Some(format!("Could not write {}: {e}", path.display()));
            return;
        }
        let mut msg = format!("Printed to {}", path.display());
        if std::env::var_os("MESHTARD_TEST_DIALOG").is_none() {
            match crate::presentation::open_with_system(&path.to_string_lossy()) {
                Ok(()) => msg.push_str(" (opened in the PDF viewer)"),
                Err(e) => msg.push_str(&format!(" (could not open a viewer: {e})")),
            }
        }
        if let Some(n) = page.note {
            msg.push_str(". ");
            msg.push_str(&n);
        } else {
            msg.push('.');
        }
        self.message = Some(msg);
    }

    pub(crate) fn print_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.print_setup_ui(ctx);
        let Some(p) = &self.present.print_preview else {
            return;
        };
        let (texture, size, note) = (p.texture.id(), p.size, p.note.clone());
        // The page at most 480 points wide and as tall as the window allows.
        let avail = egui::vec2(480.0, (ctx.content_rect().height() - 200.0).max(160.0));
        let k = (avail.x / size[0] as f32)
            .min(avail.y / size[1] as f32)
            .clamp(0.05, 2.0);
        let page = egui::vec2(size[0] as f32 * k, size[1] as f32 * k);
        let paper = self.present.print_setup.paper.label();
        let r = mt_ui::Dialog::new("print_preview", "Print Preview")
            .width(page.x.max(320.0) + 16.0)
            .modal(false)
            .show(ctx, |ui| {
                ui.weak(paper);
                if !note.is_empty() {
                    ui.weak(&note);
                }
                ui.add_space(6.0);
                ui.vertical_centered(|ui| {
                    egui::Frame::new()
                        .fill(egui::Color32::from_gray(160))
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.image((texture, page));
                        });
                });
                mt_ui::dialog::buttons(ui, &["Print...", "Close"], Some(0))
            });
        let print = r.inner == Some(0);
        if r.closed || r.inner.is_some() {
            self.present.print_preview = None;
        }
        if print {
            self.print(ctx, frame);
        }
    }

    fn print_setup_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.present.print_setup_dialog.take() else {
            return;
        };
        let parallel = !self.camera.perspective;
        let r = mt_ui::Dialog::new("print_setup", "Print Setup")
            .width(320.0)
            .show(ctx, |ui| {
                crate::export::dialog_visuals(ui);
                egui::Grid::new("mt_print_setup_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Paper:");
                        egui::ComboBox::from_id_salt("mt_print_paper")
                            .selected_text(d.paper.label())
                            .show_ui(ui, |ui| {
                                for p in [Paper::A4, Paper::Letter] {
                                    ui.selectable_value(&mut d.paper, p, p.label());
                                }
                            });
                        ui.end_row();
                        ui.label("Orientation:");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut d.landscape, false, "Portrait");
                            ui.radio_value(&mut d.landscape, true, "Landscape");
                        });
                        ui.end_row();
                        ui.label("Quality:");
                        egui::ComboBox::from_id_salt("mt_print_dpi")
                            .selected_text(format!("{} dpi", d.dpi))
                            .show_ui(ui, |ui| {
                                for dpi in [72, 150, 300] {
                                    ui.selectable_value(&mut d.dpi, dpi, format!("{dpi} dpi"));
                                }
                            });
                        ui.end_row();
                    });
                ui.add_space(10.0);
                ui.strong("Print Size");
                ui.add_space(4.0);
                ui.checkbox(&mut d.fit, "Fit to page");
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!d.fit, |ui| {
                        ui.label("Scale 1 :");
                        ui.add(
                            egui::DragValue::new(&mut d.scale)
                                .range(0.001..=1e6)
                                .speed(1.0),
                        );
                    });
                });
                if !d.fit && !parallel {
                    ui.add_space(4.0);
                    ui.weak("Scaled printing needs Camera > Parallel Projection.");
                }
                crate::presentation::ok_cancel(ui, "OK")
            });
        let done = crate::presentation::answer(r);
        match done {
            None => self.present.print_setup_dialog = Some(d),
            Some(true) => self.present.print_setup = d,
            Some(false) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_holds_the_jpeg_and_a_valid_xref() {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode_image(&img)
            .unwrap();
        let jpeg = jpeg.into_inner();
        let setup = PrintSetup::default();
        let rect = setup.placement(2.0, false);
        let pdf = pdf_with_jpeg(setup.page_pt(), rect, &jpeg, [8, 4], "a (b)");
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.windows(jpeg.len()).any(|w| w == jpeg), "image embedded");
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Filter /DCTDecode"));
        assert!(
            text.contains("/MediaBox [0 0 841.890 595.276]"),
            "A4 landscape"
        );
        assert!(text.contains("(a \\(b\\))"));
        assert!(text.trim_end().ends_with("%%EOF"));
        // startxref points at the xref table, whose entries point at the objects.
        let at: usize = text
            .rsplit("startxref\n")
            .next()
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(pdf[at..].starts_with(b"xref"));
        let entries: Vec<usize> = String::from_utf8_lossy(&pdf[at..])
            .lines()
            .skip(3)
            .take(6)
            .map(|l| l[..10].parse().unwrap())
            .collect();
        for (i, o) in entries.iter().enumerate() {
            assert!(pdf[*o..].starts_with(format!("{} 0 obj", i + 1).as_bytes()));
        }
    }

    #[test]
    fn fit_keeps_aspect_and_centers() {
        let s = PrintSetup {
            paper: Paper::Letter,
            landscape: false,
            ..Default::default()
        };
        // Printable area 540 x 720 pt; a 2:1 view is 540 x 270, centered vertically.
        let [x, y, w, h] = s.placement(2.0, false);
        assert!((w - 540.0).abs() < 1e-9 && (h - 270.0).abs() < 1e-9);
        assert!((x - 36.0).abs() < 1e-9 && (y - (36.0 + 225.0)).abs() < 1e-9);
        assert_eq!(s.placement(2.0, true), [36.0, 36.0, 540.0, 720.0]);
    }
}
