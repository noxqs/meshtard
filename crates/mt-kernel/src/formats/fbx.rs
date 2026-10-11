//! FBX import: binary (all versions, 64-bit records from 7500, zlib arrays) and ASCII
//! (6.x and 7.x), read into a node tree and interpreted for meshes.
//!
//! Written from the public descriptions of the format: Blender's FBX add-on
//! (`io_scene_fbx`, `parse_fbx.py`/`encode_bin.py`, GPL, read for the layout only) and
//! the "FBX binary file format specification" notes (code.blender.org, 2013). The
//! Rust crate `fbxcel` (MIT/Apache-2.0) reads binary FBX only, so a small own reader
//! covers both encodings.
//!
//! - Every `Mesh` geometry (FBX 7: `Geometry` objects connected to models; FBX 6: the
//!   `Model` itself) becomes polygons placed by its model's global transform (parent
//!   models, translation/rotation/scaling with offsets, pivots, pre/post rotation and
//!   rotation order, and the geometric transform). Geometry used by several models is
//!   placed once per model. A mirroring transform reverses the polygons' winding.
//! - Axes and units from `GlobalSettings` (`UpAxis`, `FrontAxis`, `CoordAxis` with
//!   signs; `UnitScaleFactor`): the mesh is converted to Z up and centimeters, so the
//!   import dialog defaults to centimeters and no axis swap.
//! - Materials: the models' connected materials by the geometry's
//!   `LayerElementMaterial` (per polygon or all same); diffuse color, opacity
//!   (`Opacity`, else `1 - TransparencyFactor`); the diffuse texture's image (embedded
//!   `Content` or the `RelativeFilename`/`FileName` file, PNG or JPEG) with the first
//!   UV layer's coordinates (by polygon vertex, by vertex or by polygon, direct or
//!   indexed).
//! - Normals, smoothing, edges, cameras, lights, skinning and animation are ignored;
//!   the model hierarchy is flattened into one group.

use std::collections::HashMap;
use std::io::Read;

use glam::{DMat4, DVec2, DVec3, DVec4, EulerRot};

use super::{Hints, ReadFile, find_texture, texture_from_bytes};
use crate::mesh_export::Units;
use crate::mesh_import::{Mesh, MeshMaterial, Polygon};

/// A property value of an FBX node.
#[derive(Debug, Clone, PartialEq)]
pub enum Prop {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Raw(Vec<u8>),
    Ints(Vec<i64>),
    Floats(Vec<f64>),
}

/// An FBX node: name, property values and child nodes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    pub name: String,
    pub props: Vec<Prop>,
    pub children: Vec<Node>,
}

impl Node {
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    pub fn str(&self, i: usize) -> Option<&str> {
        match self.props.get(i)? {
            Prop::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn float(&self, i: usize) -> Option<f64> {
        match self.props.get(i)? {
            Prop::Float(x) => Some(*x),
            Prop::Int(x) => Some(*x as f64),
            Prop::Bool(b) => Some(f64::from(u8::from(*b))),
            _ => None,
        }
    }

    pub fn int(&self, i: usize) -> Option<i64> {
        match self.props.get(i)? {
            Prop::Int(x) => Some(*x),
            Prop::Float(x) => Some(*x as i64),
            Prop::Bool(b) => Some(i64::from(*b)),
            _ => None,
        }
    }

    /// A numeric array property (also from scalar properties, as old ASCII files
    /// write arrays as plain lists).
    pub fn floats(&self) -> Vec<f64> {
        match self.props.first() {
            Some(Prop::Floats(v)) => v.clone(),
            Some(Prop::Ints(v)) => v.iter().map(|&x| x as f64).collect(),
            _ => (0..self.props.len())
                .filter_map(|i| self.float(i))
                .collect(),
        }
    }

    pub fn ints(&self) -> Vec<i64> {
        match self.props.first() {
            Some(Prop::Ints(v)) => v.clone(),
            Some(Prop::Floats(v)) => v.iter().map(|&x| x as i64).collect(),
            _ => (0..self.props.len()).filter_map(|i| self.int(i)).collect(),
        }
    }

    /// The text of child `name`'s first property.
    fn child_str(&self, name: &str) -> Option<&str> {
        self.child(name)?.str(0)
    }
}

/// Reads an FBX file into its version and top-level nodes.
pub fn parse_tree(bytes: &[u8]) -> Result<(u32, Vec<Node>), String> {
    const MAGIC: &[u8] = b"Kaydara FBX Binary  \0";
    if bytes.starts_with(MAGIC) {
        let version = bytes
            .get(23..27)
            .map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")))
            .ok_or("truncated FBX")?;
        let mut r = Binary {
            d: bytes,
            pos: 27,
            wide: version >= 7500,
        };
        let mut nodes = Vec::new();
        while r.pos < bytes.len() {
            match r.node()? {
                Some(n) => nodes.push(n),
                None => break,
            }
        }
        return Ok((version, nodes));
    }
    let text = String::from_utf8_lossy(bytes);
    let version = text
        .lines()
        .take(5)
        .find_map(|l| {
            let i = l.find("FBX ")?;
            let v: String = l[i + 4..]
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            let mut it = v.split('.');
            let major: u32 = it.next()?.parse().ok()?;
            let minor: u32 = it.next().and_then(|m| m.parse().ok()).unwrap_or(0);
            Some(major * 1000 + minor * 100)
        })
        .unwrap_or(7000);
    let nodes = parse_ascii(&text)?;
    if nodes.is_empty() {
        return Err("not an FBX file".into());
    }
    // FBX 7 ASCII files carry the version in the header.
    let version = nodes
        .iter()
        .find(|n| n.name == "FBXHeaderExtension")
        .and_then(|h| h.child("FBXVersion"))
        .and_then(|v| v.int(0))
        .map_or(version, |v| v as u32);
    Ok((version, nodes))
}

struct Binary<'a> {
    d: &'a [u8],
    pos: usize,
    /// 64-bit record headers (FBX 7500 and later).
    wide: bool,
}

impl Binary<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let s = self
            .d
            .get(self.pos..self.pos + n)
            .ok_or("truncated FBX data")?;
        self.pos += n;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }

    fn word(&mut self) -> Result<u64, String> {
        if self.wide {
            Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8")))
        } else {
            self.u32().map(u64::from)
        }
    }

    /// The next node, or `None` at a null record.
    fn node(&mut self) -> Result<Option<Node>, String> {
        let end = self.word()? as usize;
        let count = self.word()? as usize;
        let _len = self.word()?;
        let name_len = self.take(1)?[0] as usize;
        if end == 0 {
            return Ok(None);
        }
        if end > self.d.len() || end < self.pos {
            return Err("bad FBX record".into());
        }
        let name = String::from_utf8_lossy(self.take(name_len)?).into_owned();
        let mut props = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            props.push(self.prop()?);
        }
        let mut children = Vec::new();
        while self.pos < end {
            match self.node()? {
                Some(n) => children.push(n),
                None => break,
            }
        }
        self.pos = end;
        Ok(Some(Node {
            name,
            props,
            children,
        }))
    }

    fn prop(&mut self) -> Result<Prop, String> {
        let ty = self.take(1)?[0];
        Ok(match ty {
            b'Y' => Prop::Int(i64::from(i16::from_le_bytes(
                self.take(2)?.try_into().expect("2"),
            ))),
            b'C' => Prop::Bool(self.take(1)?[0] != 0),
            b'I' => Prop::Int(i64::from(i32::from_le_bytes(
                self.take(4)?.try_into().expect("4"),
            ))),
            b'F' => Prop::Float(f64::from(f32::from_le_bytes(
                self.take(4)?.try_into().expect("4"),
            ))),
            b'D' => Prop::Float(f64::from_le_bytes(self.take(8)?.try_into().expect("8"))),
            b'L' => Prop::Int(i64::from_le_bytes(self.take(8)?.try_into().expect("8"))),
            b'S' | b'R' => {
                let n = self.u32()? as usize;
                let b = self.take(n)?.to_vec();
                if ty == b'S' {
                    Prop::Str(String::from_utf8_lossy(&b).into_owned())
                } else {
                    Prop::Raw(b)
                }
            }
            b'f' | b'd' | b'l' | b'i' | b'b' | b'c' => {
                let n = self.u32()? as usize;
                let encoding = self.u32()?;
                let clen = self.u32()? as usize;
                let size = match ty {
                    b'd' | b'l' => 8,
                    b'f' | b'i' => 4,
                    _ => 1,
                };
                let raw = self.take(clen)?;
                let data = if encoding == 1 {
                    let mut out = Vec::with_capacity(n * size);
                    flate2::read::ZlibDecoder::new(raw)
                        .read_to_end(&mut out)
                        .map_err(|e| format!("FBX array: {e}"))?;
                    out
                } else {
                    raw.to_vec()
                };
                if data.len() < n * size {
                    return Err("truncated FBX array".into());
                }
                let at = |i: usize| &data[i * size..(i + 1) * size];
                match ty {
                    b'd' => Prop::Floats(
                        (0..n)
                            .map(|i| f64::from_le_bytes(at(i).try_into().expect("8")))
                            .collect(),
                    ),
                    b'f' => Prop::Floats(
                        (0..n)
                            .map(|i| f64::from(f32::from_le_bytes(at(i).try_into().expect("4"))))
                            .collect(),
                    ),
                    b'l' => Prop::Ints(
                        (0..n)
                            .map(|i| i64::from_le_bytes(at(i).try_into().expect("8")))
                            .collect(),
                    ),
                    b'i' => Prop::Ints(
                        (0..n)
                            .map(|i| i64::from(i32::from_le_bytes(at(i).try_into().expect("4"))))
                            .collect(),
                    ),
                    _ => Prop::Ints((0..n).map(|i| i64::from(data[i])).collect()),
                }
            }
            _ => return Err(format!("unknown FBX property type {}", ty as char)),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Key(String),
    Str(String),
    Num(String),
    Word(String),
    Star,
    Open,
    Close,
    Comma,
}

fn tokenize(text: &str) -> Vec<Tok> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b';' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'{' => {
                out.push(Tok::Open);
                i += 1;
            }
            b'}' => {
                out.push(Tok::Close);
                i += 1;
            }
            b',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            b'*' => {
                out.push(Tok::Star);
                i += 1;
            }
            b'"' => {
                let start = i + 1;
                i = start;
                while i < b.len() && b[i] != b'"' {
                    i += 1;
                }
                out.push(Tok::Str(text[start..i.min(b.len())].to_owned()));
                i += 1;
            }
            _ if c.is_ascii_whitespace() => i += 1,
            _ => {
                let start = i;
                while i < b.len()
                    && !b[i].is_ascii_whitespace()
                    && !matches!(b[i], b',' | b'{' | b'}' | b'"' | b':' | b';')
                {
                    i += 1;
                }
                let word = &text[start..i];
                if b.get(i) == Some(&b':') {
                    out.push(Tok::Key(word.to_owned()));
                    i += 1;
                } else if word.is_empty() {
                    i += 1; // A stray ':'.
                } else if word
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+' || c == '.')
                {
                    out.push(Tok::Num(word.to_owned()));
                } else {
                    out.push(Tok::Word(word.to_owned()));
                }
            }
        }
    }
    out
}

fn number(s: &str) -> Prop {
    if s.contains(['.', 'e', 'E']) || s.contains("inf") || s.contains("nan") {
        Prop::Float(s.parse().unwrap_or(0.0))
    } else {
        s.parse()
            .map(Prop::Int)
            .unwrap_or_else(|_| Prop::Float(s.parse().unwrap_or(0.0)))
    }
}

fn parse_ascii(text: &str) -> Result<Vec<Node>, String> {
    let toks = tokenize(text);
    let mut i = 0;
    let nodes = ascii_nodes(&toks, &mut i, 0)?;
    Ok(nodes)
}

fn ascii_nodes(t: &[Tok], i: &mut usize, depth: usize) -> Result<Vec<Node>, String> {
    if depth > 64 {
        return Err("FBX nesting too deep".into());
    }
    let mut out = Vec::new();
    while *i < t.len() {
        match &t[*i] {
            Tok::Close => return Ok(out),
            Tok::Key(name) => {
                *i += 1;
                let mut node = Node {
                    name: name.clone(),
                    ..Default::default()
                };
                let mut array = false;
                if t.get(*i) == Some(&Tok::Star) {
                    // `*N { a: ... }`: an array.
                    *i += 1;
                    if let Some(Tok::Num(_)) = t.get(*i) {
                        *i += 1;
                    }
                    array = true;
                } else {
                    // `Content: ,` with the value on the next line.
                    while t.get(*i) == Some(&Tok::Comma) {
                        *i += 1;
                    }
                    while let Some(tok) = t.get(*i) {
                        let v = match tok {
                            Tok::Str(s) => Prop::Str(s.clone()),
                            Tok::Num(s) => number(s),
                            Tok::Word(s) => Prop::Str(s.clone()),
                            _ => break,
                        };
                        node.props.push(v);
                        *i += 1;
                        if t.get(*i) == Some(&Tok::Comma) {
                            *i += 1;
                            // A trailing comma before a line of more values.
                            continue;
                        }
                        break;
                    }
                }
                if t.get(*i) == Some(&Tok::Open) {
                    *i += 1;
                    node.children = ascii_nodes(t, i, depth + 1)?;
                    if t.get(*i) == Some(&Tok::Close) {
                        *i += 1;
                    }
                }
                if array {
                    let values = node
                        .children
                        .iter()
                        .find(|c| c.name == "a")
                        .map(|a| a.props.clone())
                        .unwrap_or_default();
                    node.children.clear();
                    node.props = vec![if values.iter().all(|v| matches!(v, Prop::Int(_))) {
                        Prop::Ints(
                            values
                                .iter()
                                .map(|v| if let Prop::Int(x) = v { *x } else { 0 })
                                .collect(),
                        )
                    } else {
                        Prop::Floats(
                            values
                                .iter()
                                .map(|v| match v {
                                    Prop::Int(x) => *x as f64,
                                    Prop::Float(x) => *x,
                                    _ => 0.0,
                                })
                                .collect(),
                        )
                    }];
                }
                out.push(node);
            }
            _ => *i += 1,
        }
    }
    Ok(out)
}

/// An object's key in connections: its id (FBX 7) or full name (FBX 6).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Id(i64),
    Name(String),
}

fn key_of(p: Option<&Prop>) -> Option<Key> {
    match p? {
        Prop::Int(i) => Some(Key::Id(*i)),
        Prop::Str(s) => Some(Key::Name(s.clone())),
        _ => None,
    }
}

/// The name of an object without its class (`Name\0\x01Class` or `Class::Name`).
pub fn object_name(s: &str) -> &str {
    if let Some((n, _)) = s.split_once("\u{0}\u{1}") {
        n
    } else if let Some((_, n)) = s.split_once("::") {
        n
    } else {
        s
    }
}

/// Properties70 (`P`) or Properties60 (`Property`) values by name.
fn properties(n: &Node) -> HashMap<&str, &Node> {
    let mut out = HashMap::new();
    for (block, entry) in [("Properties70", "P"), ("Properties60", "Property")] {
        if let Some(b) = n.child(block) {
            for p in b.children_named(entry) {
                if let Some(name) = p.str(0) {
                    out.insert(name, p);
                }
            }
        }
    }
    out
}

/// Values of property `name` (after its type columns).
fn prop_values(props: &HashMap<&str, &Node>, name: &str) -> Vec<f64> {
    props.get(name).map_or_else(Vec::new, |p| {
        // P: name, type, label, flags, values; Property: name, type, flags, values.
        let skip = if p.name == "P" { 4 } else { 3 };
        (skip..p.props.len()).filter_map(|i| p.float(i)).collect()
    })
}

fn prop_vec(props: &HashMap<&str, &Node>, name: &str, default: DVec3) -> DVec3 {
    let v = prop_values(props, name);
    if v.len() >= 3 {
        DVec3::new(v[0], v[1], v[2])
    } else {
        default
    }
}

fn prop_num(props: &HashMap<&str, &Node>, name: &str) -> Option<f64> {
    prop_values(props, name).first().copied()
}

/// Rotation (degrees) in FBX order `order` (0 XYZ .. 5 ZYX, rotate about the first
/// axis first).
fn rotation(r: DVec3, order: i64) -> DMat4 {
    let r = r * (std::f64::consts::PI / 180.0);
    // glam's extrinsic orders: `EulerRot::ZYX` with (z, y, x) builds Rz * Ry * Rx.
    let (rot, a, b, c) = match order {
        1 => (EulerRot::YZX, r.y, r.z, r.x),
        2 => (EulerRot::XZY, r.x, r.z, r.y),
        3 => (EulerRot::ZXY, r.z, r.x, r.y),
        4 => (EulerRot::YXZ, r.y, r.x, r.z),
        5 => (EulerRot::XYZ, r.x, r.y, r.z),
        _ => (EulerRot::ZYX, r.z, r.y, r.x),
    };
    DMat4::from_quat(glam::DQuat::from_euler(rot, a, b, c))
}

/// A model's local transform.
fn local_transform(model: &Node) -> DMat4 {
    let p = properties(model);
    let t = |n| DMat4::from_translation(prop_vec(&p, n, DVec3::ZERO));
    let order = prop_num(&p, "RotationOrder").unwrap_or(0.0) as i64;
    let rp = t("RotationPivot");
    let sp = t("ScalingPivot");
    t("Lcl Translation")
        * t("RotationOffset")
        * rp
        * rotation(prop_vec(&p, "PreRotation", DVec3::ZERO), 0)
        * rotation(prop_vec(&p, "Lcl Rotation", DVec3::ZERO), order)
        * rotation(prop_vec(&p, "PostRotation", DVec3::ZERO), 0).inverse()
        * rp.inverse()
        * t("ScalingOffset")
        * sp
        * DMat4::from_scale(prop_vec(&p, "Lcl Scaling", DVec3::ONE))
        * sp.inverse()
}

/// A model's geometric transform (applied to its geometry only).
fn geometric_transform(model: &Node) -> DMat4 {
    let p = properties(model);
    DMat4::from_translation(prop_vec(&p, "GeometricTranslation", DVec3::ZERO))
        * rotation(prop_vec(&p, "GeometricRotation", DVec3::ZERO), 0)
        * DMat4::from_scale(prop_vec(&p, "GeometricScaling", DVec3::ONE))
}

/// The axis-and-unit conversion of `GlobalSettings` to Z up centimeters.
fn axis_conversion(nodes: &[Node]) -> DMat4 {
    let Some(gs) = nodes.iter().find(|n| n.name == "GlobalSettings") else {
        // FBX 6 files have no GlobalSettings: Y up (the common default).
        return DMat4::from_cols(DVec4::X, DVec4::Z, -DVec4::Y, DVec4::W);
    };
    let p = properties(gs);
    let get = |n, d: f64| prop_num(&p, n).unwrap_or(d);
    let axis = |i: f64| -> DVec4 {
        match i as i64 {
            0 => DVec4::X,
            2 => DVec4::Z,
            _ => DVec4::Y,
        }
    };
    let sign = |s: f64| if s < 0.0 { -1.0 } else { 1.0 };
    let up = axis(get("UpAxis", 1.0)) * sign(get("UpAxisSign", 1.0));
    let front = axis(get("FrontAxis", 2.0)) * sign(get("FrontAxisSign", 1.0));
    let coord = axis(get("CoordAxis", 0.0)) * sign(get("CoordAxisSign", 1.0));
    let scale = get("UnitScaleFactor", 1.0);
    let scale = if scale > 0.0 { scale } else { 1.0 };
    // Rows: model X = coord, model Y = -front, model Z = up (all dotted with the file
    // point), so the matrix is the transpose of [coord, -front, up].
    let rows = DMat4::from_cols(coord, -front, up, DVec4::W).transpose();
    DMat4::from_scale(DVec3::splat(scale)) * rows
}

/// Reads a base64 text (embedded ASCII FBX content).
fn base64(text: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    };
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for v in text.bytes().filter_map(val) {
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

struct Doc<'a> {
    objects: HashMap<Key, &'a Node>,
    /// (child, parent, property) of every connection, in file order.
    links: Vec<(Key, Key, Option<String>)>,
}

impl<'a> Doc<'a> {
    fn new(nodes: &'a [Node]) -> Self {
        let mut objects = HashMap::new();
        if let Some(objs) = nodes.iter().find(|n| n.name == "Objects") {
            for o in &objs.children {
                if let Some(k) = key_of(o.props.first()) {
                    objects.insert(k, o);
                }
            }
        }
        let mut links = Vec::new();
        if let Some(c) = nodes.iter().find(|n| n.name == "Connections") {
            for l in c
                .children
                .iter()
                .filter(|l| l.name == "C" || l.name == "Connect")
            {
                if let (Some(a), Some(b)) = (key_of(l.props.get(1)), key_of(l.props.get(2))) {
                    links.push((a, b, l.str(3).map(str::to_owned)));
                }
            }
        }
        Self { objects, links }
    }

    /// Objects connected below `parent` whose node name is `kind`, in file order.
    fn children(&self, parent: &Key, kind: &str) -> Vec<(&Key, &'a Node, Option<&str>)> {
        self.links
            .iter()
            .filter(|(_, p, _)| p == parent)
            .filter_map(|(c, _, prop)| {
                let n = self.objects.get(c)?;
                (n.name == kind).then_some((c, *n, prop.as_deref()))
            })
            .collect()
    }

    /// Objects above `child` whose node name is `kind`.
    fn parents(&self, child: &Key, kind: &str) -> Vec<(&Key, &'a Node)> {
        self.links
            .iter()
            .filter(|(c, _, _)| c == child)
            .filter_map(|(_, p, _)| {
                let n = self.objects.get(p)?;
                (n.name == kind).then_some((p, *n))
            })
            .collect()
    }

    fn global_transform(&self, key: &Key, model: &Node, depth: usize) -> DMat4 {
        let local = local_transform(model);
        if depth > 64 {
            return local;
        }
        match self.parents(key, "Model").first() {
            Some((pk, p)) => self.global_transform(pk, p, depth + 1) * local,
            None => local,
        }
    }
}

/// Mapping of a layer element's values to polygon corners.
fn layer_index(
    layer: &Node,
    corner: usize,
    vertex: usize,
    polygon: usize,
    index: &[i64],
) -> Option<usize> {
    let mapping = layer.child_str("MappingInformationType").unwrap_or("");
    let reference = layer
        .child_str("ReferenceInformationType")
        .unwrap_or("Direct");
    let i = match mapping {
        "ByPolygonVertex" => corner,
        "ByVertice" | "ByVertex" | "ByControlPoint" => vertex,
        "ByPolygon" => polygon,
        "AllSame" => 0,
        _ => corner,
    };
    if reference == "IndexToDirect" || reference == "Index" {
        usize::try_from(*index.get(i)?).ok()
    } else {
        Some(i)
    }
}

/// Reads an FBX file (binary or ASCII). `read` finds texture files.
pub fn parse(bytes: &[u8], read: ReadFile) -> Result<(Mesh, Hints), String> {
    let (_version, nodes) = parse_tree(bytes)?;
    let doc = Doc::new(&nodes);
    let axes = axis_conversion(&nodes);
    let mut mesh = Mesh::default();
    let mut materials: HashMap<Key, usize> = HashMap::new();

    // (geometry node, model key and node) pairs.
    let mut placed: Vec<(&Node, Option<(&Key, &Node)>)> = Vec::new();
    let mut keys: Vec<&Key> = doc.objects.keys().collect();
    // File order, so imports are deterministic.
    let order: HashMap<*const Node, usize> = nodes
        .iter()
        .find(|n| n.name == "Objects")
        .map(|o| {
            o.children
                .iter()
                .enumerate()
                .map(|(i, n)| (n as *const Node, i))
                .collect()
        })
        .unwrap_or_default();
    keys.sort_by_key(|k| order.get(&(doc.objects[*k] as *const Node)).copied());
    for key in keys {
        let node = doc.objects[key];
        if node.child("Vertices").is_none() {
            continue;
        }
        match node.name.as_str() {
            "Geometry" => {
                let models = doc.parents(key, "Model");
                if models.is_empty() {
                    placed.push((node, None));
                }
                for (mk, m) in models {
                    placed.push((node, Some((mk, m))));
                }
            }
            // FBX 6: the model holds its mesh.
            "Model" => placed.push((node, Some((key, node)))),
            _ => {}
        }
    }

    for (geom, model) in placed {
        let xf = axes
            * model.map_or(DMat4::IDENTITY, |(k, m)| {
                doc.global_transform(k, m, 0) * geometric_transform(m)
            });
        let mirrored = xf.determinant() < 0.0;
        let pts = geom.child("Vertices").map(Node::floats).unwrap_or_default();
        let base = mesh.positions.len();
        let count = pts.len() / 3;
        mesh.positions.extend(
            pts.as_chunks::<3>()
                .0
                .iter()
                .map(|&c| xf.transform_point3(DVec3::from_array(c))),
        );
        // The model's materials, in connection order.
        let model_mats: Vec<Option<usize>> = match model {
            Some((mk, _)) => doc
                .children(mk, "Material")
                .into_iter()
                .map(|(k, m, _)| Some(material(&doc, k, m, read, &mut mesh, &mut materials)))
                .collect(),
            None => Vec::new(),
        };
        let mat_layer = geom.child("LayerElementMaterial");
        let mat_index = mat_layer
            .and_then(|l| l.child("Materials"))
            .map(Node::ints)
            .unwrap_or_default();
        let uv_layer = geom.child("LayerElementUV");
        let uv_values = uv_layer
            .and_then(|l| l.child("UV"))
            .map(Node::floats)
            .unwrap_or_default();
        let uv_index = uv_layer
            .and_then(|l| l.child("UVIndex").or_else(|| l.child("UVIndices")))
            .map(Node::ints)
            .unwrap_or_default();
        let indices = geom
            .child("PolygonVertexIndex")
            .map(Node::ints)
            .unwrap_or_default();
        let mut poly: Vec<usize> = Vec::new();
        let mut uvs: Vec<DVec2> = Vec::new();
        let mut polygon = 0;
        for (corner, &raw) in indices.iter().enumerate() {
            let last = raw < 0;
            let v = if last { !raw } else { raw } as usize;
            if v >= count {
                return Err("FBX: vertex index out of range".into());
            }
            poly.push(base + v);
            if let Some(l) = uv_layer {
                match layer_index(l, corner, v, polygon, &uv_index)
                    .and_then(|i| uv_values.get(2 * i..2 * i + 2))
                {
                    Some(t) => uvs.push(DVec2::new(t[0], t[1])),
                    None => uvs.push(DVec2::ZERO),
                }
            }
            if last || corner + 1 == indices.len() {
                // The `Materials` array holds the material numbers itself, so the
                // reference type does not matter.
                let slot = mat_layer
                    .map(|l| match l.child_str("MappingInformationType") {
                        Some("AllSame") => 0,
                        _ => polygon,
                    })
                    .and_then(|i| mat_index.get(i).or(mat_index.first()))
                    .map_or(0, |&m| m.max(0) as usize);
                let material = model_mats.get(slot).copied().flatten();
                let mut vertices = std::mem::take(&mut poly);
                let mut corner_uvs = std::mem::take(&mut uvs);
                if mirrored {
                    vertices.reverse();
                    corner_uvs.reverse();
                }
                mesh.polygons.push(Polygon {
                    vertices,
                    material,
                    uvs: corner_uvs,
                });
                polygon += 1;
            }
        }
    }
    Ok((
        mesh,
        Hints {
            units: Some(Units::Centimeters),
            y_up: Some(false),
        },
    ))
}

/// The mesh material for FBX material `node`, added on first use.
fn material(
    doc: &Doc,
    key: &Key,
    node: &Node,
    read: ReadFile,
    mesh: &mut Mesh,
    done: &mut HashMap<Key, usize>,
) -> usize {
    if let Some(&i) = done.get(key) {
        return i;
    }
    let p = properties(node);
    let diffuse = prop_values(&p, "DiffuseColor");
    let diffuse = if diffuse.len() >= 3 {
        diffuse
    } else {
        prop_values(&p, "Diffuse")
    };
    let color = if diffuse.len() >= 3 {
        [0, 1, 2].map(|k| super::unit_byte(diffuse[k]))
    } else {
        [255; 3]
    };
    let opacity = prop_num(&p, "Opacity")
        .or_else(|| prop_num(&p, "TransparencyFactor").map(|t| 1.0 - t))
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    // Exporters that write a full transparency factor without meaning it: opaque.
    let opacity = if opacity < 0.01 { 1.0 } else { opacity };
    let textures = doc.children(key, "Texture");
    let texture = textures
        .iter()
        .find(|(_, _, prop)| prop.is_none_or(|p| p.contains("Diffuse")))
        .and_then(|(tk, t, _)| {
            let embedded = doc.children(tk, "Video").into_iter().find_map(|(_, v, _)| {
                let content = v.child("Content")?;
                let data = match content.props.first()? {
                    Prop::Raw(b) => b.clone(),
                    Prop::Str(_) => base64(
                        &content
                            .props
                            .iter()
                            .filter_map(|p| match p {
                                Prop::Str(s) => Some(s.as_str()),
                                _ => None,
                            })
                            .collect::<String>(),
                    ),
                    _ => return None,
                };
                let name = v
                    .child_str("RelativeFilename")
                    .or_else(|| v.child_str("Filename"))
                    .unwrap_or("texture.png");
                texture_from_bytes(name, data)
            });
            embedded.or_else(|| {
                ["RelativeFilename", "FileName", "Filename"]
                    .iter()
                    .filter_map(|n| t.child_str(n))
                    .find_map(|f| find_texture(f, read))
            })
        });
    let name = node.str(if matches!(key, Key::Id(_)) { 1 } else { 0 });
    let name = object_name(name.unwrap_or("Material"));
    mesh.materials.push(MeshMaterial {
        name: if name.is_empty() {
            "Material".into()
        } else {
            name.to_owned()
        },
        color,
        opacity,
        texture,
    });
    done.insert(key.clone(), mesh.materials.len() - 1);
    mesh.materials.len() - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASCII7: &str = r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {
	FBXVersion: 7400
}
GlobalSettings:  {
	Version: 1000
	Properties70:  {
		P: "UpAxis", "int", "Integer", "",1
		P: "UpAxisSign", "int", "Integer", "",1
		P: "FrontAxis", "int", "Integer", "",2
		P: "FrontAxisSign", "int", "Integer", "",1
		P: "CoordAxis", "int", "Integer", "",0
		P: "CoordAxisSign", "int", "Integer", "",1
		P: "UnitScaleFactor", "double", "Number", "",2.54
	}
}
Objects:  {
	Geometry: 100, "Geometry::Tri", "Mesh" {
		Vertices: *9 {
			a: 0,0,0,1,0,0,0,1,0
		}
		PolygonVertexIndex: *3 {
			a: 0,1,-3
		}
		LayerElementUV: 0 {
			MappingInformationType: "ByPolygonVertex"
			ReferenceInformationType: "IndexToDirect"
			UV: *6 {
				a: 0,0,1,0,0,1
			}
			UVIndex: *3 {
				a: 0,1,2
			}
		}
	}
	Model: 200, "Model::Tri", "Mesh" {
		Properties70:  {
			P: "Lcl Translation", "Lcl Translation", "", "A",10,0,0
			P: "Lcl Rotation", "Lcl Rotation", "", "A",0,0,90
		}
	}
	Material: 300, "Material::Red", "" {
		Properties70:  {
			P: "DiffuseColor", "Color", "", "A",1,0,0
			P: "Opacity", "double", "Number", "",0.5
		}
	}
}
Connections:  {
	C: "OO",200,0
	C: "OO",100,200
	C: "OO",300,200
}
"#;

    #[test]
    fn ascii_7_mesh_transform_material() {
        let none = |_: &str| None;
        let (v, nodes) = parse_tree(ASCII7.as_bytes()).unwrap();
        assert_eq!(v, 7400);
        assert_eq!(nodes.len(), 4);
        let (mesh, hints) = parse(ASCII7.as_bytes(), &none).unwrap();
        assert_eq!(hints.units, Some(Units::Centimeters));
        assert_eq!(mesh.polygons.len(), 1);
        assert_eq!(mesh.polygons[0].uvs.len(), 3);
        assert_eq!(mesh.materials[0].name, "Red");
        assert_eq!(mesh.materials[0].color, [255, 0, 0]);
        assert_eq!(mesh.materials[0].opacity, 0.5);
        // (1,0,0) rotated 90 degrees about Y-up's Z to (0,1,0), moved to (10,1,0),
        // in Z up: (10, 0, 1); times 2.54 cm per unit.
        let p = mesh.positions[1];
        assert!((p - DVec3::new(25.4, 0.0, 2.54)).length() < 1e-9, "{p}");
    }

    #[test]
    fn rotation_orders() {
        let r = DVec3::new(90.0, 90.0, 0.0);
        // XYZ: about X first, then Y: X axis goes to -Z.
        let m = rotation(r, 0);
        assert!((m.transform_vector3(DVec3::X) - DVec3::new(0.0, 0.0, -1.0)).length() < 1e-9);
        // ZYX (5): about Y first then X: X -> -Z -> Y.
        let m = rotation(r, 5);
        assert!((m.transform_vector3(DVec3::X) - DVec3::new(0.0, 1.0, 0.0)).length() < 1e-9);
        assert_eq!(base64("aGk="), b"hi");
        assert_eq!(object_name("Model::Box"), "Box");
        assert_eq!(object_name("Box\u{0}\u{1}Model"), "Box");
    }
}
