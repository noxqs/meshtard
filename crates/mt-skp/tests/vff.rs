//! VFF (2021+) checks for fields the Python oracle does not expose.
//! Ported from OpenSKP `packages/cpp/tests/parser_test.cpp`.

use mt_skp::{AttrValue, Model};

fn open(name: &str) -> Model {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/openskp")
        .join(name);
    mt_skp::read(&path).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn style_watermarks() {
    let model = open("style_watermark.skp");
    assert_eq!(model.styles.len(), 2);
    for style in &model.styles {
        assert_eq!(style.watermarks.len(), 1);
        let w = &style.watermarks[0];
        assert_eq!(w.name, "Logo & Co");
        let expected = if style.working_copy {
            "watermarks/Logo.png"
        } else {
            "./logo.png"
        };
        assert_eq!(w.image_path, expected);
        assert_eq!(w.file_name, "logo.png");
        assert!(!w.attributes.contains_key("name"));
        assert_eq!(w.attributes["position"], "2");
        assert_eq!(w.attributes["alphaScale"], "0.75");
        assert_eq!(w.attributes["scale"], "0.3");
        assert_eq!(w.attributes["background"], "1");
        let image = w.image.as_ref().expect("watermark image bytes");
        assert_eq!(image.len(), 70);
        assert_eq!(&image[1..4], b"PNG");
    }
}

#[test]
fn modern_untitled() {
    let model = open("Untitled.skp");
    assert_eq!(model.version, "{25.0.575}");
    assert_eq!(model.units.as_deref(), Some("Millimeter"));
    assert_eq!(model.materials.len(), 15);
    assert_eq!(model.definitions.len(), 46);

    let layers: Vec<&str> = model.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(
        layers,
        [
            "Layer0",
            "BottomPlate",
            "TopPlate",
            "Stud",
            "Nog",
            "KingStud",
            "HeaderJackStud",
            "HeaderPlate1",
            "HeaderPlate2",
            "SillPlate1",
            "VerticalHeaderStud",
            "generic_frame",
            "dimension",
            "Hat Sections",
        ]
    );
    assert!(model.layers.iter().all(|l| !l.hidden));

    let d = &model.definitions[&66];
    assert_eq!(d.name, "Group200#2");
    assert_eq!(d.guid.len(), 32);
    assert_eq!(
        (d.vertices.len(), d.edges.len(), d.faces.len()),
        (136, 158, 26)
    );
    assert!(d.faces.values().all(|f| !f.hidden && f.normal.is_some()));

    let joined = model.material_by_id(26180).expect("material 26180");
    assert_eq!(joined.name, "*");
    assert_eq!(joined.id, Some(26180));
    let layer0 = model
        .materials
        .iter()
        .find(|m| m.name == "Layer_Layer0")
        .unwrap();
    assert_eq!(layer0.transparency, 1.0);
    assert_eq!(layer0.id, None);
    assert!(layer0.texture.is_none());

    let s = &model.styles[0];
    assert_eq!(s.name, "[Construction Documentation Style]");
    assert_eq!(s.front_color, Some([255, 255, 255]));
    assert_eq!(s.back_color, Some([164, 178, 187]));
    assert_eq!(
        s.description,
        "[Default face colors. Profile Edges. White background.]"
    );
    assert_eq!(s.items.len(), 58);
    assert_eq!(
        (s.items[&1007].kind, s.items[&1007].value.as_str()),
        (4, "2")
    );
    assert_eq!(
        (s.items[&2008].kind, s.items[&2008].value.as_str()),
        (7, "0.65000000000000002")
    );
    assert!(s.watermarks.is_empty());
    assert!(s.active && !s.working_copy);
    assert_eq!(
        model.styles[1].folder,
        "[Construction Documentation Style]_1"
    );
    assert!(!model.styles[1].active && model.styles[1].working_copy);

    let cam = model.camera.as_ref().expect("saved view");
    let near = |a: f64, b: f64| (a - b).abs() < 0.01;
    assert!(near(cam.eye[0], 452.09) && near(cam.eye[1], -974.12) && near(cam.eye[2], 367.93));
    assert!(near(cam.target[0], 226.24));
    assert!(!cam.parallel);
    assert_eq!(cam.fov, 35.0);

    let batten = model
        .root
        .instances
        .iter()
        .find(|i| i.name == "BattenHatSection_1")
        .unwrap();
    assert_eq!(batten.layer, "Hat Sections");
    let w1 = model
        .root
        .instances
        .iter()
        .find(|i| i.name == "W1")
        .unwrap();
    assert!(w1.properties.is_empty());
    let sf = &w1.attribute_dictionaries["steelframer-dict"];
    assert_eq!(
        sf["generator"],
        AttrValue::String("SteelFramer::Engine::PanelGenerator".into())
    );
    assert_eq!(sf["profile"], AttrValue::String("362S200-43".into()));
}

/// Every face loop is a closed chain: each coedge ends where the next one starts.
#[test]
fn face_loops_are_closed() {
    for name in [
        "Untitled.skp",
        "SU_File.skp",
        "coedge_orientation_regression.skp",
    ] {
        let model = open(name);
        for d in model.definitions.values().chain([&model.root]) {
            for f in d.faces.values() {
                for l in &f.loops {
                    let ends: Vec<(i64, i64)> = l
                        .iter()
                        .map(|c| {
                            let e = &d.edges[&c.edge_id];
                            if c.orientation > 0 {
                                (e.v1_id, e.v2_id)
                            } else {
                                (e.v2_id, e.v1_id)
                            }
                        })
                        .collect();
                    for (i, e) in ends.iter().enumerate() {
                        let next = ends[(i + 1) % ends.len()];
                        assert_eq!(e.1, next.0, "{name}: face {} loop not closed", f.id);
                    }
                }
            }
        }
    }
}

#[test]
fn rejects_non_skp() {
    assert!(matches!(
        mt_skp::parse(b"hello"),
        Err(mt_skp::Error::NotSkp)
    ));
    // Valid magic, VFF version, but no ZIP.
    let mut d = vec![0xff, 0xfe, 0xff, 0x0e, 0, 0, 0xff, 0xfe, 0xff, 0x06];
    for c in "{25.0.1}".bytes() {
        d.extend([c, 0]);
    }
    assert!(mt_skp::parse(&d).is_err());
}

/// Truncating a real file anywhere must give an error or a model, never a panic.
#[test]
fn truncated_files_do_not_panic() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/openskp/SU_File.skp");
    let data = std::fs::read(path).unwrap();
    for len in (0..data.len()).step_by(97) {
        let _ = mt_skp::parse(&data[..len]);
    }
}
