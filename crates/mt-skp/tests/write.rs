//! Tests for the legacy-format writer (`mt_skp::write`), a port of OpenSKP's
//! `openskp.create`.
//!
//! - `byte_identical_to_openskp`: every model in `tests/oracle/write_models.py`
//!   is built here through the Rust API and must match the Python writer's
//!   output byte for byte. GUIDs and timestamps are made deterministic on both
//!   sides (SplitMix64 GUIDs from one seed, a fixed timestamp; see the script).
//! - `round_trip_ours`: every model parses with `mt_skp::parse` and has the
//!   expected entity counts.
//! - `round_trip_openskp`: OpenSKP's reader (`tools/oracle_dump.py`) sees the
//!   same counts as ours in the Rust-written files.
//!
//! The oracle tests need `python3` with `openskp` installed (see
//! `tools/README.md`) and skip otherwise.

use std::f64::consts::PI;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use mt_skp::write::{
    Arc, AttributeDict, AttributeValue as V, Circle, DEFAULT_LEADER, EdgeFlags, Face, GuideLine,
    Placement, Polyline, SkpBuilder, TextureMaterial, UvPin, WriteError,
};
use serde_json::Value;

const SEED: u64 = 0x5eed;
const TIMESTAMP: u32 = 1_700_000_000;

type R = Result<(), WriteError>;

/// Texture inputs: path strings are stored in the file, so they must match
/// what the Python side was given.
struct Ctx {
    png: String,
    jpg: String,
    png_bytes: Vec<u8>,
    jpg_bytes: Vec<u8>,
}

impl Ctx {
    fn local() -> Self {
        Self {
            png: "tex.png".into(),
            jpg: "tex.jpg".into(),
            png_bytes: b"\x89PNG\r\n\x1a\nstub".to_vec(),
            jpg_bytes: b"\xff\xd8\xff\xe0stub".to_vec(),
        }
    }

    fn from_dir(dir: &Path) -> Self {
        let png = dir.join("tex.png");
        let jpg = dir.join("tex.jpg");
        Self {
            png_bytes: std::fs::read(&png).unwrap(),
            jpg_bytes: std::fs::read(&jpg).unwrap(),
            png: png.to_str().unwrap().into(),
            jpg: jpg.to_str().unwrap().into(),
        }
    }
}

const QUAD: [[f64; 3]; 4] = [
    [0.0, 0.0, 0.0],
    [100.0, 0.0, 0.0],
    [100.0, 50.0, 0.0],
    [0.0, 50.0, 0.0],
];

fn tri(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Face {
    Face::new([a, b, c])
}

fn square(x: f64, y: f64, s: f64) -> Face {
    Face::new([
        [x, y, 0.0],
        [x + s, y, 0.0],
        [x + s, y + s, 0.0],
        [x, y + s, 0.0],
    ])
}

fn pins(p: [([f64; 3], [f64; 2]); 3]) -> Option<[UvPin; 3]> {
    Some(p.map(|(point, uv)| UvPin { point, uv }))
}

fn single_face(b: &mut SkpBuilder, _: &Ctx) -> R {
    b.add_face(&Face::new(QUAD))
}

fn box_(b: &mut SkpBuilder, _: &Ctx) -> R {
    let (x, y, z) = (40.0, 30.0, 20.0);
    let p = [
        [0.0, 0.0, 0.0],
        [x, 0.0, 0.0],
        [x, y, 0.0],
        [0.0, y, 0.0],
        [0.0, 0.0, z],
        [x, 0.0, z],
        [x, y, z],
        [0.0, y, z],
    ];
    for f in [
        [0, 3, 2, 1],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
    ] {
        b.add_face(&Face::new(f.map(|i| p[i])))?;
    }
    Ok(())
}

fn holes(b: &mut SkpBuilder, _: &Ctx) -> R {
    b.add_face(
        &Face::new([
            [0.0, 0.0, 0.0],
            [200.0, 0.0, 0.0],
            [200.0, 0.0, 100.0],
            [0.0, 0.0, 100.0],
        ])
        .hole([
            [20.0, 0.0, 20.0],
            [60.0, 0.0, 20.0],
            [60.0, 0.0, 70.0],
            [20.0, 0.0, 70.0],
        ])
        .hole([[100.0, 0.0, 30.0], [150.0, 0.0, 30.0], [125.0, 0.0, 80.0]]),
    )?;
    b.add_face(&Face::new([
        [200.0, 0.0, 0.0],
        [200.0, 50.0, 0.0],
        [200.0, 50.0, 100.0],
        [200.0, 0.0, 100.0],
    ]))
}

fn materials_layers(b: &mut SkpBuilder, _: &Ctx) -> R {
    let red = b.add_material("Red", [255, 0, 0, 255])?;
    let glass = b.add_material_with_opacity("Glass", [100, 150, 255, 128], Some(0.4))?;
    let again = b.add_material("Red", [0, 0, 0, 255])?;
    assert_eq!(red, again);
    let roof = b.add_layer("Roof", Some([200, 10, 10, 255]), true)?;
    let hidden = b.add_layer("Hidden", None, false)?;
    let plain = b.add_layer("Plain", None, true)?;
    b.add_face(
        &Face::new(QUAD)
            .material(red)
            .back_material(glass)
            .layer(roof),
    )?;
    b.add_face(&Face {
        hidden: true,
        ..tri([0.0, 0.0, 10.0], [10.0, 0.0, 10.0], [10.0, 10.0, 10.0])
            .material(again)
            .layer(hidden)
    })?;
    b.add_face(&Face {
        edges: EdgeFlags {
            hidden: true,
            soft: true,
            smooth: true,
            layer: None,
        },
        ..Face::new([
            [0.0, 0.0, 0.0],
            [0.0, 50.0, 0.0],
            [0.0, 50.0, 30.0],
            [0.0, 0.0, 30.0],
        ])
        .layer(plain)
    })
}

fn components(b: &mut SkpBuilder, _: &Ctx) -> R {
    let red = b.add_material("Red", [255, 0, 0, 255])?;
    let furniture = b.add_layer("Furniture", None, true)?;
    let mut chair = b.add_component_definition("Chair", None)?;
    chair.add_face(&square(0.0, 0.0, 20.0))?;
    chair.add_face(&Face::new([
        [0.0, 0.0, 0.0],
        [0.0, 20.0, 0.0],
        [0.0, 20.0, 30.0],
        [0.0, 0.0, 30.0],
    ]))?;
    let chair = chair.finish()?;
    b.add_instance(&chair, &Placement::at([100.0, 0.0, 0.0]))?;
    b.add_instance(
        &chair,
        &Placement::at([0.0, 100.0, 0.0])
            .named("Rotated")
            .rotated([0.0, 0.0, 1.0], PI / 4.0),
    )?;
    b.add_instance(
        &chair,
        &Placement {
            material: Some(red),
            layer: Some(furniture),
            hidden: true,
            ..Placement::default()
                .named("")
                .matrix([2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0])
        },
    )
}

fn groups(b: &mut SkpBuilder, _: &Ctx) -> R {
    let mut t = b.add_group(Placement::at([50.0, 0.0, 0.0]).named("Table"))?;
    t.add_face(&square(0.0, 0.0, 30.0))?;
    t.finish()?;
    let mut g = b.add_group(Placement {
        attributes: vec![AttributeDict::new("attributes").with("k", V::String("v".into()))],
        ..Placement::default().rotated([1.0, 0.0, 0.0], PI / 2.0)
    })?;
    g.add_face(&tri([0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0]))?;
    g.finish()?;
    b.add_face(&Face::new(QUAD))
}

fn groups_only(b: &mut SkpBuilder, _: &Ctx) -> R {
    let mut g = b.add_group(Placement {
        hidden: true,
        ..Placement::at([1.0, 2.0, 3.0]).named("Lonely")
    })?;
    g.add_face(&tri([0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0]))?;
    g.finish()?;
    Ok(())
}

fn nested(b: &mut SkpBuilder, _: &Ctx) -> R {
    let mut wheel = b.add_component_definition("Wheel", None)?;
    wheel.add_circle(&Circle::new([0.0; 3], [0.0, 1.0, 0.0], 10.0).segments(12))?;
    let wheel = wheel.finish()?;
    let mut engine = b.add_component_definition("Engine", None)?;
    engine.add_face(&square(0.0, 0.0, 30.0))?;
    let engine = engine.finish()?;
    let mut car = b.add_component_definition_with("Car", None, &[], true, true)?;
    car.add_face(&Face::new([
        [0.0, 0.0, 0.0],
        [150.0, 0.0, 0.0],
        [150.0, 60.0, 0.0],
        [0.0, 60.0, 0.0],
    ]))?;
    car.add_instance(&wheel, &Placement::at([20.0, 0.0, 0.0]))?;
    car.add_instance(&wheel, &Placement::at([130.0, 0.0, 0.0]).named("RearWheel"))?;
    car.add_group_instance(&engine, &Placement::at([50.0, 10.0, 10.0]))?;
    car.add_group_instance(&engine, &Placement::at([90.0, 10.0, 10.0]).named(""))?;
    car.add_text("Car", [0.0; 3], DEFAULT_LEADER)?;
    car.add_construction_point([75.0, 30.0, 0.0])?;
    car.add_arc(&Arc::new([0.0; 3], [0.0, 0.0, 1.0], 5.0, 0.0, PI))?;
    car.add_polyline(&Polyline::new([
        [0.0, 0.0, 5.0],
        [5.0, 0.0, 5.0],
        [5.0, 5.0, 5.0],
    ]))?;
    let car = car.finish()?;
    b.add_instance(&car, &Placement::default())?;
    b.add_instance(
        &car,
        &Placement::at([0.0, 200.0, 0.0]).rotated([0.0, 0.0, 1.0], PI),
    )?;
    b.add_text("Root", [10.0, 10.0, 10.0], DEFAULT_LEADER)
}

fn circles(b: &mut SkpBuilder, _: &Ctx) -> R {
    b.add_circle(&Circle::new([50.0, 50.0, 0.0], [0.0, 0.0, 1.0], 40.0))?;
    b.add_circle(&Circle::new([0.0, 0.0, 50.0], [1.0, 1.0, 1.0], 10.0).segments(8))?;
    b.add_circle(&Circle::new([0.0, 0.0, -50.0], [0.0, 0.0, -1.0], 5.0).segments(3))
}

fn arcs(b: &mut SkpBuilder, _: &Ctx) -> R {
    b.add_arc(&Arc::new([0.0; 3], [0.0, 0.0, 1.0], 40.0, 0.0, PI / 2.0))?;
    b.add_arc(&Arc {
        edges: EdgeFlags {
            soft: true,
            ..EdgeFlags::default()
        },
        ..Arc::new([100.0, 0.0, 0.0], [0.0, 1.0, 0.0], 25.0, PI, -PI / 3.0).segments(7)
    })
}

fn polylines(b: &mut SkpBuilder, _: &Ctx) -> R {
    b.add_polyline(&Polyline::new([
        [0.0, 0.0, 0.0],
        [10.0, 10.0, 0.0],
        [20.0, 0.0, 0.0],
        [30.0, 10.0, 0.0],
    ]))?;
    b.add_polyline(&Polyline {
        edges: EdgeFlags {
            hidden: true,
            ..EdgeFlags::default()
        },
        ..Polyline::new([[0.0, 0.0, 10.0], [10.0, 0.0, 10.0], [10.0, 10.0, 10.0]]).closed()
    })?;
    b.add_face(&tri([0.0, 0.0, 0.0], [10.0, 10.0, 0.0], [0.0, 10.0, 0.0]))
}

fn guides(b: &mut SkpBuilder, _: &Ctx) -> R {
    b.add_construction_line(GuideLine::Segment {
        start: [0.0; 3],
        end: [100.0, 50.0, 25.0],
    })?;
    b.add_construction_line(GuideLine::Infinite {
        point: [10.0, 10.0, 10.0],
        direction: [0.0, 0.0, 2.0],
    })?;
    b.add_construction_point([5.0, 5.0, 5.0])?;
    b.add_section_plane([0.0, 0.0, 30.0], [0.0, 0.0, 1.0])?;
    b.add_section_plane([10.0, 0.0, 0.0], [1.0, 2.0, 0.0])
}

fn annotations(b: &mut SkpBuilder, _: &Ctx) -> R {
    b.add_face(&Face::new(QUAD))?;
    b.add_text("Hello", [0.0; 3], DEFAULT_LEADER)?;
    b.add_dimension([0.0; 3], [100.0, 0.0, 0.0], -12.5)?;
    b.add_text(
        "\u{dc}n\u{ef}code \u{2713}",
        [100.0, 50.0, 0.0],
        [5.0, -5.0, 10.0],
    )?;
    b.add_dimension([0.0; 3], [0.0, 50.0, 0.0], 10.0)
}

fn triangulate(b: &mut SkpBuilder, _: &Ctx) -> R {
    b.add_face(
        &Face::new([
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 10.0, 3.0],
            [0.0, 10.0, 0.0],
        ])
        .auto_triangulate(),
    )?;
    b.add_face(&square(20.0, 0.0, 10.0).auto_triangulate())?;
    b.add_face(&Face {
        attributes: vec![AttributeDict::new("attributes").with("tri", V::Bool(true))],
        ..Face::new([
            [40.0, 0.0, 0.0],
            [50.0, 0.0, 0.0],
            [55.0, 5.0, 2.0],
            [50.0, 10.0, 0.0],
            [40.0, 10.0, 1.0],
        ])
        .auto_triangulate()
    })
}

fn attributes(b: &mut SkpBuilder, _: &Ctx) -> R {
    let dicts = [
        AttributeDict::new("dynamic_attributes")
            .with("a", V::Int(1))
            .with("b", V::Double(2.5)),
        AttributeDict::new("other").with("s", V::String("x".into())),
    ];
    let mut d = b.add_component_definition_with("Attr", None, &dicts, false, false)?;
    let face_attrs = AttributeDict::new("attributes")
        .with("name", V::String("front".into()))
        .with("n", V::Null)
        .with("flag", V::Bool(true))
        .with("flag2", V::Bool(false))
        .with("p", V::Point3d([1.0, 2.0, 3.0]))
        .with("v", V::Vector3d([0.0, 0.0, 1.0]))
        .with("len", V::Length(12.0))
        .with("t", V::Timestamp(1_700_000_000))
        .with(
            "arr",
            V::Array(vec![
                V::Int(1),
                V::String("two".into()),
                V::Array(vec![V::Double(3.0), V::Null]),
            ]),
        )
        .with("neg", V::Int(-5));
    d.add_face(&Face {
        attributes: vec![face_attrs],
        ..Face::new(QUAD)
    })?;
    let d = d.finish()?;
    b.add_instance(
        &d,
        &Placement {
            attributes: vec![AttributeDict::new("attributes").with("price", V::Double(9.99))],
            ..Placement::default()
        },
    )?;
    b.add_instance(
        &d,
        &Placement {
            attributes: vec![AttributeDict::new("custom").with("x", V::Int(1))],
            ..Placement::default()
        },
    )
}

fn textures(b: &mut SkpBuilder, ctx: &Ctx) -> R {
    let brick = b.add_texture_material(&TextureMaterial {
        width: Some(8.0),
        height: Some(16.0),
        ..TextureMaterial::new("Brick", &ctx.png_bytes, &ctx.png)
    })?;
    let wood = b.add_texture_material(&TextureMaterial {
        opacity: Some(0.5),
        ..TextureMaterial::new("Wood", &ctx.jpg_bytes, &ctx.jpg)
    })?;
    b.add_image(
        &ctx.png_bytes,
        &ctx.png,
        48.0,
        36.0,
        &Placement::at([0.0, 0.0, 40.0]).rotated([1.0, 0.0, 0.0], PI / 2.0),
    )?;
    b.add_face(&Face {
        front_uv: pins([
            ([0.0, 0.0, 0.0], [0.0, 0.0]),
            ([50.0, 0.0, 0.0], [1.0, 0.0]),
            ([0.0, 50.0, 0.0], [0.0, 1.0]),
        ]),
        ..Face::new(QUAD).material(brick)
    })?;
    b.add_face(&Face {
        back_uv: pins([
            ([0.0, 0.0, 0.0], [0.0, 0.0]),
            ([0.0, 0.0, 30.0], [2.0, 0.0]),
            ([0.0, 50.0, 0.0], [0.0, 3.0]),
        ]),
        ..Face::new([
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 30.0],
            [0.0, 50.0, 30.0],
            [0.0, 50.0, 0.0],
        ])
        .material(wood)
        .back_material(brick)
    })?;
    b.add_circle(&Circle {
        front_uv: pins([
            ([200.0, 0.0, 0.0], [0.0, 0.0]),
            ([210.0, 0.0, 0.0], [1.0, 0.0]),
            ([200.0, 10.0, 0.0], [0.5, 1.0]),
        ]),
        ..Circle::new([200.0, 0.0, 0.0], [0.0, 0.0, 1.0], 20.0).material(brick)
    })
}

fn large_grid(b: &mut SkpBuilder, _: &Ctx) -> R {
    for i in 0..70 {
        for j in 0..70 {
            b.add_face(&square(f64::from(i), f64::from(j), 1.0))?;
        }
    }
    Ok(())
}

type Model = fn(&mut SkpBuilder, &Ctx) -> R;

const MODELS: &[(&str, Model)] = &[
    ("single_face", single_face),
    ("box", box_),
    ("holes", holes),
    ("materials_layers", materials_layers),
    ("components", components),
    ("groups", groups),
    ("groups_only", groups_only),
    ("nested", nested),
    ("circles", circles),
    ("arcs", arcs),
    ("polylines", polylines),
    ("guides", guides),
    ("annotations", annotations),
    ("triangulate", triangulate),
    ("attributes", attributes),
    ("textures", textures),
    ("large_grid", large_grid),
];

fn build(model: Model, ctx: &Ctx) -> Vec<u8> {
    let mut b = SkpBuilder::deterministic(SEED, TIMESTAMP);
    model(&mut b, ctx).unwrap();
    b.to_bytes().unwrap()
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn oracle_available() -> bool {
    Command::new("python3")
        .args(["-I", "-c", "import openskp"])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Runs `write_models.py` once per test binary; `None` without the oracle.
fn python_models() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        if !oracle_available() {
            eprintln!("SKIPPED: python3 with openskp not available (see tools/README.md)");
            return None;
        }
        let dir = repo().join("target/write-oracle/python");
        let out = Command::new("python3")
            .arg("-I")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/write_models.py"))
            .arg(&dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "write_models.py failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        Some(dir)
    })
    .as_deref()
}

#[test]
fn byte_identical_to_openskp() {
    let Some(dir) = python_models() else { return };
    let ctx = Ctx::from_dir(dir);
    let mut failed = Vec::new();
    for &(name, model) in MODELS {
        let ours = build(model, &ctx);
        let theirs = std::fs::read(dir.join(format!("{name}.skp"))).unwrap();
        if ours != theirs {
            let at = ours.iter().zip(&theirs).position(|(a, b)| a != b);
            eprintln!(
                "{name}: differs (ours {} bytes, oracle {} bytes, first difference at {at:?})",
                ours.len(),
                theirs.len()
            );
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "differs from openskp.create: {failed:?}");
}

/// Entity counts of a dumped model (`mt_skp::dump` or `tools/oracle_dump.py`).
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
struct Counts {
    faces: usize,
    edges: usize,
    vertices: usize,
    instances: usize,
    definitions: usize,
    def_faces: usize,
    materials: usize,
    layers: usize,
    texts: usize,
    dimensions: usize,
    guides: usize,
    section_planes: usize,
}

fn len(v: &Value) -> usize {
    match v {
        Value::Array(a) => a.len(),
        Value::Object(o) => o.len(),
        _ => 0,
    }
}

fn counts(j: &Value) -> Counts {
    let root = &j["root"];
    let defs = j["definitions"].as_object().cloned().unwrap_or_default();
    Counts {
        faces: len(&root["faces"]),
        edges: len(&root["edges"]),
        vertices: len(&root["vertices"]),
        instances: len(&root["instances"]),
        definitions: defs.len(),
        def_faces: defs.values().map(|d| len(&d["faces"])).sum(),
        materials: len(&j["materials"]),
        layers: len(&j["layers"]),
        texts: len(&root["texts"]),
        dimensions: len(&root["dimensions"]),
        guides: len(&root["construction_lines"]) + len(&root["construction_points"]),
        section_planes: len(&root["section_planes"]),
    }
}

/// `(model, faces, edges, vertices, instances, definitions, materials, layers)`
/// at the root; layers include Layer0.
type Expected = (
    &'static str,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
);

const EXPECTED: &[Expected] = &[
    ("single_face", 1, 4, 4, 0, 0, 0, 1),
    ("box", 6, 12, 8, 0, 0, 0, 1),
    ("holes", 2, 14, 13, 0, 0, 0, 1),
    ("materials_layers", 3, 10, 9, 0, 0, 2, 4),
    ("components", 0, 0, 0, 3, 1, 1, 2),
    ("groups", 1, 4, 4, 2, 2, 0, 1),
    ("groups_only", 0, 0, 0, 1, 1, 0, 1),
    ("nested", 0, 0, 0, 2, 3, 0, 1),
    ("circles", 3, 35, 35, 0, 0, 0, 1),
    ("arcs", 0, 31, 33, 0, 0, 0, 1),
    ("polylines", 1, 8, 8, 0, 0, 0, 1),
    ("guides", 0, 0, 0, 0, 0, 0, 1),
    ("annotations", 1, 4, 4, 0, 0, 0, 1),
    ("triangulate", 6, 16, 13, 0, 0, 0, 1),
    ("attributes", 0, 0, 0, 2, 1, 0, 1),
    ("textures", 3, 31, 30, 1, 1, 3, 1),
    ("large_grid", 4900, 9940, 5041, 0, 0, 0, 1),
];

#[test]
fn round_trip_ours() {
    let ctx = Ctx::local();
    for &(name, model) in MODELS {
        let bytes = build(model, &ctx);
        let parsed = mt_skp::parse(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(parsed.version, "{17.0.1}", "{name}");
        let c = counts(&mt_skp::dump::to_json(&parsed));
        let &(_, faces, edges, vertices, instances, definitions, materials, layers) = EXPECTED
            .iter()
            .find(|e| e.0 == name)
            .unwrap_or_else(|| panic!("no expectation for {name}"));
        assert_eq!(
            (
                c.faces,
                c.edges,
                c.vertices,
                c.instances,
                c.definitions,
                c.materials,
                c.layers
            ),
            (
                faces,
                edges,
                vertices,
                instances,
                definitions,
                materials,
                layers
            ),
            "{name}: (faces, edges, vertices, instances, definitions, materials, layers)"
        );
    }
}

#[test]
fn round_trip_details_ours() {
    let ctx = Ctx::local();
    let parse = |m: Model| mt_skp::parse(&build(m, &ctx)).unwrap();

    let m = parse(holes);
    let wall = m.root.faces.values().find(|f| f.loops.len() == 3).unwrap();
    assert_eq!(
        wall.loops.iter().map(Vec::len).collect::<Vec<_>>(),
        [4, 4, 3]
    );

    let m = parse(materials_layers);
    let names: Vec<_> = m
        .layers
        .iter()
        .map(|l| (l.name.as_str(), l.hidden))
        .collect();
    assert_eq!(
        names,
        [
            ("Layer0", false),
            ("Roof", false),
            ("Hidden", true),
            ("Plain", false)
        ]
    );
    assert_eq!(m.layers[1].color, [200, 10, 10]);
    let glass = m.materials.iter().find(|x| x.name == "Glass").unwrap();
    assert!((glass.transparency - 0.4).abs() < 1e-12);

    let m = parse(components);
    let names: Vec<_> = m.root.instances.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["Chair", "Rotated", ""]);
    assert!(m.root.instances[2].hidden);
    assert_eq!(m.root.instances[0].matrix[9..12], [100.0, 0.0, 0.0]);

    let m = parse(nested);
    let car = m.definitions.values().find(|d| d.name == "Car").unwrap();
    assert!(car.always_faces_camera && car.shadows_face_sun);
    assert_eq!(car.instances.len(), 4);
    assert_eq!(car.texts.len(), 1);
    assert_eq!(car.texts[0].point, Some([0.0; 3]));
    assert_eq!(car.texts[0].label_point, Some(DEFAULT_LEADER));
    assert_eq!(car.construction_points.len(), 1);
    assert_eq!(m.root.texts.len(), 1);

    let m = parse(guides);
    assert_eq!(m.root.construction_lines.len(), 2);
    assert_eq!(m.root.construction_points.len(), 1);
    assert_eq!(m.root.section_planes.len(), 2);
    assert_eq!(m.root.section_planes[0].plane, [0.0, 0.0, 1.0, -30.0]);

    let m = parse(annotations);
    assert_eq!(m.root.texts.len(), 2);
    assert_eq!(m.root.texts[1].text, "\u{dc}n\u{ef}code \u{2713}");
    let t = &m.root.texts[0];
    assert_eq!(t.point, Some([0.0; 3]));
    assert_eq!(t.label_point, Some(DEFAULT_LEADER));
    assert_eq!(t.screen_position, None);
    let t = &m.root.texts[1];
    assert_eq!(t.point, Some([100.0, 50.0, 0.0]));
    assert_eq!(t.label_point, Some([105.0, 45.0, 10.0]));
    // Legacy files list root dimensions in the root and, with endpoints, at model level.
    assert_eq!(m.root.dimensions, m.dimensions);
    let d: Vec<_> = m
        .dimensions
        .iter()
        .map(|d| {
            (
                d.a.unwrap(),
                d.b.unwrap(),
                d.offset,
                d.offset_direction.unwrap(),
            )
        })
        .collect();
    // The writer's plane normal is +Z, so the offset goes along Z x (b - a).
    assert_eq!(
        d,
        [
            ([0.0; 3], [100.0, 0.0, 0.0], -12.5, [0.0, 1.0, 0.0]),
            ([0.0; 3], [0.0, 50.0, 0.0], 10.0, [-1.0, 0.0, 0.0]),
        ]
    );

    let m = parse(textures);
    let brick = m.materials.iter().find(|x| x.name == "Brick").unwrap();
    let tex = brick.texture.as_ref().unwrap();
    assert_eq!((tex.width, tex.height), (8.0, 16.0));
    assert!(m.definitions.values().any(|d| d.is_image));
}

#[test]
fn round_trip_openskp() {
    if python_models().is_none() {
        return;
    }
    let dir = repo().join("target/write-oracle/rust");
    std::fs::create_dir_all(&dir).unwrap();
    let ctx = Ctx::local();
    for &(name, model) in MODELS {
        let file = dir.join(format!("{name}.skp"));
        let bytes = build(model, &ctx);
        std::fs::write(&file, &bytes).unwrap();
        let out = Command::new("python3")
            .arg("-I")
            .arg(repo().join("tools/oracle_dump.py"))
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "openskp failed on {name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let theirs = counts(&serde_json::from_slice(&out.stdout).unwrap());
        let ours = counts(&mt_skp::dump::to_json(&mt_skp::parse(&bytes).unwrap()));
        assert_eq!(ours, theirs, "{name}: our reader vs openskp");
    }
}

#[test]
fn guid_and_clock_are_injectable() {
    let run = |seed| {
        let mut b = SkpBuilder::deterministic(seed, 7);
        components(&mut b, &Ctx::local()).unwrap();
        b.to_bytes().unwrap()
    };
    assert_eq!(run(1), run(1));
    assert_ne!(run(1), run(2));
    // Random GUIDs differ between builders.
    let random = || {
        let mut b = SkpBuilder::new();
        components(&mut b, &Ctx::local()).unwrap();
        b.to_bytes().unwrap()
    };
    assert_ne!(random(), random());
}

#[test]
fn ordering_and_validation_errors() {
    let mut b = SkpBuilder::new();
    b.add_face(&Face::new(QUAD)).unwrap();
    assert!(matches!(
        b.add_material("Late", [0; 4]),
        Err(WriteError::Order(_))
    ));
    assert!(matches!(
        b.add_layer("Late", None, true),
        Err(WriteError::Order(_))
    ));
    assert!(b.add_component_definition("Late", None).is_err());

    let mut b = SkpBuilder::new();
    let warped = Face::new([
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        [10.0, 10.0, 3.0],
        [0.0, 10.0, 0.0],
    ]);
    assert!(matches!(b.add_face(&warped), Err(WriteError::Invalid(_))));
    assert!(b.add_face(&Face::new([[0.0; 3], [1.0, 0.0, 0.0]])).is_err());
    assert!(
        b.add_face(&Face::new(QUAD).hole([[1.0, 1.0, 1.0], [2.0, 1.0, 1.0], [2.0, 2.0, 1.0]]))
            .is_err()
    );
    assert!(b.to_bytes().is_err(), "nothing was added");

    // Handles from another builder are rejected.
    let mut other = SkpBuilder::new();
    let foreign = other.add_material("Red", [255, 0, 0, 255]).unwrap();
    assert!(b.add_face(&Face::new(QUAD).material(foreign)).is_err());

    // An empty definition cannot be finished, and blocks the builder.
    let mut b = SkpBuilder::new();
    assert!(
        b.add_component_definition("Empty", None)
            .unwrap()
            .finish()
            .is_err()
    );
    assert!(matches!(
        b.add_face(&Face::new(QUAD)),
        Err(WriteError::Order(_))
    ));
}

#[test]
fn save_writes_the_file() {
    let dir = repo().join("target/write-oracle");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("save_test.skp");
    let mut b = SkpBuilder::deterministic(SEED, TIMESTAMP);
    single_face(&mut b, &Ctx::local()).unwrap();
    b.save(&path).unwrap();
    assert_eq!(mt_skp::read(&path).unwrap().root.faces.len(), 1);
}

// ── Model data beyond OpenSKP's writer API ───────────────────────────────────────

/// Writes `bytes` under `target/write-oracle/extra` and runs a Python oracle script on
/// it; `None` without the oracle.
fn python_json(script: &Path, name: &str, bytes: &[u8]) -> Option<Value> {
    if !oracle_available() {
        eprintln!("SKIPPED: python3 with openskp not available (see tools/README.md)");
        return None;
    }
    let dir = repo().join("target/write-oracle/extra");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("{name}.skp"));
    std::fs::write(&file, bytes).unwrap();
    let out = Command::new("python3")
        .arg("-I")
        .arg(script)
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{} failed on {name}: {}",
        script.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    Some(serde_json::from_slice(&out.stdout).unwrap())
}

fn oracle_dump(name: &str, bytes: &[u8]) -> Option<Value> {
    python_json(&repo().join("tools/oracle_dump.py"), name, bytes)
}

fn oracle_arcs(name: &str, bytes: &[u8]) -> Option<Value> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/read_arcs.py");
    python_json(&script, name, bytes)
}

#[test]
fn saved_view_round_trip() {
    use mt_skp::ViewCamera;
    let views = [
        ViewCamera {
            eye: [325.5, -366.25, 156.0],
            target: [291.75, -228.5, 129.5],
            up: [0.0, 0.0, 1.0],
            fov: 35.0,
            parallel: false,
            ortho_height: 120.0,
        },
        ViewCamera {
            eye: [0.0, -500.0, 12.0],
            target: [0.0, 0.0, 12.0],
            up: [0.0, 0.0, 1.0],
            fov: 30.0,
            parallel: true,
            ortho_height: 48.5,
        },
        // Top view: up along the view direction falls back to +Y.
        ViewCamera {
            eye: [10.0, 20.0, 400.0],
            target: [10.0, 20.0, 0.0],
            up: [0.0, 0.0, 1.0],
            fov: 35.0,
            parallel: true,
            ortho_height: 300.0,
        },
    ];
    for (i, view) in views.iter().enumerate() {
        let mut b = SkpBuilder::deterministic(SEED, TIMESTAMP);
        b.set_camera(view).unwrap();
        b.add_face(&Face::new(QUAD)).unwrap();
        let bytes = b.to_bytes().unwrap();
        let c = mt_skp::parse(&bytes).unwrap().camera.unwrap();
        assert_eq!((c.eye, c.target), (view.eye, view.target), "view {i}");
        assert_eq!(
            (c.fov, c.parallel, c.ortho_height),
            (view.fov, view.parallel, view.ortho_height)
        );
        let d = [0, 1, 2].map(|k| view.target[k] - view.eye[k]);
        let dot = d[0] * c.up[0] + d[1] * c.up[1] + d[2] * c.up[2];
        let len = (c.up[0] * c.up[0] + c.up[1] * c.up[1] + c.up[2] * c.up[2]).sqrt();
        assert!(
            dot.abs() < 1e-9 && (len - 1.0).abs() < 1e-12,
            "view {i}: up {:?}",
            c.up
        );
        if i == 0 {
            // SketchUp's up for this view: Z made perpendicular to the direction.
            assert!(c.up[2] > 0.9);
        }
        if i == 2 {
            assert_eq!(c.up, [0.0, 1.0, 0.0]);
        }
        // Nothing else changes: OpenSKP still reads the same model.
        if let Some(theirs) = oracle_dump(&format!("view{i}"), &bytes) {
            let ours = counts(&mt_skp::dump::to_json(&mt_skp::parse(&bytes).unwrap()));
            assert_eq!(ours, counts(&theirs));
        }
    }
    // Without a view the file keeps OpenSKP's Iso camera; an empty document takes one too.
    let mut b = SkpBuilder::deterministic(SEED, TIMESTAMP);
    b.add_face(&Face::new(QUAD)).unwrap();
    let iso = mt_skp::parse(&b.to_bytes().unwrap())
        .unwrap()
        .camera
        .unwrap();
    assert_eq!((iso.eye, iso.parallel), ([100.0, -100.0, 100.0], true));
    let mut b = SkpBuilder::new();
    b.set_camera(&views[1]).unwrap();
    let c = mt_skp::parse(&b.to_empty_document().unwrap())
        .unwrap()
        .camera
        .unwrap();
    assert_eq!((c.eye, c.ortho_height), (views[1].eye, 48.5));
    let degenerate = ViewCamera {
        target: views[1].eye,
        ..views[1].clone()
    };
    assert!(b.set_camera(&degenerate).is_err());
}

#[test]
fn arc_curves_round_trip() {
    use mt_skp::ArcCurve;
    let mut b = SkpBuilder::deterministic(SEED, TIMESTAMP);
    let tag = b.add_layer("Curves", None, true).unwrap();
    // OpenSKP's calls: a tilted circle and an arc about +Y.
    b.add_circle(&Circle::new([0.0, 0.0, 50.0], [1.0, 1.0, 1.0], 10.0).segments(8))
        .unwrap();
    b.add_arc(&Arc::new([100.0, 0.0, 0.0], [0.0, 1.0, 0.0], 25.0, 0.0, PI / 3.0).segments(7))
        .unwrap();
    // A polyline through given points carrying an arc frame (how the kernel saves its
    // arcs), on a tag.
    let frame = ArcCurve {
        center: [0.0, 0.0, 0.0],
        normal: [0.0, 0.0, 1.0],
        x_axis: [0.0, 20.0, 0.0],
        y_axis: [-20.0, 0.0, 0.0],
        start_angle: 0.0,
        end_angle: PI,
        segments: 4,
    };
    let pts: Vec<[f64; 3]> = (0..=4)
        .map(|i| {
            let t = PI * f64::from(i) / 4.0;
            [-20.0 * t.sin(), 20.0 * t.cos(), 0.0]
        })
        .collect();
    b.add_polyline(&Polyline {
        arc: Some(frame),
        edges: EdgeFlags {
            layer: Some(tag),
            ..EdgeFlags::default()
        },
        ..Polyline::new(pts.clone())
    })
    .unwrap();
    let bytes = b.to_bytes().unwrap();
    let m = mt_skp::parse(&bytes).unwrap();
    let arcs = &m.root.arc_curves;
    assert_eq!(arcs.len(), 3);
    // Every arc's y axis is normal x x axis now, also off +Z.
    for a in arcs.values() {
        let (n, x, y) = (a.normal, a.x_axis, a.y_axis);
        let c = [
            n[1] * x[2] - n[2] * x[1],
            n[2] * x[0] - n[0] * x[2],
            n[0] * x[1] - n[1] * x[0],
        ];
        assert!((0..3).all(|i| (c[i] - y[i]).abs() < 1e-12), "{a:?}");
    }
    let (&id, mine) = arcs.iter().find(|(_, a)| a.end_angle == PI).unwrap();
    assert_eq!(*mine, frame);
    // The polyline's edges keep their own points and tag and point at the arc.
    let edges: Vec<_> = m
        .root
        .edges
        .values()
        .filter(|e| e.curve_id == Some(id))
        .collect();
    assert_eq!(edges.len(), 4);
    assert!(edges.iter().all(|e| e.layer == "Curves"));
    let on_arc = m
        .root
        .vertices
        .values()
        .filter(|v| pts.contains(&[v.x, v.y, v.z]))
        .count();
    assert_eq!(on_arc, 5);
    // More than 255 segments do not fit the record.
    let many: Vec<[f64; 3]> = (0..300).map(|i| [f64::from(i), 0.0, 0.0]).collect();
    let mut b = SkpBuilder::new();
    let too_many = Polyline {
        arc: Some(frame),
        ..Polyline::new(many)
    };
    assert!(b.add_polyline(&too_many).is_err());

    // OpenSKP decodes the same frames under the same curve ids.
    let Some(theirs) = oracle_arcs("arcs", &bytes) else {
        return;
    };
    let theirs = theirs.as_object().unwrap();
    assert_eq!(theirs.len(), arcs.len());
    for (id, a) in arcs {
        let t = &theirs[&id.to_string()];
        let v3 = |k: &str| -> [f64; 3] {
            let a = t[k].as_array().unwrap();
            [0, 1, 2].map(|i| a[i].as_f64().unwrap())
        };
        assert_eq!(v3("center"), a.center);
        assert_eq!(v3("normal"), a.normal);
        assert_eq!(v3("x_axis"), a.x_axis);
        assert_eq!(v3("y_axis"), a.y_axis);
        assert_eq!(t["start_angle"].as_f64(), Some(a.start_angle));
        assert_eq!(t["end_angle"].as_f64(), Some(a.end_angle));
    }
}

#[test]
fn edge_tags_round_trip() {
    let mut b = SkpBuilder::deterministic(SEED, TIMESTAMP);
    let walls = b.add_layer("Walls", None, true).unwrap();
    let flags = EdgeFlags {
        layer: Some(walls),
        ..EdgeFlags::default()
    };
    let mut d = b.add_component_definition("Frame", None).unwrap();
    d.add_edge([0.0; 3], [10.0, 0.0, 0.0], flags).unwrap();
    let frame = d.finish().unwrap();
    b.add_edge([0.0; 3], [0.0, 10.0, 0.0], flags).unwrap();
    b.add_edge([0.0; 3], [0.0, 0.0, 10.0], EdgeFlags::default())
        .unwrap();
    b.add_polyline(&Polyline {
        edges: flags,
        ..Polyline::new([[50.0, 0.0, 0.0], [60.0, 0.0, 0.0], [60.0, 10.0, 0.0]])
    })
    .unwrap();
    // A face on the tag, to read OpenSKP's id for it.
    let face = Face::new([[0.0, 0.0, 50.0], [9.0, 0.0, 50.0], [0.0, 9.0, 50.0]]).layer(walls);
    b.add_face(&face).unwrap();
    b.add_instance(&frame, &Placement::at([0.0, 0.0, 100.0]))
        .unwrap();
    let bytes = b.to_bytes().unwrap();
    let m = mt_skp::parse(&bytes).unwrap();
    let tagged = |d: &mt_skp::Definition| {
        let mut v: Vec<_> = d.edges.values().map(|e| e.layer.clone()).collect();
        v.sort_unstable();
        v
    };
    // The face's three new edges carry no tag (as before).
    assert_eq!(tagged(&m.root), ["", "", "", "", "Walls", "Walls", "Walls"]);
    assert_eq!(tagged(m.definitions.values().next().unwrap()), ["Walls"]);
    // A foreign layer handle is refused.
    let other = SkpBuilder::new().add_layer("X", None, true).unwrap();
    let mut b2 = SkpBuilder::new();
    let bad = EdgeFlags {
        layer: Some(other),
        ..EdgeFlags::default()
    };
    assert!(b2.add_edge([0.0; 3], [1.0, 0.0, 0.0], bad).is_err());

    // OpenSKP gives the edges the same layer id as the face on that layer.
    let Some(j) = oracle_dump("edge_tags", &bytes) else {
        return;
    };
    let face_layer = j["root"]["faces"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()["layer"]
        .as_i64()
        .unwrap();
    assert_ne!(face_layer, 0);
    let mut layers: Vec<_> = j["root"]["edges"]
        .as_object()
        .unwrap()
        .values()
        .map(|e| e["layer"].as_i64())
        .collect();
    layers.sort_unstable();
    let w = Some(face_layer);
    assert_eq!(layers, [None, None, None, None, w, w, w]);
    let def = j["definitions"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    let def_layers: Vec<_> = def["edges"]
        .as_object()
        .unwrap()
        .values()
        .map(|e| e["layer"].as_i64())
        .collect();
    assert_eq!(def_layers, [w]);
}

#[test]
fn guides_and_section_planes_in_definitions() {
    let mut b = SkpBuilder::deterministic(SEED, TIMESTAMP);
    let mut d = b.add_component_definition("Cabinet", None).unwrap();
    d.add_face(&Face::new(QUAD)).unwrap();
    d.add_construction_line(GuideLine::Infinite {
        point: [0.0, 25.0, 0.0],
        direction: [1.0, 0.0, 0.0],
    })
    .unwrap();
    d.add_construction_line(GuideLine::Segment {
        start: [0.0, 0.0, 0.0],
        end: [0.0, 0.0, 30.0],
    })
    .unwrap();
    d.add_construction_point([5.0, 5.0, 5.0]).unwrap();
    d.add_section_plane([50.0, 0.0, 0.0], [-2.0, 0.0, 0.0])
        .unwrap();
    let cab = d.finish().unwrap();
    b.add_instance(&cab, &Placement::at([0.0; 3])).unwrap();
    b.add_section_plane([0.0, 0.0, 10.0], [0.0, 0.0, 1.0])
        .unwrap();
    let bytes = b.to_bytes().unwrap();
    let m = mt_skp::parse(&bytes).unwrap();
    let def = m.definitions.values().next().unwrap();
    assert_eq!(def.construction_lines.len(), 2);
    assert_eq!(def.construction_points.len(), 1);
    assert_eq!(def.section_planes.len(), 1);
    assert_eq!(def.section_planes[0].plane, [-1.0, 0.0, 0.0, 50.0]);
    assert_eq!(m.root.section_planes.len(), 1);
    let Some(j) = oracle_dump("def_guides", &bytes) else {
        return;
    };
    let ours = mt_skp::dump::to_json(&m);
    let key = def.id.to_string();
    for k in [
        "construction_lines",
        "construction_points",
        "section_planes",
    ] {
        assert_eq!(
            ours["definitions"][&key][k], j["definitions"][&key][k],
            "{k}"
        );
    }
    assert_eq!(counts(&ours), counts(&j));
}
