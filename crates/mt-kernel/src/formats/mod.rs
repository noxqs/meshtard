//! Other 3D exchange formats, next to OBJ/STL/COLLADA ([`crate::mesh_import`],
//! [`crate::mesh_export`]): FBX (import and export), PLY (import and export), 3MF
//! (import and export), 3DS (import and export), IFC (import [`ifc`], export
//! [`ifc_export`]), KMZ, VRML 2.0 and USDZ (export). See
//! `spec/parity/FORMATS.md` for what SketchUp supports and the sources followed.
//!
//! - Importers read a file into a [`Mesh`] (file units, the file's axes unless a
//!   module says it converts them) plus [`Hints`] for the options dialog; the mesh
//!   becomes faces through [`crate::mesh_import::to_entities`] like OBJ and STL
//!   (welding, coplanar merge, texture positions from texture coordinates).
//! - Files the model names (texture images) come through a [`ReadFile`] callback so
//!   the kernel does no file system access.
//! - Exporters are pure functions returning the main file and the files written next
//!   to it ([`Files`]).

use std::collections::HashMap;
use std::sync::Arc;

use glam::{DVec2, DVec3};

use crate::mesh_export::{self, Units};
use crate::mesh_import::{MeshMaterial, MeshTexture};
use crate::model::{Material, Model};

pub mod fbx;
pub mod fbx_export;
pub mod ifc;
pub mod ifc_export;
pub mod kmz;
pub mod obj;
pub mod ply;
pub mod scene;
pub mod step;
pub mod threeds;
pub mod threemf;
pub mod usdz;
pub mod vrml;
mod xml;

pub use crate::mesh_import::Mesh;

/// What a file says about itself, used as the import dialog's defaults.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Hints {
    /// The file's length unit, when it records one.
    pub units: Option<Units>,
    /// Whether the mesh is Y up (`Some(false)`: already Z up).
    pub y_up: Option<bool>,
}

/// The mesh formats File > Import reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Obj,
    Stl,
    Fbx,
    Ply,
    ThreeMf,
    ThreeDs,
}

impl Kind {
    pub const ALL: [Kind; 6] = [
        Self::Obj,
        Self::Stl,
        Self::Fbx,
        Self::Ply,
        Self::ThreeMf,
        Self::ThreeDs,
    ];

    /// The format of a file extension (any case).
    pub fn from_extension(ext: &str) -> Option<Self> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "obj" => Self::Obj,
            "stl" => Self::Stl,
            "fbx" => Self::Fbx,
            "ply" => Self::Ply,
            "3mf" => Self::ThreeMf,
            "3ds" => Self::ThreeDs,
            _ => return None,
        })
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Obj => "obj",
            Self::Stl => "stl",
            Self::Fbx => "fbx",
            Self::Ply => "ply",
            Self::ThreeMf => "3mf",
            Self::ThreeDs => "3ds",
        }
    }

    /// The format's name in dialog titles.
    pub fn name(self) -> &'static str {
        match self {
            Self::Obj => "OBJ",
            Self::Stl => "STL",
            Self::Fbx => "FBX",
            Self::Ply => "PLY",
            Self::ThreeMf => "3MF",
            Self::ThreeDs => "3DS",
        }
    }

    /// Import dialog defaults when the file does not say (guesses, unverified): STL
    /// and 3MF millimeters Z up, OBJ meters Y up, FBX centimeters (converted), PLY
    /// meters Z up, 3DS inches Z up.
    pub fn default_hints(self) -> (Units, bool) {
        match self {
            Self::Obj => (Units::Meters, true),
            Self::Stl | Self::ThreeMf => (Units::Millimeters, false),
            Self::Fbx => (Units::Centimeters, false),
            Self::Ply => (Units::Meters, false),
            Self::ThreeDs => (Units::Inches, false),
        }
    }
}

/// Reads a mesh file of format `kind`. `read` reads files the model names (MTL
/// libraries, textures); `colors` turns PLY colors into materials.
pub fn read_mesh(
    kind: Kind,
    bytes: &[u8],
    read: ReadFile,
    colors: bool,
) -> Result<(Mesh, Hints), String> {
    match kind {
        Kind::Obj => {
            let text = String::from_utf8_lossy(bytes);
            let mut mesh = crate::mesh_import::parse_obj(&text, |name| {
                read(name).map(|b| String::from_utf8_lossy(&b).into_owned())
            })?;
            obj::add_textures(&text, &mut mesh, read);
            Ok((mesh, Hints::default()))
        }
        Kind::Stl => Ok((crate::mesh_import::parse_stl(bytes)?, Hints::default())),
        Kind::Fbx => fbx::parse(bytes, read),
        Kind::Ply => ply::parse(bytes, colors, read),
        Kind::ThreeMf => threemf::parse(bytes),
        Kind::ThreeDs => threeds::parse(bytes, read),
    }
}

/// Reads a mesh file with its node hierarchy when the format has one (FBX); other
/// formats give a scene without nodes (one flat mesh). See [`scene`].
pub fn read_scene(
    kind: Kind,
    bytes: &[u8],
    read: ReadFile,
    colors: bool,
) -> Result<(scene::Scene, Hints), String> {
    match kind {
        Kind::Fbx => fbx::parse_scene(bytes, read),
        _ => {
            let (mesh, hints) = read_mesh(kind, bytes, read, colors)?;
            Ok((
                scene::Scene {
                    mesh,
                    nodes: Vec::new(),
                },
                hints,
            ))
        }
    }
}

/// Reads a file named inside a model (relative to the model's folder, or absolute):
/// its bytes, or `None` when it cannot be read.
pub type ReadFile<'a> = &'a dyn Fn(&str) -> Option<Vec<u8>>;

/// Tile size given to imported textures (one meter): faces get their own texture
/// position from the file's texture coordinates, so this only sets the size the
/// texture shows at when painted on other faces.
pub const TEXTURE_TILE: [f64; 2] = [1.0 / 0.0254, 1.0 / 0.0254];

/// Whether `data` is an image the model can hold (PNG or JPEG).
pub fn is_image(data: &[u8]) -> bool {
    data.starts_with(b"\x89PNG") || data.starts_with(&[0xff, 0xd8])
}

/// The last component of a path written with either slash.
pub fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// A texture named `path` in a model file: tried as written, with `/` for `\`, then
/// by its base name next to the model. `None` when not found or not PNG/JPEG.
pub fn find_texture(path: &str, read: ReadFile) -> Option<MeshTexture> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let unix = path.replace('\\', "/");
    let base = base_name(path);
    [path, unix.as_str(), base]
        .into_iter()
        .find_map(read)
        .and_then(|data| texture_from_bytes(base, data))
}

/// A mesh texture from image bytes, `None` unless PNG or JPEG.
pub fn texture_from_bytes(name: &str, data: Vec<u8>) -> Option<MeshTexture> {
    is_image(&data).then(|| MeshTexture {
        filename: base_name(name).to_owned(),
        data: Arc::from(data),
        size: TEXTURE_TILE,
    })
}

/// The model material for an imported mesh material. A textured material keeps its
/// color; the app replaces it with the image's average color.
pub fn model_material(m: &MeshMaterial) -> Material {
    Material {
        name: m.name.clone(),
        color: m.color,
        opacity: m.opacity,
        texture: m.texture.as_ref().map(|t| crate::Texture {
            filename: t.filename.clone(),
            data: t.data.clone(),
            width: t.size[0],
            height: t.size[1],
            colorize: false,
        }),
    }
}

/// A material named after its color, `Color_RRGGBB` (PLY and 3MF colors).
pub(crate) fn color_material(color: [u8; 3], opacity: f64) -> MeshMaterial {
    MeshMaterial {
        name: format!("Color_{:02X}{:02X}{:02X}", color[0], color[1], color[2]),
        color,
        opacity,
        texture: None,
    }
}

/// Short decimal text: 6 decimals, trailing zeros and `-0` removed.
pub(crate) fn num(x: f64) -> String {
    let s = format!("{x:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s.into()
    }
}

/// `x` (0..1) as a byte.
pub(crate) fn unit_byte(x: f64) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub(crate) fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Common export choices.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportOptions {
    pub units: Units,
    /// Write back faces as their own reversed triangles with the back material.
    pub two_sided: bool,
    /// Swap Y and Z so Y is up.
    pub y_up: bool,
    /// Write texture images and coordinates.
    pub textures: bool,
    /// File stem of the main file: prefixes texture file names.
    pub base_name: String,
    /// PLY: ASCII instead of binary.
    pub ascii: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            units: Units::Inches,
            two_sided: false,
            y_up: false,
            textures: true,
            base_name: "model".into(),
            ascii: false,
        }
    }
}

/// An export: the main file and the files written next to it (name, bytes).
#[derive(Debug, Clone, Default)]
pub struct Files {
    pub main: Vec<u8>,
    pub others: Vec<(String, Arc<[u8]>)>,
}

/// The formats File > Export > 3D Model writes through this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    Fbx,
    Ply,
    ThreeMf,
    Vrml,
    Kmz,
    ThreeDs,
    Usdz,
    Ifc,
}

impl ExportKind {
    pub const ALL: [ExportKind; 8] = [
        Self::Fbx,
        Self::Ply,
        Self::ThreeMf,
        Self::Vrml,
        Self::Kmz,
        Self::ThreeDs,
        Self::Usdz,
        Self::Ifc,
    ];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Fbx => "fbx",
            Self::Ply => "ply",
            Self::ThreeMf => "3mf",
            Self::Vrml => "wrl",
            Self::Kmz => "kmz",
            Self::ThreeDs => "3ds",
            Self::Usdz => "usdz",
            Self::Ifc => "ifc",
        }
    }
}

/// Encodes the model as `kind` (PLY: binary or ASCII with vertex colors; KMZ:
/// COLLADA inside, in inches whatever `opts.units`).
pub fn export(kind: ExportKind, model: &Model, opts: &ExportOptions) -> Result<Files, String> {
    Ok(match kind {
        ExportKind::Fbx => fbx_export::to_fbx(model, opts),
        ExportKind::Ply => ply::to_ply(
            model,
            &ply::PlyOptions {
                common: opts.clone(),
                ascii: opts.ascii,
                colors: true,
            },
        ),
        ExportKind::ThreeMf => Files {
            main: threemf::to_3mf(model, opts)?,
            others: Vec::new(),
        },
        ExportKind::Vrml => vrml::to_vrml(model, opts),
        ExportKind::Kmz => Files {
            main: kmz::to_kmz(model, &opts.base_name, opts.two_sided)?,
            others: Vec::new(),
        },
        ExportKind::ThreeDs => threeds::to_3ds(model, opts),
        ExportKind::Usdz => usdz::to_usdz(model, opts)?,
        ExportKind::Ifc => ifc_export::to_ifc(model, opts),
    })
}

/// An exported material slot.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SlotMaterial {
    pub name: String,
    pub color: [u8; 3],
    pub opacity: f64,
    /// Texture file name and bytes.
    pub texture: Option<(String, Arc<[u8]>)>,
}

/// One material's triangles with welded corners: positions (output units and axes),
/// texture coordinates and corner indices into both.
#[derive(Debug, Clone, Default)]
pub(crate) struct Group {
    pub positions: Vec<DVec3>,
    pub uvs: Vec<DVec2>,
    /// Corner indices per triangle: position, texture coordinate (when textured).
    pub triangles: Vec<[(usize, usize); 3]>,
}

/// The model's visible triangles as per-material groups in the export's units and
/// axes (welded by exact position within a group).
pub(crate) fn groups(model: &Model, opts: &ExportOptions) -> (Vec<SlotMaterial>, Vec<Group>) {
    let (sides, mats) = mesh_export::sides(model, opts.two_sided, &opts.base_name);
    let scale = opts.units.per_inch();
    let axes = |p: DVec3| {
        if opts.y_up {
            DVec3::new(p.x, p.z, -p.y)
        } else {
            p
        }
    };
    let materials: Vec<SlotMaterial> = mats
        .list
        .iter()
        .map(|m| SlotMaterial {
            name: m.name.clone(),
            color: m.color,
            opacity: m.opacity,
            texture: if opts.textures {
                m.texture.clone()
            } else {
                None
            },
        })
        .collect();
    let mut out: Vec<Group> = vec![Group::default(); materials.len()];
    // Per group: welded positions and texture coordinates by their bits.
    type Welds = (HashMap<[u64; 3], usize>, HashMap<[u64; 2], usize>);
    let mut keys: Vec<Welds> = vec![Default::default(); materials.len()];
    for s in &sides {
        let g = &mut out[s.slot];
        let (pk, tk) = &mut keys[s.slot];
        let textured = materials[s.slot].texture.is_some();
        let mut tri = [(0, 0); 3];
        for (k, corner) in tri.iter_mut().enumerate() {
            let p = axes(s.points[k] * scale);
            let pi = *pk.entry(p.to_array().map(f64::to_bits)).or_insert_with(|| {
                g.positions.push(p);
                g.positions.len() - 1
            });
            let ti = match (textured, s.uv) {
                (true, Some(uv)) => {
                    let t = uv[k];
                    *tk.entry(t.to_array().map(f64::to_bits)).or_insert_with(|| {
                        g.uvs.push(t);
                        g.uvs.len() - 1
                    })
                }
                _ => 0,
            };
            *corner = (pi, ti);
        }
        g.triangles.push(tri);
    }
    // A textured slot whose triangles had no coordinates still needs one.
    for g in &mut out {
        if g.uvs.is_empty() {
            g.uvs.push(DVec2::ZERO);
        }
    }
    (materials, out)
}

/// Texture files of `materials`, each once.
pub(crate) fn texture_files(materials: &[SlotMaterial]) -> Vec<(String, Arc<[u8]>)> {
    let mut out: Vec<(String, Arc<[u8]>)> = Vec::new();
    for (name, data) in materials.iter().filter_map(|m| m.texture.as_ref()) {
        if !out.iter().any(|(n, _)| n == name) {
            out.push((name.clone(), data.clone()));
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::mesh_import::{MeshOptions, to_entities};

    /// A 10" cube model painted with one red material on the top face.
    pub(crate) fn painted_cube() -> Model {
        let mut m = Model::default();
        crate::mesh_export::tests::cube(&mut m.root, DVec3::splat(10.0));
        m.materials.push(Material {
            name: "Red".into(),
            color: [255, 0, 0],
            opacity: 1.0,
            texture: None,
        });
        let top = m
            .root
            .faces
            .iter()
            .find(|(_, f)| f.normal.z.abs() > 0.9 && m.root.loop_points(&f.outer)[0].z > 5.0)
            .map(|(id, _)| id)
            .expect("top face");
        m.root.faces[top].attrs.material = Some(0);
        m
    }

    /// Faces, vertices and bounding box of a mesh imported with merging.
    pub(crate) fn summary(mesh: &Mesh, inches_per_unit: f64, y_up: bool) -> (usize, usize, DVec3) {
        let ids: Vec<usize> = (0..mesh.materials.len()).collect();
        let ents = to_entities(
            mesh,
            &MeshOptions {
                inches_per_unit,
                y_up,
                merge_coplanar: true,
            },
            &ids,
        );
        let max = ents
            .vertices
            .values()
            .map(|v| v.pos)
            .fold(DVec3::splat(f64::MIN), DVec3::max);
        (ents.faces.len(), ents.vertices.len(), max)
    }

    #[test]
    fn groups_weld_per_material() {
        let m = painted_cube();
        let (mats, groups) = groups(&m, &ExportOptions::default());
        assert_eq!(mats.len(), 2);
        let tris: usize = groups.iter().map(|g| g.triangles.len()).sum();
        assert_eq!(tris, 12);
        // The red top: 2 triangles over 4 corners.
        let red = mats.iter().position(|x| x.name == "Red").unwrap();
        assert_eq!(groups[red].triangles.len(), 2);
        assert_eq!(groups[red].positions.len(), 4);
    }

    #[test]
    fn texture_lookup_tries_base_name() {
        let png = b"\x89PNG....".to_vec();
        let read = |p: &str| (p == "wood.png").then(|| png.clone());
        let t = find_texture("C:\\maps\\wood.png", &read).unwrap();
        assert_eq!(t.filename, "wood.png");
        assert!(find_texture("missing.png", &read).is_none());
        assert!(texture_from_bytes("x.tga", vec![0, 0, 2]).is_none());
        assert_eq!(num(-0.0000001), "0");
        assert_eq!(num(2.5), "2.5");
    }
}
