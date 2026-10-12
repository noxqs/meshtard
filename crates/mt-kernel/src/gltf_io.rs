//! glTF 2.0 import and export (`.gltf` and binary `.glb`).
//!
//! Import (File > Import, also meant for the model library):
//! - Parsed with the `gltf` crate (MIT/Apache-2.0). Buffers and images come from the
//!   GLB binary chunk, `data:` URIs or files next to the model (through a callback).
//! - Every mesh becomes a definition: a component when several nodes use it, else a
//!   group. Node transforms are flattened: each node with a mesh becomes one instance
//!   with its world transform; empty nodes and the node hierarchy are not kept.
//! - Triangles (also strips and fans) become faces as in OBJ/STL import
//!   ([`crate::mesh_import`]), coplanar neighbors with the same material and texture
//!   position merged; edges between faces less than [`SOFTEN_ANGLE_DEG`] apart are
//!   softened and smoothed.
//! - Materials: base color factor (linear in glTF, sRGB in the model), its alpha as
//!   opacity when the alpha mode is not opaque, and the base color texture (PNG or
//!   JPEG) with each face's texture position fitted to the file's texture coordinates
//!   (first texture coordinate set of the texture). Other PBR parts are ignored.
//! - Units: glTF is meters, Y up; the model is inches, Z up.
//!
//! Export (File > Export > 3D Model, next to OBJ/STL/COLLADA in
//! [`crate::mesh_export`]):
//! - The group and component hierarchy is kept: one node per instance with its
//!   transform, one mesh per definition (and per inherited material); each material is
//!   one primitive. Hidden entities and hidden tags are left out; edges are not
//!   exported.
//! - Materials are unlit-looking PBR: base color (white times the texture when
//!   textured), metallic 0, roughness 1, `BLEND` when translucent, double-sided unless
//!   back faces are written as their own triangles. BMP textures are dropped (glTF
//!   allows PNG and JPEG only).
//! - `.glb` embeds everything; `.gltf` embeds the buffer as a base64 `data:` URI.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use glam::{DMat4, DVec2, DVec3, DVec4};

use crate::entities::{DefinitionId, Entities, Instance, MaterialId};
use crate::mesh_export::{self, Materials, Side};
use crate::mesh_import::{self, Mesh, MeshMaterial, MeshOptions, MeshTexture, Polygon};
use crate::model::{Definition, Material, Model};
use crate::texture::Texture;

/// Inches per meter.
pub const INCHES_PER_METER: f64 = 1.0 / 0.0254;

/// Edges between imported faces whose normals differ by less than this are soft and
/// smooth (SketchUp's Soften Edges default angle, 20 degrees, from memory).
pub const SOFTEN_ANGLE_DEG: f64 = 20.0;

/// Tile size (inches) given to imported textures: one meter. Faces get their own
/// texture positions from the file, so this only matters when the material is painted
/// on other faces. The app replaces it with the image's aspect ratio.
pub const TEXTURE_TILE: f64 = INCHES_PER_METER;

/// Application-specific vertex attribute (glTF allows names starting with `_`) holding
/// what f32 `POSITION` rounded off: `POSITION + _POSITION_LO` is the position to about
/// f64 precision. Other readers ignore it.
pub const POSITION_LO: &str = "_POSITION_LO";

/// glTF space (meters, Y up) to model space (inches, Z up).
pub fn gltf_to_model() -> DMat4 {
    let k = INCHES_PER_METER;
    DMat4::from_cols(
        DVec4::new(k, 0.0, 0.0, 0.0),
        DVec4::new(0.0, 0.0, k, 0.0),
        DVec4::new(0.0, -k, 0.0, 0.0),
        DVec4::W,
    )
}

fn srgb_to_linear(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f64) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u8
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Decodes standard or URL-safe base64 (padding and whitespace ignored).
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// The bytes of a `data:` URI, or `None` when `uri` is not one.
fn data_uri(uri: &str) -> Option<Vec<u8>> {
    let rest = uri.strip_prefix("data:")?;
    let (head, data) = rest.split_once(',')?;
    if head.ends_with(";base64") {
        base64_decode(data)
    } else {
        Some(percent_decode(data).into_bytes())
    }
}

/// `%xx` escapes of a relative URI decoded.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Bytes of `uri`: a `data:` URI or a file resolved by `resolve` (given the decoded
/// relative path).
fn uri_bytes(uri: &str, resolve: &dyn Fn(&str) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    if uri.starts_with("data:") {
        data_uri(uri)
    } else {
        resolve(&percent_decode(uri))
    }
}

fn is_png_or_jpeg(data: &[u8]) -> bool {
    data.starts_with(&[0x89, b'P', b'N', b'G']) || data.starts_with(&[0xff, 0xd8])
}

/// Reads a `.gltf` or `.glb` file into a model whose root holds one group or
/// component instance per mesh node. `resolve` returns the bytes of a file named by a
/// relative URI (external buffers and images), or `None`.
pub fn import(
    bytes: &[u8],
    resolve: &dyn Fn(&str) -> Option<Vec<u8>>,
    merge_coplanar: bool,
) -> Result<Model, String> {
    let g = gltf::Gltf::from_slice(bytes).map_err(|e| format!("not a glTF file: {e}"))?;
    let doc = &g.document;
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    for b in doc.buffers() {
        let data = match b.source() {
            gltf::buffer::Source::Bin => g.blob.clone(),
            gltf::buffer::Source::Uri(u) => uri_bytes(u, resolve),
        }
        .ok_or_else(|| format!("buffer {} could not be read", b.index()))?;
        if data.len() < b.length() {
            return Err(format!("buffer {} is too short", b.index()));
        }
        buffers.push(data);
    }

    let mut model = Model::default();
    // Materials, one model material per glTF material (indices match).
    let mut mesh_materials: Vec<MeshMaterial> = Vec::new();
    let mut tex_sets: Vec<u32> = Vec::new();
    for (i, m) in doc.materials().enumerate() {
        let pbr = m.pbr_metallic_roughness();
        let f = pbr.base_color_factor();
        let color = [0, 1, 2].map(|k| linear_to_srgb(f64::from(f[k])));
        let opacity = if m.alpha_mode() == gltf::material::AlphaMode::Opaque {
            1.0
        } else {
            f64::from(f[3]).clamp(0.0, 1.0)
        };
        let base = m
            .name()
            .map_or_else(|| format!("Material{}", i + 1), str::to_owned);
        let mut name = base.clone();
        let mut n = 1;
        while mesh_materials.iter().any(|x| x.name == name) {
            n += 1;
            name = format!("{base}{n}");
        }
        tex_sets.push(pbr.base_color_texture().map_or(0, |t| t.tex_coord()));
        let texture = pbr.base_color_texture().and_then(|info| {
            let image = info.texture().source();
            let (data, file) = match image.source() {
                gltf::image::Source::View { view, mime_type } => {
                    let buf = buffers.get(view.buffer().index())?;
                    let data = buf.get(view.offset()..view.offset() + view.length())?;
                    let ext = if mime_type.contains("jpeg") {
                        "jpg"
                    } else {
                        "png"
                    };
                    (data.to_vec(), format!("{name}.{ext}"))
                }
                gltf::image::Source::Uri { uri, .. } => {
                    let file = if uri.starts_with("data:") {
                        format!("{name}.png")
                    } else {
                        percent_decode(uri.rsplit('/').next().unwrap_or(uri))
                    };
                    (uri_bytes(uri, resolve)?, file)
                }
            };
            let filename = image.name().map_or(file, str::to_owned);
            is_png_or_jpeg(&data).then(|| MeshTexture {
                filename,
                data: Arc::from(data),
                size: [TEXTURE_TILE, TEXTURE_TILE],
            })
        });
        mesh_materials.push(MeshMaterial {
            name,
            color,
            opacity,
            texture,
        });
    }
    for m in &mesh_materials {
        model.materials.push(Material {
            name: m.name.clone(),
            color: m.color,
            opacity: m.opacity,
            texture: m.texture.as_ref().map(|t| Texture {
                filename: t.filename.clone(),
                data: t.data.clone(),
                width: t.size[0],
                height: t.size[1],
                colorize: false,
            }),
        });
    }
    let material_ids: Vec<MaterialId> = (0..mesh_materials.len()).collect();
    let opts = MeshOptions {
        inches_per_unit: INCHES_PER_METER,
        y_up: true,
        merge_coplanar,
    };

    // Meshes.
    let mut mesh_defs: Vec<Option<DefinitionId>> = Vec::new();
    for gm in doc.meshes() {
        let mut mesh = Mesh {
            materials: mesh_materials.clone(),
            ..Default::default()
        };
        for prim in gm.primitives() {
            use gltf::mesh::Mode;
            let mode = prim.mode();
            if !matches!(
                mode,
                Mode::Triangles | Mode::TriangleStrip | Mode::TriangleFan
            ) {
                continue;
            }
            let reader = prim.reader(|b| buffers.get(b.index()).map(Vec::as_slice));
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            let first = mesh.positions.len();
            mesh.positions
                .extend(positions.map(|p| DVec3::from_array(p.map(f64::from))));
            let count = mesh.positions.len() - first;
            // Our own exports carry the f32 rounding of each position.
            let lo = prim
                .get(&gltf::Semantic::Extras(POSITION_LO[1..].to_owned()))
                .filter(|a| {
                    a.count() == count
                        && a.data_type() == gltf::accessor::DataType::F32
                        && a.dimensions() == gltf::accessor::Dimensions::Vec3
                })
                .and_then(|a| {
                    gltf::accessor::Iter::<[f32; 3]>::new(a, |b: gltf::Buffer| {
                        buffers.get(b.index()).map(Vec::as_slice)
                    })
                });
            if let Some(lo) = lo {
                for (p, d) in mesh.positions[first..].iter_mut().zip(lo) {
                    *p += DVec3::from_array(d.map(f64::from));
                }
            }
            let material = prim.material().index();
            let set = material.and_then(|m| tex_sets.get(m).copied()).unwrap_or(0);
            let uvs: Vec<DVec2> = reader
                .read_tex_coords(set)
                .map(|t| {
                    t.into_f32()
                        .map(|[u, v]| DVec2::new(f64::from(u), 1.0 - f64::from(v)))
                        .collect()
                })
                .unwrap_or_default();
            let indices: Vec<usize> = match reader.read_indices() {
                Some(ix) => ix.into_u32().map(|i| i as usize).collect(),
                None => (0..count).collect(),
            };
            if indices.iter().any(|&i| i >= count) {
                return Err(format!("mesh {}: index out of range", gm.index()));
            }
            let tris: Vec<[usize; 3]> = match mode {
                Mode::Triangles => indices.as_chunks::<3>().0.to_vec(),
                Mode::TriangleStrip => (2..indices.len())
                    .map(|k| {
                        if k % 2 == 0 {
                            [indices[k - 2], indices[k - 1], indices[k]]
                        } else {
                            [indices[k - 1], indices[k - 2], indices[k]]
                        }
                    })
                    .collect(),
                _ => (2..indices.len())
                    .map(|k| [indices[0], indices[k - 1], indices[k]])
                    .collect(),
            };
            for t in tris {
                mesh.polygons.push(Polygon {
                    vertices: t.iter().map(|&i| first + i).collect(),
                    material,
                    uvs: if uvs.len() == count {
                        t.iter().map(|&i| uvs[i]).collect()
                    } else {
                        Vec::new()
                    },
                });
            }
        }
        let mut ents = mesh_import::to_entities(&mesh, &opts, &material_ids);
        if ents.faces.is_empty() {
            mesh_defs.push(None);
            continue;
        }
        soften(&mut ents, SOFTEN_ANGLE_DEG);
        let name = gm
            .name()
            .map_or_else(|| format!("Mesh{}", gm.index() + 1), str::to_owned);
        mesh_defs.push(Some(model.definitions.insert(Definition {
            name,
            entities: ents,
            ..Default::default()
        })));
    }

    // Nodes, flattened to world transforms.
    let c = gltf_to_model();
    let c_inv = c.inverse();
    let mut stack: Vec<(gltf::Node, DMat4, usize)> = Vec::new();
    let roots: Vec<gltf::Node> = match doc.default_scene().or_else(|| doc.scenes().next()) {
        Some(s) => s.nodes().collect(),
        None => {
            let children: std::collections::HashSet<usize> = doc
                .nodes()
                .flat_map(|n| n.children().map(|c| c.index()))
                .collect();
            doc.nodes()
                .filter(|n| !children.contains(&n.index()))
                .collect()
        }
    };
    for n in roots.into_iter().rev() {
        stack.push((n, DMat4::IDENTITY, 0));
    }
    let mut uses: HashMap<DefinitionId, usize> = HashMap::new();
    while let Some((node, parent, depth)) = stack.pop() {
        let m = node.transform().matrix();
        let local = DMat4::from_cols_array_2d(&m.map(|c| c.map(f64::from)));
        let world = parent * local;
        if let Some(def) = node
            .mesh()
            .and_then(|m| mesh_defs.get(m.index()).copied().flatten())
        {
            *uses.entry(def).or_default() += 1;
            model.root.instances.push(Instance {
                definition: def,
                transform: c * world * c_inv,
                name: node.name().unwrap_or_default().to_owned(),
                hidden: false,
                locked: false,
                layer: None,
                material: None,
                attrs: Default::default(),
            });
        }
        if depth < 64 {
            let kids: Vec<_> = node.children().collect();
            for k in kids.into_iter().rev() {
                stack.push((k, world, depth + 1));
            }
        }
    }
    // Definitions used once are groups (named Group#n, the mesh name on the instance).
    let ids: Vec<DefinitionId> = model.definitions.keys().collect();
    for d in ids {
        match uses.get(&d) {
            None => {
                model.definitions.remove(d);
            }
            Some(1) => {
                let mesh_name = std::mem::take(&mut model.definitions[d].name);
                let name = model.unique_definition_name("Group#1");
                let def = &mut model.definitions[d];
                def.name = name;
                def.is_group = true;
                if let Some(i) = model.root.instances.iter_mut().find(|i| i.definition == d)
                    && i.name.is_empty()
                {
                    i.name = mesh_name;
                }
            }
            Some(_) => {}
        }
    }
    Ok(model)
}

/// Marks edges between two faces less than `angle_deg` apart (and not coplanar) soft
/// and smooth.
pub fn soften(ents: &mut Entities, angle_deg: f64) {
    let cos = angle_deg.to_radians().cos();
    let soft: Vec<_> = ents
        .edges
        .iter()
        .filter(|(_, e)| {
            let [a, b] = e.faces[..] else { return false };
            let d = ents.faces[a].normal.dot(ents.faces[b].normal);
            d >= cos && d < 1.0 - 1e-9
        })
        .map(|(id, _)| id)
        .collect();
    for e in soft {
        let a = &mut ents.edges[e].attrs;
        a.soft = true;
        a.smooth = true;
    }
}

/// glTF export choices.
#[derive(Debug, Clone, PartialEq)]
pub struct GltfOptions {
    /// Binary `.glb` (else `.gltf` JSON with the buffer embedded).
    pub binary: bool,
    /// Write back faces as their own reversed triangles with the back material (else
    /// materials are double-sided and show the front material on both sides).
    pub two_sided: bool,
}

impl Default for GltfOptions {
    fn default() -> Self {
        Self {
            binary: true,
            two_sided: false,
        }
    }
}

/// A JSON string literal.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON number (finite; NaN and infinities become 0).
fn num(x: f64) -> String {
    if x.is_finite() {
        let s = format!("{x}");
        if s == "-0" { "0".into() } else { s }
    } else {
        "0".into()
    }
}

/// The binary buffer and the JSON pieces that describe it.
#[derive(Default)]
struct Builder {
    bin: Vec<u8>,
    views: Vec<String>,
    accessors: Vec<String>,
}

impl Builder {
    fn view(&mut self, data: &[u8], target: Option<u32>) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let offset = self.bin.len();
        self.bin.extend_from_slice(data);
        let target = target.map_or_else(String::new, |t| format!(",\"target\":{t}"));
        self.views.push(format!(
            "{{\"buffer\":0,\"byteOffset\":{offset},\"byteLength\":{}{target}}}",
            data.len()
        ));
        self.views.len() - 1
    }

    fn floats(&mut self, values: &[f32], kind: &str, width: usize, bounds: bool) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(34962));
        let count = values.len() / width;
        let mut extra = String::new();
        if bounds && count > 0 {
            let mut lo = vec![f32::INFINITY; width];
            let mut hi = vec![f32::NEG_INFINITY; width];
            for v in values.chunks(width) {
                for k in 0..width {
                    lo[k] = lo[k].min(v[k]);
                    hi[k] = hi[k].max(v[k]);
                }
            }
            let list = |x: &[f32]| {
                x.iter()
                    .map(|v| num(f64::from(*v)))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            extra = format!(",\"min\":[{}],\"max\":[{}]", list(&lo), list(&hi));
        }
        self.accessors.push(format!(
            "{{\"bufferView\":{view},\"componentType\":5126,\"count\":{count},\"type\":\"{kind}\"{extra}}}"
        ));
        self.accessors.len() - 1
    }

    fn indices(&mut self, values: &[u32]) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(34963));
        self.accessors.push(format!(
            "{{\"bufferView\":{view},\"componentType\":5125,\"count\":{},\"type\":\"SCALAR\"}}",
            values.len()
        ));
        self.accessors.len() - 1
    }
}

/// Export state while walking the model.
struct Export<'a> {
    model: &'a Model,
    opts: &'a GltfOptions,
    mats: Materials,
    /// Mesh sides by (definition or root, inherited material).
    mesh_index: HashMap<(Option<DefinitionId>, Option<MaterialId>), Option<usize>>,
    /// Mesh names and sides.
    meshes: Vec<(String, Vec<Side>)>,
    /// Node JSON objects.
    nodes: Vec<String>,
}

impl Export<'_> {
    /// The mesh of the faces of `ents` (definition `def`, `None` for the root) painted
    /// with `inherited`, or `None` when it has no visible face.
    fn mesh(
        &mut self,
        ents: &Entities,
        def: Option<DefinitionId>,
        inherited: Option<MaterialId>,
    ) -> Option<usize> {
        if let Some(&m) = self.mesh_index.get(&(def, inherited)) {
            return m;
        }
        let mut tris = Vec::new();
        mesh_export::face_triangles(self.model, ents, DMat4::IDENTITY, inherited, &mut tris);
        let m = (!tris.is_empty()).then(|| {
            let sides = mesh_export::tri_sides(
                self.model,
                tris,
                self.opts.two_sided,
                "texture",
                &mut self.mats,
            );
            let name = def.map_or_else(
                || "Model".to_owned(),
                |d| self.model.definitions[d].name.clone(),
            );
            self.meshes.push((name, sides));
            self.meshes.len() - 1
        });
        self.mesh_index.insert((def, inherited), m);
        m
    }

    /// Adds the node of `ents` and its visible instances; returns its index.
    fn node(
        &mut self,
        ents: &Entities,
        def: Option<DefinitionId>,
        name: &str,
        matrix: Option<DMat4>,
        inherited: Option<MaterialId>,
        stack: &mut Vec<DefinitionId>,
    ) -> usize {
        let index = self.nodes.len();
        self.nodes.push(String::new());
        let mesh = self.mesh(ents, def, inherited);
        let s = gltf_to_model().inverse();
        let mut children = Vec::new();
        let visible =
            |l: Option<usize>| l.is_none_or(|l| self.model.layers.get(l).is_none_or(|x| x.visible));
        for inst in &ents.instances {
            if inst.hidden || !visible(inst.layer) || stack.contains(&inst.definition) {
                continue;
            }
            let Some(d) = self.model.definitions.get(inst.definition) else {
                continue;
            };
            stack.push(inst.definition);
            let name = if inst.name.is_empty() {
                &d.name
            } else {
                &inst.name
            };
            let m = s * inst.transform * s.inverse();
            children.push(self.node(
                &d.entities,
                Some(inst.definition),
                name,
                Some(m),
                inst.material.or(inherited),
                stack,
            ));
            stack.pop();
        }
        let mut json = format!("{{\"name\":{}", json_str(name));
        if let Some(m) = mesh {
            let _ = write!(json, ",\"mesh\":{m}");
        }
        if let Some(m) = matrix.filter(|m| !m.abs_diff_eq(DMat4::IDENTITY, 1e-12)) {
            let vals: Vec<String> = m.to_cols_array().iter().map(|&x| num(x)).collect();
            let _ = write!(json, ",\"matrix\":[{}]", vals.join(","));
        }
        if !children.is_empty() {
            let list: Vec<String> = children.iter().map(usize::to_string).collect();
            let _ = write!(json, ",\"children\":[{}]", list.join(","));
        }
        json.push('}');
        self.nodes[index] = json;
        index
    }
}

/// Encodes the model as glTF 2.0: `.glb` bytes, or `.gltf` JSON text as bytes.
pub fn to_gltf(model: &Model, opts: &GltfOptions) -> Vec<u8> {
    let mut ex = Export {
        model,
        opts,
        mats: Materials::new(),
        mesh_index: HashMap::new(),
        meshes: Vec::new(),
        nodes: Vec::new(),
    };
    let mut stack = Vec::new();
    ex.node(&model.root, None, "Model", None, None, &mut stack);
    let s = gltf_to_model().inverse();
    let mut b = Builder::default();
    let mut meshes_json = Vec::new();
    for (name, sides) in &ex.meshes {
        let mut prims = Vec::new();
        for slot in 0..ex.mats.list.len() {
            let group: Vec<&Side> = sides.iter().filter(|x| x.slot == slot).collect();
            if group.is_empty() {
                continue;
            }
            let textured =
                ex.mats.list[slot].texture.is_some() && group.iter().all(|x| x.uv.is_some());
            let (mut pos, mut lo, mut nrm, mut uv, mut idx) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
            // Welded on the full-precision position (and the f32 normal and UV).
            let mut weld: HashMap<([u64; 3], [u32; 5]), u32> = HashMap::new();
            for side in &group {
                let n = s.transform_vector3(side.normal).normalize_or_zero();
                for k in 0..3 {
                    let p = s.transform_point3(side.points[k]);
                    let t = side.uv.map_or(DVec2::ZERO, |u| u[k]);
                    let t = DVec2::new(t.x, 1.0 - t.y);
                    let hi = p.to_array().map(|v| v as f32);
                    let rest = [n.x, n.y, n.z, t.x, t.y].map(|v| v as f32);
                    let key = (p.to_array().map(f64::to_bits), rest.map(f32::to_bits));
                    let next = weld.len() as u32;
                    let i = *weld.entry(key).or_insert_with(|| {
                        pos.extend_from_slice(&hi);
                        for (x, h) in p.to_array().into_iter().zip(hi) {
                            lo.push((x - f64::from(h)) as f32);
                        }
                        nrm.extend_from_slice(&rest[0..3]);
                        uv.extend_from_slice(&rest[3..5]);
                        next
                    });
                    idx.push(i);
                }
            }
            let pa = b.floats(&pos, "VEC3", 3, true);
            let na = b.floats(&nrm, "VEC3", 3, false);
            let mut attrs = format!("\"POSITION\":{pa},\"NORMAL\":{na}");
            // f32 positions are about 0.005" coarse a kilometer out, coarser than the
            // kernel's tolerance: the rounding left is written as its own attribute.
            if lo.iter().any(|&x| x != 0.0) {
                let la = b.floats(&lo, "VEC3", 3, false);
                let _ = write!(attrs, ",\"{POSITION_LO}\":{la}");
            }
            if textured {
                let ta = b.floats(&uv, "VEC2", 2, false);
                let _ = write!(attrs, ",\"TEXCOORD_0\":{ta}");
            }
            let ia = b.indices(&idx);
            prims.push(format!(
                "{{\"attributes\":{{{attrs}}},\"indices\":{ia},\"material\":{slot}}}"
            ));
        }
        meshes_json.push(format!(
            "{{\"name\":{},\"primitives\":[{}]}}",
            json_str(name),
            prims.join(",")
        ));
    }
    let (mut images, mut textures, mut materials) = (Vec::new(), Vec::new(), Vec::new());
    for m in &ex.mats.list {
        let tex = m
            .texture
            .as_ref()
            .filter(|(_, d)| is_png_or_jpeg(d))
            .map(|(file, data)| {
                let view = b.view(data, None);
                let mime = if data.starts_with(&[0xff, 0xd8]) {
                    "image/jpeg"
                } else {
                    "image/png"
                };
                images.push(format!(
                    "{{\"name\":{},\"bufferView\":{view},\"mimeType\":\"{mime}\"}}",
                    json_str(file)
                ));
                textures.push(format!("{{\"source\":{}}}", images.len() - 1));
                textures.len() - 1
            });
        let c = match tex {
            Some(_) => [1.0; 3],
            None => m.color.map(srgb_to_linear),
        };
        let mut pbr = format!(
            "\"baseColorFactor\":[{},{},{},{}],\"metallicFactor\":0,\"roughnessFactor\":1",
            num(c[0]),
            num(c[1]),
            num(c[2]),
            num(m.opacity)
        );
        if let Some(t) = tex {
            let _ = write!(pbr, ",\"baseColorTexture\":{{\"index\":{t}}}");
        }
        let blend = if m.opacity < 1.0 {
            ",\"alphaMode\":\"BLEND\""
        } else {
            ""
        };
        materials.push(format!(
            "{{\"name\":{},\"pbrMetallicRoughness\":{{{pbr}}}{blend},\"doubleSided\":{}}}",
            json_str(&m.name),
            !opts.two_sided
        ));
    }
    while !b.bin.len().is_multiple_of(4) {
        b.bin.push(0);
    }
    let mut json = String::from(
        "{\"asset\":{\"version\":\"2.0\",\"generator\":\"meshtard\"},\"scene\":0,\"scenes\":[{\"nodes\":[0]}]",
    );
    let list = |name: &str, items: &[String], json: &mut String| {
        if !items.is_empty() {
            let _ = write!(json, ",\"{name}\":[{}]", items.join(","));
        }
    };
    list("nodes", &ex.nodes, &mut json);
    list("meshes", &meshes_json, &mut json);
    list("materials", &materials, &mut json);
    list("textures", &textures, &mut json);
    list("images", &images, &mut json);
    list("accessors", &b.accessors, &mut json);
    list("bufferViews", &b.views, &mut json);
    if !b.bin.is_empty() {
        if opts.binary {
            let _ = write!(json, ",\"buffers\":[{{\"byteLength\":{}}}]", b.bin.len());
        } else {
            let _ = write!(
                json,
                ",\"buffers\":[{{\"byteLength\":{},\"uri\":\"data:application/octet-stream;base64,{}\"}}]",
                b.bin.len(),
                base64_encode(&b.bin)
            );
        }
    }
    json.push('}');
    if !opts.binary {
        return json.into_bytes();
    }
    let mut json = json.into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let total = 12 + 8 + json.len() + if b.bin.is_empty() { 0 } else { 8 + b.bin.len() };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    if !b.bin.is_empty() {
        out.extend_from_slice(&(b.bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&b.bin);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_export::tests::cube;

    fn instance(def: DefinitionId, transform: DMat4, material: Option<MaterialId>) -> Instance {
        Instance {
            definition: def,
            transform,
            name: String::new(),
            hidden: false,
            locked: false,
            layer: None,
            material,
            attrs: Default::default(),
        }
    }

    /// A model with a red 10" box component placed twice and a loose face.
    fn sample() -> (Model, [DMat4; 2]) {
        let mut m = Model::default();
        m.materials.push(Material {
            name: "Red".into(),
            color: [200, 30, 30],
            opacity: 1.0,
            texture: None,
        });
        let mut def = Entities::new();
        cube(&mut def, DVec3::splat(10.0));
        let d = m.definitions.insert(Definition {
            name: "Box".into(),
            entities: def,
            ..Default::default()
        });
        let xf = [
            DMat4::from_translation(DVec3::new(20.0, 0.0, 0.0)),
            DMat4::from_translation(DVec3::new(0.0, 40.0, 5.0))
                * DMat4::from_rotation_z(0.5)
                * DMat4::from_scale(DVec3::new(1.0, 2.0, 1.0)),
        ];
        for x in xf {
            m.root.instances.push(instance(d, x, Some(0)));
        }
        // A loose face on the ground.
        let p = [(0., 0.), (5., 0.), (5., 5.), (0., 5.)].map(|(x, y)| DVec3::new(x, y, 0.0));
        for i in 0..4 {
            crate::add_line(&mut m.root, p[i], p[(i + 1) % 4]);
        }
        (m, xf)
    }

    #[test]
    fn base64_round_trip() {
        for n in 0..10 {
            let data: Vec<u8> = (0..n).map(|i| (i * 37 + 5) as u8).collect();
            assert_eq!(base64_decode(&base64_encode(&data)).unwrap(), data);
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
    }

    #[test]
    fn color_space_round_trip() {
        for c in [0u8, 1, 30, 128, 200, 255] {
            assert_eq!(linear_to_srgb(srgb_to_linear(c)), c);
        }
    }

    fn check_round_trip(binary: bool) {
        let (m, xf) = sample();
        let bytes = to_gltf(
            &m,
            &GltfOptions {
                binary,
                two_sided: false,
            },
        );
        assert_eq!(bytes.starts_with(b"glTF"), binary);
        let back = import(&bytes, &|_| None, true).unwrap();
        // The box (a component used twice) and the loose face (a group).
        assert_eq!(back.definitions.len(), 2);
        assert_eq!(back.root.instances.len(), 3);
        let boxes: Vec<&Instance> = back
            .root
            .instances
            .iter()
            .filter(|i| !back.definitions[i.definition].is_group)
            .collect();
        assert_eq!(boxes.len(), 2);
        let def = &back.definitions[boxes[0].definition];
        assert_eq!(def.name, "Box");
        // Merged back to the 6 faces and 12 edges of the box.
        assert_eq!(def.entities.faces.len(), 6);
        assert_eq!(def.entities.edges.len(), 12);
        assert!(crate::validate(&def.entities, crate::Checks::ALL).is_empty());
        // Same transforms (f32 precision).
        for (i, x) in boxes.iter().zip(xf) {
            assert!(i.transform.abs_diff_eq(x, 1e-4), "{} vs {x}", i.transform);
        }
        // The definition is in its own coordinates (0..10 inches).
        let max = def
            .entities
            .vertices
            .values()
            .map(|v| v.pos.max_element())
            .fold(0.0, f64::max);
        assert!((max - 10.0).abs() < 1e-4, "{max}");
        // The box faces carry the instance's red (an inherited material becomes the
        // face material).
        let red = def
            .entities
            .faces
            .values()
            .next()
            .unwrap()
            .attrs
            .material
            .unwrap();
        assert_eq!(back.materials[red].color, [200, 30, 30]);
        // The loose face: one face, its group at the identity.
        let group = back
            .root
            .instances
            .iter()
            .find(|i| back.definitions[i.definition].is_group)
            .unwrap();
        assert!(group.transform.abs_diff_eq(DMat4::IDENTITY, 1e-6));
        assert_eq!(back.definitions[group.definition].entities.faces.len(), 1);
        // Same world triangles overall.
        let a = mesh_export::triangles(&m).len();
        let b = mesh_export::triangles(&back).len();
        assert_eq!(a, b);
    }

    #[test]
    fn glb_round_trip_keeps_counts_and_transforms() {
        check_round_trip(true);
    }

    #[test]
    fn gltf_json_round_trip() {
        check_round_trip(false);
    }

    #[test]
    fn positions_far_out_round_trip_to_f64_precision() {
        // A mile out, f32 is about 0.004" coarse: more than the kernel's tolerance.
        let mut m = Model::default();
        let o = DVec3::new(63_360.123_456_7, -41_000.987_654_3, 1_234.567_891);
        let p =
            [(0., 0.), (3.3, 0.), (3.3, 0.7), (0., 0.7)].map(|(x, y)| o + DVec3::new(x, y, 0.0));
        for i in 0..4 {
            crate::add_line(&mut m.root, p[i], p[(i + 1) % 4]);
        }
        let opts = GltfOptions {
            binary: true,
            two_sided: false,
        };
        let back = import(&to_gltf(&m, &opts), &|_| None, true).unwrap();
        let i = &back.root.instances[0];
        let ents = &back.definitions[i.definition].entities;
        assert_eq!(ents.faces.len(), 1);
        for v in ents.vertices.values() {
            let w = i.transform.transform_point3(v.pos);
            let d = p.iter().map(|q| q.distance(w)).fold(f64::MAX, f64::min);
            assert!(d < 1e-9, "{w} is {d} off");
        }
    }

    #[test]
    fn textures_and_uvs_round_trip() {
        let mut m = Model::default();
        cube(&mut m.root, DVec3::splat(10.0));
        let png: Vec<u8> = [0x89, b'P', b'N', b'G', 1, 2, 3, 4].to_vec();
        m.materials.push(Material {
            name: "Wood".into(),
            color: [150, 100, 50],
            opacity: 0.5,
            texture: Some(Texture {
                filename: "wood.png".into(),
                data: Arc::from(png.clone()),
                width: 4.0,
                height: 4.0,
                colorize: false,
            }),
        });
        for f in m.root.faces.values_mut() {
            f.attrs.material = Some(0);
        }
        let bytes = to_gltf(&m, &GltfOptions::default());
        let back = import(&bytes, &|_| None, true).unwrap();
        assert_eq!(back.materials.len(), 1);
        let mat = &back.materials[0];
        assert_eq!(mat.opacity, 0.5);
        assert_eq!(&*mat.texture.as_ref().unwrap().data, &png[..]);
        let def = &back.definitions[back.root.instances[0].definition];
        assert_eq!(def.entities.faces.len(), 6);
        // Each face's texture repeats every 4": the UVs at its corners match the
        // original ones up to whole repeats.
        for f in def.entities.faces.values() {
            let map = crate::texture::face_mapping(f, false, mat).unwrap();
            let orig = m
                .root
                .faces
                .values()
                .find(|g| g.normal.abs_diff_eq(f.normal, 1e-6))
                .unwrap();
            let omap = crate::texture::face_mapping(orig, false, &m.materials[0]).unwrap();
            let pts = def.entities.loop_points(&f.outer);
            let d0 = crate::texture::uv_at(&map, pts[0]) - crate::texture::uv_at(&omap, pts[0]);
            for p in pts {
                let d = crate::texture::uv_at(&map, p) - crate::texture::uv_at(&omap, p);
                assert!((d - d0).length() < 1e-4, "{d} {d0}");
                assert!((d0 - d0.round()).length() < 1e-4, "{d0}");
            }
        }
    }

    #[test]
    fn hand_written_gltf_with_node_transforms() {
        // One triangle mesh used by a parent node (translated 1 m along glTF X) and its
        // child (moved 2 m up, glTF Y), with an external buffer.
        let mut bin = Vec::new();
        for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0] {
            bin.extend_from_slice(&v.to_le_bytes());
        }
        let json = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
          "nodes":[{"mesh":0,"translation":[1,0,0],"children":[1]},{"mesh":0,"translation":[0,2,0]}],
          "meshes":[{"name":"Tri","primitives":[{"attributes":{"POSITION":0}}]}],
          "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,-1],"max":[1,0,0]}],
          "bufferViews":[{"buffer":0,"byteLength":36}],
          "buffers":[{"byteLength":36,"uri":"tri%20data.bin"}]}"#;
        let m = import(
            json.as_bytes(),
            &|name| (name == "tri data.bin").then(|| bin.clone()),
            true,
        )
        .unwrap();
        assert_eq!(m.definitions.len(), 1);
        let (_, d) = m.definitions.iter().next().unwrap();
        assert_eq!(d.name, "Tri");
        assert!(!d.is_group);
        // glTF -Z is model +Y: the triangle lies on the ground, 1 m (39.37") wide.
        let ys: Vec<f64> = d.entities.vertices.values().map(|v| v.pos.y).collect();
        assert!(ys.iter().any(|y| (y - INCHES_PER_METER).abs() < 1e-6));
        assert!(d.entities.vertices.values().all(|v| v.pos.z.abs() < 1e-9));
        let t: Vec<DVec3> = m
            .root
            .instances
            .iter()
            .map(|i| i.transform.w_axis.truncate())
            .collect();
        let k = INCHES_PER_METER;
        assert!(t[0].abs_diff_eq(DVec3::new(k, 0.0, 0.0), 1e-9));
        // The child: parent's 1 m along X plus 2 m up (model Z).
        assert!(t[1].abs_diff_eq(DVec3::new(k, 0.0, 2.0 * k), 1e-9));
        assert!(import(b"nonsense", &|_| None, true).is_err());
    }
}
