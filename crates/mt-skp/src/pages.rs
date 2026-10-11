//! VFF scenes ("pages"), view cameras and linear dimensions.
//! Port of OpenSKP `pages_dimensions.cpp` and `transforms.cpp` (the parts the reader needs).
//!
//! Sub-records here use integer tags (little-endian u16), as in OpenSKP's Python source
//! these were derived from: tag `0x34bd` is bytes `BD 34`.

use std::collections::BTreeMap;

use crate::bytes::{f64_at, hex_upper, u32_at, varint, vec3_at};
use crate::model::{EntityId, Vec3};
use crate::raw::{RawDimension, RawPage, TlvNode};
use crate::tlv;

/// Flat records with integer tags. Returns an empty list if any record is malformed
/// or has tag 0 (OpenSKP `_tlv_items`).
fn items(buf: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    let mut off = 0;
    while off < buf.len() {
        if off + 6 > buf.len() {
            return Vec::new();
        }
        let tag = u16::from_le_bytes([buf[off], buf[off + 1]]);
        let Ok(len) = u32_at(buf, off + 2) else {
            return Vec::new();
        };
        let len = len as usize;
        if tag == 0 || off + 6 + len > buf.len() {
            return Vec::new();
        }
        out.push((tag, &buf[off + 6..off + 6 + len]));
        off += 6 + len;
    }
    out
}

fn find_item<'a>(items: &[(u16, &'a [u8])], tag: u16) -> Option<&'a [u8]> {
    items.iter().find(|(t, _)| *t == tag).map(|(_, p)| *p)
}

fn vec3_of(p: Option<&[u8]>) -> Option<Vec3> {
    p.filter(|p| p.len() == 24).and_then(|p| vec3_at(p, 0).ok())
}

/// Strips a `DE05` record wrapper from an id payload.
fn strip_de05(p: &[u8]) -> &[u8] {
    if p.len() >= 6
        && p[0] == 0xde
        && p[1] == 0x05
        && let Ok(n) = u32_at(p, 2)
        && (n as usize) <= p.len() - 6
    {
        return &p[6..6 + n as usize];
    }
    p
}

/// Applies a 13-double SketchUp transform (row-major 3x3, translation at 9..12) to a point.
pub fn transform_point(m: &[f64], p: Vec3) -> Vec3 {
    if m.len() < 12 {
        return p;
    }
    [
        m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[9],
        m[3] * p[0] + m[4] * p[1] + m[5] * p[2] + m[10],
        m[6] * p[0] + m[7] * p[1] + m[8] * p[2] + m[11],
    ]
}

/// Composes parent `a` with child `b` (13-double transforms; empty = identity).
pub fn multiply_matrices(a: &[f64], b: &[f64]) -> Vec<f64> {
    if a.is_empty() {
        return b.to_vec();
    }
    if b.is_empty() {
        return a.to_vec();
    }
    let pad = |m: &[f64]| {
        let mut v = [0.0; 13];
        for (i, x) in m.iter().take(13).enumerate() {
            v[i] = *x;
        }
        if m.len() < 13 {
            v[12] = 1.0;
        }
        v
    };
    let (p, c) = (pad(a), pad(b));
    let mut o = vec![0.0; 13];
    for r in 0..3 {
        for q in 0..3 {
            o[r * 3 + q] = p[r * 3] * c[q] + p[r * 3 + 1] * c[3 + q] + p[r * 3 + 2] * c[6 + q];
        }
        o[9 + r] = p[r * 3] * c[9] + p[r * 3 + 1] * c[10] + p[r * 3 + 2] * c[11] + p[9 + r];
    }
    o[12] = p[12] * c[12];
    o
}

/// Vertex persistent id (hex) to position, for every `C409` in the subtree.
pub fn scan_vertex_positions(el: &TlvNode, out: &mut BTreeMap<String, Vec3>) {
    if el.tag == crate::bytes::Tag::new("C409")
        && let (Some(dc05), Some(c509)) = (
            tlv::find(&el.children, "DC05"),
            tlv::find(&el.children, "C509"),
        )
        && c509.payload.len() == 24
        && let Ok(p) = vec3_at(&c509.payload, 0)
    {
        out.insert(hex_upper(strip_de05(&dc05.payload)), p);
    }
    for c in &el.children {
        scan_vertex_positions(c, out);
    }
}

/// Instance persistent id (hex) to world transform, composing parents along the tree.
pub fn scan_instance_transforms(top: &TlvNode, out: &mut BTreeMap<String, Vec<f64>>) {
    fn visit(el: &TlvNode, parent: &[f64], out: &mut BTreeMap<String, Vec<f64>>) {
        if el.tag == crate::bytes::Tag::new("6419") {
            let iid = tlv::find(&el.children, "D007")
                .and_then(|d| tlv::find(&d.children, "DC05"))
                .map(|dc| hex_upper(strip_de05(&dc.payload)));
            let mat: Option<Vec<f64>> = tlv::find(&el.children, "6619")
                .filter(|m| m.payload.len() == 104)
                .map(|m| {
                    (0..13)
                        .filter_map(|i| f64_at(&m.payload, i * 8).ok())
                        .collect()
                });
            let here = match &mat {
                Some(m) => multiply_matrices(parent, m),
                None => parent.to_vec(),
            };
            if let Some(iid) = iid {
                out.insert(iid, here.clone());
            }
            for c in &el.children {
                visit(c, &here, out);
            }
        } else {
            for c in &el.children {
                visit(c, parent, out);
            }
        }
    }
    visit(top, &[], out);
}

/// Linear dimensions: `5BCC` records (bytes `CC 5B`) found by scanning `model.dat`.
///
/// `5BCD` / `5BCE` are the two connection points, each a `5208` whose `5209` type is
/// 1 (free point in `520A`, world space) or 2 (connected to geometry: `520B` > `53FC`
/// names the vertex in `53FD` and, length-prefixed in `53FE`, the instance holding it).
/// `5BCF` plane x-axis, `5BD0` normal, `5BD2` offset. A point that cannot be resolved
/// drops the dimension.
pub fn parse_dimensions(
    model: &[u8],
    id2pos: &BTreeMap<String, Vec3>,
    inst_world: &BTreeMap<String, Vec<f64>>,
) -> Vec<RawDimension> {
    let point = |block: Option<&[u8]>| -> Option<Vec3> {
        let blk_items = items(block?);
        let sub = items(find_item(&blk_items, 0x5208)?);
        let typ = find_item(&sub, 0x5209)
            .filter(|t| t.len() == 4)
            .and_then(|t| u32_at(t, 0).ok());
        if typ == Some(1) {
            return vec3_of(find_item(&sub, 0x520a));
        }
        let fi = find_item(&sub, 0x520b)
            .map(items)
            .and_then(|r| find_item(&r, 0x53fc).map(items))
            .unwrap_or_default();
        let vid = find_item(&fi, 0x53fd)?;
        let local = *id2pos.get(&hex_upper(vid))?;
        if let Some(iid) = find_item(&fi, 0x53fe)
            && let Some(&n) = iid.first()
            && n > 0
            && (n as usize) < iid.len()
            && let Some(w) = inst_world.get(&hex_upper(&iid[1..1 + n as usize]))
            && !w.is_empty()
        {
            return Some(transform_point(w, local));
        }
        Some(local)
    };

    let mut dims = Vec::new();
    let mut i = 0;
    while let Some(found) = model
        .get(i..)
        .and_then(|rest| rest.windows(2).position(|w| w == [0xcc, 0x5b]))
    {
        let j = i + found;
        i = j + 1;
        let Ok(len) = u32_at(model, j + 2) else {
            continue;
        };
        let len = len as usize;
        if len < 40 || j + 6 + len > model.len() {
            continue;
        }
        let body = items(&model[j + 6..j + 6 + len]);
        if find_item(&body, 0x5bcd).is_none() || find_item(&body, 0x5bce).is_none() {
            continue;
        }
        let (Some(a), Some(b)) = (
            point(find_item(&body, 0x5bcd)),
            point(find_item(&body, 0x5bce)),
        ) else {
            continue;
        };
        dims.push(RawDimension {
            a,
            b,
            offset: find_item(&body, 0x5bd2)
                .filter(|o| o.len() == 8)
                .and_then(|o| f64_at(o, 0).ok())
                .unwrap_or(0.0),
            plane_x: vec3_of(find_item(&body, 0x5bcf)),
            normal: vec3_of(find_item(&body, 0x5bd0)),
            text: String::new(),
            ..Default::default()
        });
    }
    dims
}

/// The `0702` scenes node in a top-level record's subtree.
pub fn find_page_node(top: &TlvNode) -> Option<&TlvNode> {
    if top.tag == crate::bytes::Tag::new("0702") {
        Some(top)
    } else {
        tlv::find(&top.children, "0702")
    }
}

#[derive(Debug, Clone)]
pub struct RawCamera {
    pub eye: Option<Vec3>,
    pub target: Option<Vec3>,
    pub up: Option<Vec3>,
    pub fov: f64,
    pub parallel: bool,
    pub ortho_height: f64,
}

/// View camera record `34BC`: `34BD` eye, `34BE` target, `34BF` up, `34C4` fov (degrees),
/// `34C2` perspective flag (0 = parallel), `34C3` visible height when parallel.
pub fn parse_camera(record: &[u8]) -> RawCamera {
    let cam = items(record);
    let f64_item = |t| {
        find_item(&cam, t)
            .filter(|p| p.len() == 8)
            .and_then(|p| f64_at(p, 0).ok())
    };
    RawCamera {
        eye: vec3_of(find_item(&cam, 0x34bd)),
        target: vec3_of(find_item(&cam, 0x34be)),
        up: vec3_of(find_item(&cam, 0x34bf)),
        fov: f64_item(0x34c4).unwrap_or(35.0),
        parallel: find_item(&cam, 0x34c2).is_some_and(|f| f.first() == Some(&0)),
        ortho_height: f64_item(0x34c3).unwrap_or(0.0),
    }
}

/// Scenes: `0702` payload > `6D60` > `6D61` > one `7148` per page with `6F54` > `6F55`
/// name, `714A` > `34BC` camera, `7150` hidden layer ids as (u8 length, var-int) runs.
/// `6D62` next to `6D61` holds the selected page's entity id.
pub fn parse_pages(node: Option<&TlvNode>) -> Vec<RawPage> {
    let mut pages = Vec::new();
    let Some(node) = node else { return pages };
    for (t60, p60) in items(&node.payload) {
        if t60 != 0x6d60 {
            continue;
        }
        let t61_items = items(p60);
        let selected_id = find_item(&t61_items, 0x6d62);
        for &(t61, p61) in &t61_items {
            if t61 != 0x6d61 {
                continue;
            }
            for (t48, p48) in items(p61) {
                if t48 != 0x7148 {
                    continue;
                }
                let it = items(p48);
                if it.is_empty() {
                    continue;
                }
                let mut page = RawPage::default();
                let head = find_item(&it, 0x6f54).map(items).unwrap_or_default();
                if let Some(name) = find_item(&head, 0x6f55)
                    && !name.is_empty()
                {
                    page.name = String::from_utf8_lossy(name).into_owned();
                }
                if let Some(selected) = selected_id {
                    let id =
                        find_item(&head, 0x05dc).and_then(|ids| find_item(&items(ids), 0x05de));
                    page.selected = id == Some(selected);
                }
                let cam_wrap = find_item(&it, 0x714a).map(items).unwrap_or_default();
                if let Some(cam) = find_item(&cam_wrap, 0x34bc) {
                    let c = parse_camera(cam);
                    page.eye = c.eye;
                    page.target = c.target;
                    page.up = c.up;
                    page.fov = c.fov;
                    page.parallel = c.parallel;
                    page.ortho_height = c.ortho_height;
                }
                if let Some(hidden) = find_item(&it, 0x7150) {
                    let mut off = 0;
                    while off < hidden.len() {
                        let n = hidden[off] as usize;
                        if n == 0 || off + 1 + n > hidden.len() {
                            break;
                        }
                        if let Ok(id) = varint(hidden, off + 1, n) {
                            page.hidden_layer_ids.push(id as EntityId);
                        }
                        off += 1 + n;
                    }
                }
                if page.eye.is_some() && page.target.is_some() {
                    pages.push(page);
                }
            }
        }
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_compose_translation() {
        let mut t = vec![
            1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 10.0, 0.0, 0.0, 1.0,
        ];
        let m = multiply_matrices(&t, &t);
        assert_eq!(transform_point(&m, [0.0; 3]), [20.0, 0.0, 0.0]);
        t[9] = 0.0;
        assert_eq!(multiply_matrices(&[], &t), t);
    }

    #[test]
    fn camera_record() {
        let mut rec = Vec::new();
        let mut put = |tag: u16, p: &[u8]| {
            rec.extend(tag.to_le_bytes());
            rec.extend((p.len() as u32).to_le_bytes());
            rec.extend(p);
        };
        let v: Vec<u8> = [1.0f64, 2.0, 3.0]
            .iter()
            .flat_map(|x| x.to_le_bytes())
            .collect();
        put(0x34bd, &v);
        put(0x34c2, &[0]);
        put(0x34c4, &50.0f64.to_le_bytes());
        let c = parse_camera(&rec);
        assert_eq!(c.eye, Some([1.0, 2.0, 3.0]));
        assert!(c.parallel);
        assert_eq!(c.fov, 50.0);
    }
}
