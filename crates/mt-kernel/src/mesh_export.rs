//! File > Export > 3D Model: Wavefront OBJ (with an MTL file and texture images),
//! binary or ASCII STL and COLLADA (.dae). Pure functions returning file contents; the app
//! writes them.
//!
//! - Geometry is flattened: groups and components are expanded with their transforms
//!   (nested too), faces are triangulated with [`crate::tessellate`]. Hidden faces and
//!   instances and entities on hidden tags are left out; edges are not exported.
//! - A face without a material takes its group's or component's material, else the
//!   style's front (back) color, exported as material `FrontColor` (`BackColor`).
//!   SketchUp's exporters use those names too (from memory, unverified).
//! - Texture coordinates come from each face's texture mapping ([`crate::texture`]), in
//!   repeats (v up); texture images are written as stored (colorized materials keep
//!   the original image colors).
//! - Units: the kernel works in inches; OBJ and STL are scaled to the chosen [`Units`]
//!   (neither format records units). COLLADA keeps inches and says so in `<unit>`.
//! - OBJ is written Y up (SketchUp's "Swap YZ coordinates" option, on by default from
//!   memory, unverified) unless asked for Z up; STL and COLLADA are Z up.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use glam::{DMat3, DMat4, DVec2, DVec3};

use crate::entities::{DefinitionId, Entities, MaterialId};
use crate::model::Model;
use crate::texture;

/// Length unit of exported coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Units {
    Inches,
    Feet,
    #[default]
    Millimeters,
    Centimeters,
    Meters,
}

impl Units {
    pub const ALL: [Units; 5] = [
        Self::Inches,
        Self::Feet,
        Self::Millimeters,
        Self::Centimeters,
        Self::Meters,
    ];

    /// Output units per inch.
    pub fn per_inch(self) -> f64 {
        match self {
            Self::Inches => 1.0,
            Self::Feet => 1.0 / 12.0,
            Self::Millimeters => 25.4,
            Self::Centimeters => 2.54,
            Self::Meters => 0.0254,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Inches => "Inches",
            Self::Feet => "Feet",
            Self::Millimeters => "Millimeters",
            Self::Centimeters => "Centimeters",
            Self::Meters => "Meters",
        }
    }
}

/// One exported triangle in world space (inches), counter-clockwise around `normal`.
#[derive(Debug, Clone, PartialEq)]
pub struct Triangle {
    pub points: [DVec3; 3],
    /// Unit front normal.
    pub normal: DVec3,
    pub material: Option<MaterialId>,
    pub back_material: Option<MaterialId>,
    /// Texture coordinates (repeats, v up) of the front and back side, when that side's
    /// material has a texture.
    pub uv: Option<[DVec2; 3]>,
    pub back_uv: Option<[DVec2; 3]>,
}

/// Every visible face of the model as world-space triangles.
pub fn triangles(model: &Model) -> Vec<Triangle> {
    let mut out = Vec::new();
    let mut stack = Vec::new();
    collect(
        model,
        &model.root,
        DMat4::IDENTITY,
        None,
        &mut stack,
        &mut out,
    );
    out
}

fn layer_visible(model: &Model, layer: Option<usize>) -> bool {
    layer.is_none_or(|l| model.layers.get(l).is_none_or(|x| x.visible))
}

/// The visible faces of `ents` itself (not of its groups and components) as
/// triangles transformed by `xf`; faces without a material take `inherited`.
pub(crate) fn face_triangles(
    model: &Model,
    ents: &Entities,
    xf: DMat4,
    inherited: Option<MaterialId>,
    out: &mut Vec<Triangle>,
) {
    let normal_xf = DMat3::from_mat4(xf).inverse().transpose();
    for (face_id, face) in &ents.faces {
        if face.attrs.hidden || !layer_visible(model, face.attrs.layer) {
            continue;
        }
        let normal = (normal_xf * face.normal).normalize_or_zero();
        if normal == DVec3::ZERO {
            continue;
        }
        let material = face.attrs.material.or(inherited);
        let back_material = face.attrs.back_material.or(inherited);
        // Texture mapping per side, evaluated at the face's own (definition) points,
        // shifted by whole repeats so the numbers stay small.
        let side = |m: Option<MaterialId>, back: bool| {
            let map = texture::face_mapping(face, back, model.materials.get(m?)?)?;
            let first = ents.loop_points(&face.outer).first().copied()?;
            Some((map, texture::uv_at(&map, first).floor()))
        };
        let (front_map, back_map) = (side(material, false), side(back_material, true));
        for local in crate::tessellate::triangulate(ents, face_id) {
            let world = local.map(|p| xf.transform_point3(p));
            let [a, b, c] = world;
            if (b - a).cross(c - a).length_squared() < 1e-18 {
                continue;
            }
            // A mirroring transform flips the winding; keep it counter-clockwise.
            let order = if (b - a).cross(c - a).dot(normal) < 0.0 {
                [0, 2, 1]
            } else {
                [0, 1, 2]
            };
            let uv = |m: &Option<(mt_skp::uv::UvMapping, DVec2)>| {
                m.as_ref()
                    .map(|(map, shift)| order.map(|k| texture::uv_at(map, local[k]) - *shift))
            };
            out.push(Triangle {
                points: order.map(|k| world[k]),
                normal,
                material,
                back_material,
                uv: uv(&front_map),
                back_uv: uv(&back_map),
            });
        }
    }
}

fn collect(
    model: &Model,
    ents: &Entities,
    xf: DMat4,
    inherited: Option<MaterialId>,
    stack: &mut Vec<DefinitionId>,
    out: &mut Vec<Triangle>,
) {
    face_triangles(model, ents, xf, inherited, out);
    for inst in &ents.instances {
        if inst.hidden || !layer_visible(model, inst.layer) {
            continue;
        }
        let Some(def) = model.definitions.get(inst.definition) else {
            continue;
        };
        // A definition containing itself would recurse forever.
        if stack.contains(&inst.definition) {
            continue;
        }
        stack.push(inst.definition);
        collect(
            model,
            &def.entities,
            xf * inst.transform,
            inst.material.or(inherited),
            stack,
            out,
        );
        stack.pop();
    }
}

/// A triangle side as exported: corners, normal, material slot and texture coordinates.
pub(crate) struct Side {
    pub(crate) points: [DVec3; 3],
    pub(crate) normal: DVec3,
    /// Index into [`Materials::list`].
    pub(crate) slot: usize,
    pub(crate) uv: Option<[DVec2; 3]>,
}

/// An exported material.
pub(crate) struct ExportMaterial {
    pub(crate) name: String,
    pub(crate) color: [u8; 3],
    pub(crate) opacity: f64,
    /// File name of the texture image written next to the model, and its bytes.
    pub(crate) texture: Option<(String, Arc<[u8]>)>,
}

/// The materials the exported sides use, in first-use order.
pub(crate) struct Materials {
    pub(crate) list: Vec<ExportMaterial>,
    /// Model material (or `None` front / back default) to slot.
    slots: HashMap<(Option<MaterialId>, bool), usize>,
    /// Sanitized names given out, with their use counts.
    used_names: HashMap<String, usize>,
}

/// File extension of image bytes (PNG, JPEG, BMP; PNG when unknown).
pub(crate) fn image_extension(data: &[u8]) -> &'static str {
    if data.starts_with(&[0xff, 0xd8]) {
        "jpg"
    } else if data.starts_with(b"BM") {
        "bmp"
    } else {
        "png"
    }
}

/// A name usable in OBJ/MTL statements and file names: letters, digits, `-` and `_`.
fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() { "material".into() } else { s }
}

impl Materials {
    pub(crate) fn new() -> Self {
        Self {
            list: Vec::new(),
            slots: HashMap::new(),
            used_names: HashMap::new(),
        }
    }

    /// The slot of model material `m` (`None`: the style's front or back color),
    /// added on first use. `base` prefixes texture file names.
    fn slot(&mut self, model: &Model, m: Option<MaterialId>, back: bool, base: &str) -> usize {
        let key = (m, back && m.is_none());
        if let Some(&s) = self.slots.get(&key) {
            return s;
        }
        let mat = m.and_then(|i| model.materials.get(i));
        let (raw, color, opacity) = match mat {
            Some(x) => (x.name.clone(), x.color, x.opacity),
            None if key.1 => ("BackColor".into(), model.back_color, 1.0),
            None => ("FrontColor".into(), model.front_color, 1.0),
        };
        let mut name = sanitize(&raw);
        let n = self.used_names.entry(name.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            name = format!("{name}_{n}");
        }
        let texture = mat.and_then(|x| x.texture.as_ref()).map(|t| {
            (
                format!("{base}_{name}.{}", image_extension(&t.data)),
                t.data.clone(),
            )
        });
        self.list.push(ExportMaterial {
            name,
            color,
            opacity: opacity.clamp(0.0, 1.0),
            texture,
        });
        self.slots.insert(key, self.list.len() - 1);
        self.list.len() - 1
    }
}

/// Export sides of `tris` (both sides of each triangle with `two_sided`, the back one
/// reversed), their materials added to `mats`. `base` prefixes texture file names.
pub(crate) fn tri_sides(
    model: &Model,
    tris: Vec<Triangle>,
    two_sided: bool,
    base: &str,
    mats: &mut Materials,
) -> Vec<Side> {
    let mut out = Vec::new();
    for t in tris {
        out.push(Side {
            points: t.points,
            normal: t.normal,
            slot: mats.slot(model, t.material, false, base),
            uv: t.uv,
        });
        if two_sided {
            let [a, b, c] = t.points;
            out.push(Side {
                points: [a, c, b],
                normal: -t.normal,
                slot: mats.slot(model, t.back_material, true, base),
                uv: t.back_uv.map(|[x, y, z]| [x, z, y]),
            });
        }
    }
    out
}

/// The sides to export (both sides of each triangle with `two_sided`, the back one
/// reversed), with their materials. `base` prefixes texture file names.
pub(crate) fn sides(model: &Model, two_sided: bool, base: &str) -> (Vec<Side>, Materials) {
    let mut mats = Materials::new();
    let out = tri_sides(model, triangles(model), two_sided, base, &mut mats);
    (out, mats)
}

/// OBJ export choices.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjOptions {
    pub units: Units,
    /// Write back faces as their own reversed triangles with the back material.
    pub two_sided: bool,
    /// Swap Y and Z so Y is up (most OBJ readers expect it).
    pub y_up: bool,
    /// File stem of the `.obj`: names the `.mtl` and prefixes texture files.
    pub base_name: String,
}

impl Default for ObjOptions {
    fn default() -> Self {
        Self {
            units: Units::Inches,
            two_sided: false,
            y_up: true,
            base_name: "model".into(),
        }
    }
}

/// The files of an OBJ export: `<base>.obj`, `<base>.mtl` and texture images (file
/// name, bytes), all in one folder.
#[derive(Debug, Clone)]
pub struct ObjFiles {
    pub obj: String,
    pub mtl: String,
    pub textures: Vec<(String, Arc<[u8]>)>,
}

fn fmt(x: f64) -> String {
    // Short but exact enough: 6 decimals of the output unit, without "-0".
    let s = format!("{x:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s.into()
    }
}

/// Encodes the model as OBJ + MTL.
pub fn to_obj(model: &Model, opts: &ObjOptions) -> ObjFiles {
    let (sides, mats) = sides(model, opts.two_sided, &opts.base_name);
    let scale = opts.units.per_inch();
    let axes = |p: DVec3| {
        if opts.y_up {
            DVec3::new(p.x, p.z, -p.y)
        } else {
            p
        }
    };
    let mut obj = String::new();
    let _ = writeln!(obj, "# Exported by meshtard ({})", opts.units.name());
    let _ = writeln!(obj, "mtllib {}.mtl", opts.base_name);
    let key = |p: DVec3| p.to_array().map(f64::to_bits);
    let mut positions: HashMap<[u64; 3], usize> = HashMap::new();
    let mut normals: HashMap<[u64; 3], usize> = HashMap::new();
    let mut uvs: HashMap<[u64; 2], usize> = HashMap::new();
    let (mut v_out, mut vn_out, mut vt_out, mut f_out) =
        (String::new(), String::new(), String::new(), String::new());
    // Faces grouped by material, in material order.
    let mut by_slot: Vec<Vec<&Side>> = vec![Vec::new(); mats.list.len()];
    for s in &sides {
        by_slot[s.slot].push(s);
    }
    for (slot, group) in by_slot.iter().enumerate() {
        if group.is_empty() {
            continue;
        }
        let _ = writeln!(f_out, "usemtl {}", mats.list[slot].name);
        for s in group {
            let n = axes(s.normal);
            let next = normals.len() + 1;
            let ni = *normals.entry(key(n)).or_insert_with(|| {
                let _ = writeln!(vn_out, "vn {} {} {}", fmt(n.x), fmt(n.y), fmt(n.z));
                next
            });
            let mut corners = Vec::with_capacity(3);
            for k in 0..3 {
                let p = axes(s.points[k] * scale);
                let next = positions.len() + 1;
                let vi = *positions.entry(key(p)).or_insert_with(|| {
                    let _ = writeln!(v_out, "v {} {} {}", fmt(p.x), fmt(p.y), fmt(p.z));
                    next
                });
                let ti = s.uv.map(|uv| {
                    let t = uv[k];
                    let next = uvs.len() + 1;
                    *uvs.entry(t.to_array().map(f64::to_bits))
                        .or_insert_with(|| {
                            let _ = writeln!(vt_out, "vt {} {}", fmt(t.x), fmt(t.y));
                            next
                        })
                });
                corners.push(match ti {
                    Some(ti) => format!("{vi}/{ti}/{ni}"),
                    None => format!("{vi}//{ni}"),
                });
            }
            let _ = writeln!(f_out, "f {}", corners.join(" "));
        }
    }
    obj.push_str(&v_out);
    obj.push_str(&vt_out);
    obj.push_str(&vn_out);
    obj.push_str(&f_out);

    let mut mtl = String::from("# Exported by meshtard\n");
    let mut textures = Vec::new();
    for m in &mats.list {
        let c = m.color.map(|x| f64::from(x) / 255.0);
        let _ = writeln!(mtl, "\nnewmtl {}", m.name);
        let _ = writeln!(mtl, "Ka 0 0 0");
        let _ = writeln!(mtl, "Kd {} {} {}", fmt(c[0]), fmt(c[1]), fmt(c[2]));
        let _ = writeln!(mtl, "Ks 0 0 0");
        let _ = writeln!(mtl, "d {}", fmt(m.opacity));
        let _ = writeln!(mtl, "illum 1");
        if let Some((file, data)) = &m.texture {
            let _ = writeln!(mtl, "map_Kd {file}");
            textures.push((file.clone(), data.clone()));
        }
    }
    ObjFiles { obj, mtl, textures }
}

/// Encodes the model as binary STL in `units` (front sides only; STL has no colors).
pub fn to_stl(model: &Model, units: Units) -> Vec<u8> {
    let tris = triangles(model);
    let scale = units.per_inch();
    let mut out = Vec::with_capacity(84 + tris.len() * 50);
    let mut header = format!("meshtard STL export, units: {}", units.name()).into_bytes();
    header.resize(80, b' ');
    out.extend_from_slice(&header);
    out.extend_from_slice(&(tris.len() as u32).to_le_bytes());
    let put = |out: &mut Vec<u8>, v: DVec3| {
        for x in v.to_array() {
            out.extend_from_slice(&(x as f32).to_le_bytes());
        }
    };
    for t in &tris {
        put(&mut out, t.normal);
        for p in t.points {
            put(&mut out, p * scale);
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    out
}

/// Encodes the model as ASCII STL in `units`: one `solid` named `name` (spaces made
/// underscores), facets as in [`to_stl`].
pub fn to_stl_ascii(model: &Model, units: Units, name: &str) -> String {
    let tris = triangles(model);
    let scale = units.per_inch();
    let name: String = name
        .chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .collect();
    let name = if name.is_empty() {
        "model".into()
    } else {
        name
    };
    let v = |p: DVec3| format!("{:e} {:e} {:e}", p.x as f32, p.y as f32, p.z as f32);
    let mut out = format!("solid {name}\n");
    for t in &tris {
        let _ = writeln!(out, "  facet normal {}", v(t.normal));
        out.push_str("    outer loop\n");
        for p in t.points {
            let _ = writeln!(out, "      vertex {}", v(p * scale));
        }
        out.push_str("    endloop\n  endfacet\n");
    }
    let _ = writeln!(out, "endsolid {name}");
    out
}

/// COLLADA export choices.
#[derive(Debug, Clone, PartialEq)]
pub struct DaeOptions {
    /// Write back faces as their own reversed triangles with the back material
    /// (SketchUp's "Export two-sided faces", on by default from memory, unverified).
    pub two_sided: bool,
    /// File stem of the `.dae`: prefixes texture files.
    pub base_name: String,
}

impl Default for DaeOptions {
    fn default() -> Self {
        Self {
            two_sided: true,
            base_name: "model".into(),
        }
    }
}

/// The files of a COLLADA export: the document and texture images.
#[derive(Debug, Clone)]
pub struct DaeFiles {
    pub dae: String,
    pub textures: Vec<(String, Arc<[u8]>)>,
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Encodes the model as a COLLADA 1.4.1 document: one mesh with a `<triangles>` block
/// per material, in inches, Z up.
pub fn to_dae(model: &Model, opts: &DaeOptions) -> DaeFiles {
    let (sides, mats) = sides(model, opts.two_sided, &opts.base_name);
    let mut d = String::new();
    d.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    d.push_str(
        "<COLLADA xmlns=\"http://www.collada.org/2005/11/COLLADASchema\" version=\"1.4.1\">\n",
    );
    d.push_str(
        "  <asset>\n    <contributor><authoring_tool>meshtard</authoring_tool></contributor>\n",
    );
    d.push_str(
        "    <unit meter=\"0.0254\" name=\"inch\"/>\n    <up_axis>Z_UP</up_axis>\n  </asset>\n",
    );

    let mut textures = Vec::new();
    d.push_str("  <library_images>\n");
    for (i, m) in mats.list.iter().enumerate() {
        if let Some((file, data)) = &m.texture {
            let _ = writeln!(
                d,
                "    <image id=\"image{i}\"><init_from>{}</init_from></image>",
                xml_escape(file)
            );
            textures.push((file.clone(), data.clone()));
        }
    }
    d.push_str("  </library_images>\n  <library_effects>\n");
    for (i, m) in mats.list.iter().enumerate() {
        let c = m.color.map(|x| f64::from(x) / 255.0);
        let _ = writeln!(d, "    <effect id=\"effect{i}\">\n      <profile_COMMON>");
        let diffuse = if m.texture.is_some() {
            let _ = writeln!(
                d,
                "        <newparam sid=\"surface{i}\"><surface type=\"2D\"><init_from>image{i}</init_from></surface></newparam>\n        <newparam sid=\"sampler{i}\"><sampler2D><source>surface{i}</source></sampler2D></newparam>"
            );
            format!("<texture texture=\"sampler{i}\" texcoord=\"UVSET0\"/>")
        } else {
            format!("<color>{} {} {} 1</color>", fmt(c[0]), fmt(c[1]), fmt(c[2]))
        };
        let _ = writeln!(
            d,
            "        <technique sid=\"COMMON\"><lambert><diffuse>{diffuse}</diffuse><transparent opaque=\"A_ONE\"><color>1 1 1 {}</color></transparent><transparency><float>1</float></transparency></lambert></technique>\n      </profile_COMMON>\n    </effect>",
            fmt(m.opacity)
        );
    }
    d.push_str("  </library_effects>\n  <library_materials>\n");
    for (i, m) in mats.list.iter().enumerate() {
        let _ = writeln!(
            d,
            "    <material id=\"material{i}\" name=\"{}\"><instance_effect url=\"#effect{i}\"/></material>",
            xml_escape(&m.name)
        );
    }
    d.push_str("  </library_materials>\n");

    // Unshared corners: corner k of side s is index 3 s + k in every source.
    let join = |vals: &mut dyn Iterator<Item = f64>| -> String {
        vals.map(fmt).collect::<Vec<_>>().join(" ")
    };
    let n = sides.len();
    let positions = join(
        &mut sides
            .iter()
            .flat_map(|s| s.points)
            .flat_map(|p| p.to_array()),
    );
    let normals = join(
        &mut sides
            .iter()
            .flat_map(|s| [s.normal; 3])
            .flat_map(|p| p.to_array()),
    );
    let uvs = join(
        &mut sides
            .iter()
            .flat_map(|s| s.uv.unwrap_or([DVec2::ZERO; 3]))
            .flat_map(|p| p.to_array()),
    );
    d.push_str("  <library_geometries>\n    <geometry id=\"mesh\">\n      <mesh>\n");
    let _ = writeln!(
        d,
        "        <source id=\"mesh-positions\"><float_array id=\"mesh-positions-array\" count=\"{}\">{positions}</float_array><technique_common><accessor source=\"#mesh-positions-array\" count=\"{}\" stride=\"3\"><param name=\"X\" type=\"float\"/><param name=\"Y\" type=\"float\"/><param name=\"Z\" type=\"float\"/></accessor></technique_common></source>",
        n * 9,
        n * 3
    );
    let _ = writeln!(
        d,
        "        <source id=\"mesh-normals\"><float_array id=\"mesh-normals-array\" count=\"{}\">{normals}</float_array><technique_common><accessor source=\"#mesh-normals-array\" count=\"{}\" stride=\"3\"><param name=\"X\" type=\"float\"/><param name=\"Y\" type=\"float\"/><param name=\"Z\" type=\"float\"/></accessor></technique_common></source>",
        n * 9,
        n * 3
    );
    let _ = writeln!(
        d,
        "        <source id=\"mesh-uvs\"><float_array id=\"mesh-uvs-array\" count=\"{}\">{uvs}</float_array><technique_common><accessor source=\"#mesh-uvs-array\" count=\"{}\" stride=\"2\"><param name=\"S\" type=\"float\"/><param name=\"T\" type=\"float\"/></accessor></technique_common></source>",
        n * 6,
        n * 3
    );
    d.push_str("        <vertices id=\"mesh-vertices\"><input semantic=\"POSITION\" source=\"#mesh-positions\"/></vertices>\n");
    for slot in 0..mats.list.len() {
        let idx: Vec<usize> = (0..n).filter(|&s| sides[s].slot == slot).collect();
        if idx.is_empty() {
            continue;
        }
        let p = idx
            .iter()
            .flat_map(|&s| (0..3).map(move |k| 3 * s + k))
            .map(|i| format!("{i} {i} {i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(
            d,
            "        <triangles material=\"material{slot}\" count=\"{}\"><input semantic=\"VERTEX\" source=\"#mesh-vertices\" offset=\"0\"/><input semantic=\"NORMAL\" source=\"#mesh-normals\" offset=\"1\"/><input semantic=\"TEXCOORD\" source=\"#mesh-uvs\" offset=\"2\" set=\"0\"/><p>{p}</p></triangles>",
            idx.len()
        );
    }
    d.push_str("      </mesh>\n    </geometry>\n  </library_geometries>\n");
    d.push_str("  <library_visual_scenes>\n    <visual_scene id=\"scene\">\n      <node id=\"model\" name=\"model\">\n        <instance_geometry url=\"#mesh\"><bind_material><technique_common>\n");
    for i in 0..mats.list.len() {
        let _ = writeln!(
            d,
            "          <instance_material symbol=\"material{i}\" target=\"#material{i}\"><bind_vertex_input semantic=\"UVSET0\" input_semantic=\"TEXCOORD\" input_set=\"0\"/></instance_material>"
        );
    }
    d.push_str("        </technique_common></bind_material></instance_geometry>\n      </node>\n    </visual_scene>\n  </library_visual_scenes>\n");
    d.push_str("  <scene><instance_visual_scene url=\"#scene\"/></scene>\n</COLLADA>\n");
    DaeFiles { dae: d, textures }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::entities::Instance;
    use crate::model::{Definition, Material};

    /// A closed box from (0,0,0) to `size`, built with the drawing operations.
    pub(crate) fn cube(ents: &mut Entities, size: DVec3) {
        let p = |x: f64, y: f64, z: f64| DVec3::new(x * size.x, y * size.y, z * size.z);
        let corners = [p(0., 0., 0.), p(1., 0., 0.), p(1., 1., 0.), p(0., 1., 0.)];
        for i in 0..4 {
            let (a, b) = (corners[i], corners[(i + 1) % 4]);
            crate::add_line(ents, a, b);
            crate::add_line(ents, a + DVec3::Z * size.z, b + DVec3::Z * size.z);
            crate::add_line(ents, a, a + DVec3::Z * size.z);
        }
    }

    fn cube_model() -> Model {
        let mut m = Model::default();
        cube(&mut m.root, DVec3::splat(10.0));
        m
    }

    #[test]
    fn cube_gives_twelve_triangles() {
        let m = cube_model();
        assert_eq!(m.root.faces.len(), 6);
        let tris = triangles(&m);
        assert_eq!(tris.len(), 12);
        // Every triangle winds around its normal.
        for t in &tris {
            let [a, b, c] = t.points;
            assert!((b - a).cross(c - a).dot(t.normal) > 0.0);
        }
    }

    #[test]
    fn stl_is_binary_with_count_and_millimeters() {
        let m = cube_model();
        let stl = to_stl(&m, Units::Millimeters);
        assert_eq!(stl.len(), 84 + 12 * 50);
        assert_eq!(u32::from_le_bytes(stl[80..84].try_into().unwrap()), 12);
        // Largest coordinate: 10" = 254 mm.
        let max = (0..12)
            .flat_map(|t| (0..9).map(move |k| 84 + t * 50 + 12 + k * 4))
            .map(|o| f32::from_le_bytes(stl[o..o + 4].try_into().unwrap()))
            .fold(0.0f32, f32::max);
        assert!((max - 254.0).abs() < 1e-3, "{max}");
    }

    #[test]
    fn ascii_stl_reads_back() {
        let m = cube_model();
        let text = to_stl_ascii(&m, Units::Millimeters, "my cube");
        assert!(text.starts_with("solid my_cube\n"));
        assert!(text.trim_end().ends_with("endsolid my_cube"));
        let back = crate::mesh_import::parse_stl(text.as_bytes()).unwrap();
        assert_eq!(back.polygons.len(), 12);
        let max = back
            .positions
            .iter()
            .map(|p| p.max_element())
            .fold(0.0, f64::max);
        assert!((max - 254.0).abs() < 1e-3, "{max}");
    }

    #[test]
    fn obj_counts_parse_back() {
        let m = cube_model();
        let files = to_obj(&m, &ObjOptions::default());
        let count = |p: &str| files.obj.lines().filter(|l| l.starts_with(p)).count();
        assert_eq!(count("v "), 8);
        // One normal per face direction (faces drawn with lines may face either way).
        let mut dirs: Vec<[i64; 3]> = triangles(&m)
            .iter()
            .map(|t| t.normal.round().as_i64vec3().to_array())
            .collect();
        dirs.sort();
        dirs.dedup();
        assert_eq!(count("vn "), dirs.len());
        assert_eq!(count("f "), 12);
        assert_eq!(count("usemtl FrontColor"), 1);
        assert!(files.mtl.contains("newmtl FrontColor"));
        // Every face index is in range.
        for l in files.obj.lines().filter(|l| l.starts_with("f ")) {
            for c in l[2..].split(' ') {
                let v: usize = c.split('/').next().unwrap().parse().unwrap();
                assert!((1..=8).contains(&v));
            }
        }
        // It reads back through the importer.
        let mesh = crate::mesh_import::parse_obj(&files.obj, |_| None).unwrap();
        assert_eq!(mesh.positions.len(), 8);
        assert_eq!(mesh.polygons.len(), 12);
    }

    #[test]
    fn components_are_flattened_with_their_transform_and_hidden_skipped() {
        let mut m = Model::default();
        let mut def = Entities::new();
        cube(&mut def, DVec3::splat(1.0));
        let d = m.definitions.insert(Definition {
            name: "Box".into(),
            entities: def,
            ..Default::default()
        });
        m.materials.push(Material {
            name: "Red Brick".into(),
            color: [200, 0, 0],
            opacity: 1.0,
            texture: None,
        });
        for (x, hidden) in [(0.0, false), (5.0, false), (10.0, true)] {
            m.root.instances.push(Instance {
                definition: d,
                transform: DMat4::from_translation(DVec3::new(x, 0.0, 0.0)),
                name: String::new(),
                hidden,
                locked: false,
                layer: None,
                material: Some(0),
                attrs: Default::default(),
            });
        }
        let tris = triangles(&m);
        assert_eq!(tris.len(), 24);
        let max_x = tris
            .iter()
            .flat_map(|t| t.points)
            .map(|p| p.x)
            .fold(0.0, f64::max);
        assert!((max_x - 6.0).abs() < 1e-9);
        // The instance's material paints the faces.
        assert!(tris.iter().all(|t| t.material == Some(0)));
        let files = to_obj(&m, &ObjOptions::default());
        assert!(files.obj.contains("usemtl Red_Brick"));
        assert!(files.mtl.contains("Kd 0.784314 0 0"));
        // Two-sided doubles the triangles.
        let two = to_obj(
            &m,
            &ObjOptions {
                two_sided: true,
                ..Default::default()
            },
        );
        assert_eq!(two.obj.lines().filter(|l| l.starts_with("f ")).count(), 48);
    }

    #[test]
    fn textures_get_uvs_and_files() {
        let mut m = cube_model();
        m.materials.push(Material {
            name: "Wood".into(),
            color: [150, 100, 50],
            opacity: 1.0,
            texture: Some(crate::Texture {
                filename: "wood.jpg".into(),
                data: Arc::from(&[0xffu8, 0xd8, 0, 0][..]),
                width: 5.0,
                height: 5.0,
                colorize: false,
            }),
        });
        for f in m.root.faces.values_mut() {
            f.attrs.material = Some(0);
        }
        let files = to_obj(
            &m,
            &ObjOptions {
                base_name: "box".into(),
                ..Default::default()
            },
        );
        assert_eq!(files.textures.len(), 1);
        assert_eq!(files.textures[0].0, "box_Wood.jpg");
        assert!(files.mtl.contains("map_Kd box_Wood.jpg"));
        assert!(files.obj.lines().any(|l| l.starts_with("vt ")));
        // 10" faces with a 5" tile span two repeats.
        let max_u = files
            .obj
            .lines()
            .filter_map(|l| l.strip_prefix("vt "))
            .flat_map(|l| l.split(' ').map(|x| x.parse::<f64>().unwrap()))
            .fold(0.0, f64::max);
        assert!((max_u - 2.0).abs() < 1e-9, "{max_u}");
        let dae = to_dae(&m, &DaeOptions::default());
        assert!(dae.dae.contains("<init_from>model_Wood.jpg</init_from>"));
    }

    #[test]
    fn dae_has_matching_counts() {
        let m = cube_model();
        let dae = to_dae(
            &m,
            &DaeOptions {
                two_sided: false,
                ..Default::default()
            },
        )
        .dae;
        assert!(dae.contains("count=\"108\""), "36 corners x 3 floats");
        assert!(dae.contains("<triangles material=\"material0\" count=\"12\">"));
        // The p list has 12 triangles x 3 corners x 3 inputs.
        let p = dae
            .split("<p>")
            .nth(1)
            .unwrap()
            .split("</p>")
            .next()
            .unwrap();
        assert_eq!(p.split(' ').count(), 108);
        assert_eq!(dae.matches("<COLLADA").count(), 1);
    }
}
