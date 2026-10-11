//! Image entities (FEATURES.md item 30, part 1): File > Import of a PNG or JPEG.
//!
//! Like SketchUp's image object, an image is a component-like instance whose
//! definition (`is_image`) holds one rectangle painted with the picture on both sides.
//! It is placed with two clicks: a corner (on the face under the cursor, else level),
//! then the size, which keeps the picture's aspect ratio. It moves, rotates and scales
//! like a component; Explode leaves the textured face. It is not opened for editing.
//!
//! Saved files keep images as groups with their textured face (see
//! `mt_kernel::export`); images read from SketchUp files are image entities.

use std::path::Path;
use std::sync::Arc;

use glam::{DMat4, DVec3, DVec4};
use mt_infer::Inference;
use mt_infer::units::{LengthUnit, parse_length};
use mt_kernel::{Entities, TOL};

use crate::App;
use crate::document::Owner;
use crate::modify::{Ctx, ModelEdit, ModifyTool};

/// The picture being placed.
pub struct PlaceImage {
    name: String,
    path: String,
    data: Arc<[u8]>,
    /// Height over width of the picture.
    aspect: f64,
    /// The material's color (shown without textures): the picture's average.
    pub color: [u8; 3],
    /// First corner and the plane's normal.
    corner: Option<(DVec3, DVec3)>,
    edit: Option<ModelEdit>,
    done: bool,
}

impl PlaceImage {
    pub fn new(name: String, path: String, data: Arc<[u8]>, width_px: u32, height_px: u32) -> Self {
        Self {
            name,
            path,
            data,
            aspect: f64::from(height_px.max(1)) / f64::from(width_px.max(1)),
            color: [255, 255, 255],
            corner: None,
            edit: None,
            done: false,
        }
    }

    /// Lower left corner, width and height vectors of the picture from the first
    /// corner to `p`: it spans towards the cursor, as wide as the cursor is far along
    /// the plane's level axis or, if more, as high as it is along the other.
    fn rect(&self, corner: DVec3, normal: DVec3, p: DVec3) -> Option<(DVec3, DVec3, DVec3)> {
        let (u, v) = crate::tools::plane_axes(normal);
        let d = p - corner;
        let (du, dv) = (d.dot(u), d.dot(v));
        let w = du.abs().max(dv.abs() / self.aspect);
        (w > TOL).then(|| self.rect_of_width(corner, normal, w, du < 0.0, dv < 0.0))
    }

    fn rect_of_width(
        &self,
        corner: DVec3,
        normal: DVec3,
        w: f64,
        left: bool,
        down: bool,
    ) -> (DVec3, DVec3, DVec3) {
        let (u, v) = crate::tools::plane_axes(normal);
        let h = w * self.aspect;
        let origin = corner
            - if left { u * w } else { DVec3::ZERO }
            - if down { v * h } else { DVec3::ZERO };
        (origin, u * w, v * h)
    }

    fn place(&mut self, (origin, wv, hv): (DVec3, DVec3, DVec3)) {
        let (name, path, data) = (self.name.clone(), self.path.clone(), self.data.clone());
        let color = self.color;
        self.edit = Some(Box::new(move |model, active| {
            let (w, h) = (wv.length(), hv.length());
            model.materials.push(mt_kernel::Material {
                name: name.clone(),
                color,
                opacity: 1.0,
                texture: Some(mt_kernel::Texture {
                    filename: path,
                    data,
                    width: w,
                    height: h,
                    colorize: false,
                }),
            });
            let material = model.materials.len() - 1;
            let def = model.definitions.insert(mt_kernel::Definition {
                name: model.unique_definition_name(&name),
                entities: image_face(w, h, material),
                is_image: true,
                ..Default::default()
            });
            let n = wv.cross(hv).normalize();
            active.instances.push(mt_kernel::Instance {
                definition: def,
                transform: DMat4::from_cols(
                    (wv / w).extend(0.0),
                    (hv / h).extend(0.0),
                    n.extend(0.0),
                    DVec4::new(origin.x, origin.y, origin.z, 1.0),
                ),
                name: String::new(),
                hidden: false,
                locked: false,
                layer: None,
                material: None,
            });
            vec![Owner::Instance(active.instances.len() - 1)]
        }));
        self.corner = None;
        self.done = true;
    }
}

/// An image definition's contents: a `w` x `h` rectangle at the origin in the XY
/// plane, facing up, with one tile of `material`'s texture fit to it on both sides.
pub fn image_face(w: f64, h: f64, material: usize) -> Entities {
    let mut ents = Entities::new();
    let pts = [
        DVec3::ZERO,
        DVec3::new(w, 0.0, 0.0),
        DVec3::new(w, h, 0.0),
        DVec3::new(0.0, h, 0.0),
    ];
    let vs: Vec<_> = pts.iter().map(|p| ents.add_vertex(*p)).collect();
    let coedges = (0..4)
        .map(|k| mt_kernel::CoEdge {
            edge: ents.add_raw_edge(vs[k], vs[(k + 1) % 4], Default::default()),
            reversed: false,
        })
        .collect();
    let fit = mt_kernel::texture::fit_uv(DVec3::Z, DVec3::ZERO, pts[1], pts[3], [w, h]);
    ents.add_raw_face(
        mt_kernel::Loop { coedges },
        Vec::new(),
        DVec3::Z,
        mt_kernel::FaceAttrs {
            material: Some(material),
            back_material: Some(material),
            uv: mt_kernel::FaceUv::boxed(Some(fit), Some(fit)),
            ..Default::default()
        },
    );
    ents
}

impl ModifyTool for PlaceImage {
    fn status(&self) -> String {
        // Draft wording (unverified).
        if self.corner.is_some() {
            "Select the opposite corner or enter the width."
        } else {
            "Select a corner to place the image."
        }
        .into()
    }

    fn measurement(&self, cur: Option<&Inference>, units: LengthUnit) -> (&'static str, String) {
        let w = match (self.corner, cur) {
            (Some((c, n)), Some(cur)) => self.rect(c, n, cur.point).map_or(0.0, |r| r.1.length()),
            _ => 0.0,
        };
        (
            "Width",
            if w > 0.0 {
                units.format(w)
            } else {
                String::new()
            },
        )
    }

    fn anchor(&self) -> Option<DVec3> {
        self.corner.map(|c| c.0)
    }

    fn click(&mut self, _ctx: &mut Ctx, inf: &Inference) -> bool {
        match self.corner {
            None => self.corner = Some((inf.point, inf.normal.unwrap_or(DVec3::Z).normalize())),
            Some((c, n)) => {
                if let Some(r) = self.rect(c, n, inf.point) {
                    self.place(r);
                }
            }
        }
        false
    }

    fn typed(
        &mut self,
        _ctx: &mut Ctx,
        cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        if let (Some((c, n)), Some(w)) = (self.corner, parse_length(text, units))
            && w.abs() > TOL
        {
            let (u, v) = crate::tools::plane_axes(n);
            let d = cur.map_or(DVec3::ZERO, |i| i.point - c);
            let r = self.rect_of_width(c, n, w.abs(), d.dot(u) < 0.0, d.dot(v) < 0.0);
            self.place(r);
        }
        false
    }

    fn preview(&self, _ents: &Entities, cur: &Inference) -> Vec<Vec<DVec3>> {
        let Some((c, n)) = self.corner else {
            return Vec::new();
        };
        self.rect(c, n, cur.point)
            .map_or_else(Vec::new, |(o, w, h)| {
                vec![vec![o, o + w, o + w + h, o + h, o], vec![o, o + w + h]]
            })
    }

    fn cancel(&mut self) {
        self.done = true;
    }

    fn finished(&self) -> bool {
        self.done && self.edit.is_none()
    }

    fn take_model_edit(&mut self) -> Option<ModelEdit> {
        self.edit.take()
    }
}

/// The average color of a picture (sampling at most about 64k pixels).
pub fn average_color(img: &image::RgbImage) -> [u8; 3] {
    let step = ((img.width() as usize * img.height() as usize) / 65536).max(1);
    let (mut sum, mut n) = ([0u64; 3], 0u64);
    for p in img.pixels().step_by(step) {
        for k in 0..3 {
            sum[k] += u64::from(p[k]);
        }
        n += 1;
    }
    sum.map(|s| (s / n.max(1)) as u8)
}

impl App {
    /// File > Import of a PNG or JPEG: starts placing it.
    pub(crate) fn import_image(&mut self, path: &Path) {
        let fail = |e: String| format!("Could not import {}: {e}", path.display());
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                self.message = Some(fail(e.to_string()));
                return;
            }
        };
        let img = match image::load_from_memory(&data) {
            Ok(i) => i.to_rgb8(),
            Err(e) => {
                self.message = Some(fail(e.to_string()));
                return;
            }
        };
        let (w, h) = img.dimensions();
        let name = path
            .file_name()
            .map_or_else(|| "Image".into(), |n| n.to_string_lossy().into_owned());
        if let Some(d) = self.draw.as_mut() {
            d.cancel();
        }
        self.tool = crate::nav::Tool::Modify("tool.place_image");
        self.draw = None;
        let mut tool = PlaceImage::new(
            name.clone(),
            path.to_string_lossy().into_owned(),
            data.into(),
            w,
            h,
        );
        tool.color = average_color(&img);
        self.modify = Some(Box::new(tool));
        self.selection.clear();
        self.message = Some(format!("Imported {name}: click a corner, then the size."));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_infer::Kind;

    fn at(p: DVec3) -> Inference {
        Inference {
            point: p,
            kind: Kind::OnFace,
            edge: None,
            guide: None,
            locked: false,
            normal: None,
            segment: None,
        }
    }

    #[test]
    fn two_clicks_place_a_textured_image_and_explode_keeps_the_texture() {
        let mut t = PlaceImage::new(
            "pic.png".into(),
            "pic.png".into(),
            Arc::from(&b"x"[..]),
            200,
            100,
        );
        let mut ents = Entities::new();
        let mut ctx = Ctx {
            ents: &mut ents,
            picked: None,
            selection: &[],
            ctrl: false,
            eye: DVec3::ZERO,
        };
        t.click(&mut ctx, &at(DVec3::new(10.0, 10.0, 0.0)));
        // Further up than along: the height decides (aspect 1:2).
        t.click(&mut ctx, &at(DVec3::new(20.0, 40.0, 0.0)));
        assert!(!t.finished(), "the edit is still to be made");
        let edit = t.take_model_edit().unwrap();
        assert!(t.finished());
        let mut model = mt_kernel::Model::default();
        let mut root = Entities::new();
        assert_eq!(edit(&mut model, &mut root), vec![Owner::Instance(0)]);
        let inst = root.instances[0].clone();
        let def = &model.definitions[inst.definition];
        assert!(def.is_image && !def.is_group);
        let tex = model.materials[0].texture.as_ref().unwrap();
        assert_eq!((tex.width, tex.height), (60.0, 30.0));
        assert_eq!(def.entities.faces.len(), 1);

        // Explode: a face with the image, mapped corner to corner in place.
        assert!(mt_kernel::group::explode(&model.definitions, &mut root, 0));
        let face = root.faces.values().next().unwrap();
        assert_eq!(face.attrs.material, Some(0));
        let map = mt_kernel::texture::face_mapping(face, false, &model.materials[0]).unwrap();
        let uv = |p: DVec3| mt_kernel::texture::uv_at(&map, p);
        assert!(uv(DVec3::new(10.0, 10.0, 0.0)).abs_diff_eq(glam::DVec2::ZERO, 1e-9));
        assert!(uv(DVec3::new(70.0, 40.0, 0.0)).abs_diff_eq(glam::DVec2::ONE, 1e-9));
    }

    #[test]
    fn typed_width_on_a_wall() {
        let mut t = PlaceImage::new("p".into(), "p".into(), Arc::from(&b"x"[..]), 100, 50);
        let mut ents = Entities::new();
        let mut ctx = Ctx {
            ents: &mut ents,
            picked: None,
            selection: &[],
            ctrl: false,
            eye: DVec3::ZERO,
        };
        let mut wall = at(DVec3::ZERO);
        wall.normal = Some(DVec3::NEG_Y);
        t.click(&mut ctx, &wall);
        t.typed(&mut ctx, None, "40", LengthUnit::Inch);
        let mut model = mt_kernel::Model::default();
        let mut root = Entities::new();
        t.take_model_edit().unwrap()(&mut model, &mut root);
        let xf = root.instances[0].transform;
        // Width along the wall's level line, height straight up.
        let top = xf.transform_point3(DVec3::new(40.0, 20.0, 0.0));
        assert!((top.z - 20.0).abs() < 1e-9 && top.y.abs() < 1e-9, "{top}");
    }

    #[test]
    fn saved_as_a_group_with_its_textured_face() {
        let mut png = Vec::new();
        image::RgbImage::from_pixel(4, 2, image::Rgb([200, 30, 30]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let mut t = PlaceImage::new("red.png".into(), "red.png".into(), png.into(), 4, 2);
        t.color = average_color(&image::RgbImage::from_pixel(
            4,
            2,
            image::Rgb([200, 30, 30]),
        ));
        assert_eq!(t.color, [200, 30, 30]);
        let mut ents = Entities::new();
        let mut ctx = Ctx {
            ents: &mut ents,
            picked: None,
            selection: &[],
            ctrl: false,
            eye: DVec3::ZERO,
        };
        t.click(&mut ctx, &at(DVec3::new(5.0, 0.0, 0.0)));
        t.typed(&mut ctx, None, "40", LengthUnit::Inch);
        let mut model = mt_kernel::Model::default();
        let mut root = Entities::new();
        t.take_model_edit().unwrap()(&mut model, &mut root);
        model.root = root;
        let bytes = mt_kernel::export::to_skp(&model).unwrap();
        let back = mt_kernel::import::from_skp(&mt_skp::parse(&bytes).unwrap());
        assert_eq!(back.root.instances.len(), 1);
        let inst = &back.root.instances[0];
        let def = &back.definitions[inst.definition];
        assert!(def.is_group && !def.is_image);
        let face = def.entities.faces.values().next().unwrap();
        let mat = &back.materials[face.attrs.material.unwrap()];
        assert!(mat.texture.is_some());
        let map = mt_kernel::texture::face_mapping(face, false, mat).unwrap();
        let world = |p: DVec3| inst.transform.inverse().transform_point3(p);
        let uv = |p| mt_kernel::texture::uv_at(&map, world(p));
        assert!(uv(DVec3::new(5.0, 0.0, 0.0)).abs_diff_eq(glam::DVec2::ZERO, 1e-6));
        assert!(uv(DVec3::new(45.0, 20.0, 0.0)).abs_diff_eq(glam::DVec2::ONE, 1e-6));
    }
}
