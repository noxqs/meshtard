//! Poly Haven (<https://polyhaven.com>): CC0 models through its public API
//! (<https://api.polyhaven.com>, terms: github.com/Poly-Haven/Public-API ToS.md).
//!
//! - `GET /assets?type=models` lists every model (about 1 MB of JSON); searching is
//!   done here on that list (name, tags, categories), cached for a day.
//! - `GET /files/{id}` gives the downloads; the 1k glTF is taken (smallest
//!   textures): the `.gltf` file plus its `include` files (buffer, textures) saved
//!   next to it under their relative paths.
//! - Thumbnails come from the CDN URL the list gives, asked for as PNG.
//! - The terms ask for a visible "where the content comes from" credit (2.5); the
//!   panel shows "Models from Poly Haven (CC0)" while this source is picked.
//!
//! Parsing is pure (tested with small hand-made JSON).

use serde_json::Value;

use super::{Asset, Source};

pub(crate) const API: &str = "https://api.polyhaven.com";
pub(crate) const LICENSE: &str = "CC0 1.0 (public domain)";
pub(crate) const LICENSE_URL: &str = "https://creativecommons.org/publicdomain/zero/1.0/";

pub(crate) fn list_url() -> String {
    format!("{API}/assets?type=models")
}

pub(crate) fn files_url(id: &str) -> String {
    format!("{API}/files/{id}")
}

/// One file of a download: where it goes (relative to the model's cache folder),
/// where it comes from, and its size in bytes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RemoteFile {
    pub path: String,
    pub url: String,
    pub size: u64,
}

/// What to download for one model: the main file first.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Download {
    pub files: Vec<RemoteFile>,
}

impl Download {
    pub(crate) fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// The models of an `/assets?type=models` answer, sorted by name.
pub(crate) fn parse_assets(json: &str) -> Result<Vec<Asset>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Poly Haven list: {e}"))?;
    let obj = v.as_object().ok_or("Poly Haven list: not a JSON object")?;
    let mut out: Vec<Asset> = obj
        .iter()
        .filter(|(_, a)| a.get("type").and_then(Value::as_u64).is_none_or(|t| t == 2))
        .map(|(id, a)| {
            let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
            let list = |k: &str| -> Vec<String> {
                a.get(k)
                    .and_then(Value::as_array)
                    .map(|v| {
                        v.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let mut authors: Vec<String> = a
                .get("authors")
                .and_then(Value::as_object)
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            authors.sort();
            let mut tags = list("tags");
            tags.extend(list("categories"));
            let thumb = s("thumbnail_url");
            Asset {
                source: Source::PolyHaven,
                id: id.clone(),
                name: if s("name").is_empty() {
                    id.clone()
                } else {
                    s("name")
                },
                author: authors.join(", "),
                license: LICENSE.into(),
                license_url: LICENSE_URL.into(),
                page_url: format!("https://polyhaven.com/a/{id}"),
                thumbnail_url: (!thumb.is_empty()).then(|| png_thumbnail(&thumb)),
                description: s("description"),
                tags,
                size: None,
                polycount: a.get("polycount").and_then(Value::as_u64),
                ..Default::default()
            }
        })
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    Ok(out)
}

/// The thumbnail URL with PNG asked for instead of WebP (no WebP decoder here).
pub(crate) fn png_thumbnail(url: &str) -> String {
    if url.contains("format=") {
        url.replace("format=webp", "format=png")
    } else if url.contains('?') {
        format!("{url}&format=png")
    } else {
        format!("{url}?format=png")
    }
}

/// The glTF download of a `/files/{id}` answer at `res` ("1k", "2k", ...), falling
/// back to the smallest resolution there is.
pub(crate) fn parse_files(json: &str, res: &str) -> Result<Download, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Poly Haven files: {e}"))?;
    let gltf = v
        .get("gltf")
        .and_then(Value::as_object)
        .ok_or("This model has no glTF download.")?;
    let entry = gltf.get(res).or_else(|| {
        let mut keys: Vec<&String> = gltf.keys().collect();
        keys.sort_by_key(|k| k.trim_end_matches('k').parse::<u32>().unwrap_or(u32::MAX));
        keys.first().and_then(|k| gltf.get(*k))
    });
    let main = entry
        .and_then(|e| e.get("gltf"))
        .ok_or("This model has no glTF download.")?;
    let file = |path: String, f: &Value| -> Option<RemoteFile> {
        Some(RemoteFile {
            path,
            url: f.get("url")?.as_str()?.to_owned(),
            size: f.get("size").and_then(Value::as_u64).unwrap_or(0),
        })
    };
    let url = main.get("url").and_then(Value::as_str).unwrap_or("");
    let name = url
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or("model.gltf");
    let mut files = vec![file(name.to_owned(), main).ok_or("Poly Haven files: no URL")?];
    if let Some(inc) = main.get("include").and_then(Value::as_object) {
        let mut inc: Vec<_> = inc.iter().collect();
        inc.sort_by_key(|(k, _)| k.as_str());
        for (path, f) in inc {
            if !super::safe_relative(path) {
                return Err(format!("Poly Haven files: unsafe path {path}"));
            }
            files.extend(file(path.clone(), f));
        }
    }
    Ok(Download { files })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"{
      "wooden_stool": {"name": "Wooden Stool", "type": 2, "authors": {"B Author": "All", "A Author": "Model"},
        "tags": ["stool", "wood"], "categories": ["furniture"], "polycount": 1200,
        "description": "A stool.",
        "thumbnail_url": "https://cdn.polyhaven.com/asset_img/thumbs/wooden_stool.png?width=256&height=256&format=webp&v=1"},
      "a_hdri": {"name": "Sky", "type": 0},
      "apple": {"name": "apple", "type": 2, "authors": {"C": "All"}}
    }"#;

    #[test]
    fn parses_models_sorted_with_license_and_png_thumbnails() {
        let a = parse_assets(LIST).unwrap();
        assert_eq!(a.len(), 2, "the HDRI is left out");
        assert_eq!(a[0].id, "apple");
        assert_eq!(a[0].thumbnail_url, None);
        let s = &a[1];
        assert_eq!(s.name, "Wooden Stool");
        assert_eq!(s.author, "A Author, B Author");
        assert_eq!(s.license, LICENSE);
        assert_eq!(s.page_url, "https://polyhaven.com/a/wooden_stool");
        assert_eq!(s.tags, ["stool", "wood", "furniture"]);
        assert_eq!(s.polycount, Some(1200));
        assert!(s.thumbnail_url.as_deref().unwrap().contains("format=png"));
        assert!(parse_assets("[]").is_err());
        assert!(parse_assets("not json").is_err());
    }

    const FILES: &str = r#"{
      "blend": {},
      "gltf": {
        "2k": {"gltf": {"url": "https://dl.example/2k/m_2k.gltf", "size": 10}},
        "1k": {"gltf": {"url": "https://dl.example/1k/m_1k.gltf", "size": 3000,
          "include": {
            "textures/m_diff_1k.jpg": {"url": "https://dl.example/t.jpg", "size": 200},
            "m.bin": {"url": "https://dl.example/m.bin", "size": 100}
          }}}
      }
    }"#;

    #[test]
    fn parses_the_1k_gltf_with_its_files() {
        let d = parse_files(FILES, "1k").unwrap();
        let paths: Vec<_> = d.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["m_1k.gltf", "m.bin", "textures/m_diff_1k.jpg"]);
        assert_eq!(d.size(), 3300);
        // Missing resolution: the smallest there is.
        let d = parse_files(FILES, "8k").unwrap();
        assert_eq!(d.files[0].path, "m_1k.gltf");
        assert!(parse_files(r#"{"fbx": {}}"#, "1k").is_err());
    }

    #[test]
    fn rejects_paths_leaving_the_folder() {
        let bad = r#"{"gltf": {"1k": {"gltf": {"url": "https://x/m.gltf", "size": 1,
            "include": {"../evil.bin": {"url": "https://x/e", "size": 1}}}}}}"#;
        assert!(parse_files(bad, "1k").is_err());
    }

    #[test]
    fn thumbnail_urls_ask_for_png() {
        assert_eq!(
            png_thumbnail("https://a/b.png"),
            "https://a/b.png?format=png"
        );
        assert_eq!(
            png_thumbnail("https://a/b?w=1"),
            "https://a/b?w=1&format=png"
        );
        assert_eq!(
            png_thumbnail("https://a/b?format=webp"),
            "https://a/b?format=png"
        );
    }
}
