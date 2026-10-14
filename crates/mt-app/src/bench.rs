//! Large-model benchmark (spec/PERFORMANCE.md): times what the app does with a big
//! model, stage by stage, on synthetic models and on real files. Ignored by default;
//! run in release mode:
//!
//! ```text
//! cargo test --release -p mt-app bench_large_models -- --ignored --nocapture
//! ```
//!
//! `MESHTARD_BENCH=grid,instances` runs only the models whose names contain one of
//! the words; `MESHTARD_BENCH_FILES` is a `:`-separated list of extra `.skp`, glTF or
//! IFC files. Frame times need a wgpu adapter (lavapipe works) and are skipped
//! without one. Each model prints a checksum of its drawn scene, so an optimization
//! can be checked to draw exactly the same thing.

use std::path::{Path, PathBuf};
use std::time::Instant;

use egui_wgpu::wgpu;
use glam::{DMat4, DVec2, DVec3};
use mt_kernel::bvh::Bvh;
use mt_kernel::{CoEdge, Definition, Entities, FaceAttrs, Instance, Loop, Model, VertexId, import};

use crate::document::{Document, Owner, Scene};
use crate::live_view;
use crate::selection::{Mode, Selection};

const VIEWPORT: [u32; 2] = [1280, 800];

/// A face from vertex ids (edges reused when present), normal by Newell's method.
fn polygon(ents: &mut Entities, vs: &[VertexId]) -> mt_kernel::FaceId {
    let mut coedges = Vec::new();
    let mut normal = DVec3::ZERO;
    for k in 0..vs.len() {
        let (a, b) = (vs[k], vs[(k + 1) % vs.len()]);
        let (pa, pb) = (ents.pos(a), ents.pos(b));
        normal += DVec3::new(
            (pa.y - pb.y) * (pa.z + pb.z),
            (pa.z - pb.z) * (pa.x + pb.x),
            (pa.x - pb.x) * (pa.y + pb.y),
        );
        let (edge, reversed) = match ents.edge_between(a, b) {
            Some(e) => (e, ents.edges[e].start != a),
            None => (ents.add_raw_edge(a, b, Default::default()), false),
        };
        coedges.push(CoEdge { edge, reversed });
    }
    ents.add_raw_face(
        Loop { coedges },
        Vec::new(),
        normal.normalize(),
        FaceAttrs::default(),
    )
}

/// A box of size `s` at `o`.
pub(crate) fn add_box(ents: &mut Entities, o: DVec3, s: DVec3) {
    let p = |x: f64, y: f64, z: f64| o + s * DVec3::new(x, y, z);
    let v: Vec<VertexId> = [
        p(0., 0., 0.),
        p(1., 0., 0.),
        p(1., 1., 0.),
        p(0., 1., 0.),
        p(0., 0., 1.),
        p(1., 0., 1.),
        p(1., 1., 1.),
        p(0., 1., 1.),
    ]
    .into_iter()
    .map(|q| ents.add_vertex(q))
    .collect();
    for f in [
        [0, 3, 2, 1],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
    ] {
        polygon(ents, &f.map(|i| v[i]));
    }
}

pub(crate) fn instance(definition: mt_kernel::DefinitionId, transform: DMat4) -> Instance {
    Instance {
        definition,
        transform,
        name: String::new(),
        hidden: false,
        locked: false,
        layer: None,
        material: None,
        attrs: Default::default(),
    }
}

pub(crate) fn definition(name: &str, entities: Entities, is_group: bool) -> Definition {
    Definition {
        name: name.into(),
        entities,
        is_group,
        ..Default::default()
    }
}

/// An `n` x `n` grid of quads at the root, gently waved so it is not one plane:
/// `2 n (n + 1)` edges.
pub(crate) fn grid(n: usize) -> Model {
    let mut model = Model::default();
    let ents = &mut model.root;
    let mut v = Vec::with_capacity((n + 1) * (n + 1));
    for j in 0..=n {
        for i in 0..=n {
            let z = ((i as f64) * 0.3).sin() * ((j as f64) * 0.2).cos() * 2.0;
            v.push(ents.add_vertex(DVec3::new(i as f64 * 10.0, j as f64 * 10.0, z)));
        }
    }
    let at = |i: usize, j: usize| v[j * (n + 1) + i];
    for j in 0..n {
        for i in 0..n {
            // Waved quads are not planar: two triangles each.
            polygon(ents, &[at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
            polygon(ents, &[at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
        }
    }
    model
}

/// `count` instances of one box component in a square layout.
pub(crate) fn instances(count: usize) -> Model {
    let mut model = Model::default();
    let mut ents = Entities::default();
    add_box(&mut ents, DVec3::ZERO, DVec3::splat(20.0));
    let d = model.definitions.insert(definition("Box", ents, false));
    let side = (count as f64).sqrt().ceil() as usize;
    for k in 0..count {
        let t = DVec3::new((k % side) as f64 * 40.0, (k / side) as f64 * 40.0, 0.0);
        model
            .root
            .instances
            .push(instance(d, DMat4::from_translation(t)));
    }
    model
}

/// `count` instances of one window component in a square layout: a frame of four
/// boxes around a pane in a translucent material (glazing, railings, curtain walls).
pub(crate) fn windows(count: usize) -> Model {
    let mut model = Model::default();
    model.materials.push(mt_kernel::Material {
        name: "Glass".into(),
        color: [120, 170, 220],
        opacity: 0.35,
        texture: None,
    });
    let mut ents = Entities::default();
    for (o, s) in [
        (DVec3::ZERO, DVec3::new(30.0, 4.0, 3.0)),
        (DVec3::new(0.0, 0.0, 37.0), DVec3::new(30.0, 4.0, 3.0)),
        (DVec3::new(0.0, 0.0, 3.0), DVec3::new(3.0, 4.0, 34.0)),
        (DVec3::new(27.0, 0.0, 3.0), DVec3::new(3.0, 4.0, 34.0)),
    ] {
        add_box(&mut ents, o, s);
    }
    let pane: Vec<VertexId> = [[3.0, 3.0], [27.0, 3.0], [27.0, 37.0], [3.0, 37.0]]
        .into_iter()
        .map(|[x, z]| ents.add_vertex(DVec3::new(x, 2.0, z)))
        .collect();
    let f = polygon(&mut ents, &pane);
    ents.faces[f].attrs.material = Some(0);
    ents.faces[f].attrs.back_material = Some(0);
    let d = model.definitions.insert(definition("Window", ents, false));
    let side = (count as f64).sqrt().ceil() as usize;
    for k in 0..count {
        let t = DVec3::new((k % side) as f64 * 40.0, (k / side) as f64 * 40.0, 0.0);
        let r = DMat4::from_rotation_z(0.3 * (k % 7) as f64);
        model
            .root
            .instances
            .push(instance(d, DMat4::from_translation(t) * r));
    }
    model
}

/// `count` groups, each with a box definition of its own (an imported model).
pub(crate) fn groups(count: usize) -> Model {
    let mut model = Model::default();
    let side = (count as f64).sqrt().ceil() as usize;
    for k in 0..count {
        let mut ents = Entities::default();
        add_box(
            &mut ents,
            DVec3::ZERO,
            DVec3::new(20.0, 20.0, 5.0 + k as f64 % 30.0),
        );
        let d = model
            .definitions
            .insert(definition(&format!("Group#{k}"), ents, true));
        let t = DVec3::new((k % side) as f64 * 40.0, (k / side) as f64 * 40.0, 0.0);
        model
            .root
            .instances
            .push(instance(d, DMat4::from_translation(t)));
    }
    model
}

/// Components nested `depth` deep, `breadth` instances per level, boxes at the
/// leaves: `breadth^depth` drawn boxes.
pub(crate) fn nested(depth: usize, breadth: usize) -> Model {
    let mut model = Model::default();
    let mut ents = Entities::default();
    add_box(&mut ents, DVec3::ZERO, DVec3::splat(10.0));
    let mut child = model.definitions.insert(definition("Leaf", ents, false));
    let mut size = 10.0;
    for level in 0..depth {
        let mut ents = Entities::default();
        let step = size * 1.5;
        for k in 0..breadth {
            let t = DVec3::new((k % 2) as f64 * step, (k / 2) as f64 * step, 0.0);
            let r = DMat4::from_rotation_z(0.02 * k as f64);
            ents.instances
                .push(instance(child, DMat4::from_translation(t) * r));
        }
        size = step * 2.0;
        child = model
            .definitions
            .insert(definition(&format!("Level{level}"), ents, false));
    }
    model.root.instances.push(instance(child, DMat4::IDENTITY));
    model
}

/// A model read from a file, placed at the origin as SketchUp's import does.
fn from_file(path: &Path) -> Option<Document> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let imported = match ext.as_str() {
        "skp" => return Document::open(path).ok(),
        "gltf" | "glb" => crate::import::read_gltf(path, true).ok()?,
        "ifc" => {
            let bytes = std::fs::read(path).ok()?;
            mt_kernel::formats::ifc::import(&bytes, &Default::default())
                .ok()?
                .0
        }
        _ => return None,
    };
    let mut doc = Document::untitled();
    let name = path.file_stem()?.to_string_lossy().into_owned();
    doc.edit_model(|m, ents| {
        let d = import::merge_as_definition(m, imported, &name);
        ents.instances.push(instance(d, DMat4::IDENTITY));
        true
    });
    Some(doc)
}

/// FNV-1a over bytes.
fn fnv(hash: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *hash ^= u64::from(*b);
        *hash = hash.wrapping_mul(0x100_0000_01b3);
    }
}

/// Checksum of what the scene draws and how it is picked. Instanced triangles count
/// as their flattened vertices (the checksum of the same scene drawn without
/// instancing), so instancing is checked to draw exactly what flattening does.
pub fn checksum(scene: &Scene) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    let vertices = scene.flattened_vertices();
    fnv(&mut h, bytemuck::cast_slice(&vertices));
    let indices: Vec<u32> = (0..vertices.len() as u32).collect();
    fnv(&mut h, bytemuck::cast_slice(&indices));
    fnv(&mut h, bytemuck::cast_slice(scene.edges()));
    fnv(&mut h, bytemuck::cast_slice(&scene.guides));
    fnv(
        &mut h,
        format!("{:?}{:?}", scene.edge_owner, scene.tri_owner).as_bytes(),
    );
    for t in scene.triangles() {
        for p in t {
            fnv(&mut h, bytemuck::cast_slice(&p.to_array()));
        }
    }
    fnv(&mut h, format!("{:?}", scene.tri_material).as_bytes());
    let mut keys: Vec<_> = scene.hidden_edges.iter().copied().collect();
    keys.sort_unstable();
    fnv(&mut h, format!("{keys:?}").as_bytes());
    // (index, container) pairs in index order.
    for list in [&scene.edge_container, &scene.tri_container] {
        let c: Vec<_> = (list.iter().enumerate())
            .filter_map(|(k, v)| Some((k, (*v)? as u8)))
            .collect();
        fnv(&mut h, format!("{c:?}").as_bytes());
    }
    h
}

/// Bytes of the scene's per-triangle and per-edge lists (and instanced meshes).
fn scene_bytes(scene: &Scene) -> usize {
    use std::mem::size_of_val;
    let meshes: usize = (scene.instancing.meshes.iter())
        .map(|m| {
            size_of_val(&m.vertices[..]) * 2
                + size_of_val(&m.indices[..])
                + size_of_val(&m.edges[..])
                + m.local_edges.as_ref().map_or(0, |l| size_of_val(&l[..]))
        })
        .sum();
    meshes
        + size_of_val(&scene.instancing.placements[..])
        + size_of_val(&scene.vertices[..])
        + size_of_val(&scene.indices[..])
        + size_of_val(scene.stored_tris())
        + size_of_val(&scene.tri_owner[..])
        + size_of_val(&scene.tri_material[..])
        + size_of_val(scene.stored_edges())
        + size_of_val(&scene.edge_owner[..])
}

/// Bytes of the vertex, index, edge and instance buffers uploaded for the scene.
fn gpu_bytes(scene: &Scene) -> usize {
    use std::mem::size_of_val;
    let (edges, instanced) = scene.flat_edges();
    let meshes: usize = (scene.instancing.meshes.iter())
        .map(|m| {
            size_of_val(&m.vertices[..]) + size_of_val(&m.indices[..]) + size_of_val(&m.edges[..])
        })
        .sum();
    // One transform per face instance, six per edge instance.
    let instances = 64 * (scene.instancing.placements.len() + 6 * instanced.len());
    // The translucent triangles of each placement, sorted with the model's: three
    // indices each.
    let translucent: Vec<usize> = (scene.instancing.meshes.iter())
        .map(|m| m.tris.iter().filter(|t| !t.opaque()).count())
        .collect();
    let sorted: usize = (scene.instancing.placements.iter())
        .map(|p| 12 * translucent[p.mesh as usize])
        .sum();
    sorted
        + size_of_val(&scene.vertices[..])
        + size_of_val(&scene.indices[..])
        + size_of_val(&edges[..])
        + meshes
        + instances
}

pub(crate) struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

pub(crate) fn gpu() -> Option<Gpu> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    eprintln!("bench: adapter {:?}", adapter.get_info().name);
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).ok()?;
    Some(Gpu { device, queue })
}

/// Results: (model, stage, milliseconds).
#[derive(Default)]
struct Report {
    rows: Vec<(String, String, f64)>,
}

impl Report {
    fn time<T>(&mut self, model: &str, stage: &str, f: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let out = f();
        self.add(model, stage, start.elapsed().as_secs_f64() * 1e3);
        out
    }

    fn add(&mut self, model: &str, stage: &str, ms: f64) {
        eprintln!("bench: {model:<24} {stage:<22} {ms:>12.3} ms");
        self.rows.push((model.into(), stage.into(), ms));
    }
}

/// Mean time of `f` over `n` runs, in milliseconds.
fn mean_ms(n: usize, mut f: impl FnMut(usize)) -> f64 {
    let start = Instant::now();
    for k in 0..n {
        f(k);
    }
    start.elapsed().as_secs_f64() * 1e3 / n as f64
}

fn run(name: &str, doc: Document, report: &mut Report, gpu: Option<&Gpu>, tmp: &Path) {
    // Save and open again: the rest runs on the model as read from a file.
    let bytes = report.time(name, "save (.skp bytes)", || doc.to_skp());
    let mut doc = match bytes {
        Ok(bytes) => {
            let path = tmp.join(format!("{name}.skp"));
            std::fs::write(&path, &bytes).unwrap();
            // The two halves of opening: parsing the file and building the model.
            if let Ok(src) = report.time(name, "open: parse .skp", || mt_skp::read(&path)) {
                report.time(name, "open: build model", || import::from_skp(&src));
            }
            match report.time(name, "open", || Document::open(&path)) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("bench: {name}: reopening failed ({e}); using the model as built");
                    doc
                }
            }
        }
        Err(e) => {
            eprintln!("bench: {name}: save failed ({e:?})");
            doc
        }
    };
    let scene = report.time(name, "scene build", || doc.scene());
    eprintln!(
        "bench: {name}: {} edges, {} triangles, checksum {:016x}",
        scene.edge_count(),
        scene.tri_count(),
        checksum(&scene)
    );
    eprintln!(
        "bench: {name}: scene {:.1} MB, GPU buffers {:.1} MB",
        scene_bytes(&scene) as f64 / 1e6,
        gpu_bytes(&scene) as f64 / 1e6
    );
    let bvh = report.time(name, "pick index (BVH)", || {
        crate::pick_index::PickIndex::build(&scene, |t| Box::new(Bvh::build(t)))
    });
    let geometry = report.time(name, "snap geometry", || {
        crate::snap::geometry(&scene, Some(&doc), DMat4::IDENTITY)
    });
    let inst_bounds = report.time(name, "instance bounds", || {
        let ents = doc.active();
        crate::selection::instance_bounds(&scene, |i| {
            ents.instances
                .get(i)
                .map(|inst| doc.drawn_transform(inst.transform, inst))
        })
    });

    let camera = crate::document::extents_camera_of(&scene, 35.0).unwrap_or_else(|| {
        mt_render::Camera::from_spec(&mt_spec::Spec::builtin().unwrap().default_model.camera)
    });
    let px = DVec2::new(f64::from(VIEWPORT[0]), f64::from(VIEWPORT[1]));
    let cursors: Vec<DVec2> = (0..49)
        .map(|k| DVec2::new((k % 7) as f64 + 0.5, (k / 7) as f64 + 0.5) * px / 7.0)
        .collect();

    if let Some(g) = gpu {
        let spec = mt_spec::Spec::builtin().unwrap();
        let mut vp =
            mt_render::Viewport::new(&g.device, mt_render::Style::from_spec(&spec.default_model));
        report.time(name, "GPU upload", || {
            crate::instancing::upload_faces(&mut vp, &g.device, &scene);
            let (edges, instanced) = scene.flat_edges();
            vp.set_edges(&g.device, &edges);
            vp.set_edge_instances(&g.device, &scene.gpu_instances(instanced));
            g.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        });
        // First frame builds pipelines and targets.
        vp.render(&g.device, &g.queue, &camera, VIEWPORT, 1.0);
        g.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let ms = mean_ms(5, |_| {
            vp.render(&g.device, &g.queue, &camera, VIEWPORT, 1.0);
            g.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        });
        report.add(name, "frame (1280x800)", ms);
    }

    // The first hover builds the culling indexes; the mean is over the others. The
    // answers are hashed so a change can be checked to pick the same things.
    let mut answers = 0xcbf2_9ce4_8422_2325;
    let mut pick = |k: usize| {
        let hit = crate::selection::pick(&scene, &*bvh, &camera, cursors[k], px);
        fnv(&mut answers, format!("{hit:?}").as_bytes());
    };
    report.time(name, "first hover pick", || pick(0));
    let ms = mean_ms(cursors.len() - 1, |k| pick(k + 1));
    report.add(name, "hover pick", ms);
    let mut engine = mt_infer::Engine::default();
    let view = mt_infer::View {
        camera: &camera,
        viewport: px,
    };
    let mut infer = |k: usize| {
        let i = engine.infer(&geometry, &*bvh, view, cursors[k], None);
        fnv(
            &mut answers,
            format!("{:?} {:?} {:?}", i.point, i.kind, i.edge).as_bytes(),
        );
    };
    report.time(name, "first hover inference", || infer(0));
    let ms = mean_ms(cursors.len() - 1, |k| infer(k + 1));
    report.add(name, "hover inference", ms);
    eprintln!("bench: {name}: hover checksum {answers:016x}");

    // Edit > Select All, and the highlight it draws.
    let mut selection = Selection::default();
    let all = report.time(name, "select all", || {
        let all = live_view::all_owners(&scene);
        selection.apply(Mode::Replace, all.clone());
        all
    });
    let stretch = crate::move_extra::Stretch::default();
    let highlight = |selection: &Selection,
                     live: Option<&(Vec<Owner>, DMat4, bool)>,
                     stretch: &crate::move_extra::Stretch| {
        live_view::Highlight {
            scene: &scene,
            selection,
            document: Some(&doc),
            stretch,
            box_only: false,
            live,
            ctx_xf: DMat4::IDENTITY,
            inst_bounds: &inst_bounds,
            ghost: None,
            cut_lines: &[],
        }
        .build()
    };
    report.time(name, "selection highlight", || {
        std::hint::black_box(highlight(&selection, None, &stretch));
    });

    // Move a tenth of everything (connected geometry stretches), one mouse move of
    // the live preview, then the move itself.
    let targets: Vec<Owner> = all.iter().copied().take(all.len().div_ceil(10)).collect();
    selection.apply(Mode::Replace, targets.clone());
    let xf = DMat4::from_translation(DVec3::new(5.0, 3.0, 1.0));
    let live = (targets.clone(), xf, false);
    report.time(name, "move preview (1/10)", || {
        let stretch =
            crate::move_extra::live_stretch(Some(&doc), &Some(live.clone()), DMat4::IDENTITY);
        let kept = live_view::kept_indices(&scene, &targets, &stretch);
        let faces = live_view::preview_faces(&scene, &targets, xf, &stretch);
        let h = highlight(&selection, Some(&live), &stretch);
        std::hint::black_box((kept, faces, h));
    });
    // The first undo step copies everything; later ones only what changed since.
    report.time(name, "undo snapshot (first)", || {
        std::hint::black_box(doc.snapshot());
    });
    report.time(name, "move commit", || {
        doc.edit(|ents| {
            crate::modify::apply(ents, &targets, xf, false);
            true
        });
    });
    report.time(name, "undo snapshot (after an edit)", || {
        std::hint::black_box(doc.snapshot());
    });
    report.time(name, "rebuild after move", || {
        std::hint::black_box(doc.scene());
    });
    report.time(name, "undo", || {
        doc.undo();
    });
    let again = doc.scene();
    assert_eq!(
        checksum(&again),
        checksum(&scene),
        "{name}: undo restores the scene"
    );
    // A small edit (one of the moved entities) and its undo step: what an undo step
    // costs when an edit touches little of a big context.
    let one: Vec<Owner> = targets.iter().copied().take(1).collect();
    report.time(name, "small edit", || {
        doc.edit(|ents| {
            crate::modify::apply(ents, &one, xf, false);
            true
        });
    });
    report.time(name, "undo snapshot (after a small edit)", || {
        std::hint::black_box(doc.snapshot());
    });
    report.time(name, "undo (small edit)", || {
        doc.undo();
    });

    // `App::rebuild` after an edit and the highlight, everything made anew (as before
    // rebuilds kept what did not change) and kept (`scene_cache`, `pick_index::Kept`,
    // `snap::Kept`, `edge_view`).
    let mut full = AppSide::new(gpu);
    let mut kept = AppSide::new(gpu);
    full.rebuild(&doc, gpu, false);
    report.time(name, "first rebuild (kept)", || {
        kept.rebuild(&doc, gpu, true)
    });
    assert_eq!(
        checksum(&kept.scene),
        checksum(&scene),
        "{name}: kept scene"
    );
    // The kept pick index and snap geometry answer as the full ones.
    let mut kept_answers = 0xcbf2_9ce4_8422_2325;
    let kept_bvh = kept.bvh.as_deref().expect("built");
    for c in &cursors {
        let hit = crate::selection::pick(&kept.scene, kept_bvh, &camera, *c, px);
        fnv(&mut kept_answers, format!("{hit:?}").as_bytes());
    }
    let mut engine = mt_infer::Engine::default();
    for c in &cursors {
        let i = engine.infer(&kept.geometry, kept_bvh, view, *c, None);
        fnv(
            &mut kept_answers,
            format!("{:?} {:?} {:?}", i.point, i.kind, i.edge).as_bytes(),
        );
    }
    eprintln!("bench: {name}: hover checksum (kept) {kept_answers:016x}");
    assert_eq!(
        kept_answers, answers,
        "{name}: kept pick index and snap geometry"
    );
    let edits: [(&str, &[Owner]); 2] = [("small edit", &one), ("move (1/10)", &targets)];
    for (edit, which) in edits {
        doc.edit(|ents| {
            crate::modify::apply(ents, which, xf, false);
            true
        });
        for (way, keep) in [("full", false), ("kept", true)] {
            let side = if keep { &mut kept } else { &mut full };
            let stage = format!("rebuild after {edit} ({way})");
            report.time(name, &stage, || side.rebuild(&doc, gpu, keep));
            // The first mouse move after it builds the culling indexes (kept: from
            // the last ones where the edges are the same).
            let stage = format!("first hover after {edit} ({way})");
            let hover = report.time(name, &stage, || side.hover(&camera, cursors[0], px));
            if keep {
                assert_eq!(hover, full.hover(&camera, cursors[0], px), "{name}: {edit}");
            }
        }
        assert_eq!(
            checksum(&kept.scene),
            checksum(&full.scene),
            "{name}: {edit}"
        );
        doc.undo();
        for (way, keep) in [("full", false), ("kept", true)] {
            let side = if keep { &mut kept } else { &mut full };
            let stage = format!("rebuild after undo of {edit} ({way})");
            report.time(name, &stage, || side.rebuild(&doc, gpu, keep));
        }
        assert_eq!(
            checksum(&kept.scene),
            checksum(&scene),
            "{name}: undo {edit}"
        );
    }
    let shows: [(&str, &[Owner]); 2] = [("highlight one", &one), ("highlight all", &all)];
    for (stage, which) in shows {
        let mut selection = Selection::default();
        selection.apply(Mode::Replace, which.to_vec());
        for (way, keep) in [("full", false), ("kept", true)] {
            let side = if keep { &mut kept } else { &mut full };
            let stage = format!("{stage} ({way})");
            report.time(name, &stage, || side.show(&doc, &selection, gpu, keep));
        }
        kept.show(&doc, &Selection::default(), gpu, true);
    }
}

/// The model-dependent part of `App::rebuild`, and the highlight, without the app.
struct AppSide {
    scene: Scene,
    bvh: Option<Box<dyn mt_kernel::bvh::RayIndex>>,
    geometry: mt_infer::Geometry,
    inst_bounds: std::collections::HashMap<usize, (DVec3, DVec3)>,
    pick: crate::pick_index::Kept,
    snap: crate::snap::Kept,
    gpu: crate::edge_view::GpuScene,
    viewport: Option<mt_render::Viewport>,
}

impl AppSide {
    fn new(gpu: Option<&Gpu>) -> Self {
        let spec = mt_spec::Spec::builtin().unwrap();
        Self {
            scene: Scene::default(),
            bvh: None,
            geometry: Default::default(),
            inst_bounds: Default::default(),
            pick: Default::default(),
            snap: Default::default(),
            gpu: Default::default(),
            viewport: gpu.map(|g| {
                mt_render::Viewport::new(
                    &g.device,
                    mt_render::Style::from_spec(&spec.default_model),
                )
            }),
        }
    }

    /// The scene, pick index, snap geometry, instance bounds and GPU buffers of `doc`
    /// (`keep`: from the last ones), and the highlight without a selection.
    fn rebuild(&mut self, doc: &Document, gpu: Option<&Gpu>, keep: bool) {
        // MESHTARD_BENCH_STEPS=1 prints the time of each part.
        let steps = std::env::var_os("MESHTARD_BENCH_STEPS").is_some();
        let mut at = Instant::now();
        let mut step = |what: &str| {
            if steps {
                eprintln!(
                    "bench:   {what:<24} {:>10.3} ms",
                    at.elapsed().as_secs_f64() * 1e3
                );
            }
            at = Instant::now();
        };
        if keep {
            let prev = std::mem::take(&mut self.scene);
            self.scene = doc.scene_from(Some(prev));
            if steps {
                let c = &self.scene.changes;
                let n = |r: Vec<std::ops::Range<usize>>| r.iter().map(|r| r.len()).sum::<usize>();
                eprintln!(
                    "bench:   changed: {} of {} vertices, {} of {} edges, {} of {} stored triangles, meshes kept {}",
                    n(c.vertices.changed(self.scene.vertices.len())),
                    self.scene.vertices.len(),
                    n(c.edges.changed(self.scene.edge_count())),
                    self.scene.edge_count(),
                    n(c.stored_tris.changed(self.scene.stored_tris().len())),
                    self.scene.stored_tris().len(),
                    c.meshes,
                );
            }
            step("scene");
            let none: Option<crate::pick_index::FlatIndex> = None;
            self.bvh = Some(crate::pick_index::PickIndex::build_kept(
                &self.scene,
                &mut self.pick,
                none,
            ));
            step("pick index");
            let old = std::mem::take(&mut self.geometry);
            self.geometry = crate::snap::geometry_kept(
                &self.scene,
                Some(doc),
                DMat4::IDENTITY,
                &mut self.snap,
                old,
            );
            step("snap geometry");
        } else {
            self.scene = doc.scene();
            self.bvh = Some(crate::pick_index::PickIndex::build(&self.scene, |t| {
                Box::new(Bvh::build(t))
            }));
            self.geometry = crate::snap::geometry(&self.scene, Some(doc), DMat4::IDENTITY);
        }
        let ents = doc.active();
        self.inst_bounds = crate::selection::instance_bounds(&self.scene, |i| {
            (ents.instances.get(i)).map(|inst| doc.drawn_transform(inst.transform, inst))
        });
        step("instance bounds");
        if let (Some(g), Some(vp)) = (gpu, &mut self.viewport) {
            if keep {
                self.gpu.upload(&self.scene, vp, &g.device, &g.queue);
            } else {
                crate::instancing::upload_faces(vp, &g.device, &self.scene);
            }
        }
        step("upload");
        self.show(doc, &Selection::default(), gpu, keep);
        step("highlight");
    }

    /// What a hover at `cursor` picks and infers (the first one after a rebuild
    /// builds the culling indexes).
    fn hover(&self, camera: &mt_render::Camera, cursor: DVec2, px: DVec2) -> String {
        let bvh = self.bvh.as_deref().expect("built");
        let hit = crate::selection::pick(&self.scene, bvh, camera, cursor, px);
        let view = mt_infer::View {
            camera,
            viewport: px,
        };
        let i = mt_infer::Engine::default().infer(&self.geometry, bvh, view, cursor, None);
        format!("{hit:?} {:?} {:?} {:?}", i.point, i.kind, i.edge)
    }

    /// The highlight of `selection`: everything sent again, or (`keep`) only what
    /// changed.
    fn show(&mut self, doc: &Document, selection: &Selection, gpu: Option<&Gpu>, keep: bool) {
        let stretch = crate::move_extra::Stretch::default();
        let h = live_view::Highlight {
            scene: &self.scene,
            selection,
            document: Some(doc),
            stretch: &stretch,
            box_only: false,
            live: None,
            ctx_xf: DMat4::IDENTITY,
            inst_bounds: &self.inst_bounds,
            ghost: None,
            cut_lines: &[],
        };
        match (gpu, &mut self.viewport) {
            (Some(g), Some(vp)) => {
                if keep {
                    self.gpu.show(&h, vp, &g.device, &g.queue);
                } else {
                    let (edges, faces, instanced) = h.build();
                    let indices: Vec<u32> = (0..faces.len() as u32).collect();
                    vp.set_selected_faces(&g.device, &faces, &indices);
                    vp.set_edges(&g.device, &edges);
                    vp.set_edge_instances(&g.device, &self.scene.gpu_instances(instanced));
                }
                g.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            }
            _ if keep => {
                std::hint::black_box(h.build_sparse());
            }
            _ => {
                std::hint::black_box(h.build());
            }
        }
    }
}

fn wanted(name: &str) -> bool {
    match std::env::var("MESHTARD_BENCH") {
        Ok(f) if !f.is_empty() => f.split(',').any(|w| name.contains(w.trim())),
        _ => true,
    }
}

#[test]
#[ignore = "benchmark: run with --release -- --ignored --nocapture"]
fn bench_large_models() {
    let tmp = std::env::temp_dir().join(format!("meshtard-bench-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    // MESHTARD_BENCH_NO_GPU=1 skips the frame times (for profiling the CPU stages).
    let gpu = std::env::var_os("MESHTARD_BENCH_NO_GPU")
        .is_none()
        .then(gpu)
        .flatten();
    if gpu.is_none() {
        eprintln!("bench: no wgpu adapter; frame times skipped");
    }
    let mut report = Report::default();
    type Make = fn() -> Model;
    let synthetic: [(&str, Make); 5] = [
        // MESHTARD_BENCH_GRID=n changes the grid's size (577: a million edges).
        ("grid 1M edges", || {
            let n = std::env::var("MESHTARD_BENCH_GRID").ok();
            grid(n.and_then(|n| n.parse().ok()).unwrap_or(577))
        }),
        ("instances 10k", || instances(10_000)),
        ("groups 5k", || groups(5_000)),
        ("nested 4^6", || nested(6, 4)),
        ("windows 5k", || windows(5_000)),
    ];
    for (name, make) in synthetic {
        if !wanted(name) {
            continue;
        }
        let model = report.time(name, "generate", make);
        let mut doc = Document::untitled();
        doc.model = model;
        run(name, doc, &mut report, gpu.as_ref(), &tmp);
    }
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
    let mut files: Vec<PathBuf> = [
        "openskp/coedge_orientation_regression.skp",
        "openskp/legacy_v2014_synthetic.skp",
        "openskp/gondola_v20.skp",
    ]
    .iter()
    .map(|f| corpus.join(f))
    .collect();
    if let Ok(extra) = std::env::var("MESHTARD_BENCH_FILES") {
        files.extend(
            extra
                .split(':')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
        );
    }
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !wanted(&name) || !path.exists() {
            continue;
        }
        let Some(doc) = report.time(&name, "read file", || from_file(&path)) else {
            eprintln!("bench: could not read {}", path.display());
            continue;
        };
        run(&name, doc, &mut report, gpu.as_ref(), &tmp);
    }
    let _ = std::fs::remove_dir_all(&tmp);
    // A table to paste into spec/PERFORMANCE.md.
    eprintln!("\n| model | stage | ms |\n|---|---|---:|");
    for (m, s, ms) in &report.rows {
        eprintln!("| {m} | {s} | {ms:.2} |");
    }
}
