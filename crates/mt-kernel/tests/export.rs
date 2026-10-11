//! Save then reopen: `export::to_skp` followed by `mt_skp::parse` and `import::from_skp`.

use glam::{DMat4, DQuat, DVec3};
use mt_kernel::group::{Selection, make_group};
use mt_kernel::{
    Checks, Entities, Layer, Material, Model, add_curve, add_line, export, import, validate,
};

fn square(ents: &mut Entities, o: DVec3, s: f64) {
    let p = [
        o,
        o + DVec3::X * s,
        o + DVec3::new(s, s, 0.0),
        o + DVec3::Y * s,
    ];
    for i in 0..4 {
        add_line(ents, p[i], p[(i + 1) % 4]);
    }
}

fn round_trip(model: &Model) -> Model {
    let bytes = export::to_skp(model).expect("writes");
    if let Ok(dir) = std::env::var("EXPORT_DUMP") {
        let n = std::fs::read_dir(&dir).map_or(0, |d| d.count());
        std::fs::write(format!("{dir}/export{n}.skp"), &bytes).unwrap();
    }
    let src = mt_skp::parse(&bytes).expect("parses");
    import::from_skp(&src)
}

#[test]
fn faces_edges_flags_materials_tags() {
    let mut m = Model::default();
    m.materials.push(Material {
        name: "Brick".into(),
        color: [180, 60, 40],
        opacity: 1.0,
        texture: None,
    });
    m.materials.push(Material {
        name: "Glass".into(),
        color: [120, 180, 220],
        opacity: 0.4,
        texture: None,
    });
    m.layers.push(Layer {
        name: "Walls".into(),
        color: [10, 200, 30],
        visible: false,
    });
    square(&mut m.root, DVec3::ZERO, 10.0);
    square(&mut m.root, DVec3::new(10.0, 0.0, 0.0), 10.0);
    // A hole in the first square.
    square(&mut m.root, DVec3::new(3.0, 3.0, 0.0), 2.0);
    add_line(
        &mut m.root,
        DVec3::new(30.0, 0.0, 0.0),
        DVec3::new(40.0, 5.0, 0.0),
    );
    let (f0, e0) = {
        let f = m.root.faces.keys().next().unwrap();
        (f, m.root.edges.keys().next().unwrap())
    };
    m.root.faces[f0].attrs.material = Some(0);
    m.root.faces[f0].attrs.back_material = Some(1);
    m.root.faces[f0].attrs.layer = Some(1);
    m.root.edges[e0].attrs.soft = true;
    m.root.edges[e0].attrs.smooth = true;
    assert!(validate(&m.root, Checks::ALL).is_empty());

    let r = round_trip(&m);
    assert_eq!(r.root.faces.len(), m.root.faces.len());
    assert_eq!(r.root.edges.len(), m.root.edges.len());
    assert_eq!(r.materials.len(), 2);
    assert!(
        (r.materials[1].opacity - 0.4).abs() < 1e-6,
        "{:?}",
        r.materials[1]
    );
    assert_eq!(r.layers.len(), 2);
    assert_eq!(r.layers[1].name, "Walls");
    assert!(!r.layers[1].visible);
    assert_eq!(
        r.root
            .edges
            .values()
            .filter(|e| e.attrs.soft && e.attrs.smooth)
            .count(),
        1
    );
    let painted: Vec<_> = r
        .root
        .faces
        .values()
        .filter(|f| f.attrs.material.is_some())
        .collect();
    assert_eq!(painted.len(), 1);
    assert_eq!(
        r.materials[painted[0].attrs.material.unwrap()].name,
        "Brick"
    );
    assert_eq!(
        r.materials[painted[0].attrs.back_material.unwrap()].name,
        "Glass"
    );
    assert_eq!(painted[0].attrs.layer, Some(1));
    assert_eq!(painted[0].holes.len(), m.root.faces[f0].holes.len());
    assert_eq!(
        r.root.faces.values().filter(|f| f.holes.len() == 1).count(),
        1
    );
    let area = |e: &Entities| {
        e.vertices
            .values()
            .map(|v| v.pos)
            .fold(DVec3::ZERO, |a, p| a.max(p))
    };
    assert_eq!(area(&r.root), area(&m.root));
}

#[test]
fn curves_components_groups_guides_sections() {
    let mut m = Model::default();
    // A circle curve.
    let arc = mt_kernel::Arc {
        center: DVec3::new(50.0, 0.0, 0.0),
        normal: DVec3::Z,
        x_axis: DVec3::X,
        radius: 5.0,
        start_angle: 0.0,
        end_angle: std::f64::consts::TAU,
        segments: 12,
    };
    let mut pts = arc.points();
    pts.pop();
    pts.push(pts[0]);
    add_curve(&mut m.root, &pts, Some(arc));
    // A component used twice, one rotated, and a group.
    square(&mut m.root, DVec3::ZERO, 10.0);
    let sel = Selection {
        edges: m
            .root
            .edges
            .keys()
            .filter(|e| m.root.edges[*e].attrs.curve.is_none())
            .collect(),
        faces: m
            .root
            .faces
            .keys()
            .filter(|f| m.root.faces[*f].outer.coedges.len() == 4)
            .collect(),
        instances: vec![],
    };
    let i = make_group(&mut m.definitions, &mut m.root, &sel, "Box".into(), false).unwrap();
    let mut copy = m.root.instances[i].clone();
    copy.transform =
        DMat4::from_rotation_translation(DQuat::from_rotation_z(0.5), DVec3::new(100.0, 20.0, 3.0));
    copy.name = "Second".into();
    m.root.instances.push(copy.clone());
    square(&mut m.root, DVec3::new(0.0, 50.0, 0.0), 4.0);
    let sel = Selection {
        edges: m
            .root
            .edges
            .keys()
            .filter(|e| m.root.pos(m.root.edges[*e].start).y >= 50.0)
            .collect(),
        faces: m
            .root
            .faces
            .keys()
            .filter(|f| m.root.faces[*f].outer.coedges.len() == 4)
            .collect(),
        instances: vec![],
    };
    let g = make_group(
        &mut m.definitions,
        &mut m.root,
        &sel,
        "Group#1".into(),
        true,
    )
    .unwrap();
    // The group nests an instance of the component.
    let box_def = copy.definition;
    let gdef = m.root.instances[g].definition;
    m.definitions[gdef]
        .entities
        .instances
        .push(mt_kernel::Instance {
            transform: DMat4::from_translation(DVec3::Z * 20.0),
            ..copy
        });
    m.root.guides.push(mt_kernel::Guide {
        point: DVec3::ZERO,
        dir: Some(DVec3::X),
    });
    m.root.guides.push(mt_kernel::Guide {
        point: DVec3::ONE,
        dir: None,
    });
    m.root.section_planes.push(mt_kernel::SectionPlane {
        point: DVec3::Z * 5.0,
        normal: DVec3::Z,
        name: String::new(),
        hidden: false,
        symbol: String::new(),
    });

    let r = round_trip(&m);
    // Circle curve: 12 edges, one curve, and its face.
    assert_eq!(r.root.edges.len(), 12);
    assert_eq!(r.root.curves.len(), 1);
    assert_eq!(r.root.faces.len(), 1);
    assert_eq!(r.root.instances.len(), 3);
    let defs: Vec<_> = r.definitions.values().collect();
    let boxd = defs
        .iter()
        .find(|d| d.name == "Box")
        .expect("component written");
    assert_eq!(
        (boxd.entities.faces.len(), boxd.entities.edges.len()),
        (1, 4)
    );
    let second = r
        .root
        .instances
        .iter()
        .find(|i| i.name == "Second")
        .expect("named instance");
    let p = DVec3::new(1.0, 2.0, 3.0);
    assert!((second.transform.transform_point3(p) - copy_xf().transform_point3(p)).length() < 1e-9);
    let group = r.root.instances.iter().find(|i| {
        r.definitions[i.definition].is_group
            || r.definitions[i.definition].entities.instances.len() == 1
    });
    assert!(group.is_some(), "group with a nested component instance");
    assert_eq!(r.root.guides.len(), 2);
    assert_eq!(r.root.section_planes.len(), 1);
    let s = &r.root.section_planes[0];
    assert!((s.point.z - 5.0).abs() < 1e-9, "{:?}", s.point);
    assert!((s.normal - DVec3::Z).length() < 1e-9);
    let _ = box_def;
}

fn copy_xf() -> DMat4 {
    DMat4::from_rotation_translation(DQuat::from_rotation_z(0.5), DVec3::new(100.0, 20.0, 3.0))
}

#[test]
#[ignore]
fn dump() {
    let mut m = Model::default();
    m.layers.push(Layer {
        name: "Walls".into(),
        color: [10, 200, 30],
        visible: true,
    });
    square(&mut m.root, DVec3::ZERO, 10.0);
    square(&mut m.root, DVec3::new(3.0, 3.0, 0.0), 2.0);
    let f = m.root.faces.keys().next().unwrap();
    m.root.faces[f].attrs.layer = Some(1);
    std::fs::write(std::env::var("OUT").unwrap(), export::to_skp(&m).unwrap()).unwrap();
}

/// Every corpus model survives save and reopen with the same geometry counts. The
/// SketchUp trial folders (`2026`, `2017`: saved by SketchUp; `meshtard`: saved by
/// meshtard) are read when present, and there a file that does not open fails.
#[test]
fn corpus_round_trips() {
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
    let paths = ["openskp", "2026", "2017", "meshtard"]
        .into_iter()
        .filter_map(|d| Some((d, std::fs::read_dir(corpus.join(d)).ok()?)))
        .flat_map(|(d, entries)| entries.map(move |e| (d, e.unwrap().path())));
    let mut failures = Vec::new();
    for (dir, path) in paths {
        if path.extension().is_none_or(|x| x != "skp") {
            continue;
        }
        let src = match mt_skp::read(&path) {
            Ok(src) => src,
            // Some OpenSKP samples are known not to open.
            Err(_) if dir == "openskp" => continue,
            Err(e) => {
                failures.push(format!("{}: does not open: {e}", path.display()));
                continue;
            }
        };
        let m = import::from_skp(&src);
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if let (Ok(dir), Ok(bytes)) = (std::env::var("EXPORT_DUMP"), export::to_skp(&m)) {
            std::fs::write(format!("{dir}/{name}"), bytes).unwrap();
        }
        let r = match export::to_skp(&m)
            .map_err(|e| e.to_string())
            .and_then(|b| mt_skp::parse(&b).map_err(|e| e.to_string()))
        {
            Ok(s) => import::from_skp(&s),
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        let counts = |m: &Model| {
            let mut defs: Vec<(String, usize, usize, usize)> = m
                .definitions
                .values()
                .map(|d| {
                    (
                        d.name.clone(),
                        d.entities.faces.len(),
                        d.entities.edges.len(),
                        d.entities.instances.len(),
                    )
                })
                .collect();
            defs.sort();
            (
                m.root.faces.len(),
                m.root.edges.len(),
                m.root.instances.len(),
                m.materials.len(),
                m.layers.len(),
                defs,
            )
        };
        let (a, b) = (counts(&m), counts(&r));
        if (a.0, a.1, a.2, a.3, a.4) != (b.0, b.1, b.2, b.3, b.4) {
            failures.push(format!(
                "{name}: root/materials/layers {:?} -> {:?}",
                (a.0, a.1, a.2, a.3, a.4),
                (b.0, b.1, b.2, b.3, b.4)
            ));
        }
        let differing: Vec<_> =
            a.5.iter()
                .zip(&b.5)
                .filter(|(x, y)| x != y)
                .take(3)
                .collect();
        if a.5.len() != b.5.len() || !differing.is_empty() {
            failures.push(format!(
                "{name}: {} vs {} definitions, e.g. {differing:?}",
                a.5.len(),
                b.5.len()
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
#[ignore]
fn gondola_defs() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/openskp/gondola_v20.skp");
    let m = import::from_skp(&mt_skp::read(&p).unwrap());
    for d in m.definitions.values() {
        let used_root = m
            .root
            .instances
            .iter()
            .filter(|i| m.definitions[i.definition].name == d.name)
            .count();
        let nested = m
            .definitions
            .values()
            .flat_map(|x| &x.entities.instances)
            .filter(|i| m.definitions[i.definition].name == d.name)
            .count();
        println!(
            "{} group={} root={} nested={}",
            d.name, d.is_group, used_root, nested
        );
    }
}

/// Textured materials and positioned textures survive save and reopen: every textured
/// face side maps its vertices to the same texture coordinates.
#[test]
fn textures_round_trip() {
    use std::collections::BTreeSet;
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/openskp/capilla_quiroz_v17.skp");
    let m = import::from_skp(&mt_skp::read(&p).unwrap());
    let r = import::from_skp(&mt_skp::parse(&export::to_skp(&m).unwrap()).unwrap());
    // (definition, material, vertex, uv) rounded, over every textured face side.
    let uvs = |m: &Model| {
        let mut out = BTreeSet::new();
        let defs = m
            .definitions
            .values()
            .map(|d| (d.name.clone(), &d.entities))
            .chain([(String::new(), &m.root)]);
        let round = |x: f64| (x * 1e4).round() as i64;
        for (name, ents) in defs {
            for face in ents.faces.values() {
                for back in [false, true] {
                    let mat = if back {
                        face.attrs.back_material
                    } else {
                        face.attrs.material
                    };
                    let Some(mat) = mat.map(|i| &m.materials[i]) else {
                        continue;
                    };
                    let Some(map) = mt_kernel::texture::face_mapping(face, back, mat) else {
                        continue;
                    };
                    for p in ents.loop_points(&face.outer) {
                        let uv = mt_kernel::texture::uv_at(&map, p);
                        out.insert((
                            name.clone(),
                            mat.name.clone(),
                            p.to_array().map(round),
                            uv.to_array().map(round),
                        ));
                    }
                }
            }
        }
        out
    };
    let textured = |m: &Model| m.materials.iter().filter(|x| x.texture.is_some()).count();
    assert_eq!(textured(&m), 4);
    assert_eq!(textured(&r), 4);
    let positioned = |m: &Model| {
        m.definitions
            .values()
            .map(|d| &d.entities)
            .chain([&m.root])
            .flat_map(|e| e.faces.values())
            .filter(|f| f.attrs.uv.is_some())
            .count()
    };
    assert!(positioned(&m) > 0);
    assert_eq!(positioned(&m), positioned(&r));
    let (a, b) = (uvs(&m), uvs(&r));
    assert!(!a.is_empty());
    let diff: Vec<_> = a.symmetric_difference(&b).take(4).collect();
    assert!(diff.is_empty(), "{} vs {}: {diff:?}", a.len(), b.len());
}

/// Leader texts and dimensions keep their placement through save and reopen.
#[test]
fn annotations_round_trip() {
    use mt_kernel::annotation::{Dimension, Text};
    let mut m = Model::default();
    add_line(&mut m.root, DVec3::ZERO, DVec3::X * 10.0);
    let dim = Dimension::linear(DVec3::ZERO, DVec3::X * 10.0, DVec3::Y * 3.0);
    let text = Text::leader(DVec3::X * 5.0, DVec3::splat(10.0), "Edge".into());
    m.root.dimensions.push(dim.clone());
    m.root.texts.push(text.clone());
    let r = round_trip(&m);
    assert_eq!(r.root.texts, [text]);
    assert_eq!(r.root.dimensions.len(), 1);
    let d = &r.root.dimensions[0];
    assert_eq!((d.a, d.b), (dim.a, dim.b));
    assert!((d.offset - dim.offset).length() < 1e-12, "{:?}", d.offset);
}

/// The model's face colors survive save and reopen (written into the file's style).
#[test]
fn face_colors_round_trip() {
    for with_geometry in [false, true] {
        let mut m = Model {
            front_color: [250, 240, 230],
            back_color: [10, 20, 30],
            ..Model::default()
        };
        if with_geometry {
            square(&mut m.root, DVec3::ZERO, 10.0);
        }
        let r = round_trip(&m);
        assert_eq!(
            (r.front_color, r.back_color),
            (m.front_color, m.back_color),
            "geometry: {with_geometry}"
        );
    }
}

/// A dimension offset to either side reopens on the same side.
#[test]
fn dimension_offset_sign_round_trips() {
    for side in [1.0, -1.0] {
        let mut m = Model::default();
        square(&mut m.root, DVec3::ZERO, 10.0);
        m.root
            .dimensions
            .push(mt_kernel::annotation::Dimension::linear(
                DVec3::ZERO,
                DVec3::X * 10.0,
                DVec3::Y * 4.0 * side,
            ));
        let r = round_trip(&m);
        let d = r.root.dimensions.first().expect("dimension reopened");
        assert!(
            (d.offset - DVec3::Y * 4.0 * side).length() < 1e-6,
            "side {side}: {:?}",
            d.offset
        );
    }
}

/// A section plane reopens in its plane, drawn around the model rather than at the
/// origin's foot on the plane (the file keeps only the plane).
#[test]
fn section_plane_reopens_around_the_model() {
    let mut m = Model::default();
    square(&mut m.root, DVec3::new(100.0, 200.0, 0.0), 10.0);
    for (point, normal) in [
        (DVec3::new(105.0, 0.0, 0.0), DVec3::X),
        (DVec3::new(0.0, 0.0, 3.0), DVec3::new(0.0, 0.6, 0.8)),
    ] {
        m.root.section_planes.push(mt_kernel::SectionPlane {
            point,
            normal,
            name: String::new(),
            hidden: false,
            symbol: String::new(),
        });
    }
    let r = round_trip(&m);
    assert_eq!(r.root.section_planes.len(), 2);
    let center = DVec3::new(105.0, 205.0, 0.0);
    for (a, b) in m.root.section_planes.iter().zip(&r.root.section_planes) {
        assert!((a.normal - b.normal).length() < 1e-9);
        // Same plane: the reopened point lies on the original plane.
        assert!(
            (b.point - a.point).dot(a.normal).abs() < 1e-9,
            "{:?}",
            b.point
        );
        // Drawn where the model is: the model center projected onto the plane.
        let expected = center - a.normal * (center - a.point).dot(a.normal);
        assert!((b.point - expected).length() < 1e-9, "{:?}", b.point);
    }
}

/// Every style setting with a known style item survives save and reopen, with and
/// without geometry; the defaults come back as they were.
#[test]
fn style_settings_round_trip() {
    use mt_kernel::model::{FaceStyle, StyleSettings};
    for with_geometry in [false, true] {
        let mut m = Model::default();
        if with_geometry {
            square(&mut m.root, DVec3::ZERO, 10.0);
        }
        assert_eq!(round_trip(&m).style, StyleSettings::default());
        m.style = StyleSettings {
            edges: false,
            profiles: false,
            profile_width: 5.0,
            section_cut_width: 7.0,
            face_style: FaceStyle::HiddenLine,
            xray: true,
            section_planes: false,
            section_cuts: false,
            section_fill: false,
            section_fill_color: [12, 34, 56],
            axes: false,
            guides: false,
            hidden_geometry: true,
            back_edges: true,
            depth_cue: true,
            depth_cue_width: 6.0,
            extension: true,
            extension_length: 9.0,
            ..StyleSettings::default()
        };
        assert_eq!(round_trip(&m).style, m.style, "geometry: {with_geometry}");
        // The section display bits are independent.
        m.style.section_planes = true;
        assert_eq!(round_trip(&m).style, m.style);
    }
    for face_style in [
        FaceStyle::Wireframe,
        FaceStyle::HiddenLine,
        FaceStyle::Shaded,
        FaceStyle::ShadedWithTextures,
    ] {
        let mut m = Model::default();
        m.style.face_style = face_style;
        assert_eq!(round_trip(&m).style.face_style, face_style);
    }
}

/// Units and snapping (the file's `UnitsOptions`) survive save and reopen.
#[test]
fn units_options_round_trip() {
    use mt_skp::OptionValue::{Bool, Double, Int};
    for with_geometry in [false, true] {
        let mut m = Model::default();
        if with_geometry {
            square(&mut m.root, DVec3::ZERO, 10.0);
        }
        let options = [
            ("UnitsOptions", "LengthUnit", Int(2)),
            ("UnitsOptions", "LengthFormat", Int(0)),
            ("UnitsOptions", "LengthPrecision", Int(1)),
            ("UnitsOptions", "LengthSnapEnabled", Bool(false)),
            ("UnitsOptions", "LengthSnapLength", Double(0.5)),
            ("UnitsOptions", "AnglePrecision", Int(2)),
            ("UnitsOptions", "AngleSnapEnabled", Bool(false)),
            ("UnitsOptions", "SnapAngle", Double(7.5)),
        ];
        let bytes = export::to_skp_with(&m, &options).unwrap();
        let src = mt_skp::parse(&bytes).unwrap();
        for (section, key, value) in options {
            assert_eq!(src.named_options[section][key], value, "{key}");
        }
        // Without options the template's stay (Architectural inches, 1/16").
        let src = mt_skp::parse(&export::to_skp(&m).unwrap()).unwrap();
        assert_eq!(src.named_options["UnitsOptions"]["LengthFormat"], Int(1));
    }
    // A type mismatch is refused rather than written.
    let m = Model::default();
    assert!(export::to_skp_with(&m, &[("UnitsOptions", "LengthUnit", Bool(true))]).is_err());
}

/// A 2 x 1 PNG.
const PNG: [u8; 70] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x7b, 0x40, 0xe8,
    0xdd, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x38, 0x21, 0x07, 0x04,
    0x27, 0x00, 0x07, 0x23, 0x02, 0x09, 0xe4, 0x58, 0x88, 0x39, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// Image entities reopen as image entities (`CImage`), at the root and inside a
/// group, alongside tags and components.
#[test]
fn images_round_trip() {
    use mt_kernel::model::Definition;
    let mut m = Model::default();
    m.layers.push(Layer {
        name: "Photos".into(),
        color: [1, 2, 3],
        visible: true,
    });
    m.materials.push(Material {
        name: "photo.png".into(),
        color: [200, 30, 30],
        opacity: 1.0,
        texture: Some(mt_kernel::texture::Texture {
            filename: "photo.png".into(),
            data: PNG.as_slice().into(),
            width: 20.0,
            height: 10.0,
            colorize: false,
        }),
    });
    let mut picture = Entities::new();
    square(&mut picture, DVec3::ZERO, 20.0);
    for f in picture.faces.values_mut() {
        f.attrs.material = Some(0);
        f.attrs.back_material = Some(0);
    }
    let image = m.definitions.insert(Definition {
        name: "photo.png".into(),
        entities: picture,
        is_image: true,
        ..Default::default()
    });
    let place = |x: f64, layer| mt_kernel::Instance {
        definition: image,
        transform: DMat4::from_translation(DVec3::X * x),
        name: String::new(),
        hidden: false,
        locked: false,
        layer,
        material: None,
        attrs: Default::default(),
    };
    m.root.instances.push(place(0.0, Some(1)));
    let mut group = Entities::new();
    square(&mut group, DVec3::Y * 50.0, 5.0);
    group.instances.push(place(30.0, None));
    let g = m.definitions.insert(Definition {
        name: "Group#1".into(),
        entities: group,
        is_group: true,
        ..Default::default()
    });
    m.root.instances.push(mt_kernel::Instance {
        definition: g,
        ..place(0.0, None)
    });
    square(&mut m.root, DVec3::Y * -50.0, 5.0);

    let r = round_trip(&m);
    let images: Vec<_> = r.definitions.values().filter(|d| d.is_image).collect();
    assert_eq!(images.len(), 1);
    assert!(!images[0].is_group);
    assert_eq!(images[0].entities.faces.len(), 1);
    let root_image = r
        .root
        .instances
        .iter()
        .find(|i| r.definitions[i.definition].is_image)
        .expect("image at the root");
    assert_eq!(r.layers[root_image.layer.unwrap()].name, "Photos");
    let group = r
        .root
        .instances
        .iter()
        .find(|i| r.definitions[i.definition].is_group)
        .expect("group");
    let nested = &r.definitions[group.definition].entities.instances;
    assert_eq!(nested.len(), 1);
    assert!(r.definitions[nested[0].definition].is_image);
    assert!((nested[0].transform.transform_point3(DVec3::ZERO) - DVec3::X * 30.0).length() < 1e-9);
}
