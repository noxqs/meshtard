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
        ..Layer::default()
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
        ..Layer::default()
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

/// Texts and dimensions keep their tags through save and reopen, and through File >
/// Import into a model whose tags are numbered differently.
#[test]
fn annotation_tags_round_trip() {
    use mt_kernel::annotation::{Dimension, Text};
    let mut m = Model::default();
    for name in ["Notes", "Dims"] {
        m.layers.push(Layer {
            name: name.into(),
            ..Layer::default()
        });
    }
    add_line(&mut m.root, DVec3::ZERO, DVec3::X * 10.0);
    let mut tagged = Text::leader(DVec3::X * 5.0, DVec3::splat(10.0), "Edge".into());
    tagged.layer = Some(1);
    let plain = Text::leader(DVec3::ZERO, DVec3::Z, "Plain".into());
    m.root.texts = vec![tagged, plain];
    m.root.dimensions = vec![
        Dimension::linear(DVec3::ZERO, DVec3::X * 10.0, DVec3::Y * 3.0),
        Dimension {
            layer: Some(2),
            ..Dimension::linear(DVec3::ZERO, DVec3::Y * 4.0, DVec3::X * 2.0)
        },
    ];
    let r = round_trip(&m);
    let names = |m: &Model, l: Option<usize>| l.map(|i| m.layers[i].name.clone());
    let texts: Vec<_> = r.root.texts.iter().map(|t| names(&r, t.layer)).collect();
    assert_eq!(texts, [Some("Notes".into()), None]);
    let dims: Vec<_> = r
        .root
        .dimensions
        .iter()
        .map(|d| names(&r, d.layer))
        .collect();
    assert_eq!(dims, [None, Some("Dims".into())]);

    let mut dst = Model::default();
    dst.layers.push(Layer {
        name: "Dims".into(),
        ..Layer::default()
    });
    let def = import::merge_as_definition(&mut dst, r, "Imported");
    let e = &dst.definitions[def].entities;
    let texts: Vec<_> = e.texts.iter().map(|t| names(&dst, t.layer)).collect();
    assert_eq!(texts, [Some("Notes".into()), None]);
    let dims: Vec<_> = e.dimensions.iter().map(|d| names(&dst, d.layer)).collect();
    assert_eq!(dims, [None, Some("Dims".into())]);
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

/// The Styles panel extras with documented style items (edge color, endpoints,
/// jitter, transparency, background, modeling colors, ambient occlusion) survive
/// save and reopen; ambient occlusion color and watermarks are model-only.
#[test]
fn style_extras_round_trip() {
    use mt_kernel::style_extras::*;
    let mut m = Model::default();
    square(&mut m.root, DVec3::ZERO, 10.0);
    let s = &mut m.style;
    s.edge_look = EdgeLook {
        color_mode: EdgeColorMode::ByAxis,
        color: [10, 20, 30],
        endpoints: true,
        endpoint_length: 11.0,
        jitter: true,
    };
    s.face_look = FaceLook {
        transparency_off: true,
        quality: TransparencyQuality::Faster,
    };
    s.background = Background {
        color: [1, 2, 3],
        sky: false,
        sky_color: [4, 5, 6],
        horizon_color: [7, 8, 9],
        horizon_blend: 0.5,
        ground: false,
        ground_color: [10, 11, 12],
        ground_transparency: 0.25,
        ground_from_below: true,
    };
    s.modeling_colors = ModelingColors {
        selected: [200, 1, 1],
        locked: [1, 200, 1],
        guides: [1, 1, 200],
        section_active: [9, 9, 9],
        section_inactive: [8, 8, 8],
        section_cut: [7, 7, 7],
    };
    s.ambient_occlusion = AmbientOcclusion {
        enabled: true,
        intensity: 0.75,
        distance: 6.0,
        ..Default::default()
    };
    let back = round_trip(&m).style;
    assert_eq!(back.edge_look, m.style.edge_look);
    assert_eq!(back.face_look, m.style.face_look);
    assert_eq!(back.background, m.style.background);
    assert_eq!(back.modeling_colors, m.style.modeling_colors);
    assert_eq!(back.ambient_occlusion, m.style.ambient_occlusion);
    for mode in EdgeColorMode::ALL {
        m.style.edge_look.color_mode = mode;
        assert_eq!(round_trip(&m).style.edge_look.color_mode, mode);
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
        ..Layer::default()
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

#[test]
fn arcs_edge_tags_and_definition_guides_round_trip() {
    use std::f64::consts::{FRAC_PI_2, TAU};
    let mut m = Model::default();
    m.layers.push(Layer {
        name: "Curves".into(),
        visible: true,
        ..Layer::default()
    });
    // A circle about a tilted axis, an arc swept clockwise (negative sweep) from a
    // nonzero start angle, and a loose edge on a tag.
    let circle = mt_kernel::Arc {
        center: DVec3::new(50.0, 0.0, 10.0),
        normal: DVec3::new(1.0, 1.0, 1.0).normalize(),
        x_axis: DVec3::new(1.0, -1.0, 0.0).normalize(),
        radius: 5.0,
        start_angle: 0.0,
        end_angle: TAU,
        segments: 12,
    };
    let mut pts = circle.points();
    pts.pop();
    pts.push(pts[0]);
    add_curve(&mut m.root, &pts, Some(circle));
    let arc = mt_kernel::Arc {
        center: DVec3::new(0.0, 100.0, 0.0),
        normal: DVec3::Z,
        x_axis: DVec3::X,
        radius: 20.0,
        start_angle: FRAC_PI_2,
        end_angle: -FRAC_PI_2 / 2.0,
        segments: 6,
    };
    let edges = add_curve(&mut m.root, &arc.points(), Some(arc));
    for e in edges {
        m.root.edges[e].attrs.layer = Some(1);
    }
    let e = add_line(
        &mut m.root,
        DVec3::new(0.0, -50.0, 0.0),
        DVec3::new(10.0, -50.0, 0.0),
    );
    for e in e {
        m.root.edges[e].attrs.layer = Some(1);
    }
    // A component holding a guide, a guide point and a section plane.
    let mut ents = Entities::new();
    square(&mut ents, DVec3::ZERO, 10.0);
    ents.guides.push(mt_kernel::Guide {
        point: DVec3::new(0.0, 5.0, 0.0),
        dir: Some(DVec3::X),
    });
    ents.guides.push(mt_kernel::Guide {
        point: DVec3::new(5.0, 5.0, 0.0),
        dir: None,
    });
    ents.section_planes.push(mt_kernel::SectionPlane {
        point: DVec3::new(5.0, 5.0, 0.0),
        normal: DVec3::X,
        name: String::new(),
        hidden: false,
        symbol: String::new(),
    });
    let def = m.definitions.insert(mt_kernel::Definition {
        name: "Cabinet".into(),
        entities: ents,
        ..Default::default()
    });
    m.root.instances.push(mt_kernel::Instance {
        definition: def,
        transform: DMat4::from_translation(DVec3::new(200.0, 0.0, 0.0)),
        name: String::new(),
        hidden: false,
        locked: false,
        layer: None,
        material: None,
        attrs: Default::default(),
    });

    let r = round_trip(&m);
    // Both curves come back as arcs, through the same points.
    let arcs: Vec<_> = r.root.curves.values().filter_map(|c| c.arc).collect();
    assert_eq!(arcs.len(), 2, "{:?}", r.root.curves);
    let c = arcs.iter().find(|a| a.is_circle()).unwrap();
    assert!((c.center - circle.center).length() < 1e-9);
    assert!((c.normal - circle.normal).length() < 1e-9);
    assert!((c.radius - 5.0).abs() < 1e-9 && c.segments == 12);
    let a = arcs.iter().find(|a| !a.is_circle()).unwrap();
    assert!((a.radius - 20.0).abs() < 1e-9 && a.segments == 6);
    assert!((a.end_angle - a.start_angle - 0.75 * std::f64::consts::PI).abs() < 1e-9);
    // Same point set: the arc's own points match the reopened vertices.
    for p in a.points() {
        assert!(
            r.root
                .vertices
                .values()
                .any(|v| (v.pos - p).length() < 1e-9),
            "{p}"
        );
    }
    // Edge tags.
    let tagged = r
        .root
        .edges
        .values()
        .filter(|e| e.attrs.layer == Some(1))
        .count();
    assert_eq!(tagged, 7);
    assert_eq!(r.layers[1].name, "Curves");
    // Guides and section planes inside the component; the plane is drawn around it.
    let d = r
        .definitions
        .values()
        .find(|d| d.name == "Cabinet")
        .unwrap();
    assert_eq!(d.entities.guides.len(), 2);
    assert_eq!(d.entities.section_planes.len(), 1);
    let s = &d.entities.section_planes[0];
    assert!((s.normal - DVec3::X).length() < 1e-9);
    assert!(
        (s.point - DVec3::new(5.0, 5.0, 0.0)).length() < 1e-9,
        "{}",
        s.point
    );
}

#[test]
fn saved_view_round_trip() {
    let mut m = Model::default();
    square(&mut m.root, DVec3::ZERO, 10.0);
    let view = mt_skp::ViewCamera {
        eye: [120.0, -340.0, 95.0],
        target: [5.0, 5.0, 0.0],
        up: [0.0, 0.0, 1.0],
        fov: 42.0,
        parallel: false,
        ortho_height: 250.0,
    };
    let bytes = export::to_skp_view(&m, &[], Some(&view)).unwrap();
    let c = mt_skp::parse(&bytes).unwrap().camera.unwrap();
    assert_eq!(
        (c.eye, c.target, c.fov, c.parallel),
        (view.eye, view.target, 42.0, false)
    );
    // Without a view: the writer's Iso camera.
    let c = mt_skp::parse(&export::to_skp(&m).unwrap())
        .unwrap()
        .camera
        .unwrap();
    assert!(c.parallel);
}

/// An instance of `def` at `at`.
fn place(def: mt_kernel::DefinitionId, at: DVec3) -> mt_kernel::Instance {
    mt_kernel::Instance {
        definition: def,
        transform: DMat4::from_translation(at),
        name: String::new(),
        hidden: false,
        locked: false,
        layer: None,
        material: None,
        attrs: Default::default(),
    }
}

/// The texts and dimensions OpenSKP's Python reader decodes in each entity list of
/// `bytes` (`crates/mt-skp/tests/oracle/read_definition_annotations.py`), or `None`
/// without `python3` and `openskp`.
fn oracle_annotations(name: &str, bytes: &[u8]) -> Option<serde_json::Value> {
    use std::process::Command;
    let ok = Command::new("python3")
        .args(["-I", "-c", "import openskp"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!("SKIPPED: python3 with openskp not available (see tools/README.md)");
        return None;
    }
    let file = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.skp"));
    std::fs::write(&file, bytes).unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../mt-skp/tests/oracle/read_definition_annotations.py");
    let out = Command::new("python3")
        .arg("-I")
        .arg(script)
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(serde_json::from_slice(&out.stdout).unwrap())
}

/// Texts (leader and screen) and dimensions inside groups and components are saved
/// in their definitions, with their tags, and reopen there: a component placed four
/// times (three at the root, one inside a group), a group nested in another group,
/// and the root's own. Checked with this reader and OpenSKP's.
#[test]
fn annotations_in_definitions_round_trip() {
    use mt_kernel::annotation::{Dimension, Text};
    let mut m = Model::default();
    for name in ["Notes", "Dims"] {
        m.layers.push(Layer {
            name: name.into(),
            ..Layer::default()
        });
    }
    let tagged = |mut t: Text, l| {
        t.layer = Some(l);
        t
    };
    // The component: a leader text on Notes, a screen text, a dimension on Dims and
    // an untagged one.
    let mut label = Entities::new();
    square(&mut label, DVec3::ZERO, 10.0);
    label.texts = vec![
        tagged(
            Text::leader(
                DVec3::new(5.0, 5.0, 0.0),
                DVec3::new(4.0, 2.0, 8.0),
                "Label".into(),
            ),
            1,
        ),
        Text::screen(0.25, 0.75, "Screen in component".into()),
    ];
    label.dimensions = vec![
        Dimension {
            layer: Some(2),
            ..Dimension::linear(DVec3::ZERO, DVec3::X * 10.0, DVec3::Y * -3.0)
        },
        Dimension::linear(DVec3::ZERO, DVec3::Y * 10.0, DVec3::X * 2.5),
    ];
    let label = m.definitions.insert(mt_kernel::Definition {
        name: "Label".into(),
        entities: label,
        ..Default::default()
    });
    // A group in a group, each with its own text and dimension; the outer one also
    // holds the component.
    let mut inner = Entities::new();
    square(&mut inner, DVec3::new(0.0, 0.0, 5.0), 4.0);
    inner.texts = vec![tagged(
        Text::leader(
            DVec3::new(2.0, 2.0, 5.0),
            DVec3::new(0.0, 0.0, 6.0),
            "Deep".into(),
        ),
        1,
    )];
    inner.dimensions = vec![Dimension::linear(
        DVec3::new(0.0, 0.0, 5.0),
        DVec3::new(4.0, 0.0, 5.0),
        DVec3::Y * 1.5,
    )];
    let inner = m.definitions.insert(mt_kernel::Definition {
        name: "Inner".into(),
        entities: inner,
        is_group: true,
        ..Default::default()
    });
    let mut outer = Entities::new();
    square(&mut outer, DVec3::ZERO, 20.0);
    outer.instances = vec![
        place(inner, DVec3::new(1.0, 1.0, 0.0)),
        place(label, DVec3::Z * 30.0),
    ];
    outer.texts = vec![Text::screen(0.5, 0.125, "Screen in group".into())];
    outer.dimensions = vec![Dimension {
        layer: Some(2),
        ..Dimension::linear(DVec3::ZERO, DVec3::X * 20.0, DVec3::Y * 4.0)
    }];
    let outer = m.definitions.insert(mt_kernel::Definition {
        name: "Outer".into(),
        entities: outer,
        is_group: true,
        ..Default::default()
    });
    m.root.instances = vec![
        place(label, DVec3::ZERO),
        place(label, DVec3::X * 50.0),
        place(label, DVec3::Y * 50.0),
        place(outer, DVec3::X * -100.0),
    ];
    m.root.texts = vec![Text::leader(DVec3::ZERO, DVec3::Z, "Root".into())];
    add_line(&mut m.root, DVec3::ZERO, DVec3::X * 10.0);

    let bytes = export::to_skp(&m).unwrap();
    let r = import::from_skp(&mt_skp::parse(&bytes).unwrap());
    let def = |name: &str| {
        let (id, d) = r
            .definitions
            .iter()
            .find(|(_, d)| d.name == name)
            .unwrap_or_else(|| panic!("{name} reopened"));
        (id, &d.entities)
    };
    let layer = |l: Option<usize>| l.map(|i| r.layers[i].name.as_str());
    for name in ["Label", "Inner", "Outer"] {
        let src = m.definitions.values().find(|d| d.name == name).unwrap();
        let (_, e) = def(name);
        // Exactly the texts saved (points, leaders, screen positions), tags by name.
        let texts = |t: &[Text], m: &Model| -> Vec<_> {
            t.iter()
                .map(|t| {
                    let l = t.layer.map(|i| m.layers[i].name.clone());
                    (t.text.clone(), t.point, t.leader, t.screen, l)
                })
                .collect()
        };
        assert_eq!(
            texts(&e.texts, &r),
            texts(&src.entities.texts, &m),
            "{name}"
        );
        assert_eq!(e.dimensions.len(), src.entities.dimensions.len(), "{name}");
        for (a, b) in e.dimensions.iter().zip(&src.entities.dimensions) {
            assert_eq!((a.a, a.b), (b.a, b.b), "{name}");
            assert!((a.offset - b.offset).length() < 1e-9, "{name}: {a:?}");
            assert_eq!(
                layer(a.layer),
                b.layer.map(|i| m.layers[i].name.as_str()),
                "{name}"
            );
        }
    }
    // Nothing moved to the root, and the component is still shared four times.
    assert_eq!(r.root.texts.len(), 1);
    assert!(r.root.dimensions.is_empty());
    let (label_id, _) = def("Label");
    let (outer_id, _) = def("Outer");
    let (inner_id, _) = def("Inner");
    let uses = r
        .definitions
        .values()
        .map(|d| &d.entities)
        .chain([&r.root])
        .flat_map(|e| &e.instances)
        .filter(|i| i.definition == label_id)
        .count();
    assert_eq!(uses, 4);
    assert!(r.definitions[outer_id].is_group && r.definitions[inner_id].is_group);
    assert!(
        r.definitions[outer_id]
            .entities
            .instances
            .iter()
            .any(|i| i.definition == inner_id)
    );

    // OpenSKP decodes the same texts and dimensions in the same definitions.
    let Some(j) = oracle_annotations("annotations_in_definitions", &bytes) else {
        return;
    };
    let p = |v: DVec3| serde_json::json!([v.x, v.y, v.z]);
    for (name, src) in [("Label", label), ("Inner", inner), ("Outer", outer)] {
        let e = &m.definitions[src].entities;
        let texts: Vec<_> = e
            .texts
            .iter()
            .map(|t| {
                let (point, lp) = if t.screen {
                    (serde_json::Value::Null, serde_json::Value::Null)
                } else {
                    (p(t.point), p(t.point + t.leader))
                };
                serde_json::json!([t.text, t.layer.map(|i| &m.layers[i].name), point, lp])
            })
            .collect();
        assert_eq!(
            j["definitions"][name]["text"],
            serde_json::json!(texts),
            "{name}"
        );
        let dims: Vec<_> = e
            .dimensions
            .iter()
            .map(|d| {
                let offset = mt_kernel::annotation::saved_offset(d).unwrap();
                serde_json::json!([p(d.a), p(d.b), offset, d.layer.map(|i| &m.layers[i].name)])
            })
            .collect();
        assert_eq!(
            j["definitions"][name]["dimension"],
            serde_json::json!(dims),
            "{name}"
        );
    }
    assert_eq!(j["root"]["text"].as_array().unwrap().len(), 1);
    assert_eq!(j["root"]["dimension"], serde_json::json!([]));
}

/// A SketchUp-saved screen text (`legacy_v7_synthetic.skp`) is saved again as a
/// screen text at the same place, and OpenSKP reads the saved file.
#[test]
fn corpus_screen_text_round_trips() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/openskp/legacy_v7_synthetic.skp");
    let Ok(data) = std::fs::read(path) else {
        return;
    };
    let m = import::from_skp(&mt_skp::parse(&data).unwrap());
    let t = &m.root.texts[0];
    assert!(t.screen && t.point.x == 0.01 && t.point.y == 0.8, "{t:?}");
    let bytes = export::to_skp(&m).unwrap();
    let src = mt_skp::parse(&bytes).unwrap();
    let saved = &src.root.texts[0];
    assert_eq!(saved.screen_position, Some([0.01, 0.8]));
    assert_eq!(saved.text, t.text);
    assert_eq!(import::from_skp(&src).root.texts, m.root.texts);
    let Some(j) = oracle_annotations("corpus_screen_text", &bytes) else {
        return;
    };
    assert_eq!(
        j["root"]["text"],
        serde_json::json!([[t.text, null, null, null]])
    );
}
