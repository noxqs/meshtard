//! Reading and writing `.skp` files gives exactly what it gave before the reader and
//! writer were optimized (spec/PERFORMANCE.md): for every corpus file, a checksum of
//! the parsed file (`mt_skp::read`), of the model built from it (`import::from_skp`)
//! and of the bytes it saves to (deterministic GUIDs and clock), plus the same for a
//! synthetic grid big enough to need the archive's long slot references.
//!
//! The expected values are in `tests/data/skp_checksums.txt`. After a change that is
//! meant to alter the output, regenerate them with `MESHTARD_BLESS=1 cargo test -p
//! mt-kernel --test skp_checksums` and review the diff.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use glam::DVec3;
use mt_kernel::{CoEdge, Entities, FaceAttrs, Loop, Model, VertexId, export, import};
use mt_skp::write::SkpBuilder;

/// FNV-1a over bytes.
fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// The three checksums of one file's bytes: parsed, built, saved again.
fn checksums(name: &str, bytes: &[u8], out: &mut String) {
    let src = mt_skp::parse(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
    let parsed = fnv(format!("{src:?}").as_bytes());
    let model = import::from_skp(&src);
    let built = fnv(format!("{model:?}").as_bytes());
    let mut b = SkpBuilder::deterministic(7, 1_700_000_000);
    let saved = match export::write_model(&mut b, &model).and_then(|()| b.to_bytes()) {
        Ok(bytes) => format!("{:016x}", fnv(&bytes)),
        Err(e) => format!("error {e:?}"),
    };
    writeln!(out, "{name} {parsed:016x} {built:016x} {saved}").unwrap();
}

/// A face from vertex ids (edges reused when present), normal by Newell's method.
fn polygon(ents: &mut Entities, vs: &[VertexId]) {
    let mut coedges = Vec::new();
    let mut normal = DVec3::ZERO;
    for k in 0..vs.len() {
        let (a, b) = (vs[k], vs[(k + 1) % vs.len()]);
        let (pa, pb) = (ents.pos(a), ents.pos(b));
        normal += DVec3::new(
            (pa.y - pb.y) * (pa.z + pb.z),
            (pa.z - pb.z) * (pa.x + pb.x),
            (pa.x - pb.x) * (pa.y + pb.y),
        );
        let (edge, reversed) = match ents.edge_between(a, b) {
            Some(e) => (e, ents.edges[e].start != a),
            None => (ents.add_raw_edge(a, b, Default::default()), false),
        };
        coedges.push(CoEdge { edge, reversed });
    }
    ents.add_raw_face(
        Loop { coedges },
        Vec::new(),
        normal.normalize(),
        FaceAttrs::default(),
    );
}

/// A waved `n` x `n` grid of triangles (as the large-model benchmark builds).
fn grid(n: usize) -> Model {
    let mut model = Model::default();
    let ents = &mut model.root;
    let mut v = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            let z = ((i as f64) * 0.3).sin() * ((j as f64) * 0.2).cos() * 2.0;
            v.push(ents.add_vertex(DVec3::new(i as f64 * 10.0, j as f64 * 10.0, z)));
        }
    }
    let at = |i: usize, j: usize| v[j * (n + 1) + i];
    for j in 0..n {
        for i in 0..n {
            polygon(ents, &[at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
            polygon(ents, &[at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
        }
    }
    model
}

#[test]
fn skp_read_and_write_are_unchanged() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = manifest.join("../../tests/corpus/openskp");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "skp"))
        .collect();
    files.sort();
    let mut got = String::new();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy();
        checksums(&name, &std::fs::read(path).unwrap(), &mut got);
    }
    let mut b = SkpBuilder::deterministic(7, 1_700_000_000);
    export::write_model(&mut b, &grid(90)).unwrap();
    checksums("grid 90", &b.to_bytes().unwrap(), &mut got);

    let expected_path = manifest.join("tests/data/skp_checksums.txt");
    if std::env::var_os("MESHTARD_BLESS").is_some() {
        std::fs::create_dir_all(expected_path.parent().unwrap()).unwrap();
        std::fs::write(&expected_path, &got).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&expected_path).unwrap();
    for (g, e) in got.lines().zip(expected.lines()) {
        assert_eq!(g, e, "file, parsed, built, saved");
    }
    assert_eq!(got.lines().count(), expected.lines().count());
}
