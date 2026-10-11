//! Model settings written into the scaffold in place: style items and named options.
//!
//! Both live in the bundled blank document as fixed-size typed values, so setting
//! them overwrites bytes without moving anything (no slot or offset changes).
//!
//! - Style items: the scaffold's two `CSkpStyle` records (the style and its working
//!   copy, see the legacy reader's `scan_styles`) each list `[u32 id][u32 1][u32 kind]
//!   [value]` items, the same ids and kinds as a VFF `style.xml`. Item meanings are
//!   SketchUp's `RenderingOptions` keys as OpenSKP's `docs/API_DESIGN.md` lists them
//!   (confirmed there against SketchUp 2026).
//! - Named options: the model's options manager in the scaffold prefix, sections
//!   (`PageOptions`, `SlideshowOptions`, `UnitsOptions`) of `[name][type][value]`
//!   entries, the names of SketchUp's Ruby `OptionsManager`.

use crate::model::OptionValue;

use super::{SCAFFOLD, WriteError, invalid};

const MARKER: [u8; 3] = [0xff, 0xfe, 0xff];

/// A short marked UTF-16 string at `p`: its text and end offset.
fn string_at(d: &[u8], p: usize) -> Option<(String, usize)> {
    if d.get(p..p + 3)? != MARKER {
        return None;
    }
    let n = usize::from(*d.get(p + 3)?);
    if n == 0xff {
        return None; // long strings do not occur here
    }
    let raw = d.get(p + 4..p + 4 + 2 * n)?;
    let units: Vec<u16> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    Some((String::from_utf16(&units).ok()?, p + 4 + 2 * n))
}

fn u32_at(d: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(p..p + 4)?.try_into().ok()?))
}

/// A style item's value: its kind (1 bool, 3/4/5 int, 6 f32, 7 f64) and offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ItemSlot {
    pub kind: u32,
    pub pos: usize,
}

/// The item list of one `CSkpStyle` body starting at its first string, or `None`.
fn style_items_at(d: &[u8], at: usize) -> Option<(Vec<(i32, ItemSlot)>, usize)> {
    let (_, mut p) = string_at(d, at)?;
    u32_at(d, p)?;
    p += 4;
    p = string_at(d, p)?.1;
    p = string_at(d, p)?.1;
    let n = u32_at(d, p)?;
    p += 4;
    if n == 0 || n > 2000 {
        return None;
    }
    let mut out = Vec::new();
    for _ in 0..n {
        let id = i32::try_from(u32_at(d, p)?).ok()?;
        let present = u32_at(d, p + 4)?;
        p += 8;
        match present {
            0 => continue,
            1 => {}
            _ => return None,
        }
        let kind = u32_at(d, p)?;
        p += 4;
        let size = match kind {
            1 => 1,
            3..=6 => 4,
            7 => 8,
            _ => return None,
        };
        out.push((id, ItemSlot { kind, pos: p }));
        p += size;
    }
    Some((out, p))
}

/// Every style record's items in the scaffold (the style, then its working copy).
pub(super) fn scaffold_style_items() -> Vec<Vec<(i32, ItemSlot)>> {
    let d = SCAFFOLD;
    let name = b"CSkpStyle";
    let Some(start) = d.windows(name.len()).position(|w| w == name) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut q = start + name.len();
    while q < d.len() {
        if d[q..].starts_with(&MARKER)
            && let Some((items, end)) = style_items_at(d, q)
            && items.len() >= 5
        {
            out.push(items);
            q = end;
        } else {
            q += 1;
        }
    }
    out
}

/// Offset and type byte of named option `section`/`key` in the scaffold.
pub(super) fn scaffold_option(section: &str, key: &str) -> Option<(u8, usize)> {
    let d = SCAFFOLD;
    let mut p = 0;
    // Find the section's name string, then walk its entries.
    loop {
        p += d[p..].windows(3).position(|w| w == MARKER)?;
        if let Some((s, end)) = string_at(d, p)
            && s == section
        {
            p = end;
            break;
        }
        p += 1;
    }
    loop {
        let (name, end) = string_at(d, p)?;
        if name.is_empty() {
            return None;
        }
        let ty = *d.get(end)?;
        let size = option_size(ty)?;
        if name == key {
            return Some((ty, end + 1));
        }
        p = end + 1 + size;
    }
}

/// Value size of a named option type byte (7 bool, 4 int, 6 double).
pub(crate) fn option_size(ty: u8) -> Option<usize> {
    match ty {
        7 => Some(1),
        4 => Some(4),
        6 => Some(8),
        _ => None,
    }
}

/// Overwrites one style item (in every style record) with `value`.
pub(super) fn patch_style_item(
    d: &mut [u8],
    records: &[Vec<(i32, ItemSlot)>],
    id: i32,
    value: OptionValue,
) -> Result<(), WriteError> {
    let mut found = false;
    for rec in records {
        let Some((_, slot)) = rec.iter().find(|(i, _)| *i == id) else {
            continue;
        };
        found = true;
        let p = slot.pos;
        match (slot.kind, value) {
            (1, OptionValue::Bool(b)) => d[p] = u8::from(b),
            (3..=5, OptionValue::Int(v)) => d[p..p + 4].copy_from_slice(&v.to_le_bytes()),
            (6, OptionValue::Double(v)) => d[p..p + 4].copy_from_slice(&(v as f32).to_le_bytes()),
            (7, OptionValue::Double(v)) => d[p..p + 8].copy_from_slice(&v.to_le_bytes()),
            (kind, v) => {
                return Err(invalid(format!(
                    "style item {id} has kind {kind}, cannot hold {v:?}"
                )));
            }
        }
    }
    if found {
        Ok(())
    } else {
        Err(invalid(format!("style item {id} is not in the scaffold")))
    }
}

/// Overwrites one named option with `value`.
pub(super) fn patch_option(
    d: &mut [u8],
    section: &str,
    key: &str,
    value: OptionValue,
) -> Result<(), WriteError> {
    let (ty, p) = scaffold_option(section, key)
        .ok_or_else(|| invalid(format!("option {section}/{key} is not in the scaffold")))?;
    match (ty, value) {
        (7, OptionValue::Bool(b)) => d[p] = u8::from(b),
        (4, OptionValue::Int(v)) => d[p..p + 4].copy_from_slice(&v.to_le_bytes()),
        (6, OptionValue::Double(v)) => d[p..p + 8].copy_from_slice(&v.to_le_bytes()),
        (ty, v) => {
            return Err(invalid(format!(
                "option {section}/{key} has type {ty}, cannot hold {v:?}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffold_has_two_styles_and_units() {
        let styles = scaffold_style_items();
        assert_eq!(styles.len(), 2);
        assert_eq!(styles[0].len(), 55);
        // Front color item 2002 at the documented face color position.
        let front = styles[0].iter().find(|(i, _)| *i == 2002).unwrap().1;
        assert_eq!(front.pos, 4317);
        assert_eq!(
            scaffold_option("UnitsOptions", "LengthUnit"),
            Some((4, 2389))
        );
        assert_eq!(scaffold_option("UnitsOptions", "SnapAngle").unwrap().0, 6);
        assert_eq!(
            scaffold_option("PageOptions", "ShowTransition").unwrap().0,
            7
        );
        assert_eq!(scaffold_option("UnitsOptions", "Missing"), None);
    }
}
