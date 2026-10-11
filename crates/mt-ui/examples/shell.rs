//! Opens the SketchUp-style shell with a plain colored rectangle as the viewport.
//! Prints every event to stdout. The tray panels show sample [`TrayData`] that the
//! demo updates from the events, like an app would.
//!
//! `cargo run -p mt-ui --example shell`
//!
//! `MT_UI_PANELS=window.tags,window.scenes` expands those tray panels at start.

use eframe::egui;
use mt_spec::Spec;
use mt_ui::{
    ComponentRow, EntityInfo, Event, Frame, MaterialRow, OutlinerKind, OutlinerNode, SelectMode,
    Shell, TagRow, TrayData,
};

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
            let mut shell = Shell::new(&spec);
            if let Ok(panels) = std::env::var("MT_UI_PANELS") {
                for id in panels.split(',') {
                    shell.open_panel(id.trim());
                }
            }
            Ok(Box::new(Demo {
                spec,
                shell,
                tool: "tool.select".to_owned(),
                last: String::new(),
                data: sample_data(),
            }))
        }),
    )
}

struct Demo {
    spec: Spec,
    shell: Shell,
    tool: String,
    last: String,
    data: TrayData,
}

fn tag(name: &str, visible: bool, color: [u8; 3]) -> TagRow {
    TagRow {
        name: name.to_owned(),
        visible,
        color,
        ..TagRow::default()
    }
}

fn material(name: &str, color: [u8; 3], opacity: u8) -> MaterialRow {
    MaterialRow {
        name: name.to_owned(),
        color,
        opacity,
        texture: None,
    }
}

fn component(name: &str, description: &str, instances: usize) -> ComponentRow {
    ComponentRow {
        name: name.to_owned(),
        description: description.to_owned(),
        instances,
        ..ComponentRow::default()
    }
}

fn node(name: &str, definition: Option<&str>, children: Vec<OutlinerNode>) -> OutlinerNode {
    OutlinerNode {
        name: name.to_owned(),
        kind: match definition {
            Some(d) => OutlinerKind::Component {
                definition: d.to_owned(),
            },
            None => OutlinerKind::Group,
        },
        children,
        ..OutlinerNode::default()
    }
}

fn sample_data() -> TrayData {
    let mut hidden = node("", Some("Tree"), vec![]);
    hidden.hidden = true;
    TrayData {
        tags: vec![
            tag("Untagged", true, [255, 255, 255]),
            tag("Walls", true, [224, 160, 96]),
            tag("Furniture", false, [96, 160, 224]),
            tag("Site", true, [120, 190, 100]),
        ],
        active_tag: 1,
        materials: vec![
            material("Brick", [178, 84, 60], 100),
            material("Concrete", [180, 180, 175], 100),
            material("Glass", [150, 200, 230], 35),
            material("Oak", [196, 150, 96], 100),
            material("Grass", [96, 150, 70], 100),
            material("Steel", [120, 125, 135], 100),
            material("Roof tiles", [140, 60, 50], 100),
        ],
        current_material: Some(2),
        outliner: vec![
            node(
                "House",
                None,
                vec![
                    node("Front", Some("Door"), vec![]),
                    node("", Some("Window"), vec![]),
                    node("", Some("Window"), vec![]),
                    node("Roof", None, vec![]),
                ],
            ),
            node("", Some("Chair"), vec![]),
            hidden,
        ],
        scenes: vec!["Scene 1".into(), "Front view".into(), "Top".into()],
        active_scene: Some(1),
        components: vec![
            component("Door", "Single interior door, 900 mm", 1),
            component("Window", "", 2),
            component("Chair", "Dining chair", 1),
            component("Tree", "Deciduous tree, 2D face-me", 1),
            component("Lamp", "Unused floor lamp", 0),
        ],
        current_component: Some(1),
        ..TrayData::default()
    }
}

/// Applies tray events to the sample data, as the app would to its model.
fn apply(data: &mut TrayData, event: &Event) {
    match event {
        Event::TagAdd => {
            let n = data.tags.len();
            data.tags
                .push(tag(&format!("Tag{n}"), true, [200, 200, 120]));
        }
        Event::TagDelete(i) if *i > 0 => {
            data.tags.remove(*i);
            if data.active_tag >= data.tags.len() || data.active_tag == *i {
                data.active_tag = 0;
            }
        }
        Event::TagRename(i, name) => data.tags[*i].name.clone_from(name),
        Event::TagVisible(i, v) => data.tags[*i].visible = *v,
        Event::TagActive(i) => data.active_tag = *i,
        Event::TagColor(i, c) => data.tags[*i].color = *c,
        Event::MaterialPick(i) => data.current_material = Some(*i),
        Event::MaterialEdit {
            index,
            name,
            color,
            opacity,
        } => data.materials[*index] = material(name, *color, *opacity),
        Event::ComponentPick(i) => data.current_component = Some(*i),
        Event::ComponentEdit {
            index,
            name,
            description,
        } => {
            data.components[*index] =
                component(name, description, data.components[*index].instances)
        }
        Event::ComponentPurge => {
            let current = data.current_component.map(|i| data.components[i].clone());
            data.components.retain(|c| c.instances > 0);
            data.current_component =
                current.and_then(|c| data.components.iter().position(|d| *d == c));
        }
        Event::OutlinerSelect { path, mode } => {
            fn walk(nodes: &mut [OutlinerNode], f: &mut dyn FnMut(&mut OutlinerNode)) {
                for n in nodes {
                    f(n);
                    walk(&mut n.children, f);
                }
            }
            if *mode == SelectMode::Replace {
                walk(&mut data.outliner, &mut |n| n.selected = false);
            }
            let mut nodes = &mut data.outliner;
            let (last, parents) = path.split_last().expect("path");
            for &i in parents {
                nodes = &mut nodes[i].children;
            }
            let n = &mut nodes[*last];
            n.selected = match mode {
                SelectMode::Replace | SelectMode::Add => true,
                SelectMode::Toggle => !n.selected,
                SelectMode::Remove => false,
            };
        }
        Event::SceneAdd => {
            data.scenes.push(format!("Scene {}", data.scenes.len() + 1));
            data.active_scene = Some(data.scenes.len() - 1);
        }
        Event::SceneDelete(i) => {
            data.scenes.remove(*i);
            data.active_scene = None;
        }
        Event::SceneActivate(i) => data.active_scene = Some(*i),
        Event::StyleChanged(s) => data.style = *s,
        Event::ShadowsChanged(s) => data.shadows = *s,
        _ => {}
    }
}

impl eframe::App for Demo {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.shell.capture_typing(ui.ctx());
        let info = if self.tool == "tool.line" {
            let mut info = EntityInfo {
                title: "Edge".to_owned(),
                ..Default::default()
            };
            info.field(
                "length",
                "Length",
                mt_ui::InfoValue::Text("1000 mm".to_owned()),
            );
            info.field("soft", "Soft", mt_ui::InfoValue::Check(false));
            info.field("smooth", "Smooth", mt_ui::InfoValue::Check(false));
            info
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
        self.shell.set_tray_data(self.data.clone());
        let events = self.shell.show(ui, &frame, |ui| {
            let rect = ui.available_rect_before_wrap();
            ui.painter()
                .rect_filled(rect, 0.0, egui::Color32::from_rgb(222, 235, 245));
            ui.allocate_rect(rect, egui::Sense::click());
        });
        for event in events {
            println!("event: {event:?}");
            apply(&mut self.data, &event);
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
