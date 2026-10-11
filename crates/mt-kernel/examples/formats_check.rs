//! Import, export and re-import round trips over mesh files:
//! `cargo run -p mt-kernel --example formats_check -- OUT_DIR FILE...`.
//!
//! Each FILE (OBJ, STL, FBX, PLY, 3MF, 3DS) is imported in its units (as the import
//! dialog defaults them) with coplanar
//! faces merged; the result is exported as OBJ, STL, COLLADA, FBX, PLY, 3MF, VRML and
//! KMZ into `OUT_DIR/<stem>/`, and the readable exports are imported again. Faces,
//! vertices, materials and the bounding box are printed for each step; a re-import
//! whose area, bounding box, material or texture count differs is marked `DIFF`
//! (merged face counts may differ: triangles of slightly non-planar polygons do not
//! merge back).

use std::path::{Path, PathBuf};

use glam::DVec3;
use mt_kernel::Model;
use mt_kernel::formats::{self, ExportKind, ExportOptions, Kind};
use mt_kernel::mesh_export::{self, DaeOptions, ObjOptions, Units};
use mt_kernel::mesh_import::{MeshOptions, to_entities};

struct Summary {
    polygons: usize,
    faces: usize,
    vertices: usize,
    materials: usize,
    textured: usize,
    /// Faces without a material (exported as `FrontColor`).
    unpainted: bool,
    triangles: usize,
    area: f64,
    min: DVec3,
    max: DVec3,
}

/// Imports `path`; `inches_per_unit` `None` uses the file's units (or the format's
/// default).
fn import(path: &Path, inches_per_unit: Option<f64>) -> Result<(Model, Summary), String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let kind = Kind::from_extension(ext).ok_or("unknown extension")?;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let read = move |name: &str| {
        let p = Path::new(name);
        std::fs::read(if p.is_absolute() {
            p.to_path_buf()
        } else {
            dir.join(p)
        })
        .ok()
    };
    let (mesh, hints) = formats::read_mesh(kind, &bytes, &read, true)?;
    let units = hints.units.unwrap_or(kind.default_hints().0);
    let inches_per_unit = inches_per_unit.unwrap_or(1.0 / units.per_inch());
    let ids: Vec<usize> = (0..mesh.materials.len()).collect();
    let ents = to_entities(
        &mesh,
        &MeshOptions {
            inches_per_unit,
            y_up: false,
            // MERGE=0 in the environment keeps the triangles.
            merge_coplanar: std::env::var_os("MERGE").is_none_or(|v| v != "0"),
        },
        &ids,
    );
    let model = Model {
        materials: mesh.materials.iter().map(formats::model_material).collect(),
        root: ents,
        ..Default::default()
    };
    let s = summary(&model, mesh.polygons.len());
    Ok((model, s))
}

fn summary(model: &Model, polygons: usize) -> Summary {
    let ents = &model.root;
    let tris = mesh_export::triangles(model);
    let area = tris
        .iter()
        .map(|t| {
            (t.points[1] - t.points[0])
                .cross(t.points[2] - t.points[0])
                .length()
                / 2.0
        })
        .sum();
    let (mut min, mut max) = if ents.vertices.is_empty() {
        (DVec3::ZERO, DVec3::ZERO)
    } else {
        (DVec3::splat(f64::MAX), DVec3::splat(f64::MIN))
    };
    for v in ents.vertices.values() {
        min = min.min(v.pos);
        max = max.max(v.pos);
    }
    let used: std::collections::BTreeSet<usize> = ents
        .faces
        .values()
        .filter_map(|f| f.attrs.material)
        .collect();
    Summary {
        polygons,
        faces: ents.faces.len(),
        vertices: ents.vertices.len(),
        materials: used.len(),
        textured: used
            .iter()
            .filter(|&&m| model.materials[m].texture.is_some())
            .count(),
        unpainted: ents.faces.values().any(|f| f.attrs.material.is_none()),
        triangles: tris.len(),
        area,
        min,
        max,
    }
}

fn line(label: &str, s: &Summary) -> String {
    format!(
        "{label:<9} polys {:>6} faces {:>6} tris {:>6} verts {:>6} area {:>12.4} mats {:>2} (tex {}) bbox [{:.3} {:.3} {:.3}]..[{:.3} {:.3} {:.3}]",
        s.polygons,
        s.faces,
        s.triangles,
        s.vertices,
        s.area,
        s.materials,
        s.textured,
        s.min.x,
        s.min.y,
        s.min.z,
        s.max.x,
        s.max.y,
        s.max.z
    )
}

fn write(
    dir: &Path,
    name: &str,
    data: &[u8],
    others: &[(String, std::sync::Arc<[u8]>)],
) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, data).expect("write export");
    for (n, d) in others {
        std::fs::write(dir.join(n), d).expect("write texture");
    }
    p
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(out) = args.next().map(PathBuf::from) else {
        eprintln!("usage: formats_check OUT_DIR FILE...");
        std::process::exit(2);
    };
    let mut diffs = 0;
    for src in args.map(PathBuf::from) {
        println!("== {}", src.display());
        let (model, s0) = match import(&src, None) {
            Ok(x) => x,
            Err(e) => {
                println!("   import failed: {e}");
                diffs += 1;
                continue;
            }
        };
        println!("   {}", line("import", &s0));
        let stem = src.file_stem().unwrap().to_string_lossy().into_owned();
        let dir = out.join(format!(
            "{stem}_{}",
            src.extension().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut back: Vec<(&str, PathBuf)> = Vec::new();
        let obj = mesh_export::to_obj(
            &model,
            &ObjOptions {
                units: Units::Inches,
                y_up: false,
                base_name: stem.clone(),
                ..Default::default()
            },
        );
        let mut others = obj.textures.clone();
        others.push((
            format!("{stem}.mtl"),
            std::sync::Arc::from(obj.mtl.as_bytes()),
        ));
        back.push((
            "obj",
            write(&dir, &format!("{stem}.obj"), obj.obj.as_bytes(), &others),
        ));
        back.push((
            "stl",
            write(
                &dir,
                &format!("{stem}.stl"),
                &mesh_export::to_stl(&model, Units::Inches),
                &[],
            ),
        ));
        let dae = mesh_export::to_dae(
            &model,
            &DaeOptions {
                two_sided: false,
                base_name: stem.clone(),
            },
        );
        write(
            &dir,
            &format!("{stem}.dae"),
            dae.dae.as_bytes(),
            &dae.textures,
        );
        let opts = ExportOptions {
            units: Units::Inches,
            base_name: stem.clone(),
            ..Default::default()
        };
        for kind in ExportKind::ALL {
            let files = formats::export(kind, &model, &opts).expect("export");
            let p = write(
                &dir,
                &format!("{stem}.{}", kind.extension()),
                &files.main,
                &files.others,
            );
            if matches!(
                kind,
                ExportKind::Fbx | ExportKind::Ply | ExportKind::ThreeMf
            ) {
                back.push((kind.extension(), p));
            }
        }
        for (label, p) in back {
            // Exports are in inches; FBX reads back in centimeters.
            let scale = if label == "fbx" { 1.0 / 2.54 } else { 1.0 };
            match import(&p, Some(scale)) {
                Ok((_, s)) => {
                    let size = (s0.max - s0.min).length().max(1e-9);
                    let same_box = (s.min - s0.min).length() / size < 1e-4
                        && (s.max - s0.max).length() / size < 1e-4;
                    // STL has no materials; PLY materials are colors.
                    // Unpainted faces come back painted `FrontColor`.
                    let expected = s0.materials + usize::from(s0.unpainted);
                    let mats_ok = match label {
                        "stl" => true,
                        "ply" => s.materials <= expected,
                        _ => s.materials == expected,
                    };
                    let tex_ok =
                        !matches!(label, "obj" | "fbx" | "3mf") || s.textured == s0.textured;
                    let area_ok = (s.area - s0.area).abs() <= 1e-4 * s0.area.max(1e-9);
                    // Triangle counts may differ by slivers that weld away.
                    let ok = same_box && area_ok && mats_ok && tex_ok;
                    if !ok {
                        diffs += 1;
                    }
                    println!(
                        "   {} {}",
                        line(&format!("<- {label}"), &s),
                        if ok { "" } else { "DIFF" }
                    );
                }
                Err(e) => {
                    diffs += 1;
                    println!("   <- {label} failed: {e}");
                }
            }
        }
    }
    println!("{diffs} differences");
}
