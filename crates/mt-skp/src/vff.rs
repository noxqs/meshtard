//! VFF container (SketchUp 2021+): header, ZIP, `model.dat` TLV walk.
//! Port of OpenSKP `vff.cpp` and `core.cpp`.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use crate::Error;
use crate::bytes::{Tag, tag_at, text};
use crate::geometry;
use crate::model::{Vec3, ViewCamera};
use crate::pages;
use crate::raw::{RawParsed, TlvNode};
use crate::tlv;
use crate::xml::{self, EntrySource};

pub fn valid_header(d: &[u8]) -> bool {
    d.starts_with(&[0xff, 0xfe, 0xff, 0x0e])
}

/// The `{major.minor.build}` version from the UTF-16 header text, or `unknown`.
pub fn extract_version(d: &[u8]) -> String {
    let limit = d.len().min(512);
    let Some(marker) = (4..limit.saturating_sub(2))
        .find(|&i| d[i] == 0xff && d[i + 1] == 0xfe && d[i + 2] == 0xff)
    else {
        return "unknown".into();
    };
    let mut s = String::new();
    let mut i = marker + 4;
    while i + 1 < d.len() && i < 512 {
        if d[i] != 0 {
            s.push(char::from(d[i]));
        }
        i += 2;
    }
    match s
        .find('{')
        .and_then(|a| s[a..].find('}').map(|b| &s[a..=a + b]))
    {
        Some(v) => v.to_owned(),
        None => "unknown".into(),
    }
}

/// Files up to SketchUp 2020 (major version 20) use the legacy MFC container.
pub fn is_legacy(d: &[u8]) -> bool {
    if !valid_header(d) {
        return false;
    }
    let v = extract_version(d);
    let Some(dot) = v.find('.') else {
        return false;
    };
    v.len() >= 2 && v[1..dot].parse::<i32>().is_ok_and(|major| major <= 20)
}

/// The model's unit system (`Millimeter`, ...) from `meta/meta.dat`: flat TLV, optionally
/// wrapped in a `6400` record, with the units string in `6D00`.
fn read_meta_units(meta: &[u8]) -> Option<String> {
    let records = tlv::flat(meta);
    if let Some(body) = tlv::flat_find(&records, "6400") {
        return read_meta_units(body);
    }
    tlv::flat_find(&records, "6D00").map(text)
}

/// The ZIP inside a VFF file, with OpenSKP's limits against decompression bombs.
struct Zip<'a> {
    archive: zip::ZipArchive<Cursor<&'a [u8]>>,
    names: Vec<String>,
}

impl<'a> Zip<'a> {
    const MAX_ENTRY: u64 = 16 * 1024 * 1024 * 1024;
    const MAX_RATIO: u64 = 1000;
    const RATIO_CHECK_FROM: u64 = 1024 * 1024;

    fn new(data: &'a [u8]) -> Result<Self, Error> {
        let mut archive = zip::ZipArchive::new(Cursor::new(data))
            .map_err(|e| Error::Malformed(format!("invalid ZIP container: {e}")))?;
        let names = (0..archive.len())
            .filter_map(|i| {
                let f = archive.by_index_raw(i).ok()?;
                let name = f.name().ok()?.into_owned();
                (!f.is_dir()).then_some(name)
            })
            .collect();
        Ok(Self { archive, names })
    }

    fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, Error> {
        let Ok(mut f) = self.archive.by_name(name) else {
            return Ok(None);
        };
        let declared = f.size();
        if declared > Self::MAX_ENTRY {
            return Err(Error::Malformed(format!(
                "ZIP entry '{name}' declares {declared} bytes, over the safety limit"
            )));
        }
        if declared >= Self::RATIO_CHECK_FROM {
            let compressed = f.compressed_size();
            if compressed == 0 || declared / compressed > Self::MAX_RATIO {
                return Err(Error::Malformed(format!(
                    "ZIP entry '{name}' has an implausible compression ratio"
                )));
            }
        }
        let mut out = Vec::with_capacity(declared.min(1 << 30) as usize);
        match f.read_to_end(&mut out) {
            Ok(_) => Ok(Some(out)),
            Err(_) => Ok(None),
        }
    }
}

/// Entry lookup for the XML readers. Errors (size limits) surface as a missing entry
/// there; the main `model.dat` read uses `Zip::read` and propagates them.
struct Entries<'z, 'a>(&'z mut Zip<'a>);

impl EntrySource for Entries<'_, '_> {
    fn names(&self) -> &[String] {
        &self.0.names
    }

    fn get(&mut self, name: &str) -> Option<Vec<u8>> {
        self.0.read(name).ok().flatten()
    }
}

fn zip_offset(d: &[u8]) -> Option<usize> {
    d.windows(4).take(4096).position(|w| w == b"PK\x03\x04")
}

/// The current style from the catalog (`0602` > `7869`): `7969` lists styles as `6C6B`
/// entries (`DC05` > `DE05` id, `6F6B` folder name), `7A69` is the current style's id,
/// `7B69` its working copy, `7C69` = 1 when edited without updating.
#[derive(Default)]
struct CurrentStyle {
    folder: String,
    working_copy: String,
    modified: bool,
}

fn current_style(catalog: &[u8]) -> CurrentStyle {
    let mut listed: Vec<(&[u8], String)> = Vec::new();
    let mut current_id = None;
    let mut out = CurrentStyle::default();
    for (tag, value) in tlv::flat(catalog) {
        if tag == Tag::new("7969") {
            for (t, entry) in tlv::flat(value) {
                if t != Tag::new("6C6B") {
                    continue;
                }
                let fields = tlv::flat(entry);
                let name = tlv::flat_find(&fields, "6F6B");
                let id = tlv::flat_find(&fields, "DC05")
                    .and_then(|ids| tlv::flat_find(&tlv::flat(ids), "DE05"));
                if let (Some(name), Some(id)) = (name, id) {
                    listed.push((id, text(name)));
                }
            }
        } else if tag == Tag::new("7A69") {
            current_id = Some(value);
        } else if tag == Tag::new("7C69") {
            out.modified = value.first().is_some_and(|&b| b != 0);
        } else if tag == Tag::new("7B69") {
            for (t, entry) in tlv::flat(value) {
                if t == Tag::new("6C6B")
                    && let Some(name) = tlv::flat_find(&tlv::flat(entry), "6F6B")
                {
                    out.working_copy = text(name);
                }
            }
        }
    }
    for (id, name) in listed {
        if current_id == Some(id) {
            out.folder = name;
        }
    }
    out
}

/// The model's current view (`FA01` > `34BC`), if it has eye and target.
fn view_camera(record: &[u8]) -> Option<ViewCamera> {
    let raw = pages::parse_camera(record);
    Some(ViewCamera {
        eye: raw.eye?,
        target: raw.target?,
        up: raw.up.unwrap_or([0.0, 0.0, 1.0]),
        fov: raw.fov,
        parallel: raw.parallel,
        ortho_height: raw.ortho_height,
    })
}

/// Definitions sit in `F901` > `7017` > `7117` > `7C15`; returns the `7C15` headers so
/// each definition is parsed on its own (bounded memory).
fn unwrap_definitions(data: &[u8], offset: usize, size: usize) -> Vec<(usize, usize)> {
    let mut level = tlv::headers(data, offset + 6, offset + 6 + size);
    for expected in ["7017", "7117"] {
        let Some(&(o, n)) = level
            .iter()
            .find(|(o, _)| tag_at(data, *o).is_ok_and(|t| t == Tag::new(expected)))
        else {
            return Vec::new();
        };
        level = tlv::headers(data, o + 6, o + 6 + n);
    }
    level
        .into_iter()
        .filter(|(o, _)| tag_at(data, *o).is_ok_and(|t| t == Tag::new("7C15")))
        .collect()
}

pub fn full_parse(data: &[u8]) -> Result<RawParsed, Error> {
    let mut p = RawParsed {
        version: extract_version(data),
        ..Default::default()
    };
    let off = zip_offset(data).ok_or_else(|| Error::Malformed("no ZIP container found".into()))?;
    let mut zip = Zip::new(&data[off..])?;
    let names = zip.names.clone();

    for n in &names {
        if !(n.starts_with("materials/") && n.ends_with("material.xml")) {
            continue;
        }
        let Some(bytes) = zip.read(n)? else { continue };
        let Some(m) = xml::material(&mut Entries(&mut zip), n, &text(&bytes)) else {
            continue;
        };
        let rest = &n["materials/".len()..];
        let folder = rest.find('/').map_or(rest, |i| &rest[..i]).to_owned();
        if let Some(layer) = m.name.strip_prefix("Layer_") {
            let layer = layer.to_owned();
            if !p.layer_colors.contains_key(&layer) {
                p.layer_order.push(layer.clone());
            }
            p.layer_colors
                .insert(layer.clone(), [m.r as u8, m.g as u8, m.b as u8]);
            p.layer_hidden.insert(layer, false);
        }
        let i = p.insert_material(m);
        p.materials_by_folder.insert(folder, i);
    }

    for n in &names {
        if !(n.starts_with("styles/") && n.ends_with("style.xml")) {
            continue;
        }
        let Some(bytes) = zip.read(n)? else { continue };
        let Some(mut s) = xml::style(&text(&bytes)) else {
            continue;
        };
        if n.len() > 17 {
            s.folder = n[7..n.len() - 10].to_owned();
        }
        // A style's images sit in its folder ("./2.jpg"); the current style's watermark
        // images are at the ZIP root ("watermarks/Watermark1.jpg").
        for w in &mut s.watermarks {
            let mut path = w.image_path.as_str();
            while let Some(rest) = path.strip_prefix("./") {
                path = rest;
            }
            let path = path.trim_start_matches('/');
            if path.is_empty() {
                continue;
            }
            for candidate in [format!("styles/{}/{path}", s.folder), path.to_owned()] {
                if let Some(img) = zip.read(&candidate)? {
                    w.image = Some(img);
                    break;
                }
            }
        }
        p.styles.push(s);
    }

    let model = zip
        .read("model.dat")?
        .ok_or_else(|| Error::Malformed("model.dat not found in ZIP container".into()))?;
    let mut hs = tlv::headers(&model, 0, model.len());
    if hs.len() == 1 && model.get(hs[0].0..hs[0].0 + 2) == Some(&[0xf4, 0x01]) {
        hs = tlv::headers(&model, hs[0].0 + 6, hs[0].0 + 6 + hs[0].1);
    }
    let hs: Vec<(usize, usize)> = hs
        .into_iter()
        .flat_map(|h| {
            if tag_at(&model, h.0).is_ok_and(|t| t == Tag::new("F901")) {
                let defs = unwrap_definitions(&model, h.0, h.1);
                if !defs.is_empty() {
                    return defs;
                }
            }
            vec![h]
        })
        .collect();

    let mut vertex_positions: BTreeMap<String, Vec3> = BTreeMap::new();
    let mut instance_world: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut page_node: Option<TlvNode> = None;
    let mut current = CurrentStyle::default();

    for &(offset, size) in &hs {
        let one = tlv::parse_recursive(&model, offset, offset + 6 + size)?;
        let Some(top) = one.first() else { continue };
        if top.tag == Tag::new("FA01") {
            for (t, record) in tlv::flat(&top.payload) {
                if t == Tag::new("BC34") {
                    p.camera = view_camera(record);
                }
            }
        }
        if top.tag == Tag::new("0602") {
            for (t, catalog) in tlv::flat(&top.payload) {
                if t == Tag::new("7869") {
                    current = current_style(catalog);
                }
            }
        }
        geometry::collect_layers(&one, &mut p.layer_id_to_name, &mut p.layer_hidden);
        geometry::collect_material_ids(&one, &mut p.material_id_to_name);
        geometry::collect_definitions(&one, &mut p.definitions);
        pages::scan_vertex_positions(top, &mut vertex_positions);
        pages::scan_instance_transforms(top, &mut instance_world);
        if page_node.is_none() {
            page_node = pages::find_page_node(top).cloned();
        }
        if top.tag == Tag::new("F601") {
            geometry::collect_geometry(&top.children, &mut p.root.builder);
            geometry::collect_model_attribute_dictionaries(top, &mut p.model_attribute_dicts);
        }
    }

    for s in &mut p.styles {
        s.active = !current.folder.is_empty() && s.folder == current.folder;
        s.modified = s.active && current.modified;
        s.working_copy = !current.working_copy.is_empty() && s.folder == current.working_copy;
    }
    p.units = zip
        .read("meta/meta.dat")
        .ok()
        .flatten()
        .and_then(|m| read_meta_units(&m));
    p.pages = pages::parse_pages(page_node.as_ref());
    if !vertex_positions.is_empty() {
        p.dimensions = pages::parse_dimensions(&model, &vertex_positions, &instance_world);
    }
    p.layer_id_to_name
        .entry(1)
        .or_insert_with(|| "Layer0".into());
    if !p.layer_colors.contains_key("Layer0") {
        p.layer_order.push("Layer0".into());
        p.layer_colors.insert("Layer0".into(), [136, 136, 136]);
    }
    p.layer_hidden.entry("Layer0".into()).or_insert(false);
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_and_legacy_detection() {
        let mut d = vec![0xff, 0xfe, 0xff, 0x0e, 0, 0, 0xff, 0xfe, 0xff, 0x06];
        for c in "{20.1.235}".bytes() {
            d.extend([c, 0]);
        }
        assert_eq!(extract_version(&d), "{20.1.235}");
        assert!(is_legacy(&d));
        let modern: Vec<u8> = d
            .iter()
            .map(|&b| if b == b'0' { b'5' } else { b })
            .collect();
        assert_eq!(extract_version(&modern), "{25.1.235}");
        assert!(!is_legacy(&modern));
        assert!(!is_legacy(b"PK"));
    }
}
