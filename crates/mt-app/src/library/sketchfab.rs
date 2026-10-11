//! Sketchfab (<https://sketchfab.com>), optional: shown only when the user has
//! entered their own API token (Sketchfab account settings > API token).
//!
//! - Search: `GET /v3/search?type=models&downloadable=true&q=...` (no token needed),
//!   24 results per page.
//! - Download: `GET /v3/models/{uid}/download` with `Authorization: Token ...` gives
//!   short-lived URLs of glTF / GLB / USDZ archives; the `.glb` is taken (one file).
//! - Each model has its own license (Creative Commons variants, some non-commercial
//!   or no-derivatives); it is shown in the details and stored on the definition.
//!   Sketchfab's download guidelines ask apps to show the license and author.
//!
//! Parsing is pure (tested with small hand-made JSON).

use serde_json::Value;

use super::polyhaven::{Download, RemoteFile};
use super::{Asset, Source};

pub(crate) const API: &str = "https://api.sketchfab.com/v3";

/// The search URL for `query` (percent-encoded).
pub(crate) fn search_url(query: &str) -> String {
    format!(
        "{API}/search?type=models&downloadable=true&count=24&q={}",
        encode(query)
    )
}

pub(crate) fn download_url(uid: &str) -> String {
    format!("{API}/models/{uid}/download")
}

/// Percent-encodes a query parameter value.
pub(crate) fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// License page for a Sketchfab license label (the API gives a label only).
fn license_url(label: &str) -> String {
    let l = label.to_lowercase();
    let slug = match () {
        _ if l.contains("cc0") || l.contains("public domain") => {
            return "https://creativecommons.org/publicdomain/zero/1.0/".into();
        }
        _ if l.contains("noncommercial") && l.contains("noderivs") => "by-nc-nd",
        _ if l.contains("noncommercial") && l.contains("sharealike") => "by-nc-sa",
        _ if l.contains("noncommercial") => "by-nc",
        _ if l.contains("noderivs") => "by-nd",
        _ if l.contains("sharealike") => "by-sa",
        _ if l.contains("attribution") => "by",
        _ => return "https://sketchfab.com/licenses".into(),
    };
    format!("https://creativecommons.org/licenses/{slug}/4.0/")
}

/// The models of a search answer, and the next page's URL.
pub(crate) fn parse_search(json: &str) -> Result<(Vec<Asset>, Option<String>), String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Sketchfab search: {e}"))?;
    let results = v
        .get("results")
        .and_then(Value::as_array)
        .ok_or("Sketchfab search: no results list")?;
    let assets = results
        .iter()
        .filter_map(|r| {
            let s = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or("").to_owned();
            let uid = s(r.get("uid"));
            if uid.is_empty() {
                return None;
            }
            let user = r.get("user");
            let mut author = s(user.and_then(|u| u.get("displayName")));
            if author.is_empty() {
                author = s(user.and_then(|u| u.get("username")));
            }
            let license = s(r.get("license").and_then(|l| l.get("label")));
            // The smallest thumbnail at least 200 pixels wide.
            let thumbnail_url = r
                .get("thumbnails")
                .and_then(|t| t.get("images"))
                .and_then(Value::as_array)
                .and_then(|imgs| {
                    imgs.iter()
                        .filter(|i| i.get("width").and_then(Value::as_u64).unwrap_or(0) >= 200)
                        .min_by_key(|i| i.get("width").and_then(Value::as_u64).unwrap_or(0))
                        .and_then(|i| i.get("url"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                });
            let glb = r.get("archives").and_then(|a| a.get("glb"));
            let tags = r
                .get("tags")
                .and_then(Value::as_array)
                .map(|t| {
                    t.iter()
                        .filter_map(|x| x.get("name").and_then(Value::as_str))
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            Some(Asset {
                source: Source::Sketchfab,
                name: s(r.get("name")),
                author,
                license_url: license_url(&license),
                license: if license.is_empty() {
                    "Unknown (see the model page)".into()
                } else {
                    license
                },
                page_url: s(r.get("viewerUrl")),
                thumbnail_url,
                description: s(r.get("description")),
                tags,
                size: glb.and_then(|g| g.get("size")).and_then(Value::as_u64),
                polycount: r.get("faceCount").and_then(Value::as_u64),
                id: uid,
                ..Default::default()
            })
        })
        .collect();
    let next = v.get("next").and_then(Value::as_str).map(str::to_owned);
    Ok((assets, next))
}

/// The `.glb` of a download answer.
pub(crate) fn parse_download(json: &str, uid: &str) -> Result<Download, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Sketchfab download: {e}"))?;
    let glb = v.get("glb").ok_or("This model has no GLB download.")?;
    let url = glb
        .get("url")
        .and_then(Value::as_str)
        .ok_or("Sketchfab download: no URL")?;
    Ok(Download {
        files: vec![RemoteFile {
            path: format!("{uid}.glb"),
            url: url.to_owned(),
            size: glb.get("size").and_then(Value::as_u64).unwrap_or(0),
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH: &str = r#"{
      "next": "https://api.sketchfab.com/v3/search?cursor=24&q=chair",
      "results": [
        {"uid": "abc123", "name": "Chair", "viewerUrl": "https://sketchfab.com/3d-models/chair-abc123",
         "license": {"label": "CC Attribution"},
         "user": {"username": "jdoe", "displayName": "J. Doe"},
         "tags": [{"name": "chair"}, {"name": "wood"}],
         "faceCount": 900,
         "thumbnails": {"images": [
            {"width": 1024, "url": "https://m/1024.jpeg"},
            {"width": 256, "url": "https://m/256.jpeg"},
            {"width": 100, "url": "https://m/100.jpeg"}]},
         "archives": {"glb": {"size": 123456}, "gltf": {"size": 1}}},
        {"name": "no uid"},
        {"uid": "x", "name": "Bare", "user": {"username": "u"}}
      ]
    }"#;

    #[test]
    fn parses_search_results() {
        let (a, next) = parse_search(SEARCH).unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].id, "abc123");
        assert_eq!(a[0].author, "J. Doe");
        assert_eq!(a[0].license, "CC Attribution");
        assert_eq!(
            a[0].license_url,
            "https://creativecommons.org/licenses/by/4.0/"
        );
        assert_eq!(a[0].thumbnail_url.as_deref(), Some("https://m/256.jpeg"));
        assert_eq!(a[0].size, Some(123456));
        assert_eq!(a[0].tags, ["chair", "wood"]);
        assert_eq!(a[1].author, "u");
        assert!(a[1].license.starts_with("Unknown"));
        assert!(next.unwrap().contains("cursor=24"));
        assert!(parse_search("{}").is_err());
    }

    #[test]
    fn license_urls() {
        let u = |l| license_url(l);
        assert!(u("CC Attribution-NonCommercial-NoDerivs").ends_with("/by-nc-nd/4.0/"));
        assert!(u("CC Attribution-ShareAlike").ends_with("/by-sa/4.0/"));
        assert!(u("CC0 Public Domain").contains("zero"));
        assert_eq!(u("Standard"), "https://sketchfab.com/licenses");
    }

    #[test]
    fn parses_download_and_encodes_queries() {
        let d = parse_download(
            r#"{"glb": {"url": "https://s/x.glb?sig=1", "size": 42, "expires": 300}}"#,
            "abc",
        )
        .unwrap();
        assert_eq!(d.files[0].path, "abc.glb");
        assert_eq!(d.size(), 42);
        assert!(parse_download(r#"{"usdz": {}}"#, "a").is_err());
        assert_eq!(encode("old chair&x"), "old%20chair%26x");
        assert!(search_url("a b").ends_with("q=a%20b"));
    }
}
