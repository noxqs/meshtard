//! Signed extension packages and catalogs (ed25519). See `spec/EXTENSIONS.md`,
//! "Signatures".
//!
//! - A package is signed by a `signature.json` at its root. The signature covers a
//!   listing of every other file of the package (sorted path and SHA-256), so any
//!   changed, added or removed file breaks it.
//! - A catalog is signed by a detached `<catalog URL>.sig` over the catalog's exact
//!   bytes.
//!
//! A signature counts only when its key is trusted: the built-in Meshtard key (a
//! placeholder until a release key exists) or a key the user added in the Extension
//! Manager. A broken signature refuses the package; a valid signature by an unknown
//! key is shown with the key's fingerprint and treated like no signature.

use std::collections::BTreeMap;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use super::catalog::sha256_hex;

/// The file in a package that holds its signature.
pub(crate) const SIGNATURE_FILE: &str = "signature.json";
/// What a package signature signs: this line, then `<sha256>  <path>\n` per file.
const PACKAGE_DOMAIN: &str = "meshtard-extension-package-v1\n";
/// What a catalog signature signs: this line, then the catalog's bytes.
const CATALOG_DOMAIN: &[u8] = b"meshtard-extension-catalog-v1\n";

/// Keys built into Meshtard: (name, hex public key).
///
/// Placeholder: Meshtard has no release signing key yet. When one is made, its
/// public key goes here (never the private key: that stays with the maintainers,
/// outside the repository). Until then only keys the user adds are trusted.
pub(crate) const BUILTIN_KEYS: &[(&str, &str)] = &[];

/// A key whose signatures are trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrustedKey {
    pub name: String,
    pub key: [u8; 32],
    /// Built into Meshtard (cannot be removed).
    pub builtin: bool,
}

/// The built-in keys plus the user's (`name -> hex`).
pub(crate) fn trusted_keys(user: &BTreeMap<String, String>) -> Vec<TrustedKey> {
    let mut out: Vec<TrustedKey> = BUILTIN_KEYS
        .iter()
        .filter_map(|(name, hex)| {
            Some(TrustedKey {
                name: (*name).to_owned(),
                key: parse_public_key(hex).ok()?,
                builtin: true,
            })
        })
        .collect();
    for (name, hex) in user {
        if let Ok(key) = parse_public_key(hex) {
            out.push(TrustedKey {
                name: name.clone(),
                key,
                builtin: false,
            });
        }
    }
    out
}

/// What a signature check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SigStatus {
    Unsigned,
    /// Valid, by a trusted key (its name in the trusted-keys list).
    Trusted {
        signer: String,
    },
    /// Valid, but by a key that is not trusted: treated as unsigned.
    UnknownKey {
        fingerprint: String,
        /// The name the signature claims (not checked).
        claimed: String,
        key: String,
    },
    /// Present but wrong (changed files, bad format): the package is refused.
    Invalid(String),
}

impl SigStatus {
    pub(crate) fn trusted(&self) -> bool {
        matches!(self, Self::Trusted { .. })
    }

    /// One line for the Extension Manager, the Warehouse and the install prompt.
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Unsigned => "Unsigned".into(),
            Self::Trusted { signer } => format!("Signed by {signer}"),
            Self::UnknownKey {
                fingerprint,
                claimed,
                ..
            } => {
                let who = if claimed.is_empty() {
                    String::new()
                } else {
                    format!(" (claims \"{claimed}\")")
                };
                format!("Signed with an untrusted key {fingerprint}{who}: treated as unsigned")
            }
            Self::Invalid(e) => format!("Invalid signature: {e}"),
        }
    }
}

/// `signature.json` and `<catalog>.sig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SigFile {
    format: u32,
    /// Hex public key.
    key: String,
    /// Who signed (informational; the trusted-keys list names the key).
    #[serde(default)]
    signer: String,
    /// Hex ed25519 signature.
    signature: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// A hex ed25519 public key (64 hex digits) that is a valid curve point.
pub(crate) fn parse_public_key(s: &str) -> Result<[u8; 32], String> {
    let bytes: [u8; 32] = unhex(s)
        .and_then(|b| b.try_into().ok())
        .ok_or("a public key is 64 hex digits")?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| "not a valid ed25519 public key")?;
    Ok(bytes)
}

pub(crate) fn public_key_hex(key: &[u8; 32]) -> String {
    hex(key)
}

/// A short, readable form of a key: the first 16 hex digits of its SHA-256 in
/// groups of four.
pub(crate) fn fingerprint(key: &[u8; 32]) -> String {
    let h = sha256_hex(key);
    h.as_bytes()[..16]
        .chunks(4)
        .map(|c| std::str::from_utf8(c).expect("hex"))
        .collect::<Vec<_>>()
        .join(":")
}

/// The bytes a package signature signs.
fn package_message(files: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut m = PACKAGE_DOMAIN.to_owned();
    for (name, data) in files {
        if name == SIGNATURE_FILE {
            continue;
        }
        m.push_str(&sha256_hex(data));
        m.push_str("  ");
        m.push_str(name);
        m.push('\n');
    }
    m.into_bytes()
}

fn catalog_message(bytes: &[u8]) -> Vec<u8> {
    let mut m = CATALOG_DOMAIN.to_vec();
    m.extend_from_slice(bytes);
    m
}

fn sign(message: &[u8], key: &SigningKey, signer: &str) -> String {
    let file = SigFile {
        format: 1,
        key: hex(key.verifying_key().as_bytes()),
        signer: signer.to_owned(),
        signature: hex(&key.sign(message).to_bytes()),
    };
    serde_json::to_string_pretty(&file).expect("serializable") + "\n"
}

fn verify(message: &[u8], sig: &[u8], trusted: &[TrustedKey]) -> SigStatus {
    let file: SigFile = match serde_json::from_slice(sig) {
        Ok(f) => f,
        Err(e) => return SigStatus::Invalid(format!("unreadable signature file ({e})")),
    };
    if file.format != 1 {
        return SigStatus::Invalid(format!("unsupported signature format {}", file.format));
    }
    let Ok(key) = parse_public_key(&file.key) else {
        return SigStatus::Invalid("bad public key".into());
    };
    let Some(sig) = unhex(&file.signature).and_then(|b| <[u8; 64]>::try_from(b).ok()) else {
        return SigStatus::Invalid("a signature is 128 hex digits".into());
    };
    let vk = VerifyingKey::from_bytes(&key).expect("checked");
    // Strict: no malleable signatures, no weak keys.
    if vk
        .verify_strict(message, &Signature::from_bytes(&sig))
        .is_err()
    {
        return SigStatus::Invalid("the signature does not match the contents".into());
    }
    match trusted.iter().find(|t| t.key == key) {
        Some(t) => SigStatus::Trusted {
            signer: t.name.clone(),
        },
        None => SigStatus::UnknownKey {
            fingerprint: fingerprint(&key),
            claimed: file.signer.chars().take(80).collect(),
            key: hex(&key),
        },
    }
}

/// Adds (or replaces) `signature.json` in a package's files.
pub(crate) fn sign_package(files: &mut BTreeMap<String, Vec<u8>>, key: &SigningKey, signer: &str) {
    files.remove(SIGNATURE_FILE);
    let sig = sign(&package_message(files), key, signer);
    files.insert(SIGNATURE_FILE.to_owned(), sig.into_bytes());
}

/// Checks a package's `signature.json` (when it has one).
pub(crate) fn verify_package(
    files: &BTreeMap<String, Vec<u8>>,
    trusted: &[TrustedKey],
) -> SigStatus {
    match files.get(SIGNATURE_FILE) {
        None => SigStatus::Unsigned,
        Some(sig) => verify(&package_message(files), sig, trusted),
    }
}

/// The detached `.sig` file for a catalog's bytes.
pub(crate) fn sign_catalog(bytes: &[u8], key: &SigningKey, signer: &str) -> String {
    sign(&catalog_message(bytes), key, signer)
}

/// Checks a catalog against its `.sig` (`None`: there was none).
pub(crate) fn verify_catalog(
    bytes: &[u8],
    sig: Option<&[u8]>,
    trusted: &[TrustedKey],
) -> SigStatus {
    match sig {
        None => SigStatus::Unsigned,
        Some(sig) => verify(&catalog_message(bytes), sig, trusted),
    }
}

/// A signing key from a key file (64 hex digits: the private seed).
pub(crate) fn read_signing_key(text: &str) -> Result<SigningKey, String> {
    let seed: [u8; 32] = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .and_then(unhex)
        .and_then(|b| b.try_into().ok())
        .ok_or("a key file holds the private key as 64 hex digits")?;
    Ok(SigningKey::from_bytes(&seed))
}

/// A new random signing key and its key file text.
pub(crate) fn generate_key() -> Result<(SigningKey, String), String> {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| format!("no random numbers: {e}"))?;
    let key = SigningKey::from_bytes(&seed);
    let text = format!(
        "# Meshtard extension signing key (PRIVATE: keep it secret, never commit it).\n\
         # Public key: {}\n{}\n",
        hex(key.verifying_key().as_bytes()),
        hex(&seed)
    );
    Ok((key, text))
}

/// `meshtard --extension-keygen <key file>`, `--extension-sign <key file> <package
/// .mtx or folder> [signer]` and `--extension-sign-catalog <key file> <catalog.json>
/// [signer]`: tools for extension authors and the Warehouse maintainers. `None` when
/// `args` is not one of these.
pub(crate) fn cli(args: &[String]) -> Option<Result<String, String>> {
    let cmd = args.first()?.as_str();
    if !matches!(
        cmd,
        "--extension-keygen" | "--extension-sign" | "--extension-sign-catalog"
    ) {
        return None;
    }
    Some(run_cli(cmd, &args[1..]))
}

fn run_cli(cmd: &str, args: &[String]) -> Result<String, String> {
    let usage = "usage: meshtard --extension-keygen <key file>\n       \
                 meshtard --extension-sign <key file> <package.mtx | folder> [signer]\n       \
                 meshtard --extension-sign-catalog <key file> <catalog.json> [signer]";
    let read_key = |p: &str| -> Result<SigningKey, String> {
        read_signing_key(&std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?)
    };
    match (cmd, args) {
        ("--extension-keygen", [path]) => {
            let p = std::path::Path::new(path);
            if p.exists() {
                return Err(format!("{path} exists; not overwritten"));
            }
            let (key, text) = generate_key()?;
            write_private(p, &text)?;
            Ok(format!(
                "Wrote the private key to {path}.\nPublic key (add it to the trusted keys): {}\nFingerprint: {}",
                hex(key.verifying_key().as_bytes()),
                fingerprint(key.verifying_key().as_bytes())
            ))
        }
        ("--extension-sign", [key, target, rest @ ..]) if rest.len() <= 1 => {
            let key = read_key(key)?;
            let signer = rest.first().map_or("", String::as_str);
            let path = std::path::Path::new(target);
            if path.is_dir() {
                let mut pkg = super::package::Package::from_folder(path)?;
                sign_package(&mut pkg.files, &key, signer);
                std::fs::write(path.join(SIGNATURE_FILE), &pkg.files[SIGNATURE_FILE])
                    .map_err(|e| format!("{target}: {e}"))?;
            } else {
                let bytes = std::fs::read(path).map_err(|e| format!("{target}: {e}"))?;
                let mut pkg = super::package::Package::from_zip(&bytes)?;
                sign_package(&mut pkg.files, &key, signer);
                let out = super::package::pack_files(&pkg.files)?;
                std::fs::write(path, out).map_err(|e| format!("{target}: {e}"))?;
            }
            Ok(format!(
                "Signed {target} with key {}.",
                fingerprint(key.verifying_key().as_bytes())
            ))
        }
        ("--extension-sign-catalog", [key, target, rest @ ..]) if rest.len() <= 1 => {
            let key = read_key(key)?;
            let signer = rest.first().map_or("", String::as_str);
            let bytes = std::fs::read(target).map_err(|e| format!("{target}: {e}"))?;
            let sig_path = format!("{target}.sig");
            std::fs::write(&sig_path, sign_catalog(&bytes, &key, signer))
                .map_err(|e| format!("{sig_path}: {e}"))?;
            Ok(format!("Wrote {sig_path}."))
        }
        _ => Err(usage.into()),
    }
}

/// Writes a private key file readable by its owner only (Unix).
fn write_private(path: &std::path::Path, text: &str) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(err)?;
        f.write_all(text.as_bytes()).map_err(err)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, text).map_err(err)
    }
}

/// TEST ONLY: a fixed key for the unit tests. Its private half is derived from a
/// public string, so anyone can sign with it: it must never be trusted outside
/// tests (it is not in [`BUILTIN_KEYS`]).
#[cfg(test)]
pub(crate) fn test_key() -> SigningKey {
    use sha2::{Digest, Sha256};
    let seed: [u8; 32] = Sha256::digest(b"meshtard TEST ONLY extension signing key").into();
    SigningKey::from_bytes(&seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> BTreeMap<String, Vec<u8>> {
        let mut f = BTreeMap::new();
        f.insert("extension.toml".to_owned(), b"id = \"a.b\"".to_vec());
        f.insert("main.rhai".to_owned(), b"let x = 1;".to_vec());
        f
    }

    fn trusted() -> Vec<TrustedKey> {
        vec![TrustedKey {
            name: "Test signer".into(),
            key: test_key().verifying_key().to_bytes(),
            builtin: false,
        }]
    }

    #[test]
    fn signed_packages_verify_and_detect_changes() {
        let mut f = files();
        assert_eq!(verify_package(&f, &trusted()), SigStatus::Unsigned);
        sign_package(&mut f, &test_key(), "Tester");
        assert_eq!(
            verify_package(&f, &trusted()),
            SigStatus::Trusted {
                signer: "Test signer".into()
            }
        );
        // Not trusted: valid, but shown as an unknown key.
        assert!(matches!(
            verify_package(&f, &[]),
            SigStatus::UnknownKey { ref claimed, .. } if claimed == "Tester"
        ));
        // A changed, added or removed file breaks it.
        let mut changed = f.clone();
        changed.insert("main.rhai".into(), b"let x = 2;".to_vec());
        assert!(matches!(
            verify_package(&changed, &trusted()),
            SigStatus::Invalid(_)
        ));
        let mut added = f.clone();
        added.insert("extra.rhai".into(), b"1".to_vec());
        assert!(matches!(
            verify_package(&added, &trusted()),
            SigStatus::Invalid(_)
        ));
        let mut removed = f.clone();
        removed.remove("main.rhai");
        assert!(matches!(
            verify_package(&removed, &trusted()),
            SigStatus::Invalid(_)
        ));
        // A swapped key (someone else re-signing with the trusted key's name) does
        // not borrow the trust.
        let other = SigningKey::from_bytes(&[7; 32]);
        let mut resigned = changed.clone();
        sign_package(&mut resigned, &other, "Test signer");
        assert!(!verify_package(&resigned, &trusted()).trusted());
        // Garbage.
        let mut junk = f.clone();
        junk.insert(SIGNATURE_FILE.into(), b"{".to_vec());
        assert!(matches!(
            verify_package(&junk, &trusted()),
            SigStatus::Invalid(_)
        ));
    }

    #[test]
    fn signed_catalogs() {
        let cat = br#"{"format": 1, "extensions": []}"#;
        let sig = sign_catalog(cat, &test_key(), "");
        assert!(verify_catalog(cat, Some(sig.as_bytes()), &trusted()).trusted());
        assert_eq!(verify_catalog(cat, None, &trusted()), SigStatus::Unsigned);
        let mut bad = cat.to_vec();
        bad[3] = b'X';
        assert!(matches!(
            verify_catalog(&bad, Some(sig.as_bytes()), &trusted()),
            SigStatus::Invalid(_)
        ));
        // A package signature does not verify as a catalog signature (domains).
        let mut f = files();
        sign_package(&mut f, &test_key(), "");
        let msg = package_message(&f);
        assert!(matches!(
            verify_catalog(&msg, Some(&f[SIGNATURE_FILE]), &trusted()),
            SigStatus::Invalid(_)
        ));
    }

    #[test]
    fn keys() {
        let pk = public_key_hex(&test_key().verifying_key().to_bytes());
        assert_eq!(
            parse_public_key(&pk).unwrap(),
            test_key().verifying_key().to_bytes()
        );
        assert!(parse_public_key("abc").is_err());
        assert!(parse_public_key(&"zz".repeat(32)).is_err());
        let fp = fingerprint(&test_key().verifying_key().to_bytes());
        assert_eq!(fp.len(), 19);
        let mut user = BTreeMap::new();
        user.insert("Me".to_owned(), pk);
        user.insert("Broken".to_owned(), "123".to_owned());
        let t = trusted_keys(&user);
        assert_eq!(t.len(), BUILTIN_KEYS.len() + 1);
        assert!(t.iter().any(|k| k.name == "Me" && !k.builtin));
        // Key files round-trip.
        let (k, text) = generate_key().unwrap();
        assert_eq!(read_signing_key(&text).unwrap().to_bytes(), k.to_bytes());
        assert!(read_signing_key("# nothing").is_err());
        // The test key is never built in.
        let test_pk = test_key().verifying_key().to_bytes();
        assert!(
            trusted_keys(&BTreeMap::new())
                .iter()
                .all(|k| k.key != test_pk)
        );
    }

    #[test]
    fn cli_signs_packages_and_catalogs() {
        let dir = super::super::package::tests::scratch("cli_sign");
        let key = dir.join("key.txt");
        let k = key.display().to_string();
        let out = cli(&["--extension-keygen".into(), k.clone()])
            .unwrap()
            .unwrap();
        assert!(out.contains("Public key"));
        // Never overwrites a key.
        assert!(
            cli(&["--extension-keygen".into(), k.clone()])
                .unwrap()
                .is_err()
        );
        let signing = read_signing_key(&std::fs::read_to_string(&key).unwrap()).unwrap();
        let mine = vec![TrustedKey {
            name: "Mine".into(),
            key: signing.verifying_key().to_bytes(),
            builtin: false,
        }];
        let src = dir.join("pkg");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("extension.toml"),
            "id = \"a.bc\"\nname = \"A\"\nversion = \"1.0\"\n",
        )
        .unwrap();
        std::fs::write(src.join("main.rhai"), "1").unwrap();
        let mtx = dir.join("a.mtx");
        std::fs::write(&mtx, super::super::package::pack(&src).unwrap()).unwrap();
        let m = mtx.display().to_string();
        cli(&["--extension-sign".into(), k.clone(), m, "Me".into()])
            .unwrap()
            .unwrap();
        let pkg = super::super::package::Package::read(&mtx).unwrap();
        assert!(verify_package(&pkg.files, &mine).trusted());
        // A folder gets its signature.json.
        cli(&[
            "--extension-sign".into(),
            k.clone(),
            src.display().to_string(),
        ])
        .unwrap()
        .unwrap();
        let pkg = super::super::package::Package::read(&src).unwrap();
        assert!(verify_package(&pkg.files, &mine).trusted());
        let cat = dir.join("catalog.json");
        std::fs::write(&cat, "{}").unwrap();
        cli(&[
            "--extension-sign-catalog".into(),
            k,
            cat.display().to_string(),
        ])
        .unwrap()
        .unwrap();
        let sig = std::fs::read(dir.join("catalog.json.sig")).unwrap();
        assert!(verify_catalog(b"{}", Some(&sig), &mine).trusted());
        assert!(cli(&["--version".into()]).is_none());
        assert!(cli(&["--extension-sign".into()]).unwrap().is_err());
    }
}
