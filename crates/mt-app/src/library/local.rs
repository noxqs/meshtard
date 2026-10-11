//! Local folders: the Components folder of Preferences > Files and folders added in
//! the panel, scanned (subfolders too) for `.skp`, `.glb`, `.gltf`, `.obj` and
//! `.stl` files. Thumbnails are rendered from the file (see `thumbs`) and cached.
//! Also reads any supported file into a model (local and downloaded files).

use std::path::{Path, PathBuf};

use mt_kernel::mesh_import::MeshOptions;

use super::{Asset, Source};

pub(crate) const EXTENSIONS: [&str; 9] = [
    "skp", "glb", "gltf", "obj", "stl", "fbx", "ply", "3mf", "3ds",
];
/// Most files listed from all folders, and how deep subfolders are looked into.
const MAX_FILES: usize = 5000;
const MAX_DEPTH: usize = 6;

fn ext(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// The supported files in `folders` (and their subfolders), sorted by name.
pub(crate) fn scan(folders: &[PathBuf]) -> Vec<Asset> {
    let mut files = Vec::new();
    for f in folders {
        walk(f, 0, &mut files);
    }
    files.sort();
    files.dedup();
    let mut out: Vec<Asset> = files.into_iter().map(|p| asset_for(&p)).collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if out.len() >= MAX_FILES {
            return;
        }
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            let hidden = e.file_name().to_string_lossy().starts_with('.');
            if depth < MAX_DEPTH && !hidden {
                walk(&p, depth + 1, out);
            }
        } else if EXTENSIONS.contains(&ext(&p).as_str()) {
            out.push(p);
        }
    }
}

fn asset_for(path: &Path) -> Asset {
    let name = path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let size = std::fs::metadata(path).ok().map(|m| m.len());
    Asset {
        source: Source::Local,
        id: path.to_string_lossy().into_owned(),
        name,
        license: "Unknown (local file)".into(),
        description: path.display().to_string(),
        size,
        local_path: Some(path.to_owned()),
        tags: vec![ext(path)],
        ..Default::default()
    }
}

/// Reads a supported file into a model: `.skp` as File > Import does, glTF/GLB
/// with coplanar faces merged, OBJ (meters, Y up) and STL (millimeters) with the
/// File > Import defaults.
pub(crate) fn load_model(path: &Path) -> Result<mt_kernel::Model, String> {
    match ext(path).as_str() {
        "skp" => {
            let src = mt_skp::read(path).map_err(|e| e.to_string())?;
            Ok(mt_kernel::import::from_skp(&src))
        }
        "glb" | "gltf" => crate::import::read_gltf(path, true),
        e => {
            let kind = mt_kernel::formats::Kind::from_extension(e)
                .ok_or_else(|| format!("Unsupported file {}.", path.display()))?;
            // Units and axes as the import dialog defaults them for this format.
            let (units, y_up) = kind.default_hints();
            let opts = MeshOptions {
                inches_per_unit: 1.0 / units.per_inch(),
                y_up,
                merge_coplanar: true,
            };
            let (materials, ents) = crate::import::read_mesh(path, kind, &opts, 0, true)?;
            if ents.faces.is_empty() {
                return Err(format!("No faces in {}.", path.display()));
            }
            Ok(mt_kernel::Model {
                materials: materials
                    .into_iter()
                    .map(|x| mt_kernel::Material {
                        name: x.name,
                        color: x.color,
                        opacity: x.opacity,
                        texture: None,
                    })
                    .collect(),
                root: ents,
                ..Default::default()
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mt_library_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const CUBE_STL: &str = "solid t
facet normal 0 0 1
outer loop
vertex 0 0 0
vertex 10 0 0
vertex 10 10 0
endloop
endfacet
endsolid t
";

    #[test]
    fn scans_supported_files_in_subfolders() {
        let d = temp_dir("scan");
        std::fs::create_dir_all(d.join("sub/.hidden")).unwrap();
        std::fs::write(d.join("b.stl"), CUBE_STL).unwrap();
        std::fs::write(d.join("sub/A.GLB"), b"x").unwrap();
        std::fs::write(d.join("sub/.hidden/c.obj"), b"x").unwrap();
        std::fs::write(d.join("notes.txt"), b"x").unwrap();
        let a = scan(&[d.clone(), d.join("sub")]);
        let names: Vec<_> = a.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            ["A", "b"],
            "sorted, no duplicates, hidden folders skipped"
        );
        assert_eq!(a[1].size, Some(CUBE_STL.len() as u64));
        assert_eq!(a[0].source, Source::Local);
        let m = load_model(&d.join("b.stl")).unwrap();
        assert_eq!(m.root.faces.len(), 1);
        assert!(load_model(&d.join("notes.txt")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
