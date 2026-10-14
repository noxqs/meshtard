//! SketchUp material files (`.skm`): Materials > Details > Open Material..., material
//! collections, and Save As... (Edit tab and the swatch's context menu).
//!
//! The layout was taken from real files (Google SketchUp 8's bundled materials, e.g.
//! `Wood_Cherry_Original.skm`, `Translucent_Glass_Blue.skm`, `Color_001.skm`): a zip
//! archive of `document.xml`, `documentProperties.xml`, `references.xml`,
//! `doc_thumbnail.png` (64 x 64) and, for textured materials, the image in `ref/`.
//! `document.xml` holds one element:
//!
//! ```xml
//! <mat:material colorBlue="42" colorGreen="86" colorRed="152" colorizeType="0"
//!     hasTexture="1" name="[Wood_Cherry_Original]" trans="0" type="1" useTrans="0">
//!   <mat:texture avgColor="2774680" textureFilename="Wood_Cherry_Original.jpg"
//!       xScale="72" yScale="72">
//!     <mat:images><mat:image id="1" path="Wood_Cherry_Original_1.jpg"/></mat:images>
//!   </mat:texture>
//! </mat:material>
//! ```
//!
//! Read as: `trans` is the opacity when `useTrans` is 1 (the glass has 0.5, plain
//! colors `trans="1" useTrans="0"`); `type` is the Ruby API's `materialType` (0 color,
//! 1 textured, 2 colorized textured; 2 sets our colorize flag); `xScale` / `yScale`
//! the tile size in inches; `avgColor` an ABGR integer with red in the low byte (the
//! cherry's 2774680 = 152, 86, 42). Unverified: the units of the scales, what
//! `colorizeType` means (written as 0), and whether newer SketchUp versions (PBR
//! materials since 2025) add elements or files; files from SketchUp 2026 were not
//! seen. Names in square brackets (bundled materials) lose them.

use std::sync::Arc;

use mt_kernel::Material;
use mt_kernel::texture::Texture;

use crate::style_zip::{attr, display_name, entries, escape, tags};

/// A material read from a `.skm` file and its thumbnail image (PNG) when it has one.
pub(crate) struct Skm {
    pub material: Material,
    pub thumbnail: Option<Vec<u8>>,
}

/// Reads a `.skm` file's bytes.
pub(crate) fn read(data: &[u8]) -> Result<Skm, String> {
    if !crate::style_zip::is_zip(data) {
        return Err("not a SketchUp material file".into());
    }
    let files = entries(data).map_err(|e| format!("not a material file: {e}"))?;
    let base = |n: &str| n.rsplit('/').next().unwrap_or(n).to_ascii_lowercase();
    let xml = files
        .iter()
        .find(|(n, _)| base(n) == "document.xml")
        .or_else(|| {
            files.iter().find(|(n, b)| {
                n.to_ascii_lowercase().ends_with(".xml")
                    && String::from_utf8_lossy(b).contains("material")
            })
        })
        .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
        .ok_or("no material found in the file")?;
    let mut it = tags(&xml);
    let head = loop {
        let Some((_, t)) = it.next() else {
            return Err("no material found in the file".into());
        };
        if !t.closing && t.name.eq_ignore_ascii_case("material") {
            break t.attrs.to_owned();
        }
    };
    let num = |a: &str, k: &str| attr(a, k).and_then(|v| v.trim().parse::<f64>().ok());
    let channel = |k: &str| num(&head, k).map_or(200, |v| v.clamp(0.0, 255.0).round() as u8);
    let mut material = Material {
        name: display_name(&attr(&head, "name").unwrap_or_default(), "Material"),
        color: [
            channel("colorRed"),
            channel("colorGreen"),
            channel("colorBlue"),
        ],
        opacity: 1.0,
        texture: None,
    };
    if num(&head, "useTrans").is_some_and(|v| v != 0.0)
        && let Some(t) = num(&head, "trans").filter(|t| t.is_finite())
    {
        material.opacity = t.clamp(0.0, 1.0);
    }
    let kind = num(&head, "type").unwrap_or(0.0);
    let mut texture: Option<(String, f64, f64)> = None;
    let mut image_path: Option<String> = None;
    for (_, t) in it {
        if t.closing && t.name.eq_ignore_ascii_case("material") {
            break;
        }
        if t.closing {
            continue;
        }
        if t.name.eq_ignore_ascii_case("texture") {
            let scale = |k: &str| {
                num(t.attrs, k)
                    .filter(|v| v.is_finite() && *v > 0.0)
                    .unwrap_or(0.0)
            };
            texture = Some((
                attr(t.attrs, "textureFilename").unwrap_or_default(),
                scale("xScale"),
                scale("yScale"),
            ));
        } else if t.name.eq_ignore_ascii_case("image") && image_path.is_none() {
            image_path = attr(t.attrs, "path");
        }
    }
    let has_texture = num(&head, "hasTexture").is_some_and(|v| v != 0.0) || texture.is_some();
    if has_texture {
        let (filename, w, h) = texture.unwrap_or_default();
        // The image: by its path (in `ref/` in real files), else by the texture's
        // file name, else any image of the archive but the thumbnail.
        let want: Vec<String> = [image_path.as_deref(), Some(filename.as_str())]
            .into_iter()
            .flatten()
            .map(|p| base(&p.replace('\\', "/")))
            .filter(|p| !p.is_empty())
            .collect();
        let is_image = |n: &str| {
            let n = n.to_ascii_lowercase();
            [".png", ".jpg", ".jpeg", ".bmp", ".tif", ".tiff"]
                .iter()
                .any(|e| n.ends_with(e))
        };
        let image = files
            .iter()
            .find(|(n, _)| want.contains(&base(n)))
            .or_else(|| {
                files
                    .iter()
                    .find(|(n, _)| is_image(n) && !base(n).contains("thumbnail"))
            });
        if let Some((n, data)) = image
            && mt_kernel::images::decode(data).is_ok()
        {
            let filename = if filename.is_empty() {
                base(n)
            } else {
                filename
            };
            let (w, h) = match (w > 0.0, h > 0.0) {
                (true, true) => (w, h),
                (true, false) => (w, w),
                (false, true) => (h, h),
                (false, false) => (0.0, 0.0),
            };
            material.texture = Some(Texture {
                filename,
                data: Arc::from(data.as_slice()),
                width: w,
                height: h,
                colorize: kind == 2.0,
            });
        }
    }
    let thumbnail = files
        .iter()
        .find(|(n, _)| base(n) == "doc_thumbnail.png")
        .or_else(|| files.iter().find(|(n, _)| base(n).contains("thumbnail")))
        .map(|(_, b)| b.clone());
    Ok(Skm {
        material,
        thumbnail,
    })
}

/// A 64 x 64 PNG thumbnail of `m`: its texture image scaled, or its color.
pub(crate) fn thumbnail(m: &Material) -> Vec<u8> {
    let img = m
        .texture
        .as_ref()
        .and_then(|t| mt_kernel::images::decode(&t.data).ok())
        .map(|i| {
            i.resize_exact(64, 64, image::imageops::FilterType::Triangle)
                .to_rgb8()
        })
        .unwrap_or_else(|| image::RgbImage::from_pixel(64, 64, image::Rgb(m.color)));
    let mut out = Vec::new();
    // Encoding a small in-memory image does not fail; an empty thumbnail if it did.
    let _ = img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png);
    out
}

/// `m` as a `.skm` file (the layout of the module docs).
pub(crate) fn write(m: &Material) -> Result<Vec<u8>, String> {
    let [r, g, b] = m.color;
    let opacity = m.opacity.clamp(0.0, 1.0);
    let use_trans = opacity < 1.0;
    let kind = match &m.texture {
        None => 0,
        Some(t) if t.colorize => 2,
        Some(_) => 1,
    };
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let body = match &m.texture {
        None => "/>".to_owned(),
        Some(t) => {
            let ext = crate::style_zip::image_ext(&t.data);
            let stem = std::path::Path::new(&t.filename.replace('\\', "/"))
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "texture".into());
            let stem: String = stem
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let image = format!("{stem}_1.{ext}");
            let texture_name = if t.filename.is_empty() {
                format!("{stem}.{ext}")
            } else {
                t.filename
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or("")
                    .to_owned()
            };
            let avg = u32::from_le_bytes([r, g, b, 0]);
            let size = |v: f64| if v > 0.0 { v } else { 1.0 };
            files.push((format!("ref/{image}"), t.data.to_vec()));
            format!(
                ">\n    <mat:texture avgColor=\"{avg}\" textureFilename=\"{}\" xScale=\"{}\" yScale=\"{}\">\n      <mat:images>\n        <mat:image id=\"1\" path=\"{}\"/>\n      </mat:images>\n    </mat:texture>\n  </mat:material>",
                escape(&texture_name),
                size(t.width),
                size(t.height),
                escape(&image),
            )
        }
    };
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="no" ?>
<materialDocument xmlns="http://www.sketchup.com/schemas/sketchup/1.0/material" xmlns:mat="http://www.sketchup.com/schemas/sketchup/1.0/material" xmlns:r="http://www.sketchup.com/schemas/1.0/references" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://www.sketchup.com/schemas/sketchup/1.0/material http://www.sketchup.com/schemas/sketchup/1.0/material.xsd">

  <mat:material colorBlue="{b}" colorGreen="{g}" colorRed="{r}" colorizeType="0" hasTexture="{}" name="{}" trans="{}" type="{kind}" useTrans="{}"{body}

</materialDocument>
"#,
        u8::from(m.texture.is_some()),
        escape(&m.name),
        if use_trans { opacity } else { 1.0 },
        u8::from(use_trans),
    );
    let mut all = vec![
        ("document.xml".to_owned(), xml.into_bytes()),
        (
            "documentProperties.xml".to_owned(),
            crate::style_zip::document_properties(&m.name, "", "Material").into_bytes(),
        ),
        ("doc_thumbnail.png".to_owned(), thumbnail(m)),
    ];
    all.push((
        "references.xml".to_owned(),
        crate::style_zip::REFERENCES.as_bytes().to_vec(),
    ));
    all.extend(files);
    crate::style_zip::zip_files(&all)
}

/// Whether `name` is a `.skm` file name.
pub(crate) fn is_skm(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("skm"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `document.xml` of Google SketchUp 8's `Wood_Cherry_Original.skm`
    /// (namespace declarations shortened).
    const CHERRY: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="no" ?>
<materialDocument xmlns="http://www.sketchup.com/schemas/sketchup/1.0/material" xmlns:mat="http://www.sketchup.com/schemas/sketchup/1.0/material">

  <mat:material colorBlue="42" colorGreen="86" colorRed="152" colorizeType="0" hasTexture="1" name="[Wood_Cherry_Original]" trans="0" type="1" useTrans="0">
    <mat:texture avgColor="2774680" textureFilename="Wood_Cherry_Original.jpg" xScale="72" yScale="72">
      <mat:images>
        <mat:image id="1" path="Wood_Cherry_Original_1.jpg"/>
      </mat:images>
    </mat:texture>
  </mat:material>

</materialDocument>
"#;

    /// `Translucent_Glass_Blue.skm`'s material element.
    const GLASS: &str = r#"<materialDocument><mat:material colorBlue="237" colorGreen="149" colorRed="100" colorizeType="1" hasTexture="0" name="[Translucent_Glass_Blue]" trans="0.5" type="0" useTrans="1"/></materialDocument>"#;

    fn png(color: [u8; 3], w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb(color));
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let owned: Vec<(String, Vec<u8>)> = files
            .iter()
            .map(|(n, b)| ((*n).to_owned(), b.to_vec()))
            .collect();
        crate::style_zip::zip_files(&owned).unwrap()
    }

    #[test]
    fn reads_a_textured_material() {
        let tex = png([152, 86, 42], 4, 4);
        let thumb = png([1, 2, 3], 2, 2);
        let data = archive(&[
            ("document.xml", CHERRY.as_bytes()),
            ("doc_thumbnail.png", &thumb),
            ("ref/Wood_Cherry_Original_1.jpg", &tex),
        ]);
        let s = read(&data).unwrap();
        let m = &s.material;
        assert_eq!(m.name, "Wood_Cherry_Original");
        assert_eq!(m.color, [152, 86, 42]);
        assert_eq!(m.opacity, 1.0);
        let t = m.texture.as_ref().unwrap();
        assert_eq!(t.filename, "Wood_Cherry_Original.jpg");
        assert_eq!((t.width, t.height), (72.0, 72.0));
        assert!(!t.colorize);
        assert_eq!(&*t.data, tex.as_slice());
        assert_eq!(s.thumbnail.as_deref(), Some(thumb.as_slice()));
    }

    #[test]
    fn reads_a_translucent_color() {
        let s = read(&archive(&[("document.xml", GLASS.as_bytes())])).unwrap();
        assert_eq!(s.material.name, "Translucent_Glass_Blue");
        assert_eq!(s.material.color, [100, 149, 237]);
        assert_eq!(s.material.opacity, 0.5);
        assert!(s.material.texture.is_none());
        assert!(s.thumbnail.is_none());
        // trans without useTrans (plain colors): opaque.
        let c = r#"<mat:material colorBlue="226" colorGreen="226" colorRed="226" hasTexture="0" name="[Color_001]" trans="1" type="0" useTrans="0"/>"#;
        let s = read(&archive(&[("document.xml", c.as_bytes())])).unwrap();
        assert_eq!(s.material.opacity, 1.0);
        assert!(read(b"not a zip").is_err());
        assert!(read(&archive(&[("document.xml", b"<a/>")])).is_err());
    }

    #[test]
    fn writes_what_it_reads() {
        let tex = png([10, 200, 30], 8, 4);
        let m = Material {
            name: "Brick & <Mortar>".into(),
            color: [10, 200, 30],
            opacity: 0.25,
            texture: Some(Texture {
                filename: "C:\\maps\\brick wall.png".into(),
                data: Arc::from(tex.as_slice()),
                width: 48.0,
                height: 24.0,
                colorize: true,
            }),
        };
        let data = write(&m).unwrap();
        let files = entries(&data).unwrap();
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "document.xml",
                "documentProperties.xml",
                "doc_thumbnail.png",
                "references.xml",
                "ref/brick_wall_1.png"
            ]
        );
        let back = read(&data).unwrap();
        let mut want = m.clone();
        if let Some(t) = want.texture.as_mut() {
            t.filename = "brick wall.png".into();
        }
        assert_eq!(back.material, want);
        let thumb = image::load_from_memory(&back.thumbnail.unwrap()).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (64, 64));
        // A plain opaque color.
        let c = Material {
            name: "Plain".into(),
            color: [1, 2, 3],
            opacity: 1.0,
            texture: None,
        };
        assert_eq!(read(&write(&c).unwrap()).unwrap().material, c);
    }
}
