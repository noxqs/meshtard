//! The Meshtard Extension Warehouse: a JSON catalog of extensions with download URLs
//! and SHA-256 checksums (`extensions/catalog.json` in the Meshtard repository by
//! default). Package URLs may be relative to the catalog's URL. A download is
//! installed only when its SHA-256 matches the catalog and its manifest matches the
//! entry. A catalog may be signed by `<catalog URL>.sig` (see [`super::signing`]).

use sha2::{Digest, Sha256};

use super::manifest::{Permission, parse_version, valid_id};
use super::signing::{self, SigStatus, TrustedKey};

/// The default catalog: the repository's `extensions/catalog.json` on its default
/// branch (`HEAD` on raw.githubusercontent.com).
pub(crate) const DEFAULT_CATALOG: &str =
    "https://raw.githubusercontent.com/noxqs/meshtard/HEAD/extensions/catalog.json";

/// One extension in the catalog.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub(crate) struct Entry {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub min_meshtard: Option<String>,
    /// The `.mtx` file, absolute or relative to the catalog.
    pub url: String,
    /// Lowercase hex SHA-256 of the `.mtx`.
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub homepage: String,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub(crate) struct Catalog {
    pub format: u32,
    #[serde(default)]
    pub name: String,
    pub extensions: Vec<Entry>,
}

impl Catalog {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let c: Self = serde_json::from_str(text).map_err(|e| format!("catalog: {e}"))?;
        if c.format != 1 {
            return Err(format!("catalog: unsupported format {}", c.format));
        }
        for e in &c.extensions {
            if !valid_id(&e.id) {
                return Err(format!("catalog: bad extension id `{}`", e.id));
            }
            if parse_version(&e.version).is_none() {
                return Err(format!("catalog: {}: bad version `{}`", e.id, e.version));
            }
            if e.sha256.len() != 64 || !e.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!("catalog: {}: sha256 must be 64 hex digits", e.id));
            }
            if let Some(p) = e
                .permissions
                .iter()
                .find(|p| Permission::from_key(p).is_none())
            {
                return Err(format!("catalog: {}: unknown permission `{p}`", e.id));
            }
        }
        Ok(c)
    }
}

/// Indices of the entries matching every word of `query` (name, id, author,
/// description).
pub(crate) fn search(entries: &[Entry], query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    (0..entries.len())
        .filter(|&i| {
            let e = &entries[i];
            let hay = format!("{} {} {} {}", e.name, e.id, e.author, e.description).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Checks a download against its catalog entry.
pub(crate) fn verify(entry: &Entry, bytes: &[u8]) -> Result<(), String> {
    let got = sha256_hex(bytes);
    if !got.eq_ignore_ascii_case(&entry.sha256) {
        return Err(format!(
            "the download's SHA-256 ({got}) does not match the catalog ({}); not installed",
            entry.sha256
        ));
    }
    Ok(())
}

/// Installed extensions with a newer version in the catalog: (entry index, the
/// installed version).
pub(crate) fn updates(entries: &[Entry], installed: &[(String, String)]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let Some((_, have)) = installed.iter().find(|(id, _)| *id == e.id) else {
            continue;
        };
        if let (Some(new), Some(old)) = (parse_version(&e.version), parse_version(have))
            && new > old
        {
            out.push((i, have.clone()));
        }
    }
    out
}

/// Fetches and parses a catalog and checks its signature (`<url>.sig`; none, or
/// one that cannot be fetched, is "unsigned"). A broken signature is an error.
pub(crate) fn fetch_catalog(
    url: &str,
    trusted: &[TrustedKey],
) -> Result<(Catalog, SigStatus), String> {
    let bytes = fetch(url)?;
    let catalog = Catalog::parse(&String::from_utf8_lossy(&bytes))?;
    let sig = fetch(&format!("{url}.sig")).ok();
    match signing::verify_catalog(&bytes, sig.as_deref(), trusted) {
        SigStatus::Invalid(e) => Err(format!(
            "the catalog's signature is invalid ({e}); not used"
        )),
        s => Ok((catalog, s)),
    }
}

/// `url` resolved against the catalog's URL (`https://`, `file://` or a local path).
pub(crate) fn resolve(base: &str, url: &str) -> String {
    if url.contains("://") || url.starts_with('/') {
        return url.to_owned();
    }
    match base.rfind('/') {
        Some(i) => format!("{}{url}", &base[..=i]),
        None => url.to_owned(),
    }
}

/// GETs `url`: `http(s)://` through the library's HTTP code, `file://` or a plain
/// path from disk.
pub(crate) fn fetch(url: &str) -> Result<Vec<u8>, String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        crate::library::net::get_bytes(url, None)
    } else {
        let path = url.strip_prefix("file://").unwrap_or(url);
        std::fs::read(path).map_err(|e| format!("{path}: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(sha: &str) -> String {
        format!(
            r#"{{"format": 1, "name": "Test", "extensions": [
            {{"id": "a.b", "name": "A", "version": "1.0.0", "url": "dist/a.mtx",
              "sha256": "{sha}", "permissions": ["model.read"]}}]}}"#
        )
    }

    #[test]
    fn parses_and_verifies() {
        let data = b"package bytes";
        let sha = sha256_hex(data);
        assert_eq!(sha.len(), 64);
        let c = Catalog::parse(&catalog(&sha)).unwrap();
        assert_eq!(c.extensions[0].id, "a.b");
        verify(&c.extensions[0], data).unwrap();
        assert!(
            verify(&c.extensions[0], b"tampered")
                .unwrap_err()
                .contains("does not match")
        );
        assert_eq!(search(&c.extensions, "a"), [0]);
        assert!(search(&c.extensions, "zzz").is_empty());
    }

    #[test]
    fn known_sha256() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn rejects_bad_catalogs() {
        assert!(Catalog::parse(&catalog("xyz")).is_err());
        assert!(Catalog::parse("{}").is_err());
        let ok = catalog(&"0".repeat(64));
        assert!(Catalog::parse(&ok.replace("\"format\": 1", "\"format\": 2")).is_err());
        assert!(Catalog::parse(&ok.replace("model.read", "root")).is_err());
    }

    #[test]
    fn finds_updates() {
        let c = Catalog::parse(&catalog(&"0".repeat(64))).unwrap();
        let have = |v: &str| vec![("a.b".to_owned(), v.to_owned())];
        assert_eq!(
            updates(&c.extensions, &have("0.9")),
            [(0, "0.9".to_owned())]
        );
        assert!(updates(&c.extensions, &have("1.0.0")).is_empty());
        assert!(updates(&c.extensions, &have("1.2")).is_empty());
        assert!(updates(&c.extensions, &[]).is_empty());
    }

    #[test]
    fn signed_catalog_files() {
        let dir = super::super::package::tests::scratch("signed_catalog");
        let text = catalog(&"0".repeat(64));
        let path = dir.join("catalog.json");
        std::fs::write(&path, &text).unwrap();
        let url = format!("file://{}", path.display());
        let key = signing::test_key();
        let trusted = [TrustedKey {
            name: "T".into(),
            key: key.verifying_key().to_bytes(),
            builtin: false,
        }];
        assert_eq!(
            fetch_catalog(&url, &trusted).unwrap().1,
            SigStatus::Unsigned
        );
        let sig = signing::sign_catalog(text.as_bytes(), &key, "");
        std::fs::write(dir.join("catalog.json.sig"), &sig).unwrap();
        assert!(fetch_catalog(&url, &trusted).unwrap().1.trusted());
        assert!(!fetch_catalog(&url, &[]).unwrap().1.trusted());
        // Changed after signing: refused.
        std::fs::write(&path, text.replace("\"A\"", "\"B\"")).unwrap();
        assert!(
            fetch_catalog(&url, &trusted)
                .unwrap_err()
                .contains("signature")
        );
    }

    #[test]
    fn resolves_relative_urls() {
        assert_eq!(
            resolve(DEFAULT_CATALOG, "dist/a.mtx"),
            "https://raw.githubusercontent.com/noxqs/meshtard/HEAD/extensions/dist/a.mtx"
        );
        assert_eq!(
            resolve("file:///x/catalog.json", "a.mtx"),
            "file:///x/a.mtx"
        );
        assert_eq!(resolve("x", "https://e/a.mtx"), "https://e/a.mtx");
    }
}
