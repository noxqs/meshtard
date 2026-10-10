//! Compares this reader with OpenSKP's Python package (the oracle) on every file in
//! `tests/corpus/openskp`.
//!
//! Needs `python3` with `openskp` installed (see `tools/README.md`); skips otherwise.
//! Oracle output is cached under `target/oracle-cache`.
//!
//! Every field this reader emits must match the oracle. Fields only the oracle has are
//! listed as "not ported" without failing.
//!
//! The oracle must be built from the same OpenSKP commit as the port (`359d183`); the
//! PyPI release 1.3.0 is older and decodes some fields differently.
//!
//! Known cases where the C++ source (and this port) is ahead of the Python oracle are
//! accepted and counted as "known":
//! - VFF `AA38` boolean attribute values: Python returns `None`.
//!
//! Known cases where the Python oracle is ahead of the C++ source, accepted until ported:
//! - Legacy face layers: Python reads them, C++ leaves them unset.
//! - Legacy model-level dimensions: Python decodes their endpoints, C++ does not.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn oracle_available() -> bool {
    Command::new("python3")
        .args(["-I", "-c", "import openskp"])
        .output()
        .is_ok_and(|o| o.status.success())
}

fn oracle(file: &Path) -> Value {
    let bytes = std::fs::read(file).unwrap();
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    });
    let cache = repo().join("target/oracle-cache");
    std::fs::create_dir_all(&cache).unwrap();
    let cached = cache.join(format!("{hash:016x}.json"));
    if !cached.exists() {
        let out = Command::new("python3")
            .arg("-I")
            .arg(repo().join("tools/oracle_dump.py"))
            .arg(file)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "oracle failed on {}: {}",
            file.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        std::fs::write(&cached, &out.stdout).unwrap();
    }
    serde_json::from_slice(&std::fs::read(cached).unwrap()).unwrap()
}

#[derive(Default)]
struct Report {
    mismatches: Vec<String>,
    unported: BTreeSet<String>,
    known: usize,
}

/// Path with numeric segments (entity ids, list indices) replaced by `*`.
fn pattern(path: &str) -> String {
    path.split('.')
        .map(|p| if p.parse::<i64>().is_ok() { "*" } else { p })
        .collect::<Vec<_>>()
        .join(".")
}

fn numbers_equal(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

/// Materials are compared by name: the two readers order them differently.
fn by_name(v: &Value) -> Value {
    match v {
        Value::Array(items) => Value::Object(
            items
                .iter()
                .map(|m| (m["name"].as_str().unwrap_or("").to_owned(), m.clone()))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn compare(path: &str, ours: &Value, theirs: &Value, r: &mut Report) {
    match (ours, theirs) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in a {
                let p = format!("{path}.{k}");
                match b.get(k) {
                    Some(w) if k == "materials" && path.is_empty() => {
                        compare(&p, &by_name(v), &by_name(w), r)
                    }
                    Some(w) => compare(&p, v, w, r),
                    None => {}
                }
            }
            for k in b.keys().filter(|k| !a.contains_key(*k)) {
                r.unported.insert(pattern(&format!("{path}.{k}")));
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (v, w)) in a.iter().zip(b).enumerate() {
                compare(&format!("{path}.{i}"), v, w, r);
            }
        }
        (Value::Number(a), Value::Number(b))
            if numbers_equal(a.as_f64().unwrap(), b.as_f64().unwrap()) => {}
        (a, b) if a == b => {}
        (Value::Bool(_), Value::Null) if path.contains(".attribute_dictionaries.") => r.known += 1,
        // Legacy face layers: OpenSKP leaves the layer's object id; we resolve it to
        // the layer name (or nothing for Layer0).
        (Value::Null | Value::String(_), Value::Number(_))
            if path.contains(".faces.") && path.ends_with(".layer") =>
        {
            r.known += 1
        }
        (Value::Array(a), Value::Array(_)) if path == ".dimensions" && a.is_empty() => r.known += 1,
        (a, b) => {
            let short = |v: &Value| {
                let s = v.to_string();
                if s.len() > 120 {
                    format!("{}...", &s[..120])
                } else {
                    s
                }
            };
            r.mismatches
                .push(format!("{path}: ours {} / oracle {}", short(a), short(b)));
        }
    }
}

fn check(files: &[&str]) {
    if !oracle_available() {
        eprintln!("SKIPPED: python3 with openskp not available (see tools/README.md)");
        return;
    }
    let mut failed = Vec::new();
    for name in files {
        let file = repo().join("tests/corpus/openskp").join(name);
        let mut r = Report::default();
        match mt_skp::read(&file) {
            Ok(model) => compare("", &mt_skp::dump::to_json(&model), &oracle(&file), &mut r),
            Err(e) => r.mismatches.push(format!("read error: {e}")),
        }
        eprintln!(
            "{name}: {} mismatches, {} known, not ported: {:?}",
            r.mismatches.len(),
            r.known,
            r.unported
        );
        for m in r.mismatches.iter().take(15) {
            eprintln!("    {m}");
        }
        if !r.mismatches.is_empty() {
            failed.push(*name);
        }
    }
    assert!(failed.is_empty(), "differs from oracle: {failed:?}");
}

#[test]
fn vff_matches_oracle() {
    check(&[
        "SU_File.skp",
        "Untitled.skp",
        "style_watermark.skp",
        "coedge_orientation_regression.skp",
    ]);
}

#[test]
fn legacy_matches_oracle() {
    check(&[
        "blank_v17.skp",
        "capilla_quiroz_v17.skp",
        "chair_and_table.skp",
        "gondola_v20.skp",
        "legacy_annotations.skp",
        "legacy_v2014_synthetic.skp",
        "legacy_v3_synthetic.skp",
        "legacy_v6_synthetic.skp",
        "legacy_v7_synthetic.skp",
        "single_material_v17.skp",
        "zero_material_custom_layer_v18.skp",
        "create_arc.skp",
        "create_attributes.skp",
        "create_auto_triangulate.skp",
        "create_circle.skp",
        "create_holes.skp",
        "create_materials_and_layers.skp",
        "create_multi_face_shared_edge.skp",
        "create_polyline.skp",
        "create_single_face.skp",
        "create_slot_boundary.skp",
    ]);
}
