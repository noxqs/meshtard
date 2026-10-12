//! USDZ export (File > Export > 3D Model): a USD text layer (`.usda`) and its texture
//! images in an uncompressed ZIP archive whose files start on 64-byte boundaries, as
//! the USDZ format asks (Pixar's "Usdz File Format Specification").
//!
//! - One `Mesh` per material (groups and components flattened), triangles with
//!   face-varying `st` texture coordinates when textured; one `UsdPreviewSurface`
//!   material each: diffuse color (linear) or a `UsdUVTexture` of the image,
//!   opacity, roughness 1, metallic 0. Back faces: `doubleSided` unless written as
//!   their own triangles (`two_sided`).
//! - Units and axes as chosen: the layer records `metersPerUnit` and `upAxis` (Y up
//!   swaps the axes as the other exporters do).
//! - The layer is text (`usda`), which the format allows; viewers that only take
//!   binary `usdc` layers (some versions of Apple's Quick Look) may not open it.
//!   Writing `usdc` (Pixar's crate format) is not done.

use std::fmt::Write as _;
use std::io::Write as _;

use super::{ExportOptions, Files, groups, num};
use crate::model::Model;

/// An identifier USD accepts: letters, digits and `_`, not starting with a digit.
fn ident(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// A file name safe inside the archive and in an asset path.
fn file_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn linear(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The `.usda` layer and the textures it names (archive path, bytes).
pub fn to_usda(model: &Model, opts: &ExportOptions) -> (String, Vec<(String, Vec<u8>)>) {
    let (mats, groups) = groups(model, opts);
    let mut out = String::from("#usda 1.0\n(\n");
    out.push_str("    defaultPrim = \"Model\"\n");
    let _ = writeln!(
        out,
        "    metersPerUnit = {}",
        num(0.0254 / opts.units.per_inch())
    );
    let _ = writeln!(
        out,
        "    upAxis = \"{}\"",
        if opts.y_up { "Y" } else { "Z" }
    );
    out.push_str("    doc = \"Exported by meshtard\"\n)\n\n");
    out.push_str("def Xform \"Model\" (\n    kind = \"component\"\n)\n{\n");
    out.push_str("    def Scope \"Materials\"\n    {\n");
    let mut names: Vec<String> = Vec::new();
    let mut textures: Vec<(String, Vec<u8>)> = Vec::new();
    for (i, m) in mats.iter().enumerate() {
        let mut name = ident(&m.name);
        if names.contains(&name) {
            name = format!("{name}_{i}");
        }
        names.push(name.clone());
        let path = format!("/Model/Materials/{name}");
        let _ = write!(
            out,
            "        def Material \"{name}\"\n        {{\n            token outputs:surface.connect = <{path}/Surface.outputs:surface>\n\n"
        );
        out.push_str("            def Shader \"Surface\"\n            {\n");
        out.push_str("                uniform token info:id = \"UsdPreviewSurface\"\n");
        let tex = m.texture.as_ref().map(|(file, data)| {
            let file = format!("textures/{}", file_name(file));
            if !textures.iter().any(|(n, _)| *n == file) {
                textures.push((file.clone(), data.to_vec()));
            }
            file
        });
        match &tex {
            Some(_) => {
                let _ = writeln!(
                    out,
                    "                color3f inputs:diffuseColor.connect = <{path}/Texture.outputs:rgb>"
                );
            }
            None => {
                let c = m.color.map(linear);
                let _ = writeln!(
                    out,
                    "                color3f inputs:diffuseColor = ({}, {}, {})",
                    num(c[0]),
                    num(c[1]),
                    num(c[2])
                );
            }
        }
        let _ = writeln!(
            out,
            "                float inputs:opacity = {}",
            num(m.opacity.clamp(0.0, 1.0))
        );
        out.push_str("                float inputs:roughness = 1\n");
        out.push_str("                float inputs:metallic = 0\n");
        out.push_str("                token outputs:surface\n            }\n");
        if let Some(file) = &tex {
            out.push_str("\n            def Shader \"Reader\"\n            {\n");
            out.push_str("                uniform token info:id = \"UsdPrimvarReader_float2\"\n");
            out.push_str("                string inputs:varname = \"st\"\n");
            out.push_str("                float2 outputs:result\n            }\n");
            out.push_str("\n            def Shader \"Texture\"\n            {\n");
            out.push_str("                uniform token info:id = \"UsdUVTexture\"\n");
            let _ = writeln!(out, "                asset inputs:file = @{file}@");
            let _ = writeln!(
                out,
                "                float2 inputs:st.connect = <{path}/Reader.outputs:result>"
            );
            out.push_str("                token inputs:wrapS = \"repeat\"\n");
            out.push_str("                token inputs:wrapT = \"repeat\"\n");
            out.push_str("                float3 outputs:rgb\n            }\n");
        }
        out.push_str("        }\n");
    }
    out.push_str("    }\n");
    for (slot, g) in groups.iter().enumerate() {
        if g.triangles.is_empty() {
            continue;
        }
        let _ = write!(
            out,
            "\n    def Mesh \"Mesh_{slot}\" (\n        prepend apiSchemas = [\"MaterialBindingAPI\"]\n    )\n    {{\n"
        );
        let (lo, hi) = g.positions.iter().fold(
            (glam::DVec3::splat(f64::MAX), glam::DVec3::splat(f64::MIN)),
            |(lo, hi), p| (lo.min(*p), hi.max(*p)),
        );
        let v3 = |p: glam::DVec3| format!("({}, {}, {})", num(p.x), num(p.y), num(p.z));
        let _ = writeln!(
            out,
            "        uniform bool doubleSided = {}",
            if opts.two_sided { 0 } else { 1 }
        );
        let _ = writeln!(out, "        float3[] extent = [{}, {}]", v3(lo), v3(hi));
        let counts = vec!["3"; g.triangles.len()].join(", ");
        let _ = writeln!(out, "        int[] faceVertexCounts = [{counts}]");
        let idx: Vec<String> = g
            .triangles
            .iter()
            .flat_map(|t| t.iter().map(|c| c.0.to_string()))
            .collect();
        let _ = writeln!(
            out,
            "        int[] faceVertexIndices = [{}]",
            idx.join(", ")
        );
        let pts: Vec<String> = g.positions.iter().map(|&p| v3(p)).collect();
        let _ = writeln!(out, "        point3f[] points = [{}]", pts.join(", "));
        if mats[slot].texture.is_some() {
            let st: Vec<String> = g
                .uvs
                .iter()
                .map(|t| format!("({}, {})", num(t.x), num(t.y)))
                .collect();
            let _ = writeln!(
                out,
                "        texCoord2f[] primvars:st = [{}] (\n            interpolation = \"faceVarying\"\n        )",
                st.join(", ")
            );
            let si: Vec<String> = g
                .triangles
                .iter()
                .flat_map(|t| t.iter().map(|c| c.1.to_string()))
                .collect();
            let _ = writeln!(
                out,
                "        int[] primvars:st:indices = [{}]",
                si.join(", ")
            );
        }
        out.push_str("        uniform token subdivisionScheme = \"none\"\n");
        let _ = writeln!(
            out,
            "        rel material:binding = </Model/Materials/{}>",
            names[slot]
        );
        out.push_str("    }\n");
    }
    out.push_str("}\n");
    (out, textures)
}

/// Encodes the model as USDZ: `model.usda` first, then the textures, stored and
/// 64-byte aligned.
pub fn to_usdz(model: &Model, opts: &ExportOptions) -> Result<Files, String> {
    let (usda, textures) = to_usda(model, opts);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .with_alignment(64);
    let files = std::iter::once(("model.usda".to_owned(), usda.into_bytes())).chain(textures);
    for (name, data) in files {
        zip.start_file(name.as_str(), options)
            .map_err(|e| e.to_string())?;
        zip.write_all(&data).map_err(|e| e.to_string())?;
    }
    let main = zip.finish().map_err(|e| e.to_string())?.into_inner();
    Ok(Files {
        main,
        others: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    #[test]
    fn usdz_is_an_aligned_stored_archive_with_the_layer_first() {
        let mut m = crate::formats::tests::painted_cube();
        m.materials[0].texture = Some(crate::Texture {
            filename: "red brick.png".into(),
            data: std::sync::Arc::from(b"\x89PNG....".to_vec()),
            width: 10.0,
            height: 10.0,
            colorize: false,
        });
        let opts = ExportOptions {
            units: crate::mesh_export::Units::Meters,
            y_up: true,
            ..Default::default()
        };
        let files = to_usdz(&m, &opts).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&files.main[..])).unwrap();
        assert_eq!(zip.len(), 2);
        let mut names = Vec::new();
        for i in 0..zip.len() {
            let f = zip.by_index(i).unwrap();
            assert_eq!(f.compression(), zip::CompressionMethod::Stored);
            assert_eq!(f.data_start().unwrap() % 64, 0, "{}", f.name().unwrap());
            names.push(f.name().unwrap().into_owned());
        }
        assert_eq!(names, ["model.usda", "textures/model_Red.png"]);
        let mut usda = String::new();
        zip.by_index(0).unwrap().read_to_string(&mut usda).unwrap();
        assert!(usda.starts_with("#usda 1.0"));
        assert!(usda.contains("metersPerUnit = 1\n"));
        assert!(usda.contains("upAxis = \"Y\""));
        assert!(usda.contains("asset inputs:file = @textures/model_Red.png@"));
        // 12 triangles over the two materials; 8 + 4 welded corners.
        let tris: usize = usda
            .lines()
            .filter(|l| l.contains("faceVertexCounts"))
            .map(|l| l.matches('3').count())
            .sum();
        assert_eq!(tris, 12);
        assert_eq!(usda.matches("def Mesh").count(), 2);
        assert_eq!(usda.matches("primvars:st:indices").count(), 1);
        // The textured top's 2 triangles have one coordinate per corner.
        let line = usda
            .lines()
            .find(|l| l.contains("primvars:st:indices"))
            .unwrap();
        assert_eq!(line.matches(',').count(), 5);
    }
}
