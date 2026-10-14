//! Mutation fuzzing of the file readers that live in the app: `.style` files (zip
//! archives and meshtard's text format) and `.mtx` extension packages, with the
//! mutator of `mt-fuzz` (see `spec/FUZZING.md`). A short run is part of the tests;
//! `cargo test -p mt-app --release fuzz_app_readers_long -- --ignored` runs
//! `FUZZ_ITERS` (default 20 000) inputs per reader. Each input must give a result or
//! an error without a panic within a second.

use std::io::{Cursor, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

const STYLE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="no" ?>
<styleDocument xmlns:sty="http://sketchup.google.com/schemas/sketchup/1.0/style">
  <sty:style xmlns:t="http://sketchup.google.com/schemas/1.0/types" name="[Fuzz]" desc="d">
    <sty:item id="1000"><t:variant type="4">1</t:variant></sty:item>
    <sty:item id="4000"><t:variant type="5">-16776961</t:variant></sty:item>
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

fn style_seeds() -> Vec<Vec<u8>> {
    let style = mt_kernel::model_data::NamedStyle {
        name: "Fuzz".into(),
        description: "d".into(),
        settings: Default::default(),
    };
    vec![
        archive(&[
            ("style/style.xml", STYLE_XML.as_bytes()),
            ("style/paper.png", b"P"),
            ("ref/logo.png", b"L"),
            ("doc_thumbnail.png", b"T"),
        ]),
        crate::tray_extra::style_file_text(&style).into_bytes(),
        crate::style_zip::write(&style, Some(b"T")).unwrap(),
    ]
}

fn skm_seeds() -> Vec<Vec<u8>> {
    let mut png = Vec::new();
    image::RgbImage::from_pixel(2, 2, image::Rgb([9, 8, 7]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let textured = mt_kernel::Material {
        name: "Fuzz".into(),
        color: [1, 2, 3],
        opacity: 0.5,
        texture: Some(mt_kernel::texture::Texture {
            filename: "f.png".into(),
            data: std::sync::Arc::from(png.as_slice()),
            width: 10.0,
            height: 5.0,
            colorize: true,
        }),
    };
    let plain = mt_kernel::Material {
        texture: None,
        ..textured.clone()
    };
    vec![
        crate::skm::write(&textured).unwrap(),
        crate::skm::write(&plain).unwrap(),
    ]
}

fn mtx_seeds() -> Vec<Vec<u8>> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/dist");
    let mut out: Vec<Vec<u8>> = std::fs::read_dir(dir)
        .map(|r| {
            r.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|x| x == "mtx"))
                .filter_map(|e| std::fs::read(e.path()).ok())
                .collect()
        })
        .unwrap_or_default();
    out.push(archive(&[
        (
            "extension.toml",
            b"id = \"a.b\"\nname = \"A\"\nversion = \"1.0.0\"\nscripts = [\"main.rhai\"]\n",
        ),
        ("main.rhai", b"let x = 1;"),
    ]));
    out
}

fn fuzz(name: &str, seeds: &[Vec<u8>], text: bool, iters: usize, run: impl Fn(&[u8])) {
    let mut m = mt_fuzz::Mutator {
        rng: mt_fuzz::Rng::new(0x5eed ^ name.len() as u64),
        pool: seeds,
        text,
        zip: true,
    };
    for i in 0..iters + seeds.len() {
        let data = match seeds.get(i) {
            Some(s) => s.clone(),
            None => {
                let k = m.rng.below(seeds.len());
                m.mutate(&seeds[k])
            }
        };
        let start = Instant::now();
        let r = catch_unwind(AssertUnwindSafe(|| run(&data)));
        if r.is_err() || start.elapsed() > Duration::from_secs(1) {
            let path = std::env::temp_dir().join(format!("meshtard-fuzz-{name}-{i}.bin"));
            let _ = std::fs::write(&path, &data);
            panic!(
                "{name}: input {i} {} (saved to {})",
                if r.is_err() { "panics" } else { "is slow" },
                path.display()
            );
        }
    }
}

fn run_all(iters: usize) {
    fuzz("style", &style_seeds(), true, iters, |d| {
        let _ = crate::tray_extra::read_style_bytes(d);
        if crate::style_zip::is_zip(d) {
            let _ = crate::style_zip::read(d);
        }
    });
    fuzz("skm", &skm_seeds(), true, iters, |d| {
        let _ = crate::skm::read(d);
    });
    fuzz("mtx", &mtx_seeds(), true, iters, |d| {
        let _ = crate::extensions::package::Package::from_zip_unguarded(d);
    });
}

#[test]
fn fuzz_app_readers_short() {
    run_all(300);
}

#[test]
#[ignore = "long fuzzing run: cargo test -p mt-app --release fuzz_app_readers_long -- --ignored"]
fn fuzz_app_readers_long() {
    let iters = std::env::var("FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    run_all(iters);
}
