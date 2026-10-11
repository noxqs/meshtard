//! Exports a `.skp` as OBJ (+ MTL, textures), STL (mm) and COLLADA, and reads the OBJ
//! and STL back: `cargo run -p mt-kernel --example export_mesh -- model.skp out_dir`.

use mt_kernel::mesh_export::{self, DaeOptions, ObjOptions, Units};
use mt_kernel::mesh_import::{self, MeshOptions};

fn main() {
    let mut args = std::env::args_os().skip(1);
    let (Some(src), Some(out)) = (args.next(), args.next()) else {
        eprintln!("usage: export_mesh MODEL.skp OUT_DIR");
        std::process::exit(2);
    };
    let out = std::path::PathBuf::from(out);
    std::fs::create_dir_all(&out).expect("output folder");
    let skp = mt_skp::read(std::path::Path::new(&src)).expect("readable .skp");
    let model = mt_kernel::import::from_skp(&skp);
    let tris = mesh_export::triangles(&model);
    println!("triangles: {}", tris.len());

    let obj = mesh_export::to_obj(
        &model,
        &ObjOptions {
            base_name: "model".into(),
            ..Default::default()
        },
    );
    std::fs::write(out.join("model.obj"), &obj.obj).unwrap();
    std::fs::write(out.join("model.mtl"), &obj.mtl).unwrap();
    for (name, data) in &obj.textures {
        std::fs::write(out.join(name), data).unwrap();
    }
    let stl = mesh_export::to_stl(&model, Units::Millimeters);
    std::fs::write(out.join("model.stl"), &stl).unwrap();
    let dae = mesh_export::to_dae(&model, &DaeOptions::default());
    std::fs::write(out.join("model.dae"), &dae.dae).unwrap();
    println!(
        "obj: {} v, {} vt, {} f, {} materials, {} textures; stl {} bytes; dae {} bytes",
        obj.obj.lines().filter(|l| l.starts_with("v ")).count(),
        obj.obj.lines().filter(|l| l.starts_with("vt ")).count(),
        obj.obj.lines().filter(|l| l.starts_with("f ")).count(),
        obj.mtl.matches("newmtl").count(),
        obj.textures.len(),
        stl.len(),
        dae.dae.len()
    );

    let mesh = mesh_import::parse_obj(&obj.obj, |n| std::fs::read_to_string(out.join(n)).ok())
        .expect("OBJ reads back");
    let back = mesh_import::to_entities(
        &mesh,
        &MeshOptions {
            inches_per_unit: 1.0,
            y_up: true,
            merge_coplanar: true,
        },
        &[],
    );
    println!(
        "OBJ read back: {} polygons -> {} faces after merging",
        mesh.polygons.len(),
        back.faces.len()
    );
    let mesh = mesh_import::parse_stl(&stl).expect("STL reads back");
    println!("STL read back: {} triangles", mesh.polygons.len());
}
