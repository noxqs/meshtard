//! The MFC `CArchive` record writer: the write-side mirror of the legacy
//! reader's slot and class-ref bookkeeping. Port of OpenSKP `create.py`'s
//! `_ArchiveWriter`.
//!
//! Each section of the file (materials, layers, definitions, root geometry)
//! gets its own writer whose slot numbering starts where the previous
//! section's ends, so references between sections stay valid.

use std::collections::HashMap;

use super::{AttributeDict, AttributeValue, Point3, WriteError, hex_bytes};

/// `CAttributeContainer` is declared in the scaffold prefix, always at slot 3.
const ATTR_CONTAINER_SLOT: u32 = 3;
/// `CAttributeNamed` is declared in the scaffold prefix, always at slot 5.
const ATTRIBUTE_NAMED_SLOT: u32 = 5;
/// `CCamera` is declared in the scaffold prefix, always at slot 7.
const CCAMERA_SLOT: u32 = 7;

pub(super) const LAYER_SCHEMA: u16 = 3;
const MATERIAL_SCHEMA: u16 = 12;
const DIB_SCHEMA: u16 = 3;
const DEFINITION_SCHEMA: u16 = 11;
const INSTANCE_SCHEMA: u16 = 6;
const GROUP_SCHEMA: u16 = 1;
const THUMBNAIL_SCHEMA: u16 = 1;
/// Not calibrated against a real SketchUp file (none with a `CImage` was
/// available to OpenSKP); chosen to match `CComponentInstance`.
const IMAGE_SCHEMA: u16 = 6;
const FTC_SCHEMA: u16 = 4;
const ARCCURVE_SCHEMA: u16 = 3;
const CCURVE_SCHEMA: u16 = 4;

/// The 176 bytes after `CCamera`'s class-ref that SketchUp writes for a
/// definition's default thumbnail camera, copied verbatim (not decoded).
const CAMERA_TEMPLATE: [u8; 176] = hex_bytes(concat!(
    "00000000000000000000000000000000000000000000f03f0000000000000000",
    "00000000000000000000000000000000004000000000000000000000000000f0",
    "3f0000000000000000000000000000000000000000000000000100000000003e",
    "40000000000000f03f0000000000000000000000000000000000000000000000",
    "0000000000000000000100fffeff00000000000000000000000000000000f03f",
    "00000000000000000000000000000000",
));

/// A definition's 22-byte base block (after its preamble): zero except bytes
/// 3-4, the same `1, 1` convention the drawbase needs.
const DEFINITION_BASE_BLOCK: [u8; 22] = {
    let mut b = [0u8; 22];
    b[3] = 1;
    b[4] = 1;
    b
};

/// A side without explicit texture positioning, next to one with it.
const IDENTITY_UV_MATRIX: [f64; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

/// The shared `CSkFont` (Tahoma) written inline at the first text or
/// dimension, harvested from a real SketchUp 2017 file.
const DIM_FONT_PAYLOAD: [u8; 34] = hex_bytes(concat!(
    "000000",
    "fffeff065400610068006f006d006100",
    "0000",
    "08000000",
    "00",
    "ecf57abd5eaf2340",
));
/// Leader-text delimiter: `[u32 1][u8 flag=1][u8 0][u32 arrow=3 closed][u8 1]`.
const TEXT_DELIM: [u8; 11] = hex_bytes("0100000001000300000001");
/// Drawbase used by texts, dimensions and guides.
const DIM_DRAWBASE: [u8; 10] = hex_bytes("00000001010000000000");

/// Persistent-ID encoding: a mask byte (bit i set when byte i of the id is
/// nonzero) followed by the nonzero bytes.
fn encode_pid(pid: u64) -> Vec<u8> {
    let mut mask = 0u8;
    let mut bytes = vec![0u8];
    for bit in 0..8 {
        let b = (pid >> (8 * bit)) as u8;
        if b != 0 {
            mask |= 1 << bit;
            bytes.push(b);
        }
    }
    bytes[0] = mask;
    bytes
}

/// Parameters of a `CArcCurve`: everything a circle or arc's edges share.
#[derive(Debug, Clone, Copy)]
pub(super) struct ArcParams {
    pub center: Point3,
    pub normal: Point3,
    /// The 0-angle direction, scaled by the radius.
    pub xaxis: Point3,
    pub start_angle: f64,
    pub end_angle: f64,
    pub radius: f64,
    pub segments: u32,
}

/// Which curve record (if any) the newly declared edges of a chain share.
#[derive(Debug, Clone, Copy)]
pub(super) enum CurveKind {
    None,
    Arc(ArcParams),
    /// A freeform `CCurve` grouping this many edges.
    Polyline(u32),
}

/// Edge display flags, applied to edges newly declared by a call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EdgeFlags {
    pub hidden: bool,
    pub soft: bool,
    pub smooth: bool,
}

/// Vertex and edge sharing state of one entity list (the root or one
/// definition). Vertices are shared where coordinates are exactly equal.
#[derive(Debug, Default)]
pub(super) struct Body {
    vertex_slots: HashMap<[u64; 3], u32>,
    /// Keyed by the (sorted) vertex slot pair; value is (edge slot, start vertex slot).
    edges: HashMap<(u32, u32), (u32, u32)>,
    /// Entities added to this list so far.
    pub new_entities: u32,
}

/// Exact-equality key; `-0.0` and `0.0` compare (and hash, in Python) equal.
fn point_key(p: Point3) -> [u64; 3] {
    p.map(|c| if c == 0.0 { 0 } else { c.to_bits() })
}

fn edge_key(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

/// Everything a `CFace` record needs beyond its loops.
pub(super) struct FaceRecord<'a> {
    pub material: u32,
    pub back_material: u32,
    pub layer: u32,
    pub hidden: bool,
    pub edges: EdgeFlags,
    pub front_matrix: Option<[f64; 9]>,
    pub back_matrix: Option<[f64; 9]>,
    pub attributes: &'a [AttributeDict],
    pub curve: CurveKind,
}

/// Everything an instance-like record (`CComponentInstance`, `CGroup`,
/// `CImage`) needs.
pub(super) struct InstanceRecord<'a> {
    pub definition_slot: u32,
    pub name: &'a str,
    pub translation: Point3,
    pub matrix: [f64; 9],
    pub material: u32,
    pub layer: u32,
    pub attributes: &'a [AttributeDict],
    pub hidden: bool,
    pub guid: [u8; 16],
}

pub(super) struct ArchiveWriter {
    pub next_slot: u32,
    pub class_slot: HashMap<String, u32>,
    pub next_pid: u64,
    pub buf: Vec<u8>,
}

impl ArchiveWriter {
    pub fn new(next_slot: u32, class_slot: HashMap<String, u32>, next_pid: u64) -> Self {
        Self {
            next_slot,
            class_slot,
            next_pid,
            buf: Vec::new(),
        }
    }

    fn alloc(&mut self) -> u32 {
        let s = self.next_slot;
        self.next_slot += 1;
        s
    }

    fn alloc_pid(&mut self) -> u64 {
        let p = self.next_pid;
        self.next_pid += 1;
        p
    }

    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn f64s(&mut self, vs: &[f64]) {
        for &v in vs {
            self.f64(v);
        }
    }

    pub fn raw(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }

    fn zeros(&mut self, n: usize) {
        self.buf.resize(self.buf.len() + n, 0);
    }

    /// A new object of `class`: a class declaration (`0xffff`, schema, name)
    /// the first time, a short class-ref afterwards. Returns the object's slot.
    pub fn new_object(&mut self, class: &str, schema: u16) -> u32 {
        let Some(&slot) = self.class_slot.get(class) else {
            self.u16(0xffff);
            self.u16(schema);
            self.u16(class.len() as u16);
            self.raw(class.as_bytes());
            let class_slot = self.alloc();
            self.class_slot.insert(class.to_owned(), class_slot);
            return self.alloc();
        };
        // 0x7fff itself must use the escape form: 0x8000 | 0x7fff is 0xffff,
        // the new-class tag.
        if slot < 0x7fff {
            self.u16(0x8000 | slot as u16);
        } else {
            self.u16(0x7fff);
            self.u32(0x8000_0000 | slot);
        }
        self.alloc()
    }

    /// A class-ref to a class the scaffold prefix declares.
    fn scaffold_object(&mut self, class_slot: u32) {
        self.u16(0x8000 | class_slot as u16);
        self.alloc();
    }

    pub fn null(&mut self) {
        self.u16(0);
    }

    pub fn backref(&mut self, slot: u32) {
        // A bare 0x7fff is the big-tag escape, so it needs the long form too.
        if slot < 0x7fff {
            self.u16(slot as u16);
        } else {
            self.u16(0x7fff);
            self.u32(slot);
        }
    }

    fn pid(&mut self, pid: u64) {
        let enc = encode_pid(pid);
        self.raw(&enc);
    }

    /// Entity preamble: a null attribute container and a fresh persistent id.
    pub fn preamble(&mut self) {
        self.null();
        let pid = self.alloc_pid();
        self.pid(pid);
    }

    /// Preamble of a structural object: null attributes, pid 0.
    fn preamble_no_pid(&mut self) {
        self.null();
        self.pid(0);
    }

    /// Preamble with a real `CAttributeContainer` holding optional texture
    /// coordinates and attribute dictionaries, and a fresh persistent id.
    fn preamble_with_attrs(
        &mut self,
        front: Option<[f64; 9]>,
        back: Option<[f64; 9]>,
        dicts: &[AttributeDict],
    ) -> Result<(), WriteError> {
        self.scaffold_object(ATTR_CONTAINER_SLOT);
        self.zeros(3); // the container's own preamble: null attrs + pid mask 0
        if front.is_some() || back.is_some() {
            self.face_texture_coords(front, back);
        }
        for d in dicts {
            self.attribute_dict(d)?;
        }
        self.null(); // children terminator
        let pid = self.alloc_pid();
        self.pid(pid);
        Ok(())
    }

    /// UTF-16LE string with the `ff fe ff` marker and a u8 length.
    pub fn string(&mut self, s: &str) -> Result<(), WriteError> {
        let units: Vec<u16> = s.encode_utf16().collect();
        if units.len() >= 0xff {
            return Err(WriteError::Invalid(format!(
                "string too long to encode (255 char limit): {s:?}"
            )));
        }
        self.raw(&[0xff, 0xfe, 0xff, units.len() as u8]);
        for u in units {
            self.u16(u);
        }
        Ok(())
    }

    fn attr_value(&mut self, v: &AttributeValue) -> Result<(), WriteError> {
        match v {
            AttributeValue::Null => self.buf.push(0x00),
            AttributeValue::Bool(b) => self.raw(&[0x07, u8::from(*b)]),
            AttributeValue::Point3d(p) => {
                self.buf.push(0x11);
                self.f64s(p);
            }
            AttributeValue::Vector3d(p) => {
                self.buf.push(0x12);
                self.f64s(p);
            }
            AttributeValue::Length(x) => {
                self.buf.push(0x0c);
                self.f64(*x);
            }
            AttributeValue::Timestamp(t) => {
                self.buf.push(0x09);
                self.u32(*t);
            }
            AttributeValue::String(s) => {
                self.buf.push(0x0a);
                self.string(s)?;
            }
            AttributeValue::Int(i) => {
                self.buf.push(0x04);
                self.raw(&i.to_le_bytes());
            }
            AttributeValue::Double(x) => {
                self.buf.push(0x06);
                self.f64(*x);
            }
            AttributeValue::Array(items) => {
                self.buf.push(0x0b);
                self.u32(items.len() as u32);
                for item in items {
                    self.attr_value(item)?;
                }
            }
        }
        Ok(())
    }

    /// One `CAttributeNamed` dictionary.
    fn attribute_dict(&mut self, d: &AttributeDict) -> Result<(), WriteError> {
        self.scaffold_object(ATTRIBUTE_NAMED_SLOT);
        self.zeros(3);
        self.u32(0);
        self.string(&d.name)?;
        for (key, value) in &d.entries {
            self.string(key)?;
            self.attr_value(value)?;
        }
        self.string("")?; // empty-key terminator
        self.u32(0);
        Ok(())
    }

    /// One `CFaceTextureCoords`: front and back UV-to-world matrices.
    fn face_texture_coords(&mut self, front: Option<[f64; 9]>, back: Option<[f64; 9]>) {
        self.new_object("CFaceTextureCoords", FTC_SCHEMA);
        self.preamble_no_pid();
        self.u32(0);
        let mut ks = [0.0; 24];
        ks[0..9].copy_from_slice(&front.unwrap_or(IDENTITY_UV_MATRIX));
        ks[12..21].copy_from_slice(&back.unwrap_or(IDENTITY_UV_MATRIX));
        self.f64s(&ks);
        self.u32(0); // front pin count: always a solved matrix, never raw pins
        self.u32(0); // back pin count
        self.u32(u32::from(front.is_some()));
        self.u32(u32::from(back.is_some()));
    }

    /// The 10-byte drawbase. Bytes 3-4 must be `1, 1`: SketchUp silently drops
    /// entities with them zeroed, although readers treat them as padding.
    fn drawbase(&mut self, mat: u32, layer: u32, hidden: bool, flags: EdgeFlags) {
        let mut b = [0u8; 10];
        b[0..2].copy_from_slice(&(mat as u16).to_le_bytes());
        b[2] = u8::from(hidden);
        b[3] = 1;
        b[4] = 1;
        b[5] = u8::from(flags.soft);
        b[6] = u8::from(flags.smooth);
        b[8..10].copy_from_slice(&(layer as u16).to_le_bytes());
        self.raw(&b);
    }

    fn vertex(&mut self, p: Point3) -> u32 {
        let slot = self.new_object("CVertex", 0);
        self.preamble();
        self.f64s(&p);
        slot
    }

    fn arc_curve(&mut self, a: &ArcParams) -> u32 {
        let slot = self.new_object("CArcCurve", ARCCURVE_SCHEMA);
        self.preamble();
        self.raw(&[0, a.segments as u8, 0, 0, 0]);
        self.f64s(&a.center);
        self.f64s(&a.normal);
        self.f64s(&a.xaxis);
        self.f64s(&[a.start_angle, a.end_angle, 0.0, a.radius, 0.0]);
        slot
    }

    fn curve(&mut self, num_edges: u32) -> u32 {
        let slot = self.new_object("CCurve", CCURVE_SCHEMA);
        self.preamble();
        self.buf.push(1);
        self.u32(num_edges);
        slot
    }

    /// One solid-color `CMaterial`. The stored f64 is transparency.
    pub fn material(
        &mut self,
        name: &str,
        rgba: [u8; 4],
        opacity: Option<f64>,
    ) -> Result<u32, WriteError> {
        let slot = self.new_object("CMaterial", MATERIAL_SCHEMA);
        self.preamble();
        self.string(name)?;
        self.u16(0); // texflag: solid color
        self.raw(&rgba);
        self.string("")?; // texture path
        self.zeros(8);
        self.f64(opacity.map_or(1.0, |o| 1.0 - o));
        self.buf.push(u8::from(opacity.is_some()));
        Ok(slot)
    }

    /// One textured `CMaterial` embedding the image verbatim in a `CDib`.
    /// `subtype` is 4 for PNG, 1 for JPEG.
    #[allow(clippy::too_many_arguments)]
    pub fn textured_material(
        &mut self,
        name: &str,
        image: &[u8],
        texture_path: &str,
        subtype: u32,
        size: (f64, f64),
        opacity: Option<f64>,
    ) -> Result<u32, WriteError> {
        let slot = self.new_object("CMaterial", MATERIAL_SCHEMA);
        self.preamble();
        self.string(name)?;
        self.u16(1); // texflag: textured
        self.zeros(2);
        self.new_object("CDib", DIB_SCHEMA);
        self.u32(subtype);
        self.u32(image.len() as u32);
        self.raw(image);
        if subtype == 1 {
            self.u32(90); // JPEG only: a constant SketchUp always writes
        }
        self.f64(size.0);
        self.f64(size.1);
        self.string(texture_path)?;
        // Placeholder average color; alpha 254 so readers do not take it as
        // the "colorized" signal.
        self.raw(&[255, 255, 255, 254, 0, 255, 255, 255, 254]);
        self.string("")?;
        self.u32(1);
        self.u32(0);
        self.f64(opacity.map_or(1.0, |o| 1.0 - o));
        self.buf.push(u8::from(opacity.is_some()));
        Ok(slot)
    }

    /// One `CLayer`. `with_pids` is false only for the copy of Layer0 a
    /// definition embeds, which carries neither of a layer's two pids.
    pub fn layer(
        &mut self,
        name: &str,
        with_pids: bool,
        hidden: bool,
        rgba: Option<[u8; 4]>,
    ) -> Result<u32, WriteError> {
        let slot = self.new_object("CLayer", LAYER_SCHEMA);
        if with_pids {
            self.preamble();
        } else {
            self.preamble_no_pid();
        }
        self.string(name)?;
        let pid2 = if with_pids { self.alloc_pid() } else { 0 };
        self.raw(&[u8::from(hidden), 0, 0]);
        self.pid(pid2);
        self.string(&format!("Layer_{name}"))?;
        self.u16(256);
        self.raw(&rgba.unwrap_or([0; 4]));
        self.string("")?;
        self.zeros(8);
        self.f64(0.5);
        self.zeros(5);
        Ok(slot)
    }

    fn thumbnail(&mut self) {
        self.new_object("CThumbnail", THUMBNAIL_SCHEMA);
        self.preamble_no_pid();
        self.scaffold_object(CCAMERA_SLOT);
        self.raw(&CAMERA_TEMPLATE);
        self.null(); // no thumbnail image
    }

    /// Starts a `CComponentDefinition` up to its entity list. Returns the
    /// definition slot and the offset of the entity count to patch.
    pub fn definition_header(
        &mut self,
        dicts: &[AttributeDict],
    ) -> Result<(u32, usize), WriteError> {
        let slot = self.new_object("CComponentDefinition", DEFINITION_SCHEMA);
        self.preamble_with_attrs(None, None, dicts)?;
        self.raw(&DEFINITION_BASE_BLOCK);
        self.u32(1); // nlayers: an embedded copy of Layer0
        let layer = self.layer("Layer0", false, false, None)?;
        self.backref(layer); // the definition's active layer
        self.u32(0); // definitions declared inline: never
        let count_pos = self.buf.len();
        self.u32(0);
        Ok((slot, count_pos))
    }

    /// Closes a `CComponentDefinition`: GUID, name, description, timestamp,
    /// behavior flags and a default thumbnail.
    pub fn definition_tail(
        &mut self,
        name: &str,
        description: &str,
        behavior: u8,
        guid: [u8; 16],
        timestamp: u32,
    ) -> Result<(), WriteError> {
        self.u32(0); // relationship count
        self.u16(0);
        self.raw(&guid);
        self.string(name)?;
        self.string(description)?;
        self.string("")?;
        self.u32(timestamp);
        let mut gap = [0u8; 43];
        gap[43 - 9] = behavior;
        self.raw(&gap);
        self.thumbnail();
        Ok(())
    }

    /// A `CComponentInstance` (`real_attrs` always), `CGroup` (container only
    /// with dictionaries) or `CImage` (never).
    pub fn instance_like(
        &mut self,
        kind: InstanceKind,
        r: &InstanceRecord<'_>,
    ) -> Result<(), WriteError> {
        let (class, schema, real_attrs) = match kind {
            InstanceKind::Component => ("CComponentInstance", INSTANCE_SCHEMA, true),
            InstanceKind::Group => ("CGroup", GROUP_SCHEMA, !r.attributes.is_empty()),
            InstanceKind::Image => ("CImage", IMAGE_SCHEMA, false),
        };
        self.new_object(class, schema);
        if real_attrs {
            self.preamble_with_attrs(None, None, r.attributes)?;
        } else {
            self.preamble();
        }
        self.drawbase(r.material, r.layer, r.hidden, EdgeFlags::default());
        self.backref(r.definition_slot);
        self.f64s(&r.matrix);
        self.f64s(&r.translation);
        self.f64(1.0);
        self.string(r.name)?;
        self.raw(&r.guid);
        Ok(())
    }

    /// A chain of `CEdge`s through `points`, sharing vertices and edges with
    /// `body`. Returns the loop's edge slots and senses (0 forward, 1 reversed).
    fn edge_chain(
        &mut self,
        points: &[Point3],
        body: &mut Body,
        closed: bool,
        flags: EdgeFlags,
        curve: CurveKind,
    ) -> (Vec<u32>, Vec<u8>) {
        let n = points.len();
        let pair_count = if closed { n } else { n - 1 };
        let mut point_slots: Vec<Option<u32>> = points
            .iter()
            .map(|&p| body.vertex_slots.get(&point_key(p)).copied())
            .collect();
        let mut edge_slots = Vec::with_capacity(pair_count);
        let mut senses = Vec::with_capacity(pair_count);
        let mut curve_slot = None;
        for i in 0..pair_count {
            let (i1, i2) = (i, (i + 1) % n);
            if let (Some(a), Some(b)) = (point_slots[i1], point_slots[i2])
                && let Some(&(edge, start)) = body.edges.get(&edge_key(a, b))
            {
                edge_slots.push(edge);
                senses.push(u8::from(start != a));
                continue;
            }
            let edge = self.new_object("CEdge", 2);
            self.preamble();
            self.drawbase(0, 0, flags.hidden, flags);
            for idx in [i1, i2] {
                match point_slots[idx] {
                    Some(s) => self.backref(s),
                    None => {
                        let s = self.vertex(points[idx]);
                        point_slots[idx] = Some(s);
                        body.vertex_slots.insert(point_key(points[idx]), s);
                    }
                }
            }
            match (curve_slot, curve) {
                (Some(s), _) => self.backref(s),
                (None, CurveKind::Arc(a)) => curve_slot = Some(self.arc_curve(&a)),
                (None, CurveKind::Polyline(n)) => curve_slot = Some(self.curve(n)),
                (None, CurveKind::None) => self.null(),
            }
            edge_slots.push(edge);
            senses.push(0);
            body.new_entities += 1;
            let (a, b) = (point_slots[i1].unwrap_or(0), point_slots[i2].unwrap_or(0));
            body.edges.insert(edge_key(a, b), (edge, a));
        }
        (edge_slots, senses)
    }

    /// An open (or, for polylines, optionally closed) edge chain with no face.
    pub fn edges(
        &mut self,
        points: &[Point3],
        body: &mut Body,
        closed: bool,
        flags: EdgeFlags,
        curve: CurveKind,
    ) {
        self.edge_chain(points, body, closed, flags, curve);
    }

    fn face_loop(&mut self, outer: bool, edge_slots: &[u32], senses: &[u8]) {
        let loop_slot = self.new_object("CLoop", 1);
        self.preamble_no_pid();
        // Opaque to readers, but SketchUp needs `01 01`; `00 01` marks a hole.
        self.raw(&[u8::from(outer), 1]);
        for (&edge, &sense) in edge_slots.iter().zip(senses) {
            self.new_object("CEdgeUse", 1);
            self.preamble_no_pid();
            self.backref(edge);
            self.buf.push(sense);
            self.backref(loop_slot);
        }
        self.null();
    }

    /// One planar face with optional holes. `plane` comes from
    /// `plane_from_polygon`; holes are validated by the caller.
    pub fn face(
        &mut self,
        points: &[Point3],
        holes: &[Vec<Point3>],
        plane: [f64; 4],
        body: &mut Body,
        r: &FaceRecord<'_>,
    ) -> Result<(), WriteError> {
        let (edges, senses) = self.edge_chain(points, body, true, r.edges, r.curve);
        let hole_loops: Vec<_> = holes
            .iter()
            .map(|h| self.edge_chain(h, body, true, r.edges, CurveKind::None))
            .collect();
        self.new_object("CFace", 3);
        if r.front_matrix.is_some() || r.back_matrix.is_some() || !r.attributes.is_empty() {
            self.preamble_with_attrs(r.front_matrix, r.back_matrix, r.attributes)?;
        } else {
            self.preamble();
        }
        self.drawbase(r.material, r.layer, r.hidden, EdgeFlags::default());
        self.f64s(&plane);
        self.u32(1 + holes.len() as u32);
        self.face_loop(true, &edges, &senses);
        for (e, s) in &hole_loops {
            self.face_loop(false, e, s);
        }
        self.u16(r.back_material as u16);
        body.new_entities += 1;
        Ok(())
    }

    /// Writes the shared `CSkFont` inline the first time, a back-ref after.
    pub fn font(&mut self, font_slot: &mut Option<u32>) {
        match *font_slot {
            Some(s) => self.backref(s),
            None => {
                *font_slot = Some(self.new_object("CSkFont", 1));
                self.raw(&DIM_FONT_PAYLOAD);
            }
        }
    }

    /// A leader text pinned at `p` with its label at `label`.
    pub fn text(
        &mut self,
        text: &str,
        p: Point3,
        label: Point3,
        font_slot: &mut Option<u32>,
    ) -> Result<(), WriteError> {
        self.new_object("CText", 9);
        self.preamble();
        self.raw(&DIM_DRAWBASE);
        self.font(font_slot);
        self.f64s(&[0.0, 0.0]); // screen-fraction slot (unused)
        self.u32(1); // free connection
        self.u32(4);
        self.f64s(&p);
        self.zeros(12);
        self.f64s(&label);
        self.zeros(16);
        self.f64(1.0);
        self.u32(2); // leader type: pushpin
        self.raw(&TEXT_DELIM);
        self.string(text)?;
        self.zeros(5);
        Ok(())
    }

    pub fn construction_point(&mut self, p: Point3) {
        self.new_object("CConstructionPoint", 0);
        self.preamble();
        self.raw(&DIM_DRAWBASE);
        self.f64s(&[p[0], p[1], p[2], 0.0, 0.0, 0.0]);
        self.zeros(1);
    }

    pub fn construction_line(&mut self, p: Point3, dir: Point3, start: f64, end: f64) {
        self.new_object("CConstructionLine", 1);
        self.preamble();
        self.raw(&DIM_DRAWBASE);
        self.f64s(&[p[0], p[1], p[2], dir[0], dir[1], dir[2], start, end]);
        self.zeros(4);
    }

    pub fn section_plane(&mut self, plane: [f64; 4]) {
        self.new_object("CSectionPlane", 3);
        self.preamble();
        self.raw(&DIM_DRAWBASE);
        self.f64s(&plane);
    }

    /// A free linear dimension between `a` and `b`.
    pub fn dimension(
        &mut self,
        a: Point3,
        b: Point3,
        offset: f64,
        font_slot: &mut Option<u32>,
    ) -> Result<(), WriteError> {
        self.new_object("CDimensionLinear", 6);
        self.preamble();
        self.raw(&DIM_DRAWBASE);
        self.string("")?; // auto-computed measurement text
        self.font(font_slot);
        // connection 1: [u8 0][u32 0][u32 type=1][u32 4][point A], null ref
        self.zeros(5);
        self.u32(1);
        self.u32(4);
        self.f64s(&a);
        self.u16(0);
        // connection 2: [u16 0][f64 0][u32 type=1][u32 4][point B], null ref
        self.zeros(10);
        self.u32(1);
        self.u32(4);
        self.f64s(&b);
        self.u16(0);
        // placement block: SDK free-dimension defaults plus the offset
        self.zeros(2);
        self.f64s(&[0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0]);
        self.u32(0);
        self.f64(offset);
        self.f64(0.0);
        self.u32(1);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum InstanceKind {
    Component,
    Group,
    Image,
}

/// Renumbers the u16 slot reference at `pos` by `shift`, keeping its 0x8000
/// class-ref bit and widening to the 6-byte escape form when the new slot no
/// longer fits. Returns how many bytes the buffer grew by (0 or 4).
pub(super) fn shift_ref(buf: &mut Vec<u8>, pos: usize, shift: u32) -> usize {
    let v = u16::from_le_bytes([buf[pos], buf[pos + 1]]);
    let tag = v & 0x8000;
    let slot = u32::from(v & 0x7fff) + shift;
    if slot < 0x7fff {
        buf[pos..pos + 2].copy_from_slice(&(tag | slot as u16).to_le_bytes());
        return 0;
    }
    let val = if tag != 0 { 0x8000_0000 | slot } else { slot };
    let mut wide = vec![0xff, 0x7f];
    wide.extend_from_slice(&val.to_le_bytes());
    buf.splice(pos..pos + 2, wide);
    4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_encoding() {
        assert_eq!(encode_pid(0), [0]);
        assert_eq!(encode_pid(19), [1, 19]);
        assert_eq!(encode_pid(0x0102_0003), [0b1101, 3, 2, 1]);
    }

    #[test]
    fn class_refs_and_backrefs_escape_at_0x7fff() {
        let mut w = ArchiveWriter::new(0x7fff, HashMap::new(), 1);
        w.new_object("CEdge", 2);
        assert_eq!(w.class_slot["CEdge"], 0x7fff);
        w.buf.clear();
        w.new_object("CEdge", 2);
        assert_eq!(w.buf, [0xff, 0x7f, 0xff, 0x7f, 0, 0x80]);
        w.buf.clear();
        w.backref(0x7fff);
        assert_eq!(w.buf, [0xff, 0x7f, 0xff, 0x7f, 0, 0]);
    }

    #[test]
    fn shift_ref_widens() {
        let mut b = vec![0x05, 0x80, 0xaa];
        assert_eq!(shift_ref(&mut b, 0, 2), 0);
        assert_eq!(b, [0x07, 0x80, 0xaa]);
        assert_eq!(shift_ref(&mut b, 0, 0x7ff8), 4);
        assert_eq!(b, [0xff, 0x7f, 0xff, 0x7f, 0, 0x80, 0xaa]);
    }
}
