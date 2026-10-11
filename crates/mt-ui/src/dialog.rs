//! Dialog windows in the style of Windows 11 system dialogs: a white title bar with
//! the title and a close button, drawn over a dimmed backdrop that blocks the rest of
//! the app, a 1 px border, rounded corners and a soft shadow. The dialog opens
//! centered and is moved by dragging its title bar. Escape or the close button
//! cancels it; [`buttons`] lays out the right-aligned footer row, where Enter presses
//! the default button.
//!
//! Sizes and colors follow Windows 11 from memory (unverified against SketchUp's own
//! dialogs; check in the trial).

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Order, Pos2, Rect, Sense, Shadow,
    Stroke, Ui, UiBuilder, Vec2, vec2,
};

const TITLE_HEIGHT: f32 = 30.0;
const CLOSE_WIDTH: f32 = 46.0;
const RADIUS: u8 = 6;
const MARGIN: f32 = 16.0;
/// Windows 11's close-button hover red.
const CLOSE_HOVER: Color32 = Color32::from_rgb(196, 43, 28);

/// A modal dialog. Show it every frame while it is open.
pub struct Dialog<'a> {
    id: Id,
    title: &'a str,
    width: f32,
    modal: bool,
    max_height: Option<f32>,
}

/// What a dialog returned this frame.
pub struct DialogResponse<R> {
    /// The body closure's value.
    pub inner: R,
    /// The close button was clicked or Escape pressed: treat as Cancel.
    pub closed: bool,
}

impl<'a> Dialog<'a> {
    pub fn new(id_salt: &str, title: &'a str) -> Self {
        Self {
            id: Id::new(("mt_dialog", id_salt)),
            title,
            width: 340.0,
            modal: true,
            max_height: None,
        }
    }

    /// Width of the dialog's content in points.
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Whether the rest of the app is dimmed and blocked (default true). A modeless
    /// dialog sits with the app's other windows instead of above everything, and
    /// Escape closes it only while the pointer is over it.
    pub fn modal(mut self, modal: bool) -> Self {
        self.modal = modal;
        self
    }

    /// Caps the body's height in points: taller content scrolls. The whole body
    /// scrolls, so a dialog with [`buttons`] should not need it.
    pub fn max_height(mut self, height: f32) -> Self {
        self.max_height = Some(height);
        self
    }

    pub fn show<R>(
        self,
        ctx: &egui::Context,
        body: impl FnOnce(&mut Ui) -> R,
    ) -> DialogResponse<R> {
        let screen = ctx.content_rect();
        if self.modal {
            egui::Area::new(self.id.with("backdrop"))
                .order(Order::Foreground)
                .fade_in(false)
                .fixed_pos(screen.min)
                .interactable(true)
                .show(ctx, |ui| {
                    let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
                    ui.painter()
                        .rect_filled(rect, 0.0, Color32::from_black_alpha(48));
                });
        }

        // Position: centered on first show (once the size is known), then dragged.
        let pos_id = self.id.with("pos");
        let size_id = self.id.with("size");
        let size: Option<Vec2> = ctx.data(|d| d.get_temp(size_id));
        let pos: Pos2 = ctx.data(|d| d.get_temp(pos_id)).unwrap_or_else(|| {
            let s = size.unwrap_or(vec2(self.width + 2.0 * MARGIN, 160.0));
            screen.center() - s / 2.0
        });

        let visuals = ctx.global_style().visuals.clone();
        let title_fill = visuals.window_fill;
        let body_fill = visuals.panel_fill;
        let border = visuals.window_stroke.color;
        let text = visuals.widgets.noninteractive.fg_stroke.color;

        let mut closed = false;
        let mut drag = Vec2::ZERO;
        let order = if self.modal {
            Order::Foreground
        } else {
            Order::Middle
        };
        let max_height = self.max_height;
        let area = egui::Area::new(self.id)
            .order(order)
            .fade_in(false)
            .fixed_pos(pos)
            .interactable(true)
            .show(ctx, |ui| {
                let frame = egui::Frame::new()
                    .fill(body_fill)
                    .stroke(Stroke::new(1.0, border))
                    .corner_radius(CornerRadius::same(RADIUS))
                    .shadow(Shadow {
                        offset: [0, 6],
                        blur: 24,
                        spread: 0,
                        color: Color32::from_black_alpha(70),
                    });
                frame
                    .show(ui, |ui| {
                        let width = self.width + 2.0 * MARGIN;
                        ui.set_width(width);
                        // Title bar.
                        let (bar, bar_resp) =
                            ui.allocate_exact_size(vec2(width, TITLE_HEIGHT), Sense::drag());
                        let r = RADIUS;
                        ui.painter().rect_filled(
                            bar,
                            CornerRadius {
                                nw: r,
                                ne: r,
                                sw: 0,
                                se: 0,
                            },
                            title_fill,
                        );
                        ui.painter().text(
                            bar.left_center() + vec2(12.0, 0.0),
                            Align2::LEFT_CENTER,
                            self.title,
                            FontId::proportional(12.0),
                            text,
                        );
                        let close_rect = Rect::from_min_max(
                            Pos2::new(bar.right() - CLOSE_WIDTH, bar.top()),
                            bar.right_bottom(),
                        );
                        let close = ui.interact(close_rect, self.id.with("close"), Sense::click());
                        let x_color = if close.hovered() {
                            ui.painter().rect_filled(
                                close_rect,
                                CornerRadius {
                                    nw: 0,
                                    ne: r,
                                    sw: 0,
                                    se: 0,
                                },
                                CLOSE_HOVER,
                            );
                            Color32::WHITE
                        } else {
                            text
                        };
                        let c = close_rect.center();
                        let d = 5.0;
                        let stroke = Stroke::new(1.0, x_color);
                        ui.painter()
                            .line_segment([c + vec2(-d, -d), c + vec2(d, d)], stroke);
                        ui.painter()
                            .line_segment([c + vec2(-d, d), c + vec2(d, -d)], stroke);
                        if close.clicked() {
                            closed = true;
                        }
                        if bar_resp.dragged() && !close.hovered() {
                            drag = bar_resp.drag_delta();
                        }
                        ui.painter().line_segment(
                            [bar.left_bottom(), bar.right_bottom()],
                            Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
                        );
                        // Body.
                        let body_rect = Rect::from_min_size(
                            Pos2::new(bar.left() + MARGIN, bar.bottom() + MARGIN - 4.0),
                            vec2(self.width, 0.0),
                        );
                        let mut child = ui.new_child(
                            UiBuilder::new()
                                .max_rect(body_rect.with_max_y(f32::INFINITY))
                                .layout(Layout::top_down(Align::Min)),
                        );
                        let inner = match max_height {
                            Some(h) => {
                                egui::ScrollArea::vertical()
                                    .id_salt(self.id.with("scroll"))
                                    .max_height(h)
                                    .auto_shrink([false, true])
                                    .show(&mut child, body)
                                    .inner
                            }
                            None => body(&mut child),
                        };
                        let used = child.min_rect();
                        ui.allocate_rect(
                            Rect::from_min_max(
                                bar.left_bottom(),
                                Pos2::new(bar.right(), used.bottom() + MARGIN),
                            ),
                            Sense::hover(),
                        );
                        inner
                    })
                    .inner
            });
        if self.modal && !egui::Popup::is_any_open(ctx) {
            // A click on the backdrop must not raise it over the dialog. Not while a
            // popup (a combo box's list) is open: that would cover the popup.
            ctx.move_to_top(egui::LayerId::new(Order::Foreground, self.id));
        }
        ctx.data_mut(|d| {
            d.insert_temp(size_id, area.response.rect.size());
            // Keep the title bar on screen.
            let mut p = pos + drag;
            p.x = p.x.clamp(
                screen.left() - area.response.rect.width() + 80.0,
                screen.right() - 80.0,
            );
            p.y = p.y.clamp(screen.top(), screen.bottom() - TITLE_HEIGHT);
            if size.is_some() || drag != Vec2::ZERO {
                d.insert_temp(pos_id, p);
            }
        });
        if size.is_none() {
            // The first frame only measured the dialog; draw it centered next frame.
            ctx.request_repaint();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape))
            && (self.modal || area.response.contains_pointer())
        {
            closed = true;
        }
        DialogResponse {
            inner: area.inner,
            closed,
        }
    }
}

/// Forgets a dialog's position so it opens centered again next time.
pub fn reset(ctx: &egui::Context, id_salt: &str) {
    let id = Id::new(("mt_dialog", id_salt));
    ctx.data_mut(|d| {
        d.remove::<Pos2>(id.with("pos"));
        d.remove::<Vec2>(id.with("size"));
    });
}

/// The footer row: buttons right-aligned at a common width, the `default` one
/// highlighted and pressed by Enter. Returns the index of the clicked button.
pub fn buttons(ui: &mut Ui, labels: &[&str], default: Option<usize>) -> Option<usize> {
    ui.add_space(14.0);
    let accent = ui.visuals().selection.bg_fill;
    let border = ui.visuals().window_stroke.color;
    let mut clicked = None;
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, 26.0),
        Layout::right_to_left(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (i, label) in labels.iter().enumerate().rev() {
                let is_default = default == Some(i);
                let mut b = egui::Button::new(if is_default {
                    egui::RichText::new(*label).color(Color32::WHITE)
                } else {
                    egui::RichText::new(*label)
                })
                .min_size(vec2(84.0, 26.0))
                .corner_radius(CornerRadius::same(4));
                b = if is_default {
                    b.fill(accent).stroke(Stroke::new(1.0, accent))
                } else {
                    b.fill(Color32::WHITE).stroke(Stroke::new(1.0, border))
                };
                if ui.add(b).clicked() {
                    clicked = Some(i);
                }
            }
        },
    );
    if clicked.is_none()
        && let Some(d) = default
        && ui.input(|i| i.key_pressed(egui::Key::Enter))
    {
        clicked = Some(d);
    }
    clicked
}

/// A dialog message with an optional icon-like heading line, for simple prompts.
pub fn message(ui: &mut Ui, heading: &str, detail: Option<&str>) {
    ui.label(egui::RichText::new(heading).size(13.0));
    if let Some(detail) = detail {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(detail).color(ui.visuals().weak_text_color()));
    }
}
