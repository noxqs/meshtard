//! Material and style XML inside the VFF ZIP, read with regexes as OpenSKP's C++ does.
//! Port of the XML parts of OpenSKP `core.cpp`.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::model::{StyleItem, StyleWatermark};
use crate::raw::{RawMaterial, RawStyle, RawTexture};

/// Namespace prefix allowed in front of a tag name.
const NS: &str = r"(?:[A-Za-z_][\w.-]*:)?";

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regex")
}

/// Decodes the five XML entities and numeric references in one pass, so `&amp;lt;`
/// becomes `&lt;`, not `<`.
pub fn decode_entities(value: &str) -> String {
    static ENTITY: LazyLock<Regex> =
        LazyLock::new(|| re(r"&(lt|gt|amp|apos|quot|#[0-9]+|#[xX][0-9a-fA-F]+);"));
    ENTITY
        .replace_all(value, |c: &regex::Captures| {
            let name = &c[1];
            match name {
                "lt" => "<".into(),
                "gt" => ">".into(),
                "amp" => "&".into(),
                "apos" => "'".into(),
                "quot" => "\"".into(),
                _ => {
                    let code =
                        if let Some(hex) = name.strip_prefix("#x").or(name.strip_prefix("#X")) {
                            u32::from_str_radix(hex, 16).ok()
                        } else {
                            name[1..].parse().ok()
                        };
                    match code.and_then(char::from_u32) {
                        Some(ch) => ch.to_string(),
                        None => c[0].to_owned(),
                    }
                }
            }
        })
        .into_owned()
}

/// Value of attribute `key` in a tag's attribute text, entity-decoded, or empty.
pub fn attr(s: &str, key: &str) -> String {
    let r = re(&format!(r#"(?i)(?:^|\s){key}\s*=\s*["']([^"']*)["']"#));
    r.captures(s)
        .map(|c| decode_entities(&c[1]))
        .unwrap_or_default()
}

/// Every `name="value"` attribute, entity-decoded.
fn attributes(s: &str) -> BTreeMap<String, String> {
    static ATTR: LazyLock<Regex> =
        LazyLock::new(|| re(r#"([A-Za-z_][\w.:-]*)\s*=\s*(?:"([^"]*)"|'([^']*)')"#));
    ATTR.captures_iter(s)
        .map(|c| {
            let v = c.get(2).or(c.get(3)).map_or("", |m| m.as_str());
            (c[1].to_owned(), decode_entities(v))
        })
        .collect()
}

/// Leading integer as C's `stoi` reads it ("12px" -> 12), or `fallback`.
pub fn integer(s: &str, fallback: i32) -> i32 {
    let t = s.trim_start();
    let end = t
        .char_indices()
        .take_while(|&(i, c)| c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+')))
        .count();
    t[..end].parse().unwrap_or(fallback)
}

/// Leading decimal number as C's `stod` reads it, or `fallback`.
pub fn number(s: &str, fallback: f64) -> f64 {
    static NUM: LazyLock<Regex> =
        LazyLock::new(|| re(r"^\s*[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?"));
    NUM.find(s)
        .and_then(|m| m.as_str().trim().parse().ok())
        .unwrap_or(fallback)
}

fn basename(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

/// Reads one ZIP entry by name, if present.
pub trait EntrySource {
    fn names(&self) -> &[String];
    fn get(&mut self, name: &str) -> Option<Vec<u8>>;
}

/// Parses `materials/<folder>/material.xml`, loading its texture image from the ZIP.
pub fn material(zip: &mut impl EntrySource, path: &str, xml: &str) -> Option<RawMaterial> {
    static TAG: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"(?i)<{NS}material\b([^>]*)>")));
    static TEX: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"(?i)<{NS}texture\b([^>]*)>")));
    static IMG: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"(?i)<{NS}image\b([^>]*)")));

    let a = TAG.captures(xml)?.get(1)?.as_str().to_owned();
    let mut out = RawMaterial {
        name: attr(&a, "name"),
        r: integer(&attr(&a, "colorRed"), 128),
        g: integer(&attr(&a, "colorGreen"), 128),
        b: integer(&attr(&a, "colorBlue"), 128),
        colorized: attr(&a, "type") == "2",
        colorize_type: integer(&attr(&a, "colorizeType"), 0),
        ..Default::default()
    };
    if out.name.is_empty() {
        out.name = "unknown".into();
    }
    if attr(&a, "useTrans") == "1" {
        out.transparency = (1.0 - number(&attr(&a, "trans"), 0.0)).clamp(0.0, 1.0);
    }
    if let Some(tc) = TEX.captures(xml) {
        let ta = &tc[1];
        let mut t = RawTexture {
            filename: attr(ta, "textureFilename"),
            x_scale: number(&attr(ta, "xScale"), 0.0),
            y_scale: number(&attr(ta, "yScale"), 0.0),
            data: None,
        };
        let folder = path.rfind('/').map_or("", |i| &path[..i]).to_owned();
        if !t.filename.is_empty() {
            t.data = zip.get(&format!("{folder}/{}", t.filename));
        }
        if t.data.is_none() {
            let prefix = format!("{folder}/");
            let candidate = zip
                .names()
                .iter()
                .find(|n| {
                    n.starts_with(&prefix) && *n != path && n.len() > 4 && !n.ends_with(".xml")
                })
                .cloned();
            if let Some(n) = candidate {
                t.data = zip.get(&n);
                if t.filename.is_empty() {
                    t.filename = basename(&n).to_owned();
                }
            }
        }
        if t.data.is_none()
            && let Some(ic) = IMG.captures(xml)
        {
            let ip = attr(&ic[1], "path");
            let ip = ip.trim_start_matches(['.', '/']).to_owned();
            for c in [ip.clone(), format!("{folder}/{ip}")] {
                if c.is_empty() {
                    continue;
                }
                if let Some(d) = zip.get(&c) {
                    t.data = Some(d);
                    if t.filename.is_empty() {
                        t.filename = basename(&c).to_owned();
                    }
                    break;
                }
            }
        }
        out.texture = Some(t);
    }
    Some(out)
}

/// Watermarks in style item 5001: `<screenimage>` entries, without the `<MODEL SPACE>`
/// separator. Image bytes are filled in by the caller.
fn watermarks(list: &str) -> Vec<StyleWatermark> {
    static SI: LazyLock<Regex> = LazyLock::new(|| {
        re(&format!(
            r"(?i)<{NS}screenimage\b([^>]*?)(?:/>|>([\s\S]*?)</{NS}screenimage>)"
        ))
    });
    static IM: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"(?i)<{NS}image\b([^>]*)>")));
    SI.captures_iter(list)
        .filter_map(|c| {
            let head = &c[1];
            let name = attr(head, "name");
            if name == "<MODEL SPACE>" {
                return None;
            }
            let mut w = StyleWatermark {
                attributes: attributes(head),
                name,
                ..Default::default()
            };
            w.attributes.remove("name");
            if let Some(m) = c.get(2).and_then(|b| IM.captures(b.as_str())) {
                w.image_path = attr(&m[1], "path");
                w.file_name = attr(&m[1], "file_name");
            }
            Some(w)
        })
        .collect()
}

/// Parses a `styles/<folder>/style.xml`.
pub fn style(xml: &str) -> Option<RawStyle> {
    static ST: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"(?i)<{NS}style\b([^>]*)>")));
    static ITEM: LazyLock<Regex> =
        LazyLock::new(|| re(&format!(r"(?i)<{NS}item\b([^>]*)>([\s\S]*?)</{NS}item>")));
    static VAR: LazyLock<Regex> = LazyLock::new(|| {
        re(&format!(
            r"(?i)<{NS}variant\b([^>]*)>([\s\S]*?)</{NS}variant>"
        ))
    });
    let head = ST.captures(xml)?;
    let mut o = RawStyle {
        name: attr(&head[1], "name"),
        description: attr(&head[1], "desc"),
        ..Default::default()
    };
    for c in ITEM.captures_iter(xml) {
        let id = integer(&attr(&c[1], "id"), -1);
        if id < 0 {
            continue;
        }
        let Some(v) = VAR.captures(&c[2]) else {
            continue;
        };
        let item = StyleItem {
            kind: integer(&attr(&v[1], "type"), 0),
            value: v[2].trim_matches([' ', '\t', '\r', '\n']).to_owned(),
        };
        // Face colors: 2002 front, 2003 back, signed int32 ABGR (red in the low byte).
        if (id == 2002 || id == 2003)
            && let Ok(n) = item.value.parse::<i64>()
        {
            let n = n as u32;
            let color = Some([n as u8, (n >> 8) as u8, (n >> 16) as u8]);
            if id == 2002 {
                o.front_color = color;
            } else {
                o.back_color = color;
            }
        }
        o.items.insert(id, item);
    }
    if let Some(wm) = o.items.get(&5001) {
        o.watermarks = watermarks(&wm.value);
    }
    Some(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_decode_once() {
        assert_eq!(decode_entities("&lt;auto&gt;"), "<auto>");
        assert_eq!(decode_entities("&amp;lt;"), "&lt;");
        assert_eq!(decode_entities("&#39;&#x41;"), "'A");
        assert_eq!(decode_entities("&bogus;"), "&bogus;");
    }

    #[test]
    fn lenient_numbers() {
        assert_eq!(integer(" 12px", 0), 12);
        assert_eq!(integer("", 7), 7);
        assert_eq!(integer("-3", 0), -3);
        assert_eq!(number("0.25", 1.0), 0.25);
        assert_eq!(number("1e2x", 0.0), 100.0);
        assert_eq!(number("x", 4.0), 4.0);
    }

    #[test]
    fn style_colors_and_watermarks() {
        let xml = r#"<sty:style name="S" desc="d &amp; e">
            <sty:item id="2002"><t:variant type="4">255</t:variant></sty:item>
            <sty:item id="5001"><t:variant type="13">
              <screenimage name="&lt;MODEL SPACE&gt;"/>
              <screenimage name="W" alphaScale="0.5"><image path="./2.jpg" file_name="2.jpg"/></screenimage>
            </t:variant></sty:item>
        </sty:style>"#;
        let s = style(xml).unwrap();
        assert_eq!(s.name, "S");
        assert_eq!(s.description, "d & e");
        assert_eq!(s.front_color, Some([255, 0, 0]));
        assert_eq!(s.watermarks.len(), 1);
        assert_eq!(s.watermarks[0].image_path, "./2.jpg");
        assert_eq!(s.watermarks[0].attributes["alphaScale"], "0.5");
    }
}
