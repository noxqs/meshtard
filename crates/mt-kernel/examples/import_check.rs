//! Imports DXF, glTF / GLB and FBX files the way File > Import does and prints what
//! came in: `cargo run -p mt-kernel --example import_check -- FILE...`.
//!
//! For each file: definitions (components and groups), instances, faces, edges,
//! curves, texts, dimensions and materials over the whole model, the nesting depth,
//! and whether every definition and the root pass `mt_kernel::validate` (`valid`, or
//! `INVALID` with the first problem).

use std::path::Path;

use mt_kernel::Model;
use mt_kernel::entities::{DefinitionId, Entities};

fn depth(model: &Model, ents: &Entities, seen: &mut Vec<DefinitionId>) -> usize {
    let mut best = 0;
    for i in &ents.instances {
        if seen.contains(&i.definition) {
            continue;
        }
        if let Some(d) = model.definitions.get(i.definition) {
            seen.push(i.definition);
            best = best.max(1 + depth(model, &d.entities, seen));
            seen.pop();
        }
    }
    best
}

fn import(path: &Path) -> Result<Model, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let read = move |name: &str| std::fs::read(dir.join(name)).ok();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "dxf" => {
            let d = mt_kernel::dxf::parse(&bytes)?;
            Ok(mt_kernel::dxf::to_model(&d, &Default::default()))
        }
        "gltf" | "glb" => mt_kernel::gltf_io::import(&bytes, &read, true),
        "fbx" => {
            let (scene, _) = mt_kernel::formats::fbx::parse_scene(&bytes, &read)?;
            let mut model = Model::default();
            model.materials.extend(
                scene
                    .mesh
                    .materials
                    .iter()
                    .map(mt_kernel::formats::model_material),
            );
            let ids: Vec<usize> = (0..scene.mesh.materials.len()).collect();
            let opts = mt_kernel::mesh_import::MeshOptions {
                inches_per_unit: 1.0 / 2.54,
                y_up: false,
                merge_coplanar: true,
            };
            // `FLAT=1` imports the mesh flattened, for comparison.
            model.root = if std::env::var_os("FLAT").is_some() {
                mt_kernel::mesh_import::to_entities(&scene.mesh, &opts, &ids)
            } else {
                mt_kernel::formats::scene::to_entities(&mut model, &scene, &opts, &ids)
            };
            Ok(model)
        }
        _ => Err("unknown extension".into()),
    }
}

fn main() {
    // `--texts` also lists texts and dimension overrides.
    let show_texts = std::env::args().any(|a| a == "--texts");
    for arg in std::env::args().skip(1).filter(|a| a != "--texts") {
        let path = Path::new(&arg);
        let start = std::time::Instant::now();
        let model = match import(path) {
            Ok(m) => m,
            Err(e) => {
                println!("{arg}: ERROR {e}");
                continue;
            }
        };
        let all: Vec<&Entities> = std::iter::once(&model.root)
            .chain(model.definitions.values().map(|d| &d.entities))
            .collect();
        let sum = |f: &dyn Fn(&Entities) -> usize| all.iter().map(|e| f(e)).sum::<usize>();
        let problem = all.iter().find_map(|e| {
            mt_kernel::validate(e, mt_kernel::Checks::ALL)
                .into_iter()
                .next()
        });
        let components = model.definitions.values().filter(|d| !d.is_group).count();
        println!(
            "{arg}: {} components, {} groups, {} instances, depth {}, {} faces, {} edges, {} curves, {} texts, {} dimensions, {} materials, {} ({:.1}s)",
            components,
            model.definitions.len() - components,
            sum(&|e| e.instances.len()),
            depth(&model, &model.root, &mut Vec::new()),
            sum(&|e| e.faces.len()),
            sum(&|e| e.edges.len()),
            sum(&|e| e.curves.len()),
            sum(&|e| e.texts.len()),
            sum(&|e| e.dimensions.len()),
            model.materials.len(),
            match problem {
                None => "valid".to_owned(),
                Some(p) => format!("INVALID: {p}"),
            },
            start.elapsed().as_secs_f64(),
        );
        if show_texts {
            for e in &all {
                for t in &e.texts {
                    println!("  text {:?} at {:.3}", t.text, t.point);
                }
                for d in &e.dimensions {
                    println!(
                        "  dimension {:?} {:.3} - {:.3}, offset {:.3}, text {:?}",
                        d.kind, d.a, d.b, d.offset, d.text
                    );
                }
            }
        }
    }
}
