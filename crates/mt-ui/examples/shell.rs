//! Opens the SketchUp-style shell with a plain colored rectangle as the viewport.
//! Prints every event to stdout.
//!
//! `cargo run -p mt-ui --example shell`

use eframe::egui;
use mt_spec::Spec;
use mt_ui::{EntityInfo, Event, Frame, Shell};

fn main() -> eframe::Result {
    let spec = Spec::builtin().expect("spec");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Untitled - meshtard")
            .with_inner_size([1600.0, 900.0]),
        ..Default::default()
    };
    eframe::run_native(
        "mt-ui shell",
        options,
        Box::new(move |_cc| {
            let shell = Shell::new(&spec);
            Ok(Box::new(Demo {
                spec,
                shell,
                tool: "tool.select".to_owned(),
                last: String::new(),
            }))
        }),
    )
}

struct Demo {
    spec: Spec,
    shell: Shell,
    tool: String,
    last: String,
}

impl eframe::App for Demo {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.shell.capture_typing(ui.ctx());
        let info = if self.tool == "tool.line" {
            EntityInfo {
                title: "Edge".to_owned(),
                rows: vec![
                    ("Tag".to_owned(), "Untagged".to_owned()),
                    ("Length".to_owned(), "1000 mm".to_owned()),
                    ("Soft".to_owned(), "No".to_owned()),
                ],
            }
        } else {
            EntityInfo::no_selection()
        };
        let status = format!(
            "Active: {}. Last event: {}",
            self.spec.command_label(&self.tool).unwrap_or_default(),
            if self.last.is_empty() {
                "none"
            } else {
                &self.last
            }
        );
        let frame = Frame {
            spec: &self.spec,
            active_tool: &self.tool,
            status: &status,
            measurements_label: "Measurements",
            measurements_value: "",
            is_enabled: &|id| {
                !id.starts_with("file.print") && id != "edit.paste" && id != "camera.walk"
            },
            is_checked: &|id| id == "view.axes" || id == "view.shaded",
            entity_info: &info,
        };
        let events = self.shell.show(ui, &frame, |ui| {
            let rect = ui.available_rect_before_wrap();
            ui.painter()
                .rect_filled(rect, 0.0, egui::Color32::from_rgb(222, 235, 245));
            ui.allocate_rect(rect, egui::Sense::click());
        });
        for event in events {
            println!("event: {event:?}");
            match &event {
                Event::Command(id) if id.starts_with("tool.") || id.starts_with("camera.") => {
                    self.tool.clone_from(id);
                }
                Event::Command(id) if id == "file.exit" => {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                _ => {}
            }
            self.last = format!("{event:?}");
        }
    }
}
