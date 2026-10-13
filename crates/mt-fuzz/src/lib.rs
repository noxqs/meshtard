//! A mutation fuzzer for meshtard's file readers that runs on stable Rust (the
//! workspace pins a stable toolchain, so `cargo fuzz` / libFuzzer is not used).
//! See `spec/FUZZING.md`.
//!
//! - [`targets`]: one entry per reader (the `.skp` containers, IFC / STEP, DXF, glTF,
//!   the mesh formats, texture images), each running the reader and the conversion
//!   the app does next, discarding the result: an `Err` is a pass, a panic, hang or
//!   oversized allocation a failure.
//! - [`seeds`]: valid files per target (the OpenSKP corpus and files exported from
//!   it with meshtard's own writers).
//! - [`Mutator`]: bit flips, interesting bytes and integers (length-field
//!   inflation), insertion, deletion, duplication, truncation, splicing between
//!   seeds, number replacement in text formats and re-zipped entry mutation for
//!   zip containers.
//!
//! The `fuzz_readers` binary drives it with a counting allocator ([`alloc`]) and a
//! watchdog; `tests/regressions.rs` replays the minimized crashing inputs under
//! `regressions/`.

pub mod alloc;

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use mt_kernel::formats::{self, Kind};
use mt_kernel::mesh_import::MeshOptions;

/// A reader under test.
pub struct Target {
    pub name: &'static str,
    /// Runs the reader over a file's bytes: whether it accepted them (read without
    /// an error), which tells how deep mutated inputs get.
    pub run: fn(&[u8]) -> bool,
    /// Text format: mutate numbers and lines too.
    pub text: bool,
    /// A zip archive (after a prefix): mutate its entries and re-zip too.
    pub zip: bool,
}

fn no_file(_: &str) -> Option<Vec<u8>> {
    None
}

fn skp(d: &[u8]) -> bool {
    let _ = mt_skp::version(d);
    mt_skp::parse(d)
        .map(|m| mt_kernel::import::from_skp(&m))
        .is_ok()
}

fn step(d: &[u8]) -> bool {
    formats::step::parse(d).is_ok()
}

fn ifc(d: &[u8]) -> bool {
    formats::ifc::import(d, &Default::default()).is_ok()
}

fn dxf(d: &[u8]) -> bool {
    mt_kernel::dxf::parse(d)
        .map(|drawing| mt_kernel::dxf::to_model(&drawing, &Default::default()))
        .is_ok()
}

fn gltf(d: &[u8]) -> bool {
    mt_kernel::gltf_io::import(d, &no_file, true).is_ok()
}

/// A mesh file read and built into faces as File > Import does.
fn mesh(kind: Kind, d: &[u8]) -> bool {
    let Ok((scene, _)) = formats::read_scene(kind, d, &no_file, true) else {
        return false;
    };
    let mut model = mt_kernel::Model::default();
    let ids: Vec<usize> = (0..scene.mesh.materials.len()).collect();
    let _ = formats::scene::to_entities(
        &mut model,
        &scene,
        &MeshOptions {
            inches_per_unit: 1.0,
            y_up: kind == Kind::Obj,
            merge_coplanar: true,
        },
        &ids,
    );
    true
}

fn image(d: &[u8]) -> bool {
    mt_kernel::images::decode(d).map(|i| i.to_rgba8()).is_ok()
}

pub fn targets() -> Vec<Target> {
    fn t(name: &'static str, run: fn(&[u8]) -> bool, text: bool, zip: bool) -> Target {
        Target {
            name,
            run,
            text,
            zip,
        }
    }
    vec![
        t("skp_legacy", skp, false, false),
        t("skp_vff", skp, false, true),
        t("step", step, true, false),
        t("ifc", ifc, true, false),
        t("dxf", dxf, true, false),
        t("gltf", gltf, true, false),
        t("glb", gltf, false, false),
        t("obj", |d| mesh(Kind::Obj, d), true, false),
        t("stl", |d| mesh(Kind::Stl, d), false, false),
        t("stl_ascii", |d| mesh(Kind::Stl, d), true, false),
        t("fbx", |d| mesh(Kind::Fbx, d), false, false),
        t("ply", |d| mesh(Kind::Ply, d), false, false),
        t("ply_ascii", |d| mesh(Kind::Ply, d), true, false),
        t("3mf", |d| mesh(Kind::ThreeMf, d), false, true),
        t("3ds", |d| mesh(Kind::ThreeDs, d), false, false),
        t("png", image, false, false),
        t("jpeg", image, false, false),
    ]
}

pub fn target(name: &str) -> Option<Target> {
    targets().into_iter().find(|t| t.name == name)
}

/// The repository's root folder.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Corpus `.skp` files up to this size are seeds (bigger ones make runs slow).
const MAX_SEED_SKP: u64 = 250_000;

/// Valid files per target name.
pub fn seeds() -> BTreeMap<&'static str, Vec<Vec<u8>>> {
    let mut out: BTreeMap<&'static str, Vec<Vec<u8>>> = BTreeMap::new();
    let corpus = repo_root().join("tests/corpus/openskp");
    let mut models = Vec::new();
    let mut names: Vec<PathBuf> = std::fs::read_dir(&corpus)
        .map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    names.sort();
    for p in names {
        if p.extension().is_none_or(|e| e != "skp")
            || std::fs::metadata(&p).map_or(true, |m| m.len() > MAX_SEED_SKP)
        {
            continue;
        }
        let Ok(bytes) = std::fs::read(&p) else {
            continue;
        };
        let vff = bytes.windows(9).any(|w| w == b"model.dat");
        if let Ok(m) = mt_skp::parse(&bytes) {
            let small = bytes.len() < 40_000;
            if small {
                models.push(mt_kernel::import::from_skp(&m));
            }
        }
        out.entry(if vff { "skp_vff" } else { "skp_legacy" })
            .or_default()
            .push(bytes);
    }
    // Files written by meshtard's exporters from small corpus models.
    let opts = formats::ExportOptions::default();
    for model in models.iter().filter(|m| !m.root.faces.is_empty()).take(4) {
        if let Ok(b) = mt_kernel::export::to_skp(model) {
            out.entry("skp_legacy").or_default().push(b);
        }
        let mut add = |name: &'static str, data: Vec<u8>| {
            if !data.is_empty() && data.len() < 300_000 {
                out.entry(name).or_default().push(data);
            }
        };
        let obj = mt_kernel::mesh_export::to_obj(model, &Default::default());
        add("obj", obj.obj.into_bytes());
        add(
            "stl",
            mt_kernel::mesh_export::to_stl(model, mt_kernel::mesh_export::Units::Inches),
        );
        add(
            "stl_ascii",
            mt_kernel::mesh_export::to_stl_ascii(
                model,
                mt_kernel::mesh_export::Units::Inches,
                "model",
            )
            .into_bytes(),
        );
        for (kind, name) in [
            (formats::ExportKind::Fbx, "fbx"),
            (formats::ExportKind::Ply, "ply"),
            (formats::ExportKind::ThreeMf, "3mf"),
            (formats::ExportKind::ThreeDs, "3ds"),
            (formats::ExportKind::Ifc, "ifc"),
        ] {
            if let Ok(f) = formats::export(kind, model, &opts) {
                add(name, f.main.clone());
                if name == "ifc" {
                    add("step", f.main);
                }
            }
        }
        let ascii = formats::ExportOptions {
            ascii: true,
            ..opts.clone()
        };
        if let Ok(f) = formats::export(formats::ExportKind::Ply, model, &ascii) {
            add("ply_ascii", f.main);
        }
        for binary in [true, false] {
            let opts = mt_kernel::gltf_io::GltfOptions {
                binary,
                two_sided: false,
            };
            let data = mt_kernel::gltf_io::to_gltf(model, &opts);
            add(if binary { "glb" } else { "gltf" }, data);
        }
    }
    // Hand-written files with what the exporters do not write (`seeds/<target>/`).
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("seeds");
    for t in targets() {
        let Ok(entries) = std::fs::read_dir(dir.join(t.name)) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        files.sort();
        for f in files {
            if let Ok(b) = std::fs::read(&f) {
                out.entry(t.name).or_default().push(b);
            }
        }
    }
    let img = image_seed();
    out.entry("png").or_default().push(img.0);
    out.entry("jpeg").or_default().push(img.1);
    out
}

/// A small PNG and JPEG.
fn image_seed() -> (Vec<u8>, Vec<u8>) {
    let img = image::RgbaImage::from_fn(13, 7, |x, y| {
        image::Rgba([(x * 19) as u8, (y * 31) as u8, ((x ^ y) * 7) as u8, 200])
    });
    let mut png = Vec::new();
    let _ = img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png);
    let mut jpeg = Vec::new();
    let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
    let _ = rgb.write_to(&mut Cursor::new(&mut jpeg), image::ImageFormat::Jpeg);
    (png, jpeg)
}

/// A small, fast pseudo-random generator (xorshift64*).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// `0..n` (0 when `n` is 0).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    pub fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
}

const INTERESTING_BYTES: [u8; 9] = [0, 1, 0x7f, 0x80, 0xff, 0xfe, 0x10, 0x40, 0x20];
const INTERESTING_INTS: [u64; 14] = [
    0,
    1,
    2,
    0x7f,
    0xff,
    0x7fff,
    0xffff,
    0x10000,
    0x00ff_ffff,
    0x1000_0000,
    0x7fff_ffff,
    0x8000_0000,
    0xffff_ffff,
    u64::MAX,
];
const INTERESTING_TEXT: [&str; 22] = [
    "0",
    "1",
    "2",
    "3",
    "-1",
    "-2147483649",
    "2147483648",
    "4294967296",
    "18446744073709551616",
    "99999999999999999999999",
    "1e308",
    "-1e308",
    "1e999",
    "NaN",
    "nan",
    "inf",
    "-inf",
    "1e-320",
    "0.0",
    "65535",
    "1000000",
    "",
];

/// Mutates files of one target.
pub struct Mutator<'a> {
    pub rng: Rng,
    /// Other inputs of the same format, for splicing.
    pub pool: &'a [Vec<u8>],
    pub text: bool,
    pub zip: bool,
}

impl Mutator<'_> {
    /// A mutated copy of `data`: one to eight mutations (zip containers sometimes get
    /// one entry mutated and the archive written again).
    pub fn mutate(&mut self, data: &[u8]) -> Vec<u8> {
        if self.zip
            && self.rng.chance(2)
            && let Some(out) = self.mutate_zip(data)
        {
            return out;
        }
        let mut d = data.to_vec();
        let many = self.rng.chance(3);
        let n = 1 + self.rng.below(if many { 8 } else { 2 });
        for _ in 0..n {
            self.once(&mut d);
        }
        d
    }

    fn pos(&mut self, d: &[u8]) -> usize {
        self.rng.below(d.len().max(1))
    }

    fn once(&mut self, d: &mut Vec<u8>) {
        let choices = if self.text { 14 } else { 11 };
        match self.rng.below(choices) {
            0 | 1 if !d.is_empty() => {
                let i = self.pos(d);
                d[i] ^= 1 << self.rng.below(8);
            }
            2 if !d.is_empty() => {
                let i = self.pos(d);
                d[i] = INTERESTING_BYTES[self.rng.below(INTERESTING_BYTES.len())];
            }
            3 | 4 => {
                // An integer field set to a boundary value: lengths, counts, offsets.
                let width = [1, 2, 4, 8][self.rng.below(4)];
                if d.len() < width {
                    return;
                }
                let i = self.rng.below(d.len() - width + 1);
                let mut v = INTERESTING_INTS[self.rng.below(INTERESTING_INTS.len())];
                if self.rng.chance(4) {
                    v = (d.len() as u64).wrapping_mul(1 + self.rng.below(16) as u64);
                }
                let bytes = if self.rng.chance(4) {
                    v.to_be_bytes()
                } else {
                    v.to_le_bytes()
                };
                let src = if self.rng.chance(4) {
                    &bytes[8 - width..]
                } else {
                    &bytes[..width]
                };
                d[i..i + width].copy_from_slice(src);
            }
            5 if d.len() >= 4 => {
                // Add to a 32-bit field.
                let i = self.rng.below(d.len() - 3);
                let mut b = [0; 4];
                b.copy_from_slice(&d[i..i + 4]);
                let delta = [1i32, -1, 2, -2, 16, -16, 256, 4096, 1 << 20][self.rng.below(9)];
                let v = i32::from_le_bytes(b).wrapping_add(delta);
                d[i..i + 4].copy_from_slice(&v.to_le_bytes());
            }
            6 if !d.is_empty() => {
                let i = self.pos(d);
                let max = if self.rng.chance(4) { 4096 } else { 16 };
                let len = 1 + self.rng.below((d.len() - i).min(max));
                d.drain(i..i + len);
            }
            7 => {
                let i = self.rng.below(d.len() + 1);
                let len = 1 + self.rng.below(16);
                let bytes: Vec<u8> = (0..len).map(|_| self.rng.next_u64() as u8).collect();
                d.splice(i..i, bytes);
            }
            8 if !d.is_empty() => {
                // Duplicate a chunk (repeated records, nesting).
                let i = self.pos(d);
                let len = 1 + self.rng.below((d.len() - i).min(256));
                let chunk = d[i..i + len].to_vec();
                let at = self.rng.below(d.len() + 1);
                let times = if self.rng.chance(8) { 16 } else { 1 };
                for _ in 0..times {
                    d.splice(at..at, chunk.iter().copied());
                }
            }
            9 if !d.is_empty() => {
                let keep = self.rng.below(d.len());
                d.truncate(keep);
            }
            10 if !self.pool.is_empty() => {
                // Splice: a chunk of another input over or into this one.
                let other = &self.pool[self.rng.below(self.pool.len())];
                if other.is_empty() {
                    return;
                }
                let i = self.rng.below(other.len());
                let len = 1 + self.rng.below((other.len() - i).min(512));
                let chunk = other[i..i + len].to_vec();
                let at = self.rng.below(d.len() + 1);
                let end = if self.rng.chance(2) {
                    (at + len).min(d.len())
                } else {
                    at
                };
                d.splice(at..end, chunk);
            }
            11 | 12 => self.replace_number(d),
            13 => self.line_op(d),
            _ => {}
        }
    }

    /// A number in a text file replaced by a boundary value.
    fn replace_number(&mut self, d: &mut Vec<u8>) {
        if d.is_empty() {
            return;
        }
        let start = self.pos(d);
        let Some(off) = d[start..].iter().position(u8::is_ascii_digit) else {
            return;
        };
        let mut a = start + off;
        while a > 0 && matches!(d[a - 1], b'0'..=b'9' | b'.' | b'-' | b'+') {
            a -= 1;
        }
        let mut b = start + off;
        while b < d.len() && matches!(d[b], b'0'..=b'9' | b'.' | b'e' | b'E' | b'-' | b'+') {
            b += 1;
        }
        let new = INTERESTING_TEXT[self.rng.below(INTERESTING_TEXT.len())];
        d.splice(a..b, new.bytes());
    }

    /// A line deleted, duplicated or moved.
    fn line_op(&mut self, d: &mut Vec<u8>) {
        let starts: Vec<usize> = std::iter::once(0)
            .chain(
                d.iter()
                    .enumerate()
                    .filter(|&(_, &c)| c == b'\n')
                    .map(|(i, _)| i + 1),
            )
            .filter(|&i| i < d.len())
            .collect();
        if starts.is_empty() {
            return;
        }
        let k = self.rng.below(starts.len());
        let a = starts[k];
        let b = starts.get(k + 1).copied().unwrap_or(d.len());
        let line = d[a..b].to_vec();
        match self.rng.below(3) {
            0 => {
                d.drain(a..b);
            }
            1 => {
                let at = starts[self.rng.below(starts.len())];
                d.splice(at..at, line);
            }
            _ => {
                d.drain(a..b);
                let at = self.rng.below(d.len() + 1);
                let at = d[..at]
                    .iter()
                    .rposition(|&c| c == b'\n')
                    .map_or(0, |i| i + 1);
                d.splice(at..at, line);
            }
        }
    }

    /// One entry of a zip archive (after any prefix, as in VFF `.skp` files) mutated,
    /// and the archive written again so it stays readable.
    fn mutate_zip(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        let at = data
            .windows(4)
            .take(4096)
            .position(|w| w == b"PK\x03\x04")?;
        let mut zip = zip::ZipArchive::new(Cursor::new(&data[at..])).ok()?;
        let mut entries = Vec::new();
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).ok()?;
            let name = f.name().ok()?.into_owned();
            let mut bytes = Vec::new();
            f.read_to_end(&mut bytes).ok()?;
            entries.push((name, bytes));
        }
        if entries.is_empty() {
            return None;
        }
        let k = self.rng.below(entries.len());
        let text = std::str::from_utf8(&entries[k].1).is_ok();
        let mut inner = Mutator {
            rng: Rng::new(self.rng.next_u64()),
            pool: &[],
            text,
            zip: false,
        };
        entries[k].1 = inner.mutate(&entries[k].1);
        let mut out = data[..at].to_vec();
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in &entries {
            w.start_file(name.as_str(), opts).ok()?;
            w.write_all(bytes).ok()?;
        }
        out.extend(w.finish().ok()?.into_inner());
        Some(out)
    }
}

/// The minimized crashing inputs kept as regression tests: `regressions/<target>/*`.
pub fn regression_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("regressions")
}
