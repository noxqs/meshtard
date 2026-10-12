//! TLV framing of VFF `model.dat`: 2-byte tag, u32 length, payload. Port of OpenSKP `tlv.cpp`.

use crate::Error;
use crate::bytes::{Tag, tag_at, u32_at};
use crate::raw::TlvNode;

/// Tags whose payload is itself a TLV list. Everything else is a raw leaf.
const CONTAINERS: &[&str] = &[
    "F401", "F701", "D430", "D530", "C832", "7C15", "8813", "8913", "8A13", "8B13", "8C13", "8D13",
    "4C1D", "6419", "F901", "7017", "7117", "D007", "C409", "9411", "9511", "0F01", "384A", "B80B",
    "9713", "2C4C", "AC0D", "AE0D", "F601", "F801", "983A", "993A", "8C3C", "8D3C", "9013", "401F",
    "9113", "9213", "6942", "6C42", "6842",
];

/// [`CONTAINERS`] as tags.
const CONTAINER_TAGS: [Tag; CONTAINERS.len()] = {
    let mut out = [Tag([0, 0]); CONTAINERS.len()];
    let mut i = 0;
    while i < out.len() {
        out[i] = Tag::new(CONTAINERS[i]);
        i += 1;
    }
    out
};

fn is_container(t: Tag) -> bool {
    CONTAINER_TAGS.contains(&t)
}

/// Parses `d[start..end]` into a tree, descending into container tags.
/// Stops quietly at the first record whose length runs past `end`.
pub fn parse_recursive(d: &[u8], start: usize, end: usize) -> Result<Vec<TlvNode>, Error> {
    if end > d.len() || start > end {
        return Err(Error::Malformed("invalid TLV range".into()));
    }
    let mut out = Vec::new();
    let mut p = start;
    while end - p >= 6 {
        let tag = tag_at(d, p)?;
        let size = u32_at(d, p + 2)? as usize;
        if size > end - p - 6 {
            break;
        }
        let mut node = TlvNode {
            tag,
            children: Vec::new(),
            payload: Vec::new(),
        };
        if size > 0 && is_container(tag) {
            node.children = parse_recursive(d, p + 6, p + 6 + size)?;
        }
        if node.children.is_empty() && size > 0 {
            node.payload = d[p + 6..p + 6 + size].to_vec();
        }
        out.push(node);
        p += 6 + size;
    }
    Ok(out)
}

/// One level of records as (tag, payload) slices. Stops at the first overrun.
pub fn flat(d: &[u8]) -> Vec<(Tag, &[u8])> {
    let mut out = Vec::new();
    let mut p = 0;
    while d.len() - p >= 6 {
        let (Ok(tag), Ok(n)) = (tag_at(d, p), u32_at(d, p + 2)) else {
            break;
        };
        let n = n as usize;
        if n > d.len() - p - 6 {
            break;
        }
        out.push((tag, &d[p + 6..p + 6 + n]));
        p += 6 + n;
    }
    out
}

/// First payload with `tag` in a flat list.
#[inline(always)]
pub fn flat_find<'a>(items: &[(Tag, &'a [u8])], tag: &str) -> Option<&'a [u8]> {
    let t = Tag::new(tag);
    items.iter().find(|(k, _)| *k == t).map(|(_, v)| *v)
}

/// Record headers (offset, payload size) of one level of `d[start..end]`.
pub fn headers(d: &[u8], start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut h = Vec::new();
    let mut p = start;
    while p + 6 <= end {
        let Ok(n) = u32_at(d, p + 2) else { break };
        let n = n as usize;
        if n > end - p - 6 {
            break;
        }
        h.push((p, n));
        p += 6 + n;
    }
    h
}

/// Depth-first search for the first node with `tag`.
#[inline(always)]
pub fn find<'a>(nodes: &'a [TlvNode], tag: &str) -> Option<&'a TlvNode> {
    find_tag(nodes, Tag::new(tag))
}

fn find_tag(nodes: &[TlvNode], t: Tag) -> Option<&TlvNode> {
    nodes.iter().find_map(|n| {
        if n.tag == t {
            Some(n)
        } else {
            find_tag(&n.children, t)
        }
    })
}

/// Every node with `tag`, depth-first.
#[inline(always)]
pub fn find_all<'a>(nodes: &'a [TlvNode], tag: &str, out: &mut Vec<&'a TlvNode>) {
    find_all_tag(nodes, Tag::new(tag), out);
}

fn find_all_tag<'a>(nodes: &'a [TlvNode], t: Tag, out: &mut Vec<&'a TlvNode>) {
    for n in nodes {
        if n.tag == t {
            out.push(n);
        }
        find_all_tag(&n.children, t, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(tag: [u8; 2], payload: &[u8]) -> Vec<u8> {
        let mut v = tag.to_vec();
        v.extend((payload.len() as u32).to_le_bytes());
        v.extend(payload);
        v
    }

    #[test]
    fn containers_nest_and_leaves_keep_payload() {
        let leaf = rec([0xC5, 0x09], &[1, 2, 3]);
        let vertex = rec([0xC4, 0x09], &leaf);
        let tree = parse_recursive(&vertex, 0, vertex.len()).unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].tag, Tag::new("C409"));
        assert_eq!(tree[0].children[0].payload, [1, 2, 3]);
        assert!(find(&tree, "C509").is_some());
    }

    #[test]
    fn overrun_stops_without_error() {
        let mut d = rec([0xAA, 0xBB], &[1]);
        d.extend([0xCC, 0xDD, 0xFF, 0xFF, 0, 0]);
        assert_eq!(parse_recursive(&d, 0, d.len()).unwrap().len(), 1);
        assert_eq!(flat(&d).len(), 1);
    }
}
