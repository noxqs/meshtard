//! VRML 2.0 (VRML97, ISO/IEC 14772) export: one `Shape` per material with an
//! `IndexedFaceSet` of triangles, `Material` diffuse color and transparency and an
//! `ImageTexture` with texture coordinates for textured materials.
//!
//! - Options after SketchUp's VRML exporter (help.sketchup.com, "Exporting VRML
//!   files"): "Output Texture Maps", "Ignore Back of Face Material" (one-sided
//!   triangles drawn from both sides, `solid FALSE`; else back faces are written as
//!   their own triangles with the back material), "Use VRML Standard Orientation"
//!   (Y up). Edges, cameras, mirrored-component and material-override options are not
//!   offered.
//! - VRML's unit is the meter; another unit can be chosen.

use std::fmt::Write as _;

use super::{ExportOptions, Files, groups, num, texture_files};
use crate::model::Model;

/// A VRML name: letters, digits and `_` (SketchUp replaces spaces with `_`).
fn def_name(name: &str, i: usize) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("M{i}_{s}")
}

/// Encodes the model as VRML 2.0. `opts.two_sided` false ignores back materials.
pub fn to_vrml(model: &Model, opts: &ExportOptions) -> Files {
    let (mats, groups) = groups(model, opts);
    let mut d = String::from("#VRML V2.0 utf8\n");
    let _ = writeln!(
        d,
        "WorldInfo {{\n  title \"{}\"\n  info [ \"Exported by meshtard\" \"Units: {}\" ]\n}}",
        opts.base_name.replace('"', "'"),
        opts.units.name()
    );
    for (i, (g, m)) in groups.iter().zip(&mats).enumerate() {
        if g.triangles.is_empty() {
            continue;
        }
        let c = m.color.map(|x| f64::from(x) / 255.0);
        let _ = writeln!(d, "Shape {{\n  appearance Appearance {{");
        let _ = writeln!(
            d,
            "    material DEF {} Material {{ diffuseColor {} {} {} transparency {} }}",
            def_name(&m.name, i),
            num(c[0]),
            num(c[1]),
            num(c[2]),
            num(1.0 - m.opacity)
        );
        if let Some((file, _)) = &m.texture {
            let _ = writeln!(
                d,
                "    texture ImageTexture {{ url \"{}\" }}",
                file.replace('"', "'")
            );
        }
        let _ = writeln!(d, "  }}\n  geometry IndexedFaceSet {{");
        let _ = writeln!(
            d,
            "    solid {}\n    ccw TRUE",
            if opts.two_sided { "TRUE" } else { "FALSE" }
        );
        d.push_str("    coord Coordinate { point [\n");
        for p in &g.positions {
            let _ = writeln!(d, "      {} {} {},", num(p.x), num(p.y), num(p.z));
        }
        d.push_str("    ] }\n    coordIndex [\n");
        for t in &g.triangles {
            let _ = writeln!(d, "      {}, {}, {}, -1,", t[0].0, t[1].0, t[2].0);
        }
        d.push_str("    ]\n");
        if m.texture.is_some() {
            d.push_str("    texCoord TextureCoordinate { point [\n");
            for t in &g.uvs {
                let _ = writeln!(d, "      {} {},", num(t.x), num(t.y));
            }
            d.push_str("    ] }\n    texCoordIndex [\n");
            for t in &g.triangles {
                let _ = writeln!(d, "      {}, {}, {}, -1,", t[0].1, t[1].1, t[2].1);
            }
            d.push_str("    ]\n");
        }
        d.push_str("  }\n}\n");
    }
    Files {
        main: d.into_bytes(),
        others: texture_files(&mats),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::tests::painted_cube;

    #[test]
    fn shapes_per_material() {
        let m = painted_cube();
        let f = to_vrml(&m, &ExportOptions::default());
        let text = String::from_utf8(f.main).unwrap();
        assert!(text.starts_with("#VRML V2.0 utf8"));
        assert_eq!(text.matches("Shape {").count(), 2);
        assert_eq!(text.matches(", -1,").count(), 12);
        assert!(text.contains("diffuseColor 1 0 0"));
        assert!(text.contains("solid FALSE"));
        // Braces balance.
        assert_eq!(text.matches('{').count(), text.matches('}').count());
        assert_eq!(text.matches('[').count(), text.matches(']').count());
    }
}
