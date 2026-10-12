//! Extracts entities from the VFF TLV tree. Port of OpenSKP `geometry.cpp`.

use std::collections::BTreeMap;

use crate::Error;
use crate::bytes::{Tag, f64_at, hex_upper, i32_at, text, u32_at, varint, varint_all, vec3_at};
use crate::model::{
    AttrDict, AttrDicts, AttrValue, ConstructionLine, ConstructionPoint, EntityId,
    stringify_attr_dict,
};
use crate::raw::{CoEdge, GeometryBuilder, RawDefinition, RawFace, RawInstance, TlvNode};
use crate::tlv::{find, find_all, flat, flat_find};

#[inline(always)]
fn is(n: &TlvNode, tag: &str) -> bool {
    n.tag == Tag::new(tag)
}

/// Entity id: a `DE05` var-int, or a `DC05` whose payload starts with a `DE05` record.
/// Searches direct children first, then descends.
fn entity_id(n: &TlvNode) -> Option<EntityId> {
    for c in &n.children {
        if is(c, "DE05") && !c.payload.is_empty() {
            return varint_all(&c.payload).ok().map(|v| v as EntityId);
        }
        if is(c, "DC05") && c.payload.len() >= 6 && c.payload[0] == 0xde && c.payload[1] == 5 {
            let z = u32_at(&c.payload, 2).ok()? as usize;
            if z <= c.payload.len() - 6 {
                return varint(&c.payload, 6, z).ok().map(|v| v as EntityId);
            }
        }
    }
    n.children.iter().find_map(entity_id)
}

/// Id from a `DC05` payload (wrapped `DE05` record or bare var-int).
pub(crate) fn dc_id(p: &[u8]) -> Result<EntityId, Error> {
    if p.len() >= 6 && p[0] == 0xde && p[1] == 5 {
        let z = u32_at(p, 2)? as usize;
        if z <= p.len() - 6 {
            return Ok(varint(p, 6, z)? as EntityId);
        }
    }
    Ok(varint_all(p)? as EntityId)
}

fn opt_varint(p: &[u8]) -> Option<EntityId> {
    (!p.is_empty())
        .then(|| varint_all(p).ok().map(|v| v as EntityId))
        .flatten()
}

type Uv = Option<[f64; 9]>;

/// Front and back UV transforms: DD05 > B136 > B236 > 1027 > (1127 front | 1227 back)
/// > 1327 > 1527 (9 f64).
fn uv(p: &[u8]) -> (Uv, Uv) {
    let sides = (|| {
        let q = flat_find(&flat(p), "DD05")?;
        let q = flat_find(&flat(q), "B136")?;
        let q = flat_find(&flat(q), "B236")?;
        flat_find(&flat(q), "1027")
    })();
    let Some(sides) = sides else {
        return (None, None);
    };
    let sides = flat(sides);
    let side = |t: &str| -> Uv {
        let x = flat_find(&sides, t)?;
        let x = flat_find(&flat(x), "1327")?;
        let x = flat_find(&flat(x), "1527")?;
        if x.len() != 72 {
            return None;
        }
        let mut m = [0.0; 9];
        for (i, v) in m.iter_mut().enumerate() {
            *v = f64_at(x, i * 8).ok()?;
        }
        Some(m)
    };
    (side("1127"), side("1227"))
}

/// One level of (tag, start, end) spans within `p[a..z]`.
fn flat_spans(p: &[u8], mut a: usize, z: usize) -> Vec<(Tag, usize, usize)> {
    let mut out = Vec::new();
    while a + 6 <= z {
        let Ok(n) = u32_at(p, a + 2) else { break };
        let n = n as usize;
        if n > z - a - 6 {
            break;
        }
        out.push((Tag([p[a], p[a + 1]]), a + 6, a + 6 + n));
        a += 6 + n;
    }
    out
}

/// Tags that wrap more attribute structure (OpenSKP `_PROP_CONTAINER_TAGS`).
fn is_prop_container(t: Tag) -> bool {
    const TAGS: [Tag; 8] = [
        Tag::new("DD05"),
        Tag::new("B536"),
        Tag::new("B136"),
        Tag::new("B236"),
        Tag::new("B336"),
        Tag::new("B036"),
        Tag::new("A438"),
        Tag::new("AE38"),
    ];
    TAGS.contains(&t)
}

/// An `A438` value wrapper: its single child's tag gives the type.
fn decode_a438(p: &[u8], a: usize, z: usize) -> AttrValue {
    let spans = flat_spans(p, a, z);
    let Some(&(tag, sa, sz)) = spans.first() else {
        return AttrValue::Null;
    };
    let len = sz - sa;
    let t = |s: &str| tag == Tag::new(s);
    if t("AD38") {
        AttrValue::String(text(&p[sa..sz]))
    } else if t("AA38") && len == 1 {
        AttrValue::Boolean(p[sa] != 0)
    } else if t("AF38") && len == 8 {
        f64_at(p, sa).map_or(AttrValue::Null, AttrValue::Length)
    } else if t("A938") && len == 8 {
        f64_at(p, sa).map_or(AttrValue::Null, AttrValue::Float)
    } else if t("A738") && len == 4 {
        i32_at(p, sa).map_or(AttrValue::Null, |n| AttrValue::Integer(n.into()))
    } else if (t("B438") || t("B538")) && len == 24 {
        vec3_at(p, sa).map_or(AttrValue::Null, AttrValue::Vec)
    } else if t("AE38") {
        AttrValue::Array(
            flat_spans(p, sa, sz)
                .into_iter()
                .filter(|(ct, ..)| *ct == Tag::new("A438"))
                .map(|(_, ca, cz)| decode_a438(p, ca, cz))
                .collect(),
        )
    } else {
        AttrValue::Null
    }
}

/// Every named attribute dictionary in a `DC05` payload. A dictionary is a `B436` name
/// followed by a sibling container of `B636` key / `A438` value pairs; both can sit at
/// any depth reachable through container tags.
fn extract_attribute_dictionaries(p: &[u8], out: &mut AttrDicts) {
    fn entries(p: &[u8], a: usize, z: usize, out: &mut AttrDict) {
        let mut key = String::new();
        for (tag, sa, sz) in flat_spans(p, a, z) {
            if tag == Tag::new("B636") {
                key = text(&p[sa..sz]);
            } else if tag == Tag::new("A438") && !key.is_empty() {
                out.insert(std::mem::take(&mut key), decode_a438(p, sa, sz));
            } else if is_prop_container(tag) {
                entries(p, sa, sz, out);
            }
        }
    }
    fn dicts(p: &[u8], a: usize, z: usize, out: &mut AttrDicts) {
        let here = flat_spans(p, a, z);
        for (i, &(tag, sa, sz)) in here.iter().enumerate() {
            if tag == Tag::new("B436") {
                let mut e = AttrDict::new();
                if let Some(&(_, ea, ez)) = here.get(i + 1) {
                    entries(p, ea, ez, &mut e);
                }
                out.insert(text(&p[sa..sz]), e);
            } else if is_prop_container(tag) {
                dicts(p, sa, sz, out);
            }
        }
    }
    dicts(p, 0, p.len(), out);
}

fn vertex(e: &TlvNode, b: &mut GeometryBuilder) {
    if let (Some(id), Some(p)) = (entity_id(e), find(&e.children, "C509"))
        && let Ok(v) = vec3_at(&p.payload, 0)
    {
        b.vertices.insert(id, v);
    }
}

fn edge(e: &TlvNode, b: &mut GeometryBuilder) {
    let Some(id) = entity_id(e) else { return };
    let end = |t| find(&e.children, t).and_then(|n| opt_varint(&n.payload));
    b.edges.insert(id, (end("B90B"), end("BA0B")));
    for z in e.children.iter().filter(|z| is(z, "D007")) {
        for w in z.children.iter().filter(|w| is(w, "D307")) {
            if let Some(&flag) = w.payload.first() {
                b.edge_flags.insert(id, i32::from(flag));
            }
        }
    }
}

fn face(e: &TlvNode, b: &mut GeometryBuilder) {
    let Some(id) = entity_id(e) else { return };
    let mut f = RawFace::default();
    if let Some(n) = find(&e.children, "AD0D")
        && let Ok(v) = vec3_at(&n.payload, 0)
    {
        f.normal = v;
    }
    if let Some(l) = find(&e.children, "AE0D") {
        let mut loops = Vec::new();
        find_all(&l.children, "9411", &mut loops);
        for ln in loops {
            let mut cs = Vec::new();
            find_all(&ln.children, "A00F", &mut cs);
            let co: Vec<CoEdge> = cs
                .iter()
                .filter_map(|cn| {
                    let mut eid = None;
                    let mut ori = None;
                    for (t, v) in flat(&cn.payload) {
                        if t == Tag::new("A10F") {
                            eid = varint_all(v).ok().map(|x| x as EntityId);
                        } else if t == Tag::new("A20F") {
                            ori = varint_all(v).ok().map(|x| if x == 0 { 1 } else { -1 });
                        }
                    }
                    Some(CoEdge {
                        edge_id: eid?,
                        orientation: ori?,
                    })
                })
                .collect();
            if !co.is_empty() {
                f.loops.push(co);
            }
        }
    }
    for d in e.children.iter().filter(|d| is(d, "D007")) {
        for x in &d.children {
            if is(x, "D107") && !x.payload.is_empty() {
                f.material_id = opt_varint(&x.payload);
            } else if is(x, "DC05") {
                (f.uv_transform, f.uv_transform_back) = uv(&x.payload);
            } else if is(x, "D307") && !x.payload.is_empty() {
                // Display flags, same record as edges: +1 hidden.
                f.hidden = x.payload[0] & 1 != 0;
            } else if is(x, "D207")
                && let Some(l) = opt_varint(&x.payload)
            {
                f.layer = l.to_string();
            }
        }
    }
    for x in e.children.iter().filter(|x| is(x, "AF0D")) {
        if !x.payload.is_empty() {
            f.back_material_id = opt_varint(&x.payload);
        }
    }
    b.faces.insert(id, f);
}

fn instance(e: &TlvNode, b: &mut GeometryBuilder) {
    let mut i = RawInstance {
        id: entity_id(e),
        ..Default::default()
    };
    if let Some(g) = find(&e.children, "6819")
        && g.payload.len() == 16
    {
        i.ref_guid = hex_upper(&g.payload);
    }
    if let Some(r) = find(&e.children, "6719") {
        i.ref_idx = opt_varint(&r.payload);
    }
    if let Some(n) = find(&e.children, "6519") {
        i.name = text(&n.payload);
    }
    if let Some(m) = find(&e.children, "6619")
        && m.payload.len() >= 104
    {
        i.matrix = (0..13)
            .filter_map(|q| f64_at(&m.payload, q * 8).ok())
            .collect();
    }
    for d in e.children.iter().filter(|d| is(d, "D007")) {
        for x in &d.children {
            if is(x, "D107") && !x.payload.is_empty() {
                i.material_id = opt_varint(&x.payload);
            } else if is(x, "D207")
                && let Some(l) = opt_varint(&x.payload)
            {
                i.layer = l.to_string();
            } else if is(x, "DC05") {
                let mut all = AttrDicts::new();
                extract_attribute_dictionaries(&x.payload, &mut all);
                for (name, entries) in all {
                    if name == "dynamic_attributes" {
                        i.properties = stringify_attr_dict(&entries);
                    }
                    i.attribute_dicts.insert(name, entries);
                }
            } else if is(x, "D307") && !x.payload.is_empty() {
                i.hidden = x.payload[0] & 1 != 0;
            }
        }
    }
    b.instances.push(i);
}

/// Construction line: point, unit direction, start/end parameters along it;
/// |parameter| >= 1e20 means unbounded.
fn construction_line(e: &TlvNode, b: &mut GeometryBuilder) {
    let Some(p) = find(&e.children, "6A42") else {
        return;
    };
    let p = &p.payload;
    let (Ok(point), Ok(direction), Ok(sp), Ok(ep)) =
        (vec3_at(p, 0), vec3_at(p, 24), f64_at(p, 48), f64_at(p, 56))
    else {
        return;
    };
    const HUGE: f64 = 1e20;
    let at = |t: f64| {
        (t.abs() < HUGE).then(|| {
            [
                point[0] + direction[0] * t,
                point[1] + direction[1] * t,
                point[2] + direction[2] * t,
            ]
        })
    };
    b.construction_lines.push(ConstructionLine {
        point,
        direction,
        start: at(sp),
        end: at(ep),
    });
}

pub fn collect_geometry(es: &[TlvNode], b: &mut GeometryBuilder) {
    for e in es {
        if is(e, "C409") {
            vertex(e, b);
        } else if is(e, "B80B") {
            edge(e, b);
        } else if is(e, "AC0D") {
            face(e, b);
        } else if is(e, "6419") {
            instance(e, b);
        } else if is(e, "6942") {
            construction_line(e, b);
        } else if is(e, "6C42") {
            if let Some(p) = find(&e.children, "6D42")
                && let Ok(position) = vec3_at(&p.payload, 0)
            {
                b.construction_points.push(ConstructionPoint { position });
            }
        } else if !e.children.is_empty() {
            collect_geometry(&e.children, b);
        }
    }
}

/// Layer manager `993A` > `8C3C` entries: `DC05` id, `8D3C` name, `8E3C` hidden byte.
pub fn collect_layers(
    ns: &[TlvNode],
    out: &mut BTreeMap<EntityId, String>,
    hidden: &mut BTreeMap<String, bool>,
) {
    for e in ns {
        if is(e, "993A") {
            for c in e.children.iter().filter(|c| is(c, "8C3C")) {
                let n = find(&c.children, "8D3C");
                if let (Some(d), Some(n)) = (find(&c.children, "DC05"), n)
                    && !d.payload.is_empty()
                    && let Ok(id) = dc_id(&d.payload)
                {
                    out.insert(id, text(&n.payload));
                }
                if let Some(n) = n
                    && let Some(h) = find(&c.children, "8E3C")
                    && let Some(&flag) = h.payload.first()
                {
                    hidden.insert(text(&n.payload), flag == 1);
                }
            }
        }
        collect_layers(&e.children, out, hidden);
    }
}

/// The model record (`F601`) holds the model's own dictionaries under 8813 > D007 > DC05.
pub fn collect_model_attribute_dictionaries(model: &TlvNode, out: &mut AttrDicts) {
    let Some(entity) = find(&model.children, "8813") else {
        return;
    };
    for d in entity.children.iter().filter(|d| is(d, "D007")) {
        for x in d.children.iter().filter(|x| is(x, "DC05")) {
            extract_attribute_dictionaries(&x.payload, out);
        }
    }
}

/// Material records `C832`: `DC05` id, `CC32` name.
pub fn collect_material_ids(ns: &[TlvNode], out: &mut BTreeMap<EntityId, String>) {
    for e in ns {
        if is(e, "C832")
            && let (Some(d), Some(n)) = (find(&e.children, "DC05"), find(&e.children, "CC32"))
            && !d.payload.is_empty()
            && let Ok(id) = dc_id(&d.payload)
        {
            out.insert(id, text(&n.payload));
        }
        collect_material_ids(&e.children, out);
    }
}

/// Component definitions `7C15`.
pub fn collect_definitions(ns: &[TlvNode], out: &mut BTreeMap<EntityId, RawDefinition>) {
    for e in ns {
        if is(e, "7C15") {
            let mut d = RawDefinition::default();
            for c in &e.children {
                if is(c, "7D15") && c.payload.len() == 16 {
                    d.guid = hex_upper(&c.payload);
                } else if is(c, "7E15") {
                    d.name = text(&c.payload);
                } else if is(c, "8315") && !c.payload.is_empty() {
                    d.is_image = varint_all(&c.payload).is_ok_and(|v| v == 2);
                } else if is(c, "581B") {
                    for (t, v) in flat(&c.payload) {
                        if t == Tag::new("5D1B") && !v.is_empty() {
                            d.always_faces_camera = varint_all(v).is_ok_and(|x| x == 1);
                        } else if t == Tag::new("5E1B") && !v.is_empty() {
                            d.shadows_face_sun = varint_all(v).is_ok_and(|x| x == 1);
                        }
                    }
                }
            }
            collect_geometry(&e.children, &mut d.builder);
            if let Some(id) = entity_id(e) {
                out.insert(id, d);
            }
        }
        collect_definitions(&e.children, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(tag: &str, payload: &[u8]) -> Vec<u8> {
        let t = Tag::new(tag);
        let mut v = t.0.to_vec();
        v.extend((payload.len() as u32).to_le_bytes());
        v.extend(payload);
        v
    }

    #[test]
    fn attribute_dictionary_types() {
        let value = |tag: &str, p: &[u8]| rec("A438", &rec(tag, p));
        let mut entries = Vec::new();
        entries.extend(rec("B636", b"s"));
        entries.extend(value("AD38", b"hi"));
        entries.extend(rec("B636", b"n"));
        entries.extend(value("A738", &(-7i32).to_le_bytes()));
        entries.extend(rec("B636", b"b"));
        entries.extend(value("AA38", &[1]));
        entries.extend(rec("B636", b"len"));
        entries.extend(value("AF38", &2.5f64.to_le_bytes()));
        let mut p = rec("B436", b"dict");
        p.extend(rec("B536", &entries));

        let mut out = AttrDicts::new();
        extract_attribute_dictionaries(&p, &mut out);
        let d = &out["dict"];
        assert_eq!(d["s"], AttrValue::String("hi".into()));
        assert_eq!(d["n"], AttrValue::Integer(-7));
        assert_eq!(d["b"], AttrValue::Boolean(true));
        assert_eq!(d["len"], AttrValue::Length(2.5));
        assert_eq!(d["n"].to_string(), "-7");
    }

    #[test]
    fn dc_id_unwraps_de05() {
        let wrapped = rec("DE05", &[0x2a]);
        assert_eq!(dc_id(&wrapped).unwrap(), 42);
        assert_eq!(dc_id(&[0x2a, 0x01]).unwrap(), 0x12a);
    }
}
