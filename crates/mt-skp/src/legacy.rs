//! Legacy (SketchUp 3 to 2020) MFC CArchive reader. Port of OpenSKP `legacy.cpp`.
//!
//! The file is an MFC object stream: every object is introduced by a u16 tag
//! that is either null (0), a class definition (0xffff + schema + name), a
//! class-ref to an already defined class (0x8000 | slot), a back-ref to an
//! already read object (slot), or the big-tag escape (0x7fff + u32) once the
//! store map grows past 0x7ffe entries. The reader numbers objects exactly
//! as the writer did (the "slot" numbering), starting at a base it has to
//! recover first because the walk starts mid-file.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use regex::Regex;

use crate::Error;
use crate::bytes::{f64_at, i32_at, u16_at, u32_at};
use crate::model::{
    AttrDicts, AttrValue, ConstructionLine, ConstructionPoint, Dimension, EntityId, SectionPlane,
    TextEntity, Vec3, stringify_attr_dict,
};
use crate::raw::{
    CoEdge, GeometryBuilder, RawDefinition, RawFace, RawInstance, RawMaterial, RawPage, RawParsed,
    RawTexture,
};

// The trailing GUID looked plausible as a class-schema gate at first
// (CComponentInstance schema >= 5, CGroup schema >= 1), but it doesn't
// generalize: a v7 file's CComponentInstance reports schema 6 yet has no
// GUID; CGroup reports schema 1 on every version tested. Forcing the 16-byte
// read anyway silently eats bytes belonging to the next sibling's own tag
// (openskp#284/#310). The file's own version number is the only signal that
// actually holds (openskp#391).
fn legacy_instance_has_guid(ver: i32) -> bool {
    ver >= 14
}

// SketchUp 3 (header "{3.0.x}") predates several fields every later file
// carries. The differences are gated on `ver < FIRST_V4` and were calibrated
// byte-by-byte on one real SketchUp-written V3 file, not on a corpus, so they
// are exactly what that file proves and no more (openskp#284, #407).
// SketchUp 4 needs none of them: a real V4 file parses with the later layout.
const FIRST_V4: i32 = 4;

// Widest zero padding seen between the v20 filler's empty string and the
// count that follows it (9 and 13 bytes occur in real files; the ceiling
// leaves room without letting the probe wander into unrelated records).
const MAX_V20_FILLER_PAD: usize = 29;

const HEX: &[u8; 16] = b"0123456789ABCDEF";

fn bad(msg: impl Into<String>) -> Error {
    Error::Malformed(msg.into())
}

fn is_marker(d: &[u8], i: usize) -> bool {
    d.get(i..i.saturating_add(3)) == Some(&[255, 254, 255][..])
}

fn bytes_eq(d: &[u8], at: usize, s: &str) -> bool {
    d.get(at..at.saturating_add(s.len())) == Some(s.as_bytes())
}

fn hex_guid(g: &[u8]) -> String {
    let mut s = String::with_capacity(g.len() * 2);
    for &x in g {
        s.push(char::from(HEX[usize::from(x >> 4)]));
        s.push(char::from(HEX[usize::from(x & 15)]));
    }
    s
}

/// Result of [`find_count_after_v20_filler`]: the recovered count and the
/// offset just past it.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct V20FillerHit {
    pub count: u32,
    pub next: usize,
}

/// Locates the count that follows a v20 filler record, given the offset the
/// bad count was read from. Pure byte logic, kept for tests; the walk uses
/// `Archive::retry_count_after_v20_filler`, which adds validation.
///
/// SketchUp 2020 (v20) writes an extra, undocumented record ahead of some
/// counts that v17 does not have, which leaves the reader a few bytes early
/// and makes it read garbage as the count. The filler is an empty UTF-16
/// string record followed by zero padding:
///
/// ```text
///   <ff fe ff> <u8 0>        empty string
///   <zero padding>           runs up to the real count
/// ```
///
/// Rather than hard-code an offset (the number of bytes before the marker
/// differs per call site), locate the marker in the short window ahead, then
/// take the first plausible u32 that follows the padding. Only the EMPTY-string
/// form counts as filler: a real string here would mean genuine data.
#[cfg_attr(not(test), allow(dead_code))]
pub fn find_count_after_v20_filler(d: &[u8], count_pos: usize, limit: u32) -> Option<V20FillerHit> {
    let marker_at = find_filler_marker(d, count_pos)?;
    // The count sits past a run of zero padding whose length varies per call
    // site (9 and 13 bytes both occur), but always lands at marker_at + 4 + pad
    // with pad % 4 == 1. Deliberately NOT "scan forward to the first non-zero
    // byte": a count that is an exact multiple of 256 has a 0x00 low byte,
    // which such a scan cannot tell apart from padding. Probing whole u32s at
    // 4-byte strides never inspects an individual byte.
    for pad in (1..=MAX_V20_FILLER_PAD).step_by(4) {
        let at = marker_at + 4 + pad;
        if at + 4 > d.len() {
            break;
        }
        let count = u32_at(d, at).ok()?;
        if count > 0 && count <= limit {
            return Some(V20FillerHit {
                count,
                next: at + 4,
            });
        }
    }
    None
}

/// The empty-string filler marker within 12 bytes of `count_pos`, or `None`
/// (also when the string there is not empty: real data).
fn find_filler_marker(d: &[u8], count_pos: usize) -> Option<usize> {
    let mut i = count_pos;
    while i + 4 <= d.len() && i < count_pos + 12 {
        if d[i] == 255 && d[i + 1] == 254 && d[i + 2] == 255 {
            return if d[i + 3] != 0 { None } else { Some(i) };
        }
        i += 1;
    }
    None
}

/// Byte cursor over the archive. Every read is bounds-checked.
struct R<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> R<'a> {
    fn need(&self, n: usize) -> Result<(), Error> {
        if self.p > self.d.len() || n > self.d.len() - self.p {
            return Err(Error::Truncated);
        }
        Ok(())
    }

    fn u8(&mut self) -> Result<u8, Error> {
        self.need(1)?;
        let v = self.d[self.p];
        self.p += 1;
        Ok(v)
    }

    fn u16(&mut self) -> Result<u16, Error> {
        let v = u16_at(self.d, self.p)?;
        self.p += 2;
        Ok(v)
    }

    fn u32(&mut self) -> Result<u32, Error> {
        let v = u32_at(self.d, self.p)?;
        self.p += 4;
        Ok(v)
    }

    fn f64(&mut self) -> Result<f64, Error> {
        let v = f64_at(self.d, self.p)?;
        self.p += 8;
        Ok(v)
    }

    fn f64s(&mut self, n: usize) -> Result<Vec<f64>, Error> {
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(self.f64()?);
        }
        Ok(v)
    }

    fn raw(&mut self, n: usize) -> Result<&'a [u8], Error> {
        self.need(n)?;
        let v = &self.d[self.p..self.p + n];
        self.p += n;
        Ok(v)
    }

    fn marker(&self) -> bool {
        is_marker(self.d, self.p)
    }

    fn peek_u16(&self) -> Result<u16, Error> {
        self.need(2)?;
        u16_at(self.d, self.p)
    }

    /// An MFC `CString` record: `ff fe ff`, then a u8 length (escalating to
    /// u16 at 0xff and u32 at 0xffff), then UTF-16LE code units.
    fn utf16(&mut self) -> Result<String, Error> {
        if !self.marker() {
            return Err(bad("expected legacy string record"));
        }
        self.p += 3;
        let n = self.u8()?;
        let mut z = u32::from(n);
        if n == 255 {
            z = u32::from(self.u16()?);
            if z == 65535 {
                z = self.u32()?;
            }
        }
        let len = (z as usize).checked_mul(2).ok_or(Error::Truncated)?;
        let b = self.raw(len)?;
        Ok(decode_utf16le(b))
    }
}

/// UTF-16LE to UTF-8. OpenSKP's C++ converts each code unit on its own
/// (surrogates come out as CESU-8, which a Rust `String` cannot hold);
/// pairs are joined here and lone surrogates become U+FFFD.
fn decode_utf16le(b: &[u8]) -> String {
    char::decode_utf16(b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])))
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// One decoded entity (material/layer/component/edge/face/... - `k` says which).
#[derive(Debug, Clone)]
struct V {
    k: &'static str,
    name: String,
    label: String,
    text: String,
    guid: String,
    xyz: Vec3,
    /// Construction lines only: direction and bounded segment ends.
    direction: Vec3,
    start: Option<Vec3>,
    end: Option<Vec3>,
    plane: Vec<f64>,
    xf: Vec<f64>,
    uvf: Vec<f64>,
    uvb: Vec<f64>,
    front_projected: bool,
    back_projected: bool,
    v1: u64,
    v2: u64,
    edge: u64,
    def: u64,
    /// Slot of this entity's CAttributeContainer, or `None` when it has none
    /// (slot 0 is a legitimate real slot, so 0 can't be the sentinel).
    attrs: Option<u64>,
    /// "dict" (CAttributeNamed) entities only.
    entries: crate::model::AttrDict,
    tex_dib: u64,
    /// SketchUp 3: the texture image sits inline (bytes in `blob`), no store slot.
    inline_tex: bool,
    sense: bool,
    faces_camera: bool,
    shadows_face_sun: bool,
    colorized: bool,
    mat: i32,
    back_mat: i32,
    layer: i32,
    hidden: i32,
    soft: i32,
    smooth: i32,
    r: i32,
    g: i32,
    b: i32,
    a: i32,
    opacity: f64,
    tw: f64,
    th: f64,
    tex_file: String,
    blob: Vec<u8>,
    loops: Vec<Rc<V>>,
    uses: Vec<Rc<V>>,
    ents: Vec<Obj>,
}

impl Default for V {
    fn default() -> Self {
        Self {
            k: "",
            name: String::new(),
            label: String::new(),
            text: String::new(),
            guid: String::new(),
            xyz: [0.0; 3],
            direction: [0.0; 3],
            start: None,
            end: None,
            plane: Vec::new(),
            xf: Vec::new(),
            uvf: Vec::new(),
            uvb: Vec::new(),
            front_projected: false,
            back_projected: false,
            v1: 0,
            v2: 0,
            edge: 0,
            def: 0,
            attrs: None,
            entries: Default::default(),
            tex_dib: 0,
            inline_tex: false,
            sense: false,
            faces_camera: false,
            shadows_face_sun: false,
            colorized: false,
            mat: 0,
            back_mat: 0,
            layer: 0,
            hidden: 0,
            soft: 0,
            smooth: 0,
            r: 128,
            g: 128,
            b: 128,
            a: 255,
            opacity: 0.0,
            tw: 0.0,
            th: 0.0,
            tex_file: String::new(),
            blob: Vec::new(),
            loops: Vec::new(),
            uses: Vec::new(),
            ents: Vec::new(),
        }
    }
}

/// One object read: its slot, class name, and decoded value. A null tag is
/// `(0, "", None)`; back-refs to undecoded slots carry `None`.
#[derive(Debug, Clone)]
struct Obj {
    slot: u64,
    name: Rc<str>,
    v: Option<Rc<V>>,
}

impl Obj {
    fn null() -> Self {
        Self {
            slot: 0,
            name: Rc::from(""),
            v: None,
        }
    }
}

/// One store-map slot: a class declaration (`cls`, `name`/`schema` identify
/// the class) or a read object (`v` holds its decoded value once read).
#[derive(Debug, Clone)]
struct Entry {
    cls: bool,
    name: Rc<str>,
    #[allow(dead_code)]
    schema: i32,
    v: Option<Rc<V>>,
}

impl Entry {
    fn new(cls: bool, name: Rc<str>, schema: i32) -> Self {
        Self {
            cls,
            name,
            schema,
            v: None,
        }
    }
}

type Slots = HashMap<u64, Entry>;
type ClassSlots = HashMap<Rc<str>, u64>;

// True when the bytes at p are an MFC class-ref to class `slot`. Mirrors both
// encodings `Archive::object` decodes: the short 16-bit form (0x8000|slot)
// and, for slots past 0x7fff, the big-tag escape (0x7fff followed by a u32 of
// 0x80000000|slot).
fn is_class_ref(d: &[u8], p: usize, slot: u64) -> bool {
    if slot <= 0x7fff {
        return u16_at(d, p).is_ok_and(|t| u64::from(t) == (0x8000 | slot));
    }
    p + 6 <= d.len()
        && u16_at(d, p).is_ok_and(|t| t == 0x7fff)
        && u32_at(d, p + 2).is_ok_and(|v| u64::from(v) == (0x8000_0000 | (slot & 0xffff_ffff)))
}

struct Archive<'a> {
    r: R<'a>,
    ver: i32,
    pid: bool,
    next: u64,
    base: u64,
    current_loop: u64,
    slots: Slots,
    class_slot: ClassSlots,

    // Burned store-map indices (see the CEdgeUse branch of read()): the
    // writer maps an annotation's connection points into the store map
    // WITHOUT writing bytes, so file back-references beyond each burn run
    // ahead of the walker's numbering. Registrations always stay at WALKER
    // indices and back() translates file references through the burn bands
    // instead. `burns` holds (file_band_start, width) per event; `cum_delta`
    // their total; `annot_watermark` the walker slot right after the last
    // annotation record - the only place a band can start.
    burns: Vec<(u64, u64)>,
    cum_delta: u64,
    annot_watermark: Option<u64>,
    /// Per-entity-list burned-item credits.
    burn_stack: Vec<i32>,
    cline_tail: Option<usize>,
}

impl<'a> Archive<'a> {
    fn new(d: &'a [u8], ver: i32) -> Self {
        Self {
            r: R { d, p: 0 },
            ver,
            pid: ver >= 17,
            next: 0,
            base: 0,
            current_loop: 0,
            slots: HashMap::new(),
            class_slot: HashMap::new(),
            burns: Vec::new(),
            cum_delta: 0,
            annot_watermark: None,
            burn_stack: Vec::new(),
            cline_tail: None,
        }
    }

    fn d(&self) -> &'a [u8] {
        self.r.d
    }

    fn alloc(&mut self, e: Entry) -> u64 {
        let s = self.next;
        self.next += 1;
        self.slots.insert(s, e);
        s
    }

    fn class_slot_of(&self, name: &str) -> Option<u64> {
        self.class_slot.get(name).copied()
    }

    fn object(&mut self, expect: Option<&str>) -> Result<Obj, Error> {
        let tag = self.r.u16()?;
        if tag == 0 {
            return Ok(Obj::null());
        }
        if tag == 0x7fff {
            let big = self.r.u32()?;
            if big & 0x8000_0000 != 0 {
                return self.new_class_ref(u64::from(big & 0x7fff_ffff), expect);
            }
            return self.back(u64::from(big));
        }
        if tag == 0xffff {
            let schema = self.r.u16()?;
            let n = self.r.u16()?;
            if n > 40 {
                return Err(bad("implausible legacy class name"));
            }
            let b = self.r.raw(usize::from(n))?;
            let name: Rc<str> = Rc::from(String::from_utf8_lossy(b).as_ref());
            let cs = self.alloc(Entry::new(true, name.clone(), i32::from(schema)));
            self.class_slot.insert(name.clone(), cs);
            return self.new_obj(name);
        }
        if tag & 0x8000 != 0 {
            return self.new_class_ref(u64::from(tag & 0x7fff), expect);
        }
        self.back(u64::from(tag))
    }

    fn new_class_ref(&mut self, s: u64, expect: Option<&str>) -> Result<Obj, Error> {
        let e = match self.slots.entry(s) {
            std::collections::hash_map::Entry::Occupied(o) => o.into_mut(),
            std::collections::hash_map::Entry::Vacant(vac) => {
                let Some(expect) = expect else {
                    return Err(bad("unknown legacy class slot"));
                };
                let name: Rc<str> = Rc::from(expect);
                self.class_slot.insert(name.clone(), s);
                vac.insert(Entry::new(true, name, 0))
            }
        };
        if !e.cls {
            return Err(bad("class ref points to object"));
        }
        let name = e.name.clone();
        self.new_obj(name)
    }

    // Maps a FILE store-map index to the walker's numbering through the burn
    // bands. Returns the walker slot, or None when the reference points INTO
    // a band (a phantom, never-serialized connection point).
    fn translate_ref(&self, slot: u64) -> Option<u64> {
        let mut offset = 0;
        for &(start, width) in &self.burns {
            if slot < start {
                break;
            }
            if slot < start + width {
                return None;
            }
            offset += width;
        }
        Some(slot - offset)
    }

    fn back(&mut self, mut s: u64) -> Result<Obj, Error> {
        if let Some(&(first, _)) = self.burns.first()
            && s >= first
        {
            match self.translate_ref(s) {
                // a phantom (burned) connection-point index - annotation
                // metadata only; nothing was ever serialized for it
                None => {
                    return Ok(Obj {
                        slot: s,
                        name: Rc::from("reserved"),
                        v: None,
                    });
                }
                Some(w) => s = w,
            }
        }
        let Some(e) = self.slots.get(&s) else {
            if s < self.base {
                return Ok(Obj {
                    slot: s,
                    name: Rc::from("premodel"),
                    v: None,
                });
            }
            return Err(bad("legacy backref to unknown slot"));
        };
        if e.cls {
            return Err(bad("legacy backref to class"));
        }
        Ok(Obj {
            slot: s,
            name: e.name.clone(),
            v: e.v.clone(),
        })
    }

    // Records that the writer burned `delta` store-map indices without
    // serializing any bytes for them.
    fn register_burn(&mut self, delta: u64) {
        let start = self.annot_watermark.unwrap_or(0) + self.cum_delta;
        self.burns.push((start, delta));
        self.cum_delta += delta;
        self.annot_watermark = None;
        // each burn event corresponds to ONE phantom top-level entity that the
        // entity list's declared count includes but the stream never carries -
        // credit it so the list doesn't run past its real end
        if let Some(last) = self.burn_stack.last_mut() {
            *last += 1;
        }
    }

    // A reference-to-entity tag: dimension connection points and text leader
    // attachments. Unlike object()'s back-ref path, this tolerates a slot the
    // walk has not reached yet - SketchUp serializes a label/dimension BEFORE
    // the entity it anchors to when both live in the same entity list.
    fn entity_ref(&mut self) -> Result<Option<u64>, Error> {
        let tag = self.r.u16()?;
        if tag == 0 {
            return Ok(None);
        }
        if tag == 0x7fff {
            let big = self.r.u32()?;
            if big & 0x8000_0000 != 0 {
                return Err(bad("entity ref is a new object"));
            }
            return Ok(Some(u64::from(big)));
        }
        if tag == 0xffff || tag & 0x8000 != 0 {
            return Err(bad("entity ref is a new object"));
        }
        Ok(Some(u64::from(tag)))
    }

    // True when the u16 at `at` starts an object read in one of the
    // UNAMBIGUOUS forms: null, escape, class definition, or a class-ref to a
    // class already known. Plain object back-refs are excluded on purpose -
    // any 2-byte junk below 0x8000 would qualify.
    fn strict_next_tag(&self, at: usize, allow_null: bool) -> bool {
        let Ok(t) = u16_at(self.d(), at) else {
            return false;
        };
        if t == 0 {
            return allow_null;
        }
        if t == 0x7fff || t == 0xffff {
            return true;
        }
        if t & 0x8000 != 0 {
            return self
                .slots
                .get(&u64::from(t & 0x7fff))
                .is_some_and(|e| e.cls);
        }
        false
    }

    // The textured-material payload: an embedded CDib plus applied size,
    // source file name, average colour, and opacity. Shared between a
    // CMaterial with a texture and a colour-by-layer CLayer that carries a
    // textured material.
    //
    // The texture is an entity in its own right, so right after the u8 "has
    // texture" flag it opens with the standard entity preamble (attribute
    // container ref and, from v17 on, the persistent-id mask). Callers read
    // the flag as a u16, so step back over its high byte - the first byte of
    // that preamble.
    //
    // SketchUp 3 has neither: the image follows the flag directly as a bare
    // subtype/length/bytes record - no preamble, no object tag, no slot.
    fn texture_block(&mut self, v: &mut V) -> Result<(), Error> {
        if self.ver < FIRST_V4 {
            self.r.u32()?;
            let z = self.r.u32()?;
            if z as usize > self.d().len() {
                return Err(bad("implausible dib length"));
            }
            v.blob = self.r.raw(z as usize)?.to_vec();
            v.inline_tex = true;
        } else {
            self.r.p = self.r.p.saturating_sub(1);
            self.preamble()?;
            let q = self.object(Some("CDib"))?;
            v.tex_dib = q.slot;
        }
        let d = self.d();
        let begin = self.r.p;
        let limit = d.len().min(self.r.p + 28);
        let mut marker = begin;
        while marker + 3 <= limit {
            if is_marker(d, marker) {
                break;
            }
            marker += 1;
        }
        if marker - begin == 20 {
            self.r.u32()?;
        } else if marker - begin != 16 {
            return Err(bad("texture size block misaligned"));
        }
        v.tw = self.r.f64()?;
        v.th = self.r.f64()?;
        v.tex_file = self.r.utf16()?;
        let c = self.r.raw(9)?; // RGBA + 00 + RGBA (colour stored twice)
        v.r = i32::from(c[0]);
        v.g = i32::from(c[1]);
        v.b = i32::from(c[2]);
        // c[3] is not a colour alpha byte - it feeds the colorized check
        // below alongside blob[4]. v.a keeps its default (255).
        self.r.utf16()?;
        let blob = self.r.raw(8)?; // u32 + u32 colorized flag
        v.opacity = self.r.f64()?;
        if self.r.u8()? == 0 {
            v.opacity = 0.0;
        }
        v.colorized = blob[4] != 0 || c[3] == 255;
        Ok(())
    }

    // Returns the CAttributeContainer's slot (None for a null reference) and
    // the class name the leading object resolved to.
    fn preamble_cls(&mut self) -> Result<(Option<u64>, Rc<str>), Error> {
        let attrs = self.object(Some("CAttributeContainer"))?;
        if self.pid {
            let mask = self.r.u8()?;
            for i in 0..8 {
                if mask & (1 << i) != 0 {
                    self.r.u8()?;
                }
            }
        }
        if attrs.name.is_empty() {
            return Ok((None, attrs.name));
        }
        Ok((Some(attrs.slot), attrs.name))
    }

    fn preamble(&mut self) -> Result<Option<u64>, Error> {
        Ok(self.preamble_cls()?.0)
    }

    // `preamble` for the entity types that carry NO attribute-container
    // pointer in a SketchUp 3 file: vertex, loop, edge-use, material, layer.
    fn early_preamble(&mut self) -> Result<Option<u64>, Error> {
        if self.ver < FIRST_V4 {
            return Ok(None);
        }
        self.preamble()
    }

    fn draw(&mut self, v: &mut V) -> Result<(), Error> {
        // SketchUp 3's draw block is 5 bytes, identical for every entity in
        // the calibration file, so nothing in them is decodable. Material
        // comes from the face's own pointer instead (see the CFace reader).
        let v3 = self.ver < FIRST_V4;
        let b = self.r.raw(if v3 { 5 } else { 8 })?;
        if !v3 {
            v.mat = i32::from(b[0]) | i32::from(b[1]) << 8;
            v.hidden = i32::from(b[2]);
            v.soft = i32::from(b[5]);
            v.smooth = i32::from(b[6]);
        }
        // The layer field is normally a u16 id, but an entity can carry the
        // layer BY OBJECT instead (seen on real 2018 instances): a full inline
        // CLayer record on first use, an escaped back-ref to it on later
        // siblings. Layer ids never have the 0x8000 bit and never equal
        // 0x7fff, so both object forms are unambiguous.
        let lay_cs = self.class_slot_of("CLayer");
        let tag = u16_at(self.d(), self.r.p)?;
        if lay_cs.is_some_and(|cs| u64::from(tag) == (0x8000 | cs)) {
            self.object(Some("CLayer"))?;
            v.layer = 0; // by-object layer: keep the default id
        } else if tag == 0x7fff {
            self.r.u16()?;
            let big = self.r.u32()?;
            if big & 0x8000_0000 != 0 {
                return Err(bad("drawbase layer: unexpected class"));
            }
            v.layer = 0; // by-object layer (back-ref)
        } else {
            v.layer = i32::from(self.r.u16()?);
        }
        Ok(())
    }

    // 0xffff CLayer, a class-ref to a CLayer already in class_slot, or - when
    // `unmatched_class_ref` - any short 0x8000 class-ref. The last form is
    // only for the throwaway slot-base probe, which numbers from 1<<20 so the
    // file's 0x8000|real_slot will not match class_slot.
    fn clayer_record_at(&self, at: usize, unmatched_class_ref: bool) -> bool {
        let d = self.d();
        let Ok(tag) = u16_at(d, at) else {
            return false;
        };
        if tag == 0xffff
            && at + 12 <= d.len()
            && u16_at(d, at + 4).is_ok_and(|n| n == 6)
            && bytes_eq(d, at + 6, "CLayer")
        {
            return true;
        }
        if let Some(cs) = self.class_slot_of("CLayer")
            && is_class_ref(d, at, cs)
        {
            return true;
        }
        unmatched_class_ref && tag & 0x8000 != 0 && tag != 0xffff
    }

    // Colour CLayer ends in a 21-byte tail. Some v18 zero-material files
    // (custom tag listed first, then Layer0) write 12 zero bytes + u32 after
    // that, then another CLayer. Skip those 16 bytes only when they precede a
    // CLayer; leave a definition-list back-ref alone.
    fn skip_clayer_colour_ext(&mut self) {
        const PAD: usize = 12;
        const EXT: usize = 16;
        let d = self.d();
        if self.r.p + EXT + 2 > d.len() {
            return;
        }
        if d[self.r.p..self.r.p + PAD].iter().any(|&b| b != 0) {
            return;
        }
        if self.clayer_record_at(self.r.p + EXT, true) {
            self.r.p += EXT;
        }
    }

    // v18 template Layer0 after a custom tag: <parent u16><active u16><dc u32>.
    // Consume parent only when both u16s are already CLayer objects. A lone
    // back-ref then a definition count (usual layout) stays put.
    fn skip_clayer_parent_ref(&mut self, self_slot: u64) {
        let d = self.d();
        if self.r.p + 4 > d.len() {
            return;
        }
        let (Ok(a), Ok(b)) = (u16_at(d, self.r.p), u16_at(d, self.r.p + 2)) else {
            return;
        };
        if a == 0 || a & 0x8000 != 0 || a == 0x7fff || u64::from(a) == self_slot {
            return;
        }
        if b == 0 || b & 0x8000 != 0 || b == 0x7fff {
            return;
        }
        let (Some(ia), Some(ib)) = (self.slots.get(&u64::from(a)), self.slots.get(&u64::from(b)))
        else {
            return;
        };
        if ia.cls || ib.cls {
            return;
        }
        if &*ia.name != "CLayer" || &*ib.name != "CLayer" {
            return;
        }
        self.r.p += 2;
    }

    // Extra CLayer records past declared layer_count (v18: count=1 custom
    // tag, then Layer0). Skip null separators; stop at the definition-list
    // anchor. `unmatched_class_ref` only on the throwaway probe.
    fn collect_trailing_layers(
        &mut self,
        mut slots_out: Option<&mut Vec<u64>>,
        mut layers_out: Option<&mut Vec<(u64, Rc<V>)>>,
        unmatched_class_ref: bool,
    ) -> Result<(), Error> {
        while self.r.p + 2 <= self.d().len() {
            let tag = u16_at(self.d(), self.r.p)?;
            if tag == 0 {
                self.r.p += 2;
                continue;
            }
            if !self.clayer_record_at(self.r.p, unmatched_class_ref) {
                break;
            }
            let q = self.object(Some("CLayer"))?;
            if let Some(v) = q.v {
                if let Some(s) = slots_out.as_deref_mut() {
                    s.push(q.slot);
                }
                if let Some(l) = layers_out.as_deref_mut() {
                    l.push((q.slot, v));
                }
            }
        }
        Ok(())
    }

    fn new_obj(&mut self, n: Rc<str>) -> Result<Obj, Error> {
        let slot = self.alloc(Entry::new(false, n.clone(), 0));
        let v = Rc::new(self.read(&n, slot)?);
        if let Some(e) = self.slots.get_mut(&slot) {
            e.v = Some(v.clone());
        }
        if &*n == "CDimensionLinear" || &*n == "CText" {
            self.annot_watermark = Some(self.next);
        }
        Ok(Obj {
            slot,
            name: n,
            v: Some(v),
        })
    }

    fn read(&mut self, n: &str, self_slot: u64) -> Result<V, Error> {
        let mut v = V::default();
        match n {
            "CVertex" => {
                self.early_preamble()?;
                v.k = "vertex";
                let a = self.r.f64s(3)?;
                v.xyz = [a[0], a[1], a[2]];
            }
            "CEdge" => {
                self.preamble()?;
                v.k = "edge";
                self.draw(&mut v)?;
                v.v1 = self.object(Some("CVertex"))?.slot;
                v.v2 = self.object(Some("CVertex"))?.slot;
                self.object(None)?;
            }
            "CCurve" => {
                self.preamble()?;
                v.k = "curve";
                self.r.u8()?;
                self.r.u32()?;
            }
            "CArcCurve" => {
                self.preamble()?;
                v.k = "curve";
                self.r.raw(5)?;
                self.r.f64s(14)?;
            }
            "CEdgeUse" => {
                self.early_preamble()?;
                v.k = "edgeuse";
                v.edge = self.object(Some("CEdge"))?.slot;
                v.sense = self.r.u8()? != 0;
                // parent-loop back-ref: the alignment oracle. Read as a RAW
                // file index - after annotations the claimed index can sit
                // AHEAD of the walker's numbering (burned MapObject indices,
                // see register_burn()), which is a correction signal, not a
                // mis-parse.
                let p0 = self.r.p;
                let tag = self.r.u16()?;
                let mut parent = None;
                if tag == 0x7fff {
                    let big = self.r.u32()?;
                    if big & 0x8000_0000 != 0 {
                        return Err(bad("edge-use parent is a new object"));
                    }
                    parent = Some(u64::from(big));
                } else if tag == 0xffff || tag & 0x8000 != 0 {
                    return Err(bad("edge-use parent is a new object"));
                } else if tag != 0 {
                    parent = Some(u64::from(tag));
                }
                let expected = self.current_loop + self.cum_delta;
                if parent != Some(expected) {
                    let delta = parent.map_or(0, |p| p as i64 - expected as i64);
                    if delta > 0 && delta <= 4096 && self.annot_watermark.is_some() {
                        self.register_burn(delta as u64);
                    } else {
                        self.r.p = p0;
                        return Err(bad("edge-use parent mismatch"));
                    }
                }
            }
            "CLoop" => {
                let old = self.current_loop;
                self.current_loop = self_slot;
                self.early_preamble()?;
                self.r.raw(2)?;
                v.k = "loop";
                while self.r.p + 2 <= self.d().len() && u16_at(self.d(), self.r.p)? != 0 {
                    if let Some(q) = self.object(Some("CEdgeUse"))?.v {
                        v.uses.push(q);
                    }
                }
                self.r.u16()?;
                self.current_loop = old;
            }
            "CFace" => {
                let (attrs, lead_cls) = self.preamble_cls()?;
                v.attrs = attrs;
                v.k = "face";
                self.draw(&mut v)?;
                if self.ver < FIRST_V4 && &*lead_cls == "CMaterial" {
                    // SketchUp 3: the pointer that opens a face is its FRONT
                    // MATERIAL (null for an unpainted face), not an attribute
                    // container.
                    if let Some(a) = v.attrs.take() {
                        v.mat = a as i32;
                    }
                }
                v.plane = self.r.f64s(4)?;
                let count = self.r.u32()?;
                if count > 10000 {
                    return Err(bad("implausible loop count"));
                }
                for _ in 0..count {
                    if let Some(q) = self.object(Some("CLoop"))?.v {
                        v.loops.push(q);
                    }
                }
                v.back_mat = i32::from(self.r.u16()?);
            }
            "CAttributeContainer" => {
                self.preamble()?;
                v.k = "attrs";
                while self.r.p + 2 <= self.d().len() && u16_at(self.d(), self.r.p)? != 0 {
                    let o = self.object(Some("CAttributeNamed"))?;
                    v.ents.push(o);
                }
                self.r.u16()?;
            }
            "CAttributeNamed" => {
                self.preamble()?;
                v.k = "dict";
                self.r.raw(4)?;
                v.name = self.r.utf16()?;
                loop {
                    let key = self.r.utf16()?;
                    if key.is_empty() {
                        break;
                    }
                    let t = self.r.u8()?;
                    let val = self.typed(t)?;
                    v.entries.insert(key, val);
                }
                // Trailing u32 is a v7+ addition - a real SketchUp 6 file ends
                // the record at the empty-key terminator. Reading it
                // unconditionally eats 4 bytes of the next sibling's tag
                // (openskp#284, #385). CAttributeNamed's class is always
                // learned from the unwalked pre-model region, so its schema
                // can't gate this - the file version is the only signal.
                if self.ver >= 7 {
                    self.r.u32()?;
                }
            }
            "CLayer" => {
                v.attrs = self.early_preamble()?;
                v.k = "layer";
                v.name = self.r.utf16()?;
                let mut mid = Vec::new();
                while mid.len() < 8 && !self.r.marker() {
                    mid.push(self.r.u8()?);
                }
                v.hidden = mid.first().map_or(0, |&b| i32::from(b));
                self.r.utf16()?;
                let flags = self.r.u16()?;
                if flags & 0x00ff != 0 {
                    // Colour-by-layer with a TEXTURED material: instead of the
                    // flat RGBA, the layer embeds the same texture block a
                    // CMaterial carries. Low byte of the flag word set =
                    // textured; a plain colour layer has 0 there (its high
                    // byte carries an unrelated flag).
                    self.texture_block(&mut v)?;
                    self.r.raw(4)?; // trailing u32
                } else {
                    let c = self.r.raw(4)?;
                    v.r = i32::from(c[0]);
                    v.g = i32::from(c[1]);
                    v.b = i32::from(c[2]);
                    self.r.utf16()?;
                    // SketchUp 3's tail is 4 bytes shorter
                    self.r.raw(if self.ver < FIRST_V4 { 17 } else { 21 })?;
                    self.skip_clayer_colour_ext();
                    self.skip_clayer_parent_ref(self_slot);
                }
            }
            "CMaterial" => {
                self.early_preamble()?;
                v.k = "material";
                v.name = self.r.utf16()?;
                let flag = self.r.u16()?;
                if flag == 0 {
                    let c = self.r.raw(4)?;
                    v.r = i32::from(c[0]);
                    v.g = i32::from(c[1]);
                    v.b = i32::from(c[2]);
                    v.a = i32::from(c[3]);
                    self.r.utf16()?;
                    self.r.raw(8)?;
                    v.opacity = self.r.f64()?;
                    if self.r.u8()? == 0 {
                        v.opacity = 0.0;
                    }
                } else {
                    self.texture_block(&mut v)?;
                }
            }
            "CDib" => {
                v.k = "dib";
                self.r.u32()?;
                let z = self.r.u32()?;
                if z as usize > self.d().len() {
                    return Err(bad("implausible dib length"));
                }
                v.blob = self.r.raw(z as usize)?.to_vec();
            }
            "CFaceTextureCoords" => {
                self.preamble()?;
                v.k = "ftc";
                self.r.u32()?;
                let a = self.r.f64s(24)?;
                v.uvf = a[0..9].to_vec();
                v.uvb = a[12..21].to_vec();
                let z = self.r.u32()?;
                for _ in 0..z {
                    self.r.f64s(4)?;
                }
                let z = self.r.u32()?;
                for _ in 0..z {
                    self.r.f64s(4)?;
                }
                let fflags = self.r.u32()?;
                let bflags = self.r.u32()?;
                v.front_projected = fflags & 2 != 0;
                v.back_projected = bflags & 2 != 0;
            }
            "CCamera" => read_camera_body(&mut self.r)?,
            "CThumbnail" => {
                self.preamble()?;
                if self.ver < FIRST_V4 {
                    // SketchUp 3: a fixed 129-byte camera block, no name, no image.
                    self.r.raw(129)?;
                } else {
                    self.object(Some("CCamera"))?;
                    self.object(Some("CDib"))?;
                }
            }
            "CImage" => {
                // An Image entity - instance-shaped: a back-ref to the
                // (already walked) CComponentDefinition holding the image's
                // face and texture, a 3x4 placement, a constant 1.0, the
                // source path string, and a 16-byte GUID.
                self.preamble()?;
                v.k = "image";
                self.draw(&mut v)?;
                v.def = self.object(None)?.slot;
                v.xf = self.r.f64s(12)?;
                self.r.f64()?; // constant 1.0
                self.r.utf16()?; // source path
                v.guid = hex_guid(self.r.raw(16)?);
            }
            "CRelationship" => {
                // two object pointers binding an annotation to the entity it
                // labels; the annotation side is routinely serialized BEFORE
                // the geometry side, so these can point forward - entity_ref()
                // tolerates that.
                self.preamble()?;
                v.k = "relationship";
                self.entity_ref()?;
                self.entity_ref()?;
            }
            "CConstructionLine" => {
                self.preamble()?;
                v.k = "constructionline";
                self.draw(&mut v)?;
                // point(3) + direction(3) + two signed distance params along
                // direction marking where the visible segment starts/ends. A
                // parameter magnitude past HUGE_PARAM means unbounded.
                let lp = self.r.f64s(8)?;
                v.xyz = [lp[0], lp[1], lp[2]];
                v.direction = [lp[3], lp[4], lp[5]];
                const HUGE_PARAM: f64 = 1e20;
                let (start_param, end_param) = (lp[6], lp[7]);
                let (p, dir) = (v.xyz, v.direction);
                let at = |t: f64| [p[0] + dir[0] * t, p[1] + dir[1] * t, p[2] + dir[2] * t];
                if start_param.abs() < HUGE_PARAM {
                    v.start = Some(at(start_param));
                }
                if end_param.abs() < HUGE_PARAM {
                    v.end = Some(at(end_param));
                }
                // The trailing block varies by the WRITING BUILD, not cleanly
                // by version: 7 bytes on the v17 calibration corpus, 4 on v16
                // and on a real v18, 0 on another real v17. Self-calibrate on
                // the first guide line of the file - the length that lands on
                // a legitimate next tag (strict forms only) - and cache it.
                if self.cline_tail.is_none() {
                    let dflt = if self.ver == 17 { 7 } else { 4 };
                    let mut order = vec![dflt];
                    order.extend([0, 4, 7].into_iter().filter(|&c| c != dflt));
                    // Strong tags first (escape / known class / class
                    // definition). The weak (null) pass is the last-in-list
                    // case: a construction line that is the last entity in a
                    // definition is followed by nrel=0, which looks like a
                    // null tag at every candidate; the weak pass prefers 4
                    // (the 4-byte trailer real SketchUp 2025 uses).
                    let mut k = order
                        .iter()
                        .copied()
                        .find(|&c| self.strict_next_tag(self.r.p + c, false));
                    if k.is_none() {
                        k = [4, 0, 7]
                            .into_iter()
                            .find(|&c| self.strict_next_tag(self.r.p + c, true));
                    }
                    self.cline_tail = Some(k.unwrap_or(4));
                }
                self.r.raw(self.cline_tail.unwrap_or(4))?;
            }
            "CConstructionPoint" => {
                self.preamble()?;
                v.k = "constructionpoint";
                self.draw(&mut v)?;
                let pos = self.r.f64s(3)?;
                v.xyz = [pos[0], pos[1], pos[2]];
                self.r.f64s(3)?; // reserved/unused (observed all-zero)
                self.r.u8()?; // reserved/unused (observed 0)
            }
            "CSectionPlane" => {
                self.preamble()?;
                v.k = "sectionplane";
                self.draw(&mut v)?;
                let first = f64_at(self.d(), self.r.p)?;
                if first.abs() > 1.0001 {
                    self.object(None)?;
                }
                v.plane = self.r.f64s(4)?;
                if self.r.marker() {
                    v.name = self.r.utf16()?;
                    v.label = self.r.utf16()?;
                }
            }
            "CSkFont" => {
                self.object(Some("CAttributeContainer"))?;
                if self.pid {
                    self.r.u8()?;
                }
                self.r.utf16()?;
                self.r.raw(15)?;
            }
            "CDimensionLinear" => {
                self.preamble()?;
                v.k = "dimension";
                self.draw(&mut v)?;
                v.text = self.r.utf16()?;
                self.object(Some("CSkFont"))?;
                // The tail is NOT a fixed 165-byte blob: it embeds two object
                // references (the connection points). Each is a normal MFC
                // tag - 2 bytes in small files, 6 once the 0x7fff big-tag
                // escape kicks in.
                self.r.raw(37)?;
                self.entity_ref()?; // connection point 1 (may be null)
                self.r.raw(42)?;
                self.entity_ref()?; // connection point 2 (may be null)
                self.r.raw(82)?;
            }
            "CText" => {
                self.preamble()?;
                v.k = "text";
                self.draw(&mut v)?;
                self.object(Some("CSkFont"))?;
                let d = self.d();
                let end = d.len().min(self.r.p + 512);
                let mut found = None;
                let mut q = self.r.p;
                while q + 14 <= end {
                    if d[q] == 1
                        && d[q + 1] == 0
                        && d[q + 2] == 0
                        && d[q + 3] == 0
                        && d[q + 6] == 3
                        && d[q + 7] == 0
                        && d[q + 8] == 0
                        && d[q + 9] == 0
                        && d[q + 10] == 1
                        && d[q + 11] == 255
                        && d[q + 12] == 254
                        && d[q + 13] == 255
                    {
                        found = Some(q + 11);
                        break;
                    }
                    q += 1;
                }
                let Some(found) = found else {
                    return Err(bad("text delimiter not found"));
                };
                self.r.raw(found - self.r.p)?;
                v.text = self.r.utf16()?;
                self.r.raw(5)?;
                // Optional leader-attachment refs follow the fixed tail; they
                // can point FORWARD. Only the escaped 6-byte form is
                // recognisable without risk: a 2-byte back-ref here would be
                // indistinguishable from the next list item's tag.
                while self.r.p + 2 <= d.len() && d[self.r.p] == 255 && d[self.r.p + 1] == 127 {
                    if self.r.p + 6 > d.len() {
                        break;
                    }
                    let val = u32_at(d, self.r.p + 2)?;
                    if val & 0x8000_0000 != 0 {
                        break; // new-object tag - the next entity
                    }
                    self.r.raw(6)?;
                }
            }
            "CComponentDefinition" => self.read_definition(&mut v)?,
            "CComponentInstance" | "CGroup" => {
                v.attrs = self.preamble()?;
                v.k = "instance";
                self.draw(&mut v)?;
                v.def = self.object(Some("CComponentDefinition"))?.slot;
                v.xf = self.r.f64s(13)?;
                // SketchUp 3 instances carry no name string.
                if self.ver >= FIRST_V4 || self.r.marker() {
                    v.name = self.r.utf16()?;
                }
                if legacy_instance_has_guid(self.ver) {
                    self.r.raw(16)?;
                }
            }
            _ => return Err(bad(format!("no legacy reader for {n}"))),
        }
        Ok(v)
    }

    fn read_definition(&mut self, v: &mut V) -> Result<(), Error> {
        self.preamble()?;
        v.k = "definition";
        // undecoded base block (13 bytes in SketchUp 3)
        let base_block = if self.ver >= 17 {
            22
        } else if self.ver >= FIRST_V4 {
            20
        } else {
            13
        };
        self.r.raw(base_block)?;
        let nl = self.r.u32()?;
        if nl > 10000 {
            return Err(bad("implausible def layers"));
        }
        // like the model-level layer list, the count is REAL layers (new
        // records or back-refs); SketchUp 2020 interleaves null separators
        let mut got = 0;
        while got < nl {
            if self.r.p + 2 <= self.d().len() && u16_at(self.d(), self.r.p)? == 0 {
                self.r.p += 2;
                continue;
            }
            self.object(Some("CLayer"))?;
            got += 1;
        }
        let decl = self.r.u16()?;
        if decl == 0x7fff {
            self.r.u32()?;
        }
        // v20 can drop its undocumented filler right here, swallowing the u32
        // field (and, behind a layer-separator null, even the decl itself): if
        // the empty-string marker sits in the next few bytes, the real count
        // is the first non-zero u32 after its padding.
        let mut count = 0;
        let mut filled = false;
        if self.ver >= 20
            && let Some(c) = self.retry_count_after_v20_filler(self.r.p, 5_000_000)
        {
            count = c;
            filled = true;
        }
        if !filled {
            self.r.u32()?;
            count = self.r.u32()?;
        }
        // A zero count is as much a symptom of the v20 filler as an
        // implausibly large one: the reader lands on the leading zero bytes
        // of the filler instead of the count. A genuinely empty definition
        // reads zero with no filler ahead, and the retry leaves those alone.
        if (count > 5_000_000 || count == 0)
            && let Some(retry) = self.retry_count_after_v20_filler(self.r.p - 4, 5_000_000)
        {
            count = retry;
        }
        if count > 5_000_000 {
            return Err(bad("implausible def entities"));
        }
        v.ents = self.entity_list(count, false)?;
        let mut nr = self.r.u32()?;
        if nr > 100_000
            && let Some(retry) = self.retry_count_after_v20_filler(self.r.p - 4, 100_000)
        {
            nr = retry;
        }
        if nr > 100_000 {
            return Err(bad("definition list misaligned"));
        }
        for _ in 0..nr {
            self.object(Some("CRelationship"))?;
        }
        self.r.u16()?;
        // The GUID is followed immediately by the name string. Some files
        // (SketchUp 2020) carry two extra bytes ahead of the GUID; anchor on
        // the string marker that must follow the 16 GUID bytes instead of
        // trusting the fixed prefix width.
        let d = self.d();
        let anchored = |at: usize| at + 19 <= d.len() && is_marker(d, at + 16);
        if !anchored(self.r.p) {
            for skip in 1..=4 {
                let at = self.r.p + skip;
                if anchored(at) {
                    self.r.p = at;
                    break;
                }
            }
        }
        v.guid = hex_guid(self.r.raw(16)?);
        v.name = self.r.utf16()?;
        self.r.utf16()?;
        self.r.utf16()?;
        self.r.u32()?;
        let mut tpos = None;
        let thumb_cs = self.class_slot_of("CThumbnail");
        let mut off = 0;
        while off < 96 && self.r.p + off + 26 <= d.len() {
            let p = self.r.p + off;
            if d[p] == 255
                && d[p + 1] == 255
                && d[p + 4] == 10
                && d[p + 5] == 0
                && bytes_eq(d, p + 6, "CThumbnail")
            {
                tpos = Some(p);
                break;
            }
            if thumb_cs.is_some_and(|cs| is_class_ref(d, p, cs)) {
                tpos = Some(p);
                break;
            }
            off += 1;
        }
        let Some(tpos) = tpos else {
            return Err(bad("definition thumbnail not found"));
        };
        let gap = self.r.raw(tpos - self.r.p)?;
        let flag = if gap.len() >= 9 {
            gap[gap.len() - 9]
        } else {
            0
        };
        v.faces_camera = flag & 1 != 0;
        v.shadows_face_sun = flag & 2 != 0;
        self.object(Some("CThumbnail"))?;
        Ok(())
    }

    // Reads one typed CAttributeNamed value off the stream.
    fn typed(&mut self, t: u8) -> Result<AttrValue, Error> {
        Ok(match t {
            0 => AttrValue::Null,
            4 => AttrValue::Integer(i64::from(i32_at(self.r.raw(4)?, 0)?)),
            6 => AttrValue::Float(self.r.f64()?),
            7 => AttrValue::Boolean(self.r.u8()? != 0),
            9 => AttrValue::Integer(i64::from(self.r.u32()?)),
            10 => AttrValue::String(self.r.utf16()?),
            12 => AttrValue::Length(self.r.f64()?), // Length (a double, inches)
            11 => {
                let n = self.r.u32()?;
                if n > 100_000 {
                    return Err(bad("attr array too large"));
                }
                let mut items = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    let t = self.r.u8()?;
                    items.push(self.typed(t)?);
                }
                AttrValue::Array(items)
            }
            // 3D point (Geom::Point3d) / 3D vector (Geom::Vector3d)
            17 | 18 => {
                let v = self.r.f64s(3)?;
                AttrValue::Vec([v[0], v[1], v[2]])
            }
            _ => return Err(bad("unknown legacy attribute type")),
        })
    }

    fn entity_list(&mut self, n: u32, root: bool) -> Result<Vec<Obj>, Error> {
        self.burn_stack.push(0);
        let res = self.entity_list_inner(n, root);
        self.burn_stack.pop();
        res
    }

    fn entity_list_inner(&mut self, n: u32, root: bool) -> Result<Vec<Obj>, Error> {
        let mut v = Vec::new();
        while v.len() < n as usize {
            let save = self.r.p;
            let d = self.d();
            let has_burn_credit = !root && self.burn_stack.last().is_some_and(|&c| c > 0);
            if has_burn_credit
                && save + 25 <= d.len()
                && u32_at(d, save)? == 0
                && is_marker(d, save + 22)
            {
                // burned MapObject indices mean the declared count includes
                // phantom entities the stream never carries; the definition
                // tail signature (nrel=0 + pad + 16-byte GUID + name marker at
                // +22) marks the list's REAL end
                break;
            }
            match self.object(None) {
                Ok(o) => v.push(o),
                Err(e) => {
                    if root {
                        // over-declared root counts run into the document
                        // tail - stop
                        self.r.p = save;
                        break;
                    }
                    if has_burn_credit {
                        // the phantom connection points were also counted as
                        // items, so the declared count overshoots the real
                        // records. Stop at the failed item - the definition
                        // tail that follows validates the cut.
                        self.r.p = save;
                        break;
                    }
                    return Err(e);
                }
            }
        }
        Ok(v)
    }

    // True when the u16 at `at` can legally start an object read: a null, an
    // escape, a class definition, a class-ref to a KNOWN class, or an object
    // back-ref within the allocated range.
    fn plausible_list_tag(&self, at: usize) -> bool {
        let Ok(t) = u16_at(self.d(), at) else {
            return false;
        };
        if t == 0 || t == 0x7fff || t == 0xffff {
            return true;
        }
        if t & 0x8000 != 0 {
            return self
                .slots
                .get(&u64::from(t & 0x7fff))
                .is_some_and(|e| e.cls);
        }
        u64::from(t) < self.next
    }

    // SketchUp 2020's filler (see find_count_after_v20_filler) can appear at
    // MORE call sites than a count-only read - a v20 layer-separator null
    // before a definition's decl field, for one. Same byte-stride search, but
    // each candidate must ALSO look like a legal object-read start
    // (plausible_list_tag) to reduce false positives. This only ever runs
    // after a count came back implausible (or zero), or on v20 files.
    //
    // count_pos is the offset the count was read FROM. On success moves the
    // cursor past the count and returns it.
    fn retry_count_after_v20_filler(&mut self, count_pos: usize, limit: u32) -> Option<u32> {
        let d = self.d();
        let marker_at = find_filler_marker(d, count_pos)?;
        for pad in (1..=MAX_V20_FILLER_PAD).step_by(4) {
            let at = marker_at + 4 + pad;
            if at + 4 > d.len() {
                break;
            }
            let count = u32_at(d, at).ok()?;
            if count > 0 && count <= limit && self.plausible_list_tag(at + 4) {
                self.r.p = at + 4;
                return Some(count);
            }
        }
        None
    }
}

struct WalkResult<'a> {
    ar: Archive<'a>,
    root: Vec<Obj>,
    layers: Vec<(u64, Rc<V>)>,
    materials: Vec<(u64, Rc<V>)>,
}

// Bootstrap the absolute slot base: parse material 1 with a throwaway
// archive; material 2's class-ref tag names CMaterial's true slot.
fn bootstrap_two_materials(data: &[u8], ver: i32, mat_hdr: usize) -> Result<u64, Error> {
    let mut boot = Archive::new(data, ver);
    boot.next = 1 << 20;
    boot.base = 1 << 20;
    boot.r.p = mat_hdr;
    boot.object(Some("CMaterial"))?;
    let tag = u16_at(data, boot.r.p)?;
    if tag == 0xffff || tag & 0x8000 == 0 {
        return Err(bad("cannot bootstrap the slot base"));
    }
    Ok(u64::from(tag & 0x7fff))
}

// Slot-base candidates for files where the two-material trick is unavailable
// (0 or 1 materials).
//
// Parse the model prefix (materials, layer list) with a throwaway base; the
// object right after the layer list is the definition-list anchor - an
// ABSOLUTE back-ref to the active layer, an object we just allocated
// relatively. Each walked layer yields one candidate base; with a single
// layer (the common case) the answer is exact.
fn probe_layer_anchor_bases(
    data: &[u8],
    ver: i32,
    start: usize,
    mat_count: u32,
) -> Result<Vec<u64>, Error> {
    const B0: u64 = 1 << 20;
    let mut boot = Archive::new(data, ver);
    boot.next = B0;
    boot.base = B0;
    boot.r.p = start;
    for _ in 0..mat_count {
        boot.object(Some("CMaterial"))?;
    }
    boot.r.u32()?;
    if ver >= 17 {
        boot.r.u8()?;
    }
    let layer_count = boot.r.u32()?;
    if !(1..=100_000).contains(&layer_count) {
        return Err(bad("implausible layer count in base probe"));
    }
    let mut layer_slots = Vec::new();
    for _ in 0..layer_count {
        layer_slots.push(boot.object(Some("CLayer"))?.slot);
    }
    boot.collect_trailing_layers(Some(&mut layer_slots), None, true)?;
    let anchor = boot.object(None)?;
    if &*anchor.name != "premodel" {
        // under the throwaway base every absolute back-ref classifies as
        // premodel; anything else means the prefix did not parse
        return Err(bad(format!(
            "base probe: anchor resolved to {}",
            anchor.name
        )));
    }
    let s = anchor.slot as i64;
    Ok(layer_slots
        .iter()
        .map(|&rel| s - (rel as i64 - B0 as i64))
        .filter(|&c| c > 0 && (c as u64) < B0)
        .map(|c| c as u64)
        .collect())
}

// Tries each candidate base in turn, returning the first one that walks cleanly.
fn walk_with_bases<'a>(
    data: &'a [u8],
    ver: i32,
    start: usize,
    mat_count: u32,
    bases: &[u64],
) -> Result<WalkResult<'a>, Error> {
    let mut last = None;
    for &base in bases {
        match walk_model(data, ver, start, mat_count, base) {
            Ok(w) => return Ok(w),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| bad("no viable slot base candidate")))
}

fn walk_model(
    data: &[u8],
    ver: i32,
    start: usize,
    mat_count: u32,
    base: u64,
) -> Result<WalkResult<'_>, Error> {
    let mut ar = Archive::new(data, ver);
    ar.next = base;
    ar.base = base;
    ar.r.p = start;
    let mut mats = Vec::new();
    let mut layers = Vec::new();
    for _ in 0..mat_count {
        let q = ar.object(Some("CMaterial"))?;
        if let Some(v) = q.v {
            mats.push((q.slot, v));
        }
    }
    ar.r.u32()?;
    if ver >= 17 {
        ar.r.u8()?;
    }
    // SketchUp 3 writes no layer count: the layer records simply run until
    // the definition-list anchor, which the loop below already stops at.
    let lc = if ver < FIRST_V4 {
        1u32 << 30
    } else {
        ar.r.u32()?
    };
    if lc > 100_000 && ver >= FIRST_V4 {
        return Err(bad("invalid layer count"));
    }
    // lc counts REAL layers. SketchUp 2020 interleaves a null object-ref
    // after each layer record (a separator, not a layer); count parsed layers
    // instead, skip the separators, and stop early if the next tag is a
    // back-ref (the definition-list anchor) - a v20 variant where the count
    // over-includes separators.
    while layers.len() < lc as usize {
        if ar.r.p + 2 > data.len() {
            break;
        }
        let tag = u16_at(data, ar.r.p)?;
        if tag == 0 {
            ar.r.p += 2;
            continue;
        }
        if tag != 0xffff && tag & 0x8000 == 0 {
            break;
        }
        let q = ar.object(Some("CLayer"))?;
        let Some(v) = q.v else { continue };
        layers.push((q.slot, v));
    }
    // trailing separators (and any layer records past the declared count)
    ar.collect_trailing_layers(None, Some(&mut layers), false)?;
    let anchor = ar.object(None)?;
    if &*anchor.name != "CLayer" {
        return Err(bad("definition anchor is not a layer"));
    }
    let mut dc = ar.r.u32()?;
    if dc > 1_000_000
        && let Some(retry) = ar.retry_count_after_v20_filler(ar.r.p - 4, 1_000_000)
    {
        dc = retry;
    }
    if dc > 1_000_000 {
        return Err(bad("invalid definition count"));
    }
    for _ in 0..dc {
        ar.object(Some("CComponentDefinition"))?;
    }
    let cs = ar.class_slot_of("CComponentDefinition");
    while ar.r.p + 2 <= data.len() {
        let t = u16_at(data, ar.r.p)?;
        let mut yes = cs.is_some_and(|cs| u64::from(t) == (0x8000 | cs));
        if !yes && t == 0xffff && ar.r.p + 26 <= data.len() {
            yes = bytes_eq(data, ar.r.p + 6, "CComponentDefinition");
        }
        if !yes {
            break;
        }
        ar.object(None)?;
    }
    let mut root_count = ar.r.u32()?;
    if root_count > 5_000_000
        && let Some(retry) = ar.retry_count_after_v20_filler(ar.r.p - 4, 5_000_000)
    {
        root_count = retry;
    }
    if root_count > 5_000_000 {
        return Err(bad("implausible root entity count"));
    }
    let root = ar.entity_list(root_count, true)?;
    Ok(WalkResult {
        ar,
        root,
        layers,
        materials: mats,
    })
}

fn add_edge(b: &mut GeometryBuilder, s: u64, e: &V, slots: &Slots) {
    let id = s as EntityId;
    if b.edges.contains_key(&id) {
        return;
    }
    for vid in [e.v1, e.v2] {
        if let Some(v) = slots.get(&vid).and_then(|x| x.v.as_ref())
            && v.k == "vertex"
        {
            b.vertices.insert(vid as EntityId, v.xyz);
        }
    }
    b.edges
        .insert(id, (Some(e.v1 as EntityId), Some(e.v2 as EntityId)));
    let f = (if e.soft != 0 { 8 } else { 0 })
        | (if e.smooth != 0 { 16 } else { 0 })
        | (if e.hidden != 0 { 1 } else { 0 });
    if f != 0 {
        b.edge_flags.insert(id, f);
    }
}

// SketchUp's Dynamic Components extension stores its data in an attribute
// dictionary literally named "dynamic_attributes". This collects every named
// dictionary of the entity's attribute container.
fn extract_legacy_attribute_dictionaries(attrs_slot: Option<u64>, slots: &Slots) -> AttrDicts {
    let mut out = AttrDicts::new();
    let Some(a) = attrs_slot else {
        return out;
    };
    let Some(av) = slots.get(&a).and_then(|e| e.v.as_ref()) else {
        return out;
    };
    for ent in &av.ents {
        let Some(ev) = &ent.v else { continue };
        if ev.k != "dict" || ev.name.is_empty() {
            continue;
        }
        out.insert(ev.name.clone(), ev.entries.clone());
    }
    out
}

fn fill(b: &mut GeometryBuilder, ents: &[Obj], slots: &Slots) {
    for x in ents {
        let s = x.slot;
        let Some(v) = &x.v else { continue };
        match v.k {
            "edge" => add_edge(b, s, v, slots),
            "face" => {
                let mut f = RawFace::default();
                if let [x, y, z, ..] = v.plane[..] {
                    f.normal = [x, y, z];
                }
                if v.mat != 0 {
                    f.material_id = Some(EntityId::from(v.mat));
                }
                if v.back_mat != 0 {
                    f.back_material_id = Some(EntityId::from(v.back_mat));
                }
                f.hidden = v.hidden != 0;
                for lp in &v.loops {
                    let mut co = Vec::new();
                    for u in &lp.uses {
                        if let Some(ev) = slots.get(&u.edge).and_then(|e| e.v.as_ref()) {
                            add_edge(b, u.edge, ev, slots);
                            co.push(CoEdge {
                                edge_id: u.edge as EntityId,
                                orientation: if u.sense { -1 } else { 1 },
                            });
                        }
                    }
                    f.loops.push(co);
                }
                // A positioned/photo-fitted texture mapping
                // (CFaceTextureCoords) lives as a child of this face's
                // attribute container, alongside any CAttributeNamed
                // dictionaries.
                if let Some(av) = v
                    .attrs
                    .and_then(|a| slots.get(&a))
                    .and_then(|e| e.v.as_ref())
                {
                    for ent in &av.ents {
                        let Some(ev) = &ent.v else { continue };
                        if ev.k != "ftc" {
                            continue;
                        }
                        if let Ok(m) = <[f64; 9]>::try_from(&ev.uvf[..]) {
                            f.uv_transform = Some(m);
                        }
                        if let Ok(m) = <[f64; 9]>::try_from(&ev.uvb[..]) {
                            f.uv_transform_back = Some(m);
                        }
                        f.uv_projected = ev.front_projected;
                        f.uv_projected_back = ev.back_projected;
                    }
                }
                b.faces.insert(s as EntityId, f);
            }
            "instance" => {
                let mut i = RawInstance {
                    name: v.name.clone(),
                    ref_idx: Some(v.def as EntityId),
                    matrix: v.xf.clone(),
                    hidden: v.hidden != 0,
                    ..Default::default()
                };
                if v.mat != 0 {
                    i.material_id = Some(EntityId::from(v.mat));
                }
                if v.layer != 0 {
                    i.layer = v.layer.to_string();
                }
                let dicts = extract_legacy_attribute_dictionaries(v.attrs, slots);
                if let Some(dc) = dicts.get("dynamic_attributes") {
                    i.properties = stringify_attr_dict(dc);
                }
                i.attribute_dicts = dicts;
                b.instances.push(i);
            }
            "image" => {
                // Placed exactly like an ordinary component instance; the
                // definition it points at is flagged is_image.
                let mut i = RawInstance {
                    ref_idx: Some(v.def as EntityId),
                    ref_guid: v.guid.clone(),
                    matrix: v.xf.clone(),
                    hidden: v.hidden != 0,
                    ..Default::default()
                };
                if v.mat != 0 {
                    i.material_id = Some(EntityId::from(v.mat));
                }
                if v.layer != 0 {
                    i.layer = v.layer.to_string();
                }
                b.instances.push(i);
            }
            "sectionplane" => {
                let mut sp = SectionPlane::default();
                if let [a, bb, c, d] = v.plane[..] {
                    sp.plane = [a, bb, c, d];
                }
                sp.name = v.name.clone();
                sp.label = v.label.clone();
                sp.hidden = v.hidden != 0;
                b.section_planes.push(sp);
            }
            "text" => b.texts.push(TextEntity {
                text: v.text.clone(),
                hidden: v.hidden != 0,
            }),
            "dimension" => b.dimensions.push(Dimension {
                text: v.text.clone(),
                hidden: v.hidden != 0,
                ..Default::default()
            }),
            "constructionline" => b.construction_lines.push(ConstructionLine {
                point: v.xyz,
                direction: v.direction,
                start: v.start,
                end: v.end,
            }),
            "constructionpoint" => b
                .construction_points
                .push(ConstructionPoint { position: v.xyz }),
            _ => {}
        }
    }
}

// ── pages (scenes) ────────────────────────────────────────────────────
//
// CViewPage's full record embeds a camera, an optional thumbnail image, a
// font, and - whenever any of its several independent "use_*" capture flags
// beyond hidden-layers is set - an entire Style sub-object with no
// documented fixed size. Only the narrow case is supported: a page whose
// captured flags are exactly the baseline (nothing captured) or baseline +
// hidden-layers. Ground truth was captured from real v17-native saves.
const PAGE_ALLOWED_FLAGS: [u32; 2] = [0xc00, 0xc20];
const PAGE_HIDDEN_LAYERS_BIT: u32 = 0x20;

/// A minimal view into an archive's slot-table bookkeeping - the only pieces
/// the page scanner needs.
struct LegacySlotTable<'s> {
    slots: &'s mut Slots,
    class_slot: &'s mut ClassSlots,
    next: &'s mut u64,
}

impl LegacySlotTable<'_> {
    fn alloc(&mut self, e: Entry) -> u64 {
        let s = *self.next;
        *self.next += 1;
        self.slots.insert(s, e);
        s
    }
}

// Consume one reference to an object of `cls_name` - a null tag, a back-ref
// (left unresolved), a class-ref to an already-declared class, or a fresh
// declaration - without relying on the referenced object's slot being known
// the way Archive::object() requires. The page scan runs over a region of the
// file the main walk never visits, so back-refs into it can't be resolved.
fn skip_typed_ref(
    slots: &mut LegacySlotTable<'_>,
    r: &mut R<'_>,
    cls_name: &str,
    read_body: fn(&mut R<'_>) -> Result<(), Error>,
) -> Result<(), Error> {
    let tag = r.peek_u16()?;
    if tag == 0 {
        r.u16()?;
        return Ok(());
    }
    if tag == 0xffff {
        r.u16()?;
        let schema = r.u16()?;
        let namelen = r.u16()?;
        if namelen > 40 {
            return Err(bad("implausible class name length"));
        }
        let raw_name = r.raw(usize::from(namelen))?;
        let name = String::from_utf8_lossy(raw_name);
        if name != cls_name {
            return Err(bad(format!("expected {cls_name} decl, got {name}")));
        }
        if !slots.class_slot.contains_key(cls_name) {
            let n: Rc<str> = Rc::from(cls_name);
            let s = slots.alloc(Entry::new(true, n.clone(), i32::from(schema)));
            slots.class_slot.insert(n, s);
        }
        return read_body(r);
    }
    if tag & 0x8000 != 0 {
        let cslot = u64::from(tag & 0x7fff);
        let ok = slots
            .slots
            .get(&cslot)
            .is_some_and(|e| e.cls && &*e.name == cls_name);
        if !ok {
            return Err(bad(format!("unexpected {cls_name} class-ref")));
        }
        r.u16()?;
        return read_body(r);
    }
    r.u16()?; // plain back-ref - the referenced value isn't needed here
    Ok(())
}

// Same byte layout as the CCamera/CDib branches of Archive::read.
fn read_camera_body(r: &mut R<'_>) -> Result<(), Error> {
    r.raw(137)?;
    r.u16()?;
    r.utf16()?;
    r.raw(33)?;
    Ok(())
}

fn read_dib_body(r: &mut R<'_>) -> Result<(), Error> {
    r.u32()?;
    let z = r.u32()?;
    if z as usize > r.d.len() {
        return Err(bad("implausible dib length"));
    }
    r.raw(z as usize)?;
    Ok(())
}

// Best-effort scan for CViewPage (scene) records: name + hidden-layer slot ids
// only, for the narrow flag combinations this reader understands.
//
// Runs as an independent scan over the file tail (from where the main entity
// walk stops) rather than as a sequential archive read: each candidate is
// validated by its own local structure (a name string immediately followed by
// an empty second string and a recognized flags word), and any candidate that
// doesn't fully validate is silently skipped rather than guessed at.
fn scan_pages_for_layers(
    data: &[u8],
    start_pos: usize,
    mut slots: LegacySlotTable<'_>,
) -> Result<Vec<RawPage>, Error> {
    let mut pages = Vec::new();
    let mut pos = start_pos;
    let n = data.len();
    while let Some(idx) = (pos..n.saturating_sub(2)).find(|&i| is_marker(data, i)) {
        pos = idx + 1;
        let nlen_pos = idx + 3;
        if nlen_pos >= n {
            break;
        }
        let nlen = data[nlen_pos];
        if nlen == 0 || nlen > 40 {
            continue;
        }
        let name_start = nlen_pos + 1;
        let name_end = name_start + 2 * usize::from(nlen);
        if name_end + 7 > n {
            continue;
        }
        if !(is_marker(data, name_end) && data[name_end + 3] == 0) {
            continue; // second name must be empty
        }
        let flags_pos = name_end + 4;
        // may run 1 byte past the end; the error then drops every page (as
        // OpenSKP's caller does)
        let flags = u32_at(data, flags_pos)?;
        if !PAGE_ALLOWED_FLAGS.contains(&flags) {
            continue;
        }
        let name = decode_utf16le(&data[name_start..name_end]);
        let mut r = R {
            d: data,
            p: flags_pos + 4,
        };
        let mut candidate = || -> Result<Vec<EntityId>, Error> {
            skip_typed_ref(&mut slots, &mut r, "CCamera", read_camera_body)?;
            skip_typed_ref(&mut slots, &mut r, "CDib", read_dib_body)?;
            let mut hidden_ids = Vec::new();
            if flags & PAGE_HIDDEN_LAYERS_BIT != 0 {
                loop {
                    let save = r.p;
                    let v = r.u16()?;
                    let is_layer = slots
                        .slots
                        .get(&u64::from(v))
                        .is_some_and(|e| !e.cls && &*e.name == "CLayer");
                    if is_layer {
                        hidden_ids.push(EntityId::from(v));
                    } else {
                        r.p = save;
                        break;
                    }
                }
                if r.u32()? != 1 {
                    return Err(bad("unexpected hidden-layers marker"));
                }
            }
            Ok(hidden_ids)
        };
        if let Ok(hidden_layer_ids) = candidate() {
            pages.push(RawPage {
                name,
                hidden_layer_ids,
                ..Default::default()
            });
        }
    }
    Ok(pages)
}

pub fn parse_legacy(data: &[u8]) -> Result<RawParsed, Error> {
    let mut out = RawParsed {
        version: crate::vff::extract_version(data),
        ..Default::default()
    };
    // Legacy files carry no meta/meta.dat container, so `units` stays unset.
    parse_into(data, &mut out)
        .map_err(|e| Error::Malformed(format!("legacy .skp parse failed: {e}")))?;
    Ok(out)
}

fn parse_into(data: &[u8], out: &mut RawParsed) -> Result<(), Error> {
    let ascii: String = data
        .iter()
        .take(96)
        .filter(|&&b| b != 0)
        .map(|&b| char::from(b))
        .collect();
    let re = Regex::new(r"\{([0-9]+)\.").map_err(|e| bad(e.to_string()))?;
    let ver: i32 = re
        .captures(&ascii)
        .and_then(|c| c[1].parse().ok())
        .ok_or_else(|| bad("no version in legacy header"))?;

    // anchor: the material manager (u32 count right before the first
    // CMaterial new-class record); zero-material files have no CMaterial
    // record anywhere, so fall back to the first CLayer class record and
    // start at the layer-list marker just before it
    let class_record = |i: usize, name: &str| {
        data[i] == 255
            && data[i + 1] == 255
            && u16_at(data, i + 4).is_ok_and(|n| usize::from(n) == name.len())
            && bytes_eq(data, i + 6, name)
    };
    let mh = (0..data.len().saturating_sub(15)).find(|&i| class_record(i, "CMaterial"));
    let start;
    let mc;
    match mh {
        Some(mh) if mh >= 4 => {
            start = mh;
            mc = u32_at(data, mh - 4)?;
            if mc > 100_000 {
                return Err(bad("implausible material count"));
            }
        }
        _ => {
            let lh = (0..data.len().saturating_sub(12))
                .find(|&i| class_record(i, "CLayer"))
                .ok_or_else(|| bad("no CMaterial or CLayer class record found"))?;
            mc = 0;
            start = lh
                .checked_sub(if ver >= 17 { 9 } else { 8 })
                .ok_or(Error::Truncated)?;
        }
    }

    let bases = if mc >= 2 {
        vec![bootstrap_two_materials(data, ver, start)?]
    } else {
        probe_layer_anchor_bases(data, ver, start, mc)?
    };

    let WalkResult {
        mut ar,
        root,
        layers,
        materials,
    } = walk_with_bases(data, ver, start, mc, &bases)?;

    for (slot, v) in &materials {
        let mut x = RawMaterial {
            name: v.name.clone(),
            r: v.r,
            g: v.g,
            b: v.b,
            a: v.a,
            transparency: (1.0 - v.opacity).clamp(0.0, 1.0),
            colorized: v.colorized,
            colorize_type: i32::from(v.colorized),
            texture: None,
        };
        if v.tex_dib != 0 || v.inline_tex {
            let mut t = RawTexture {
                filename: v.tex_file.clone(),
                x_scale: v.tw,
                y_scale: v.th,
                data: None,
            };
            if v.inline_tex {
                t.data = Some(v.blob.clone());
            } else if let Some(dv) = ar.slots.get(&v.tex_dib).and_then(|e| e.v.as_ref()) {
                t.data = Some(dv.blob.clone());
            }
            if t.filename.is_empty() {
                let png = t
                    .data
                    .as_ref()
                    .is_some_and(|d| d.len() >= 4 && d[0] == 0x89);
                t.filename = format!("{}{}", v.name, if png { ".png" } else { ".jpg" });
            }
            x.texture = Some(t);
        }
        out.material_id_to_name
            .insert(*slot as EntityId, x.name.clone());
        out.insert_material(x);
    }
    for (slot, l) in &layers {
        out.layer_id_to_name
            .insert(*slot as EntityId, l.name.clone());
        if !out.layer_colors.contains_key(&l.name) {
            out.layer_order.push(l.name.clone());
        }
        out.layer_colors
            .insert(l.name.clone(), [l.r as u8, l.g as u8, l.b as u8]);
        out.layer_hidden.insert(l.name.clone(), l.hidden != 0);
        let dicts = extract_legacy_attribute_dictionaries(l.attrs, &ar.slots);
        // Layer::attribute_dictionaries stays string-typed by design.
        if !dicts.is_empty() {
            out.layer_attribute_dictionaries.insert(
                l.name.clone(),
                dicts
                    .iter()
                    .map(|(k, d)| (k.clone(), stringify_attr_dict(d)))
                    .collect(),
            );
        }
    }
    if !out.layer_colors.contains_key("Layer0") {
        out.layer_order.push("Layer0".into());
        out.layer_colors.insert("Layer0".into(), [136, 136, 136]);
    }
    out.layer_hidden.entry("Layer0".into()).or_insert(false);

    // Scanned before the definitions loop so is_image is set the first time a
    // definition is built.
    let image_def_ids: HashSet<u64> = ar
        .slots
        .values()
        .filter(|e| !e.cls && &*e.name == "CImage")
        .filter_map(|e| e.v.as_ref().map(|v| v.def))
        .collect();
    for (&slot, e) in &ar.slots {
        if e.cls || &*e.name != "CComponentDefinition" {
            continue;
        }
        let Some(v) = &e.v else { continue };
        let mut d = RawDefinition {
            name: v.name.clone(),
            guid: v.guid.clone(),
            always_faces_camera: v.faces_camera,
            shadows_face_sun: v.shadows_face_sun,
            is_image: image_def_ids.contains(&slot),
            ..Default::default()
        };
        fill(&mut d.builder, &v.ents, &ar.slots);
        out.definitions.insert(slot as EntityId, d);
    }
    fill(&mut out.root.builder, &root, &ar.slots);
    let start_pos = ar.r.p;
    let table = LegacySlotTable {
        slots: &mut ar.slots,
        class_slot: &mut ar.class_slot,
        next: &mut ar.next,
    };
    out.pages = scan_pages_for_layers(data, start_pos, table).unwrap_or_default();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Model;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    // ── v20 filler probe on synthetic records ─────────────────────────
    //
    // Layout: <ff fe ff> <u8 0> (empty string), zero padding (pad % 4 == 1),
    // <u32 count>, then whatever record comes next.
    fn filler(count: u32, pad: usize) -> Vec<u8> {
        let mut b = vec![0xff, 0xfe, 0xff, 0x00];
        b.extend(std::iter::repeat_n(0, pad));
        b.extend(count.to_le_bytes());
        b.extend([0xff, 0xff, 0x0b, 0x00]);
        b
    }

    #[test]
    fn v20_filler_reads_counts_and_paddings_seen_in_real_files() {
        let hit = find_count_after_v20_filler(&filler(20, 9), 0, 1_000_000).unwrap();
        assert_eq!(
            hit,
            V20FillerHit {
                count: 20,
                next: 17
            }
        );
        let hit = find_count_after_v20_filler(&filler(5425, 13), 0, 5_000_000).unwrap();
        assert_eq!(
            hit,
            V20FillerHit {
                count: 5425,
                next: 21
            }
        );
    }

    #[test]
    fn v20_filler_reads_a_count_that_is_a_multiple_of_256() {
        // a 0x00 low byte is indistinguishable from padding to a byte scan
        for count in [256, 512, 1024, 65536, 16_777_216 / 16] {
            for pad in [9, 13] {
                let hit = find_count_after_v20_filler(&filler(count, pad), 0, 5_000_000);
                assert_eq!(hit.map(|h| h.count), Some(count), "count={count} pad={pad}");
            }
        }
    }

    #[test]
    fn v20_filler_ignores_a_non_empty_string_record() {
        let b = [0xff, 0xfe, 0xff, 0x05, 0, 0, 0, 0, 0, 0, 0, 0, 20, 0, 0, 0];
        assert_eq!(find_count_after_v20_filler(&b, 0, 1_000_000), None);
    }

    #[test]
    fn v20_filler_needs_a_marker_and_stays_in_bounds() {
        assert_eq!(find_count_after_v20_filler(&[0; 32], 0, 1_000_000), None);
        let b = [0xff, 0xfe, 0xff, 0x00, 0, 0];
        assert_eq!(find_count_after_v20_filler(&b, 0, 1_000_000), None);
    }

    #[test]
    fn v20_filler_respects_the_callers_limit() {
        assert_eq!(
            find_count_after_v20_filler(&filler(200_000, 13), 0, 100_000),
            None
        );
        let hit = find_count_after_v20_filler(&filler(200_000, 13), 0, 5_000_000);
        assert_eq!(hit.map(|h| h.count), Some(200_000));
    }

    // ── page scan on synthetic records ────────────────────────────────
    fn page_record(name: &str, flags: u32, hidden_ids: &[u16]) -> Vec<u8> {
        let mut out = vec![255, 254, 255, name.len() as u8];
        for c in name.bytes() {
            out.extend([c, 0]);
        }
        out.extend([255, 254, 255, 0]); // second name: always empty
        out.extend(flags.to_le_bytes());
        out.extend([0, 0, 0, 0]); // camera and thumbnail dib: null refs
        if flags & PAGE_HIDDEN_LAYERS_BIT != 0 {
            for id in hidden_ids {
                out.extend(id.to_le_bytes());
            }
            out.extend(1u32.to_le_bytes()); // marker (also stops the u16 list)
        }
        out
    }

    fn scan(data: &[u8], slots: &mut Slots) -> Vec<RawPage> {
        let mut class_slot = ClassSlots::new();
        let mut next = 0;
        let table = LegacySlotTable {
            slots,
            class_slot: &mut class_slot,
            next: &mut next,
        };
        scan_pages_for_layers(data, 0, table).unwrap()
    }

    #[test]
    fn scan_pages_synthetic() {
        let mut data = page_record("Baseline", 0xc00, &[]);
        data.extend(page_record("WithHidden", 0xc20, &[5, 6]));
        data.extend(page_record("TooFull", 0xfff, &[5]));
        let mut slots = Slots::new();
        slots.insert(5, Entry::new(false, Rc::from("CLayer"), 0));
        slots.insert(6, Entry::new(false, Rc::from("CLayer"), 0));
        let pages = scan(&data, &mut slots);
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].name, "Baseline");
        assert!(pages[0].hidden_layer_ids.is_empty());
        assert_eq!(pages[1].name, "WithHidden");
        assert_eq!(pages[1].hidden_layer_ids, vec![5, 6]);
    }

    #[test]
    fn scan_pages_ignores_unrelated_noise() {
        let noise = [255, 254, 255, 5, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        assert!(scan(&noise, &mut Slots::new()).is_empty());
    }

    // ── real files ────────────────────────────────────────────────────
    fn fixture(name: &str) -> Option<Model> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/openskp")
            .join(name);
        let data = std::fs::read(path).ok()?;
        Some(crate::parse(&data).unwrap())
    }

    fn def_names(m: &Model) -> BTreeSet<&str> {
        m.definitions.values().map(|d| d.name.as_str()).collect()
    }

    fn def_name<'m>(m: &'m Model, i: &crate::model::Instance) -> &'m str {
        &m.definitions[&i.ref_idx.unwrap()].name
    }

    fn layer_names(m: &Model) -> BTreeSet<&str> {
        m.layers.iter().map(|l| l.name.as_str()).collect()
    }

    #[test]
    fn zero_and_single_material_files() {
        // No CMaterial record anywhere: exercises the CLayer fallback anchor.
        let Some(m) = fixture("blank_v17.skp") else {
            return;
        };
        assert_eq!(m.version, "{17.0.1}");
        assert!(m.materials.is_empty());
        assert_eq!(layer_names(&m), BTreeSet::from(["Layer0"]));
        assert!(m.definitions.is_empty());
        assert!(m.root.instances.is_empty());

        let Some(m) = fixture("single_material_v17.skp") else {
            return;
        };
        assert_eq!(m.materials.len(), 1);
        assert_eq!(m.materials[0].name, "RedMat");
        assert_eq!(layer_names(&m), BTreeSet::from(["Layer0"]));
    }

    #[test]
    fn v18_zero_material_custom_tag_ahead_of_layer0() {
        let Some(m) = fixture("zero_material_custom_layer_v18.skp") else {
            return;
        };
        assert_eq!(m.version, "{18.0.16975}");
        assert!(m.materials.is_empty());
        let names = layer_names(&m);
        assert!(names.contains("Layer0") && names.contains("Guide"));
        assert!(m.root.faces.is_empty());
        assert_eq!(m.root.construction_lines.len(), 2);
    }

    #[test]
    fn v20_file() {
        let Some(m) = fixture("gondola_v20.skp") else {
            return;
        };
        assert_eq!(m.version, "{20.1.235}");
        assert_eq!(m.definitions.len(), 20);
        assert_eq!(m.materials.len(), 24);
        assert_eq!(
            layer_names(&m),
            BTreeSet::from(["Layer0", "Gondulas Laterais"])
        );
        let count = |f: fn(&crate::model::Definition) -> usize| -> usize {
            m.definitions.values().map(f).sum()
        };
        assert_eq!(count(|d| d.faces.len()), 1887);
        assert_eq!(count(|d| d.edges.len()), 9174);
        assert_eq!(count(|d| d.vertices.len()), 6543);
        assert_eq!(m.root.instances.len(), 23);
    }

    #[test]
    fn v6_attribute_trailer() {
        let Some(m) = fixture("legacy_v6_synthetic.skp") else {
            return;
        };
        assert_eq!(m.version, "{6.0.1}");
        let names: Vec<&str> = m.root.instances.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "", "P-1", "P-2", "P-3", "P-4", "P-5", "P-6", "P-7", "P-8", "P-9"
            ]
        );
        assert_eq!(
            def_names(&m),
            BTreeSet::from(["InnerFrame", "Frame", "Panel", "Post"])
        );
        let panel = m.definitions.values().find(|d| d.name == "Panel").unwrap();
        assert_eq!(panel.instances.len(), 1);
        let inner = &panel.instances[0];
        assert_eq!(inner.name, "InnerFrame");
        let dict = &inner.attribute_dictionaries["fbd-info"];
        assert_eq!(dict["role"], AttrValue::String("bracing".into()));
        assert_eq!(dict["count"], AttrValue::Integer(3));
    }

    #[test]
    fn v3_format() {
        let Some(m) = fixture("legacy_v3_synthetic.skp") else {
            return;
        };
        assert_eq!(m.version, "{3.0.1}");
        assert_eq!(
            def_names(&m),
            BTreeSet::from(["InnerFrame", "Frame", "Panel", "Post"])
        );
        for d in m.definitions.values() {
            assert_eq!(
                (d.faces.len(), d.edges.len(), d.vertices.len()),
                (1, 4, 4),
                "{}",
                d.name
            );
            assert!(d.faces.values().all(|f| !f.hidden));
        }
        let by = |n: &str| m.definitions.values().find(|d| d.name == n).unwrap();
        let xs: BTreeSet<u64> = by("Frame")
            .vertices
            .values()
            .map(|v| v.x.to_bits())
            .collect();
        assert_eq!(xs, BTreeSet::from([0f64.to_bits(), 20f64.to_bits()]));

        let roots: Vec<&str> = m.root.instances.iter().map(|i| def_name(&m, i)).collect();
        assert_eq!(
            roots,
            [
                "Frame", "Panel", "Post", "Frame", "Panel", "Post", "Frame", "Panel", "Post",
                "Frame"
            ]
        );
        let panel = by("Panel");
        assert_eq!(panel.instances.len(), 1);
        assert_eq!(def_name(&m, &panel.instances[0]), "InnerFrame");

        // SketchUp 3's face opens with its FRONT MATERIAL pointer.
        let painted = by("Frame").faces.values().next().unwrap();
        let red = m.material_by_id(painted.material_id.unwrap()).unwrap();
        assert_eq!(
            (red.name.as_str(), &red.color[..3]),
            ("Red", &[255, 0, 0][..])
        );
        for n in ["Panel", "Post", "InnerFrame"] {
            assert!(
                by(n).faces.values().next().unwrap().material_id.is_none(),
                "{n}"
            );
        }
        let names = layer_names(&m);
        assert!(names.contains("Layer0") && names.contains("Roof"));

        // Embedded template textures are inline JPEGs.
        assert_eq!(m.materials.len(), 35);
        let textured: Vec<_> = m
            .materials
            .iter()
            .filter_map(|x| x.texture.as_ref())
            .collect();
        assert_eq!(textured.len(), 10);
        for t in textured {
            let d = t.data.as_ref().unwrap();
            assert!(d.starts_with(&[0xff, 0xd8, 0xff]) && d.ends_with(&[0xff, 0xd9]));
        }

        // Same structure and placements as the SketchUp 6 save of the same model.
        let Some(v6) = fixture("legacy_v6_synthetic.skp") else {
            return;
        };
        assert_eq!(m.root.instances.len(), v6.root.instances.len());
        for (a, b) in m.root.instances.iter().zip(&v6.root.instances) {
            assert_eq!(def_name(&m, a), def_name(&v6, b));
            assert_eq!(a.matrix.len(), b.matrix.len());
            for (x, y) in a.matrix.iter().zip(&b.matrix) {
                assert!((x - y).abs() < 1e-9);
            }
        }
    }
}
