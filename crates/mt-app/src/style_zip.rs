//! SketchUp's own `.style` files (Styles > Details > Open and style collections;
//! written by [`write`] for Details > Save As > SketchUp Style), and the zip and XML
//! helpers the `.skm` material files share ([`crate::skm`]).
//!
//! What is known: a `.style` file is a zip archive (forum posts describe renaming
//! one to `.zip` and finding a few XML files, a `ref` folder and a
//! `doc_thumbnail.png`); the style itself is a `<sty:style name=".." desc="..">`
//! element whose `<sty:item id="N"><t:variant type="T">value</t:variant></sty:item>`
//! children are the same items a `.skp` file keeps in `styles/<folder>/style.xml`
//! (seen in real `.skp` files). So the reader looks in every XML file of the archive
//! for that element and maps its items with
//! [`mt_kernel::import::read_style_items`]. Unverified (no real `.style` file was
//! available): the archive's file names, which XML file holds the style, the
//! thumbnail's name (any PNG / JPEG with "thumbnail" in its name is taken) and the
//! watermark attributes besides `alphaScale` (watermarks listed before the
//! `<MODEL SPACE>` entry are taken as backgrounds, those after it as overlays; all
//! are stretched keeping their aspect). Names in square brackets (SketchUp's own
//! styles, e.g. `[Construction Documentation Style]`) lose the brackets. Face
//! colors (items 2002 / 2003) belong to the model here and are not taken.

use std::io::{Cursor, Read};
use std::sync::Arc;

use mt_kernel::model_data::NamedStyle;
use mt_kernel::style_extras::{Watermark, WatermarkMode};

/// Whether `data` starts like a zip archive.
pub(crate) fn is_zip(data: &[u8]) -> bool {
    data.starts_with(b"PK\x03\x04")
}

/// A style read from a SketchUp `.style` archive, with its thumbnail image bytes
/// (PNG or JPEG) when it has one.
pub(crate) struct ZipStyle {
    pub style: NamedStyle,
    pub thumbnail: Option<Vec<u8>>,
}

/// The largest archive entry read (bytes).
const MAX_ENTRY: u64 = 64 << 20;
/// Most bytes read from all entries.
const MAX_TOTAL: u64 = 256 << 20;
/// Most entries looked at.
const MAX_ENTRIES: usize = 10_000;

/// The files of a zip archive (directories skipped, names with `/`), read within
/// the size limits above.
pub(crate) fn entries(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut zip =
        zip::ZipArchive::new(Cursor::new(data)).map_err(|e| format!("not a zip archive: {e}"))?;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total = 0u64;
    for i in 0..zip.len().min(MAX_ENTRIES) {
        let Ok(mut f) = zip.by_index(i) else { continue };
        if f.is_dir() || f.size() > MAX_ENTRY {
            continue;
        }
        let Ok(name) = f.name().map(|n| n.replace('\\', "/")) else {
            continue;
        };
        // The declared size can lie: read no more than the limits allow.
        let mut bytes = Vec::new();
        let room = MAX_TOTAL.saturating_sub(total).min(MAX_ENTRY);
        if (&mut f).take(room + 1).read_to_end(&mut bytes).is_ok() {
            if bytes.len() as u64 > room {
                return Err("the file is too large".into());
            }
            total += bytes.len() as u64;
            files.push((name, bytes));
        }
    }
    Ok(files)
}

/// Reads a SketchUp `.style` archive.
pub(crate) fn read(data: &[u8]) -> Result<ZipStyle, String> {
    let files = entries(data).map_err(|e| format!("not a style file: {e}"))?;
    let (folder, raw) = files
        .iter()
        .filter(|(n, _)| n.to_ascii_lowercase().ends_with(".xml"))
        .find_map(|(n, b)| {
            let text = String::from_utf8_lossy(b);
            parse_style_xml(&text).map(|s| (n.rsplit_once('/').map_or("", |x| x.0), s))
        })
        .ok_or("no style found in the archive")?;
    let mut style = NamedStyle {
        name: display_name(&raw.name, "Style"),
        description: display_name(&raw.description, ""),
        settings: Default::default(),
    };
    mt_kernel::import::read_style_items(
        &|id| raw.items.get(&id).map(String::as_str),
        &mut style.settings,
    );
    // Written by us: our text style holds what the items lack (fog, Monochrome, ...).
    if let Some(ours) = files
        .iter()
        .find(|(n, _)| n == OURS)
        .and_then(|(_, b)| std::str::from_utf8(b).ok())
        .and_then(|t| crate::tray_extra::parse_style_file(t).ok())
    {
        style.settings = ours.settings;
        style.settings.watermarks.clear();
    }
    let find = |path: &str, file_name: &str| -> Option<Arc<[u8]>> {
        let mut p = path.trim();
        while let Some(rest) = p.strip_prefix("./") {
            p = rest;
        }
        let p = p.trim_start_matches('/');
        let candidates = [
            if folder.is_empty() {
                p.to_owned()
            } else {
                format!("{folder}/{p}")
            },
            p.to_owned(),
        ];
        let base = |n: &str| n.rsplit('/').next().unwrap_or(n).to_ascii_lowercase();
        let want = if file_name.is_empty() {
            base(p)
        } else {
            file_name.to_ascii_lowercase()
        };
        files
            .iter()
            .find(|(n, _)| !p.is_empty() && candidates.iter().any(|c| c == n))
            .or_else(|| {
                files
                    .iter()
                    .find(|(n, _)| !want.is_empty() && base(n) == want)
            })
            .map(|(_, b)| Arc::from(b.as_slice()))
    };
    for w in &raw.watermarks {
        let Some(image) = find(&w.path, &w.file_name) else {
            continue;
        };
        style.settings.watermarks.push(Watermark {
            name: if w.name.is_empty() {
                format!("Watermark{}", style.settings.watermarks.len() + 1)
            } else {
                w.name.clone()
            },
            image,
            overlay: w.overlay,
            opacity: w.opacity,
            mode: w
                .mode
                .unwrap_or(WatermarkMode::Stretched { lock_aspect: true }),
            mask: w.mask,
        });
    }
    let thumbnail = files
        .iter()
        .find(|(n, _)| {
            let n = n.to_ascii_lowercase();
            n.contains("thumbnail")
                && (n.ends_with(".png") || n.ends_with(".jpg") || n.ends_with(".jpeg"))
        })
        .map(|(_, b)| b.clone());
    Ok(ZipStyle { style, thumbnail })
}

/// `[Name]` (a SketchUp built-in name) without its brackets.
pub(crate) fn display_name(s: &str, empty: &str) -> String {
    let t = s.trim();
    let t = t
        .strip_prefix('[')
        .and_then(|x| x.strip_suffix(']'))
        .unwrap_or(t);
    if t.is_empty() {
        empty.to_owned()
    } else {
        t.to_owned()
    }
}

#[derive(Debug, Default)]
struct RawWatermark {
    name: String,
    path: String,
    file_name: String,
    overlay: bool,
    opacity: f32,
    /// Placement and Create Mask from our own attributes (see [`write`]).
    mode: Option<WatermarkMode>,
    mask: bool,
}

#[derive(Debug, Default)]
struct RawStyle {
    name: String,
    description: String,
    items: std::collections::HashMap<i32, String>,
    watermarks: Vec<RawWatermark>,
}

/// An XML tag at `start` (the `<`): its local name (prefix dropped), its attribute
/// text, whether it closes (`</x>`) or is empty (`<x/>`), and the index after `>`.
pub(crate) struct Tag<'a> {
    pub name: &'a str,
    pub attrs: &'a str,
    pub closing: bool,
    pub empty: bool,
    pub end: usize,
}

fn tag_at(xml: &str, start: usize) -> Option<Tag<'_>> {
    let rest = &xml[start + 1..];
    let close = rest.find('>')?;
    let inner = &rest[..close];
    let end = start + 1 + close + 1;
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(x) => (true, x),
        None => (false, inner),
    };
    let (inner, empty) = match inner.strip_suffix('/') {
        Some(x) => (x, true),
        None => (inner, false),
    };
    let split = inner
        .find(|c: char| c.is_ascii_whitespace())
        .unwrap_or(inner.len());
    let qname = &inner[..split];
    let name = qname.rsplit(':').next().unwrap_or(qname);
    Some(Tag {
        name,
        attrs: &inner[split..],
        closing,
        empty,
        end,
    })
}

/// The tags of `xml` in order (comments, declarations and processing instructions
/// skipped), with the index of each `<`.
pub(crate) fn tags(xml: &str) -> impl Iterator<Item = (usize, Tag<'_>)> {
    xml.match_indices('<').filter_map(move |(i, _)| {
        let next = xml[i + 1..].chars().next()?;
        if next == '!' || next == '?' {
            return None;
        }
        tag_at(xml, i).map(|t| (i, t))
    })
}

/// Attribute `key` of a tag's attribute text, entities decoded.
pub(crate) fn attr(attrs: &str, key: &str) -> Option<String> {
    let mut rest = attrs;
    while let Some(i) = rest.find(key) {
        let before_ok = i == 0 || rest.as_bytes()[i - 1].is_ascii_whitespace();
        let after = rest[i + key.len()..].trim_start();
        if before_ok && let Some(after) = after.strip_prefix('=') {
            let after = after.trim_start();
            let q = after.chars().next()?;
            if q == '"' || q == '\'' {
                let v = &after[1..];
                let end = v.find(q)?;
                return Some(decode(&v[..end]));
            }
        }
        rest = &rest[i + key.len()..];
    }
    None
}

/// XML entities decoded once.
pub(crate) fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(semi) = tail.find(';').filter(|&k| k <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..semi];
        let c = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The first `<style>` element of `xml` with its items, `None` without one.
fn parse_style_xml(xml: &str) -> Option<RawStyle> {
    let mut it = tags(xml);
    let mut style = loop {
        let (_, t) = it.next()?;
        if !t.closing && t.name.eq_ignore_ascii_case("style") && attr(t.attrs, "name").is_some() {
            break RawStyle {
                name: attr(t.attrs, "name").unwrap_or_default(),
                description: attr(t.attrs, "desc").unwrap_or_default(),
                ..Default::default()
            };
        }
    };
    let mut item: Option<i32> = None;
    for (_, t) in it {
        if t.closing && t.name.eq_ignore_ascii_case("style") {
            break;
        }
        if t.closing || t.empty {
            continue;
        }
        if t.name.eq_ignore_ascii_case("item") {
            item = attr(t.attrs, "id").and_then(|v| v.trim().parse().ok());
        } else if t.name.eq_ignore_ascii_case("variant")
            && let Some(id) = item.take()
        {
            // The value runs to this variant's closing tag (values hold no variants).
            let body = &xml[t.end..];
            let close = tags(body)
                .find(|(_, c)| c.closing && c.name.eq_ignore_ascii_case("variant"))
                .map_or(body.len(), |(i, _)| i);
            let value = body[..close].trim();
            if id == 5001 {
                let list = if value.to_ascii_lowercase().contains("&lt;screenimage") {
                    decode(value)
                } else {
                    value.to_owned()
                };
                style.watermarks = watermarks(&list);
            }
            style.items.insert(id, decode(value));
        }
    }
    Some(style)
}

/// Watermark entries of item 5001 (see the module docs).
fn watermarks(list: &str) -> Vec<RawWatermark> {
    let mut out: Vec<RawWatermark> = Vec::new();
    let mut overlay = false;
    let mut open: Option<RawWatermark> = None;
    for (_, t) in tags(list) {
        if t.name.eq_ignore_ascii_case("screenimage") {
            if t.closing {
                out.extend(open.take());
                continue;
            }
            out.extend(open.take());
            let name = attr(t.attrs, "name").unwrap_or_default();
            if name == "<MODEL SPACE>" {
                overlay = true;
                continue;
            }
            let w = RawWatermark {
                name,
                path: String::new(),
                file_name: String::new(),
                overlay,
                opacity: attr(t.attrs, "alphaScale")
                    .and_then(|v| v.trim().parse::<f32>().ok())
                    .map_or(1.0, |a| a.clamp(0.0, 1.0)),
                mode: attr(t.attrs, MODE_ATTR).and_then(|m| parse_mode(&m)),
                mask: attr(t.attrs, MASK_ATTR).is_some_and(|m| m.trim() == "1"),
            };
            if t.empty {
                out.push(w);
            } else {
                open = Some(w);
            }
        } else if t.name.eq_ignore_ascii_case("image")
            && !t.closing
            && let Some(w) = open.as_mut()
        {
            w.path = attr(t.attrs, "path").unwrap_or_default();
            w.file_name = attr(t.attrs, "file_name").unwrap_or_default();
        }
    }
    out.extend(open);
    out.retain(|w| !w.path.is_empty() || !w.file_name.is_empty());
    out
}

/// Our text style inside archives we write (SketchUp ignores it, presumably).
const OURS: &str = "meshtard.style";
/// Our own `<screenimage>` attributes: placement ([`mode_text`]) and Create Mask.
const MODE_ATTR: &str = "meshtardMode";
const MASK_ATTR: &str = "meshtardMask";

/// Style items holding a color (variant type 5 in real files; the rest by value:
/// 1 for flags, 4 for numbers, 3 for decimals, the last unverified).
const COLOR_ITEMS: [i32; 12] = [
    1014, 4000, 4001, 4003, 4007, 7000, 7001, 7002, 7003, 7004, 7005, 7016,
];

/// A watermark placement as text: `stretched 1|0`, `tiled <scale>` or
/// `positioned <x> <y> <scale>` (our own notation, used in both kinds of `.style`).
pub(crate) fn mode_text(m: &WatermarkMode) -> String {
    match m {
        WatermarkMode::Stretched { lock_aspect } => {
            format!("stretched {}", u8::from(*lock_aspect))
        }
        WatermarkMode::Tiled { scale } => format!("tiled {scale}"),
        WatermarkMode::Positioned { anchor, scale } => {
            format!("positioned {} {} {scale}", anchor[0], anchor[1])
        }
    }
}

/// Reads [`mode_text`]'s notation.
pub(crate) fn parse_mode(t: &str) -> Option<WatermarkMode> {
    let mut it = t.split_whitespace();
    let kind = it.next()?;
    let mut num = || -> Option<f32> { it.next()?.parse::<f32>().ok().filter(|v| v.is_finite()) };
    Some(match kind {
        "stretched" => WatermarkMode::Stretched {
            lock_aspect: num()? != 0.0,
        },
        "tiled" => WatermarkMode::Tiled {
            scale: num()?.max(1e-3),
        },
        "positioned" => {
            let (x, y, scale) = (num()?, num()?, num()?);
            WatermarkMode::Positioned {
                anchor: [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)],
                scale: scale.max(1e-3),
            }
        }
        _ => return None,
    })
}

/// `s` with XML's special characters escaped (attribute values and text).
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\n' => out.push_str("&#10;"),
            c => out.push(c),
        }
    }
    out
}

/// The extension of an image's bytes (`png`, `jpg` or `bmp`; `png` when unknown).
pub(crate) fn image_ext(data: &[u8]) -> &'static str {
    if data.starts_with(&[0xFF, 0xD8]) {
        "jpg"
    } else if data.starts_with(b"BM") {
        "bmp"
    } else {
        "png"
    }
}

/// A `documentProperties.xml` in the shape of real `.skm` files (generator `name`:
/// `Material` there, `Style` here unverified).
pub(crate) fn document_properties(title: &str, description: &str, generator: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="no" ?>
<documentProperties xmlns="http://www.sketchup.com/schemas/1.0/documentproperties" xmlns:dp="http://www.sketchup.com/schemas/1.0/documentproperties" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://www.sketchup.com/schemas/1.0/documentproperties http://www.sketchup.com/schemas/1.0/documentproperties.xsd">

  <dp:title>{}</dp:title>

  <dp:description>{}</dp:description>

  <dp:creator></dp:creator>

  <dp:keywords></dp:keywords>

  <dp:lastModifiedBy></dp:lastModifiedBy>

  <dp:revision>0</dp:revision>

  <dp:thumbnail>doc_thumbnail.png</dp:thumbnail>

  <dp:generator dp:name="{generator}" dp:version="1"/>

</documentProperties>
"#,
        escape(title),
        escape(description)
    )
}

/// The (empty) `references.xml` of real `.skm` files.
pub(crate) const REFERENCES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="no" ?>
<references xmlns="http://www.sketchup.com/schemas/1.0/references" xmlns:r="http://www.sketchup.com/schemas/1.0/references" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://www.sketchup.com/schemas/1.0/references http://www.sketchup.com/schemas/1.0/references.xsd"/>
"#;

/// A zip archive of `files` (name, bytes), deflated.
pub(crate) fn zip_files(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in files {
        zip.start_file(name.as_str(), o)
            .map_err(|e| e.to_string())?;
        zip.write_all(data).map_err(|e| e.to_string())?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

/// Writes `style` as a SketchUp `.style` archive, laid out like a `.skm` file (the
/// one archive of SketchUp's that was seen): `document.xml` holding the
/// `<sty:style>` element as a `.skp` file's `style.xml` does, `documentProperties.xml`,
/// `references.xml`, `doc_thumbnail.png` (when given) and the watermark images in
/// `ref/`. Our text style goes in as `meshtard.style` for what the items lack.
/// Unverified against SketchUp's own files.
pub(crate) fn write(style: &NamedStyle, thumbnail: Option<&[u8]>) -> Result<Vec<u8>, String> {
    use mt_skp::OptionValue;
    let s = &style.settings;
    let mut xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="no" ?>
<styleDocument xmlns="http://sketchup.google.com/schemas/sketchup/1.0/style" xmlns:sty="http://sketchup.google.com/schemas/sketchup/1.0/style">
  <sty:style xmlns:t="http://sketchup.google.com/schemas/1.0/types" name="{}" desc="{}">
"#,
        escape(&style.name),
        escape(&style.description)
    );
    let mut items = mt_kernel::export::style_items(s);
    items.sort_by_key(|(id, _)| *id);
    for (id, v) in items {
        let (kind, value) = match v {
            OptionValue::Bool(b) => (1, i32::from(b).to_string()),
            OptionValue::Int(i) if COLOR_ITEMS.contains(&id) => (5, i.to_string()),
            OptionValue::Int(i) => (4, i.to_string()),
            OptionValue::Double(d) => (3, d.to_string()),
        };
        xml.push_str(&format!(
            "    <sty:item id=\"{id}\">\n      <t:variant type=\"{kind}\">{value}</t:variant>\n    </sty:item>\n"
        ));
    }
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    if !s.watermarks.is_empty() {
        xml.push_str("    <sty:item id=\"5001\">\n      <t:variant type=\"13\">\n");
        // Backgrounds, the model, then overlays (bottom first, as the reader takes them).
        let mut list: Vec<&Watermark> = s.watermarks.iter().filter(|w| !w.overlay).collect();
        let split = list.len();
        list.extend(s.watermarks.iter().filter(|w| w.overlay));
        for (k, w) in list.iter().enumerate() {
            if k == split {
                xml.push_str("        <screenimage name=\"&lt;MODEL SPACE&gt;\"/>\n");
            }
            let file = format!("watermark_{}.{}", k + 1, image_ext(&w.image));
            xml.push_str(&format!(
                "        <screenimage name=\"{}\" alphaScale=\"{}\" {MODE_ATTR}=\"{}\" {MASK_ATTR}=\"{}\"><image path=\"{file}\" file_name=\"{file}\"/></screenimage>\n",
                escape(&w.name),
                w.opacity.clamp(0.0, 1.0),
                mode_text(&w.mode),
                u8::from(w.mask),
            ));
            files.push((format!("ref/{file}"), w.image.to_vec()));
        }
        if split == list.len() {
            xml.push_str("        <screenimage name=\"&lt;MODEL SPACE&gt;\"/>\n");
        }
        xml.push_str("      </t:variant>\n    </sty:item>\n");
    }
    xml.push_str("  </sty:style>\n</styleDocument>\n");
    let mut all = vec![
        ("document.xml".to_owned(), xml.into_bytes()),
        (
            "documentProperties.xml".to_owned(),
            document_properties(&style.name, &style.description, "Style").into_bytes(),
        ),
        ("references.xml".to_owned(), REFERENCES.as_bytes().to_vec()),
    ];
    if let Some(t) = thumbnail {
        all.push(("doc_thumbnail.png".to_owned(), t.to_vec()));
    }
    let mut plain = style.clone();
    plain.settings.watermarks.clear();
    all.push((
        OURS.to_owned(),
        crate::tray_extra::style_file_text(&plain).into_bytes(),
    ));
    all.extend(files);
    zip_files(&all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// The head of a real `style.xml` (from a `.skp` file's `styles/` folder), with
    /// a watermark list in the shape the `.skp` reader expects.
    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="no" ?>
<styleDocument xmlns="http://sketchup.google.com/schemas/sketchup/1.0/style" xmlns:sty="http://sketchup.google.com/schemas/sketchup/1.0/style">
  <sty:style xmlns:t="http://sketchup.google.com/schemas/1.0/types" name="[Construction Documentation Style]" desc="[Default face colors. Profile Edges. White background.]">
    <sty:item id="1000">
      <t:variant type="4">1</t:variant>
    </sty:item>
    <sty:item id="1006">
      <t:variant type="1">0</t:variant>
    </sty:item>
    <sty:item id="1015">
      <t:variant type="1">1</t:variant>
    </sty:item>
    <sty:item id="2001">
      <t:variant type="4">1</t:variant>
    </sty:item>
    <sty:item id="4000">
      <t:variant type="5">-16776961</t:variant>
    </sty:item>
    <sty:item id="5001">
      <t:variant type="13">
        <screenimage name="Paper" alphaScale="0.25"><image path="./paper.png" file_name="paper.png"/></screenimage>
        <screenimage name="&lt;MODEL SPACE&gt;"/>
        <screenimage name="Logo"><image path="./logo.png" file_name="logo.png"/></screenimage>
      </t:variant>
    </sty:item>
  </sty:style>
</styleDocument>
"#;

    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            zip.start_file(*name, o).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn reads_items_watermarks_and_thumbnail() {
        let data = archive(&[
            ("document.xml", b"<document/>"),
            ("style/style.xml", XML.as_bytes()),
            ("style/paper.png", b"P"),
            ("ref/logo.png", b"L"),
            ("doc_thumbnail.png", b"T"),
        ]);
        assert!(is_zip(&data));
        let z = read(&data).unwrap();
        let s = &z.style;
        assert_eq!(s.name, "Construction Documentation Style");
        assert_eq!(
            s.description,
            "Default face colors. Profile Edges. White background."
        );
        assert!(s.settings.edges);
        assert!(!s.settings.profiles);
        assert!(s.settings.back_edges);
        assert_eq!(s.settings.face_style, mt_kernel::FaceStyle::HiddenLine);
        // -16776961 = 0xFF0000FF: red in the low byte.
        assert_eq!(s.settings.background.color, [255, 0, 0]);
        let w = &s.settings.watermarks;
        assert_eq!(w.len(), 2);
        assert_eq!((w[0].name.as_str(), w[0].overlay), ("Paper", false));
        assert_eq!(&*w[0].image, b"P");
        assert!((w[0].opacity - 0.25).abs() < 1e-6);
        // Found by its file name elsewhere in the archive.
        assert_eq!((w[1].name.as_str(), w[1].overlay), ("Logo", true));
        assert_eq!(&*w[1].image, b"L");
        assert_eq!(z.thumbnail.as_deref(), Some(&b"T"[..]));
    }

    #[test]
    fn escaped_watermark_list_and_errors() {
        let escaped = r#"<sty:style name="E"><sty:item id="5001"><t:variant type="13">&lt;screenimage name="&amp;lt;MODEL SPACE&amp;gt;"/&gt;&lt;screenimage name="P" alphaScale="2"&gt;&lt;image path="paper.png"/&gt;&lt;/screenimage&gt;</t:variant></sty:item></sty:style>"#;
        let data = archive(&[("a/style.xml", escaped.as_bytes()), ("a/paper.png", b"P")]);
        let z = read(&data).unwrap();
        let w = &z.style.settings.watermarks;
        assert_eq!(w.len(), 1);
        assert!(w[0].overlay);
        assert_eq!(w[0].opacity, 1.0);
        assert!(z.thumbnail.is_none());
        assert!(read(&archive(&[("x.xml", b"<a/>")])).is_err());
        assert!(read(b"# meshtard style 1\n").is_err());
        assert!(!is_zip(b"# meshtard style 1\n"));
    }

    #[test]
    fn entities_and_attributes() {
        assert_eq!(
            decode("a &amp;lt; &#65;&#x42; &bogus; &"),
            "a &lt; AB &bogus; &"
        );
        assert_eq!(
            attr(r#" xname="no" name='yes'"#, "name").as_deref(),
            Some("yes")
        );
        assert_eq!(display_name(" [X] ", ""), "X");
        assert_eq!(display_name("", "Style"), "Style");
    }
}
