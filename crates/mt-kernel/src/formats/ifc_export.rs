//! IFC4 export, geometry only (Reference View style): one site, building and
//! storey; every visible group or component instance is an
//! `IfcBuildingElementProxy` named after the instance (or its definition), placed by
//! the translation and rotation of its transform, nested ones aggregated under
//! their parent's proxy (`IfcRelAggregates`).
//!
//! - Groups' faces are `IfcTriangulatedFaceSet`s in the proxy's coordinates, one per
//!   material; components are an `IfcRepresentationMap` per definition (and
//!   inherited material) owned by an `IfcBuildingElementProxyType` named after the
//!   definition, placed with an `IfcMappedItem` whose transformation operator
//!   carries the transform's scale and mirroring (shear is lost).
//! - Loose faces of the model root become one proxy named after the file.
//! - Materials are `IfcSurfaceStyle`s (shading color and transparency) on the face
//!   sets; faces with the default material have no style. Textures, edges, hidden
//!   entities and hidden tags are left out.
//! - Units: the chosen length unit (SI with prefix, or inch / foot as conversion
//!   based units), square and cubic meters, radians.
//! - GlobalIds are made from the file name, so the same model exports the same file;
//!   the header time stamp is fixed.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use glam::{DMat3, DMat4, DVec3};

use super::ifc::split_rigid;
use super::step::{quote, real};
use super::{ExportOptions, Files};
use crate::entities::{DefinitionId, Instance, MaterialId};
use crate::mesh_export::{Triangle, Units, face_triangles};
use crate::model::Model;

/// A component's representation map and type, per (definition, inherited material).
type Maps = HashMap<(DefinitionId, Option<MaterialId>), Option<(u64, u64)>>;

struct Writer<'a> {
    model: &'a Model,
    out: String,
    next: u64,
    seed: u64,
    guids: u64,
    /// Output units per inch.
    scale: f64,
    body: u64,
    wcs: u64,
    styles: BTreeMap<Option<MaterialId>, Option<u64>>,
    /// (definition, inherited material) -> (representation map, type).
    maps: Maps,
    /// Type -> proxies typed by it.
    typed: BTreeMap<u64, Vec<u64>>,
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// A 128-bit value in IFC's 22-character base 64 GlobalId form.
pub(crate) fn compress_guid(v: u128) -> String {
    const CHARS: &[u8; 64] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$";
    let mut s = String::with_capacity(22);
    s.push(CHARS[(v >> 126) as usize & 3] as char);
    for k in (0..21).rev() {
        s.push(CHARS[(v >> (6 * k)) as usize & 63] as char);
    }
    s
}

fn layer_visible(model: &Model, layer: Option<usize>) -> bool {
    layer.is_none_or(|l| model.layers.get(l).is_none_or(|x| x.visible))
}

impl Writer<'_> {
    fn add(&mut self, s: impl AsRef<str>) -> u64 {
        let id = self.next;
        self.next += 1;
        let _ = writeln!(self.out, "#{id}={};", s.as_ref());
        id
    }

    fn guid(&mut self) -> String {
        self.guids += 1;
        let a = splitmix(self.seed ^ self.guids);
        let b = splitmix(a);
        quote(&compress_guid((u128::from(a) << 64) | u128::from(b)))
    }

    fn point(&mut self, p: DVec3) -> u64 {
        self.add(format!(
            "IFCCARTESIANPOINT(({},{},{}))",
            real(p.x),
            real(p.y),
            real(p.z)
        ))
    }

    fn direction(&mut self, d: DVec3) -> u64 {
        self.add(format!(
            "IFCDIRECTION(({},{},{}))",
            real(d.x),
            real(d.y),
            real(d.z)
        ))
    }

    /// An `IfcAxis2Placement3D` for a rigid transform (inches).
    fn axis2(&mut self, m: DMat4) -> u64 {
        let p = self.point(m.w_axis.truncate() * self.scale);
        if m.abs_diff_eq(DMat4::from_translation(m.w_axis.truncate()), 1e-12) {
            return self.add(format!("IFCAXIS2PLACEMENT3D(#{p},$,$)"));
        }
        let z = self.direction(m.z_axis.truncate());
        let x = self.direction(m.x_axis.truncate());
        self.add(format!("IFCAXIS2PLACEMENT3D(#{p},#{z},#{x})"))
    }

    /// The transformation operator of a transform (inches): axes, scales, origin.
    fn operator(&mut self, m: DMat4) -> u64 {
        let m3 = DMat3::from_mat4(m);
        let cols = [m3.x_axis, m3.y_axis, m3.z_axis];
        let lens = cols.map(DVec3::length);
        let dirs: Vec<u64> = (0..3)
            .map(|k| {
                let d = cols[k].try_normalize().unwrap_or(DVec3::AXES[k]);
                self.direction(d)
            })
            .collect();
        let o = self.point(m.w_axis.truncate() * self.scale);
        let uniform = (lens[0] - lens[1]).abs() < 1e-9 && (lens[0] - lens[2]).abs() < 1e-9;
        if uniform {
            self.add(format!(
                "IFCCARTESIANTRANSFORMATIONOPERATOR3D(#{},#{},#{o},{},#{})",
                dirs[0],
                dirs[1],
                real(lens[0]),
                dirs[2]
            ))
        } else {
            self.add(format!(
                "IFCCARTESIANTRANSFORMATIONOPERATOR3DNONUNIFORM(#{},#{},#{o},{},#{},{},{})",
                dirs[0],
                dirs[1],
                real(lens[0]),
                dirs[2],
                real(lens[1]),
                real(lens[2])
            ))
        }
    }

    /// The surface style of a material, made once (`None` for the default).
    fn style(&mut self, m: Option<MaterialId>) -> Option<u64> {
        if let Some(s) = self.styles.get(&m) {
            return *s;
        }
        let model = self.model;
        let s = m.and_then(|i| model.materials.get(i)).map(|mat| {
            let [r, g, b] = mat.color.map(|c| real(f64::from(c) / 255.0));
            let colour = self.add(format!("IFCCOLOURRGB($,{r},{g},{b})"));
            let shading = self.add(format!(
                "IFCSURFACESTYLESHADING(#{colour},{})",
                real(1.0 - mat.opacity.clamp(0.0, 1.0))
            ));
            self.add(format!(
                "IFCSURFACESTYLE({},.BOTH.,(#{shading}))",
                quote(&mat.name)
            ))
        });
        self.styles.insert(m, s);
        s
    }

    /// Triangulated face sets (one per material, styled) of triangles in inches.
    fn face_sets(&mut self, tris: Vec<Triangle>) -> Vec<u64> {
        let mut by_material: BTreeMap<Option<MaterialId>, Vec<[DVec3; 3]>> = BTreeMap::new();
        for t in tris {
            by_material.entry(t.material).or_default().push(t.points);
        }
        let mut out = Vec::new();
        for (m, tris) in by_material {
            let mut index: HashMap<[u64; 3], usize> = HashMap::new();
            let mut coords = String::new();
            let mut faces = String::new();
            let mut n = 0;
            for t in &tris {
                let mut ids = [0; 3];
                for (k, p) in t.iter().enumerate() {
                    let p = *p * self.scale;
                    ids[k] = *index
                        .entry(p.to_array().map(f64::to_bits))
                        .or_insert_with(|| {
                            if n > 0 {
                                coords.push(',');
                            }
                            let _ = write!(coords, "({},{},{})", real(p.x), real(p.y), real(p.z));
                            n += 1;
                            n
                        });
                }
                if !faces.is_empty() {
                    faces.push(',');
                }
                let _ = write!(faces, "({},{},{})", ids[0], ids[1], ids[2]);
            }
            let pl = self.add(format!("IFCCARTESIANPOINTLIST3D(({coords}))"));
            let set = self.add(format!("IFCTRIANGULATEDFACESET(#{pl},$,$,({faces}),$)"));
            if let Some(s) = self.style(m) {
                self.add(format!("IFCSTYLEDITEM(#{set},(#{s}),$)"));
            }
            out.push(set);
        }
        out
    }

    fn shape(&mut self, kind: &str, items: &[u64]) -> u64 {
        let list = refs(items);
        self.add(format!(
            "IFCSHAPEREPRESENTATION(#{},'Body','{kind}',({list}))",
            self.body
        ))
    }

    /// The representation map and type of a component (faces of the definition
    /// itself, default faces painted `inherited`).
    fn map(&mut self, def: DefinitionId, inherited: Option<MaterialId>) -> Option<(u64, u64)> {
        if let Some(m) = self.maps.get(&(def, inherited)) {
            return *m;
        }
        let model = self.model;
        let d = &model.definitions[def];
        let mut tris = Vec::new();
        face_triangles(model, &d.entities, DMat4::IDENTITY, inherited, &mut tris);
        let result = if tris.is_empty() {
            None
        } else {
            let sets = self.face_sets(tris);
            let rep = self.shape("Tessellation", &sets);
            let map = self.add(format!("IFCREPRESENTATIONMAP(#{},#{rep})", self.wcs));
            let g = self.guid();
            let ty = self.add(format!(
                "IFCBUILDINGELEMENTPROXYTYPE({g},$,{},$,$,$,(#{map}),$,$,.NOTDEFINED.)",
                quote(&d.name)
            ));
            Some((map, ty))
        };
        self.maps.insert((def, inherited), result);
        result
    }

    fn proxy(
        &mut self,
        name: &str,
        object_type: &str,
        placement: u64,
        items: &[u64],
        mapped: bool,
    ) -> u64 {
        let rep = if items.is_empty() {
            "$".to_owned()
        } else {
            let kind = if mapped {
                "MappedRepresentation"
            } else {
                "Tessellation"
            };
            let shape = self.shape(kind, items);
            format!(
                "#{}",
                self.add(format!("IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}))"))
            )
        };
        let g = self.guid();
        let ot = if object_type.is_empty() {
            "$".to_owned()
        } else {
            quote(object_type)
        };
        self.add(format!(
            "IFCBUILDINGELEMENTPROXY({g},$,{},$,{ot},#{placement},{rep},$,.NOTDEFINED.)",
            quote(name)
        ))
    }

    /// The proxy of an instance and its nested instances' proxies. `parent_world`
    /// and `parent_rigid` are inches; `None` when there is nothing to write.
    fn instance(
        &mut self,
        inst: &Instance,
        parent_world: DMat4,
        parent_rigid: DMat4,
        parent_placement: u64,
        inherited: Option<MaterialId>,
        stack: &mut Vec<DefinitionId>,
    ) -> Option<u64> {
        let model = self.model;
        if inst.hidden || !layer_visible(model, inst.layer) || stack.contains(&inst.definition) {
            return None;
        }
        let def = model.definitions.get(inst.definition)?;
        if def.entities.faces.is_empty() && def.entities.instances.is_empty() {
            return None;
        }
        let world = parent_world * inst.transform;
        let (rigid, local) = split_rigid(world);
        let rel = parent_rigid.inverse() * rigid;
        let axis = self.axis2(rel);
        let placement = self.add(format!("IFCLOCALPLACEMENT(#{parent_placement},#{axis})"));
        let material = inst.material.or(inherited);
        let mut ty = None;
        let (items, mapped) = if def.is_group || def.is_image {
            let mut tris = Vec::new();
            face_triangles(model, &def.entities, local, material, &mut tris);
            (self.face_sets(tris), false)
        } else {
            match self.map(inst.definition, material) {
                Some((map, t)) => {
                    ty = Some(t);
                    let op = self.operator(local);
                    (vec![self.add(format!("IFCMAPPEDITEM(#{map},#{op})"))], true)
                }
                None => (Vec::new(), true),
            }
        };
        let name = if inst.name.is_empty() {
            &def.name
        } else {
            &inst.name
        };
        let proxy = self.proxy(name, &def.name, placement, &items, mapped);
        if let Some(t) = ty {
            self.typed.entry(t).or_default().push(proxy);
        }
        stack.push(inst.definition);
        let kids: Vec<u64> = def
            .entities
            .instances
            .iter()
            .filter_map(|k| self.instance(k, world, rigid, placement, material, stack))
            .collect();
        stack.pop();
        if !kids.is_empty() {
            let g = self.guid();
            self.add(format!(
                "IFCRELAGGREGATES({g},$,$,$,#{proxy},({}))",
                refs(&kids)
            ));
        }
        Some(proxy)
    }

    fn units(&mut self, units: Units) -> u64 {
        let length = match units {
            Units::Millimeters => self.add("IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.)"),
            Units::Centimeters => self.add("IFCSIUNIT(*,.LENGTHUNIT.,.CENTI.,.METRE.)"),
            Units::Meters => self.add("IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.)"),
            Units::Inches | Units::Feet => {
                let metre = self.add("IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.)");
                let dims = self.add("IFCDIMENSIONALEXPONENTS(1,0,0,0,0,0,0)");
                let (name, f) = if units == Units::Feet {
                    ("FOOT", 0.3048)
                } else {
                    ("INCH", 0.0254)
                };
                let m = self.add(format!(
                    "IFCMEASUREWITHUNIT(IFCLENGTHMEASURE({}),#{metre})",
                    real(f)
                ));
                self.add(format!(
                    "IFCCONVERSIONBASEDUNIT(#{dims},.LENGTHUNIT.,'{name}',#{m})"
                ))
            }
        };
        let area = self.add("IFCSIUNIT(*,.AREAUNIT.,$,.SQUARE_METRE.)");
        let volume = self.add("IFCSIUNIT(*,.VOLUMEUNIT.,$,.CUBIC_METRE.)");
        let angle = self.add("IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.)");
        self.add(format!(
            "IFCUNITASSIGNMENT((#{length},#{area},#{volume},#{angle}))"
        ))
    }
}

fn refs(ids: &[u64]) -> String {
    ids.iter()
        .map(|i| format!("#{i}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Encodes the model as an IFC4 file (`opts.units`; axes, two-sided and texture
/// options do not apply).
pub fn to_ifc(model: &Model, opts: &ExportOptions) -> Files {
    let seed = opts
        .base_name
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        });
    let mut w = Writer {
        model,
        out: String::new(),
        next: 1,
        seed,
        guids: 0,
        scale: opts.units.per_inch(),
        body: 0,
        wcs: 0,
        styles: BTreeMap::new(),
        maps: HashMap::new(),
        typed: BTreeMap::new(),
    };
    let origin = w.point(DVec3::ZERO);
    w.wcs = w.add(format!("IFCAXIS2PLACEMENT3D(#{origin},$,$)"));
    let ctx = w.add(format!(
        "IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#{},$)",
        w.wcs
    ));
    w.body = w.add(format!(
        "IFCGEOMETRICREPRESENTATIONSUBCONTEXT('Body','Model',*,*,*,*,#{ctx},$,.MODEL_VIEW.,$)"
    ));
    let units = w.units(opts.units);
    let g = w.guid();
    let project = w.add(format!(
        "IFCPROJECT({g},$,{},$,$,$,$,(#{ctx}),#{units})",
        quote(&opts.base_name)
    ));
    let site_pl = w.add(format!("IFCLOCALPLACEMENT($,#{})", w.wcs));
    let g = w.guid();
    let site = w.add(format!(
        "IFCSITE({g},$,'Site',$,$,#{site_pl},$,$,.ELEMENT.,$,$,$,$,$)"
    ));
    let building_pl = w.add(format!("IFCLOCALPLACEMENT(#{site_pl},#{})", w.wcs));
    let g = w.guid();
    let building = w.add(format!(
        "IFCBUILDING({g},$,'Building',$,$,#{building_pl},$,$,.ELEMENT.,$,$,$)"
    ));
    let storey_pl = w.add(format!("IFCLOCALPLACEMENT(#{building_pl},#{})", w.wcs));
    let g = w.guid();
    let storey = w.add(format!(
        "IFCBUILDINGSTOREY({g},$,'Level 1',$,$,#{storey_pl},$,$,.ELEMENT.,0.)"
    ));
    for (whole, part) in [(project, site), (site, building), (building, storey)] {
        let g = w.guid();
        w.add(format!("IFCRELAGGREGATES({g},$,$,$,#{whole},(#{part}))"));
    }
    let mut top = Vec::new();
    // Loose faces of the root.
    let mut tris = Vec::new();
    face_triangles(model, &model.root, DMat4::IDENTITY, None, &mut tris);
    if !tris.is_empty() {
        let sets = w.face_sets(tris);
        let pl = w.add(format!("IFCLOCALPLACEMENT(#{storey_pl},#{})", w.wcs));
        top.push(w.proxy(&opts.base_name, "", pl, &sets, false));
    }
    let mut stack = Vec::new();
    for inst in &model.root.instances {
        if let Some(p) = w.instance(
            inst,
            DMat4::IDENTITY,
            DMat4::IDENTITY,
            storey_pl,
            None,
            &mut stack,
        ) {
            top.push(p);
        }
    }
    if !top.is_empty() {
        let g = w.guid();
        w.add(format!(
            "IFCRELCONTAINEDINSPATIALSTRUCTURE({g},$,$,$,({}),#{storey})",
            refs(&top)
        ));
    }
    for (ty, objs) in std::mem::take(&mut w.typed) {
        let g = w.guid();
        w.add(format!(
            "IFCRELDEFINESBYTYPE({g},$,$,$,({}),#{ty})",
            refs(&objs)
        ));
    }
    let mut text = String::new();
    let _ = write!(
        text,
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('ViewDefinition [ReferenceView_V1.2]'),'2;1');\n\
         FILE_NAME({},'1970-01-01T00:00:00',(''),(''),'Meshtard','Meshtard','');\n\
         FILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n",
        quote(&format!("{}.ifc", opts.base_name))
    );
    text += &w.out;
    text += "ENDSEC;\nEND-ISO-10303-21;\n";
    Files {
        main: text.into_bytes(),
        others: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::Entities;
    use crate::formats::ifc::{IfcOptions, import};
    use crate::model::Definition;
    use crate::{Checks, validate};

    fn inst(definition: DefinitionId, transform: DMat4, name: &str) -> Instance {
        Instance {
            definition,
            transform,
            name: name.into(),
            hidden: false,
            locked: false,
            layer: None,
            material: None,
            attrs: Default::default(),
        }
    }

    /// Area-weighted sum of world face points, for comparing placements.
    fn world_area(model: &Model, ents: &Entities, xf: DMat4, acc: &mut (f64, DVec3)) {
        let mut tris = Vec::new();
        face_triangles(model, ents, xf, None, &mut tris);
        for t in tris {
            let [a, b, c] = t.points;
            let area = (b - a).cross(c - a).length() / 2.0;
            acc.0 += area;
            acc.1 += (a + b + c) / 3.0 * area;
        }
        for i in &ents.instances {
            world_area(
                model,
                &model.definitions[i.definition].entities,
                xf * i.transform,
                acc,
            );
        }
    }

    #[test]
    fn exports_and_reads_back() {
        let mut m = super::super::tests::painted_cube();
        // A group holding a scaled, mirrored component, and the painted cube loose.
        let mut box_ents = Entities::default();
        crate::mesh_export::tests::cube(&mut box_ents, DVec3::new(4.0, 2.0, 1.0));
        let comp = m.definitions.insert(Definition {
            name: "Box".into(),
            entities: box_ents,
            ..Default::default()
        });
        let mut group_ents = Entities::default();
        crate::mesh_export::tests::cube(&mut group_ents, DVec3::splat(3.0));
        group_ents.instances.push(inst(
            comp,
            DMat4::from_translation(DVec3::new(0.0, 0.0, 10.0))
                * DMat4::from_scale(DVec3::new(-2.0, 1.0, 1.0)),
            "",
        ));
        let group = m.definitions.insert(Definition {
            name: "Group#1".into(),
            entities: group_ents,
            is_group: true,
            ..Default::default()
        });
        m.root.instances.push(inst(
            group,
            DMat4::from_translation(DVec3::new(100.0, 0.0, 0.0)) * DMat4::from_rotation_z(0.5),
            "Kiosk",
        ));
        let opts = ExportOptions {
            units: Units::Millimeters,
            base_name: "t\u{e9}st".into(),
            ..Default::default()
        };
        let files = to_ifc(&m, &opts);
        let text = String::from_utf8(files.main.clone()).unwrap();
        assert!(text.contains("FILE_SCHEMA(('IFC4'))"));
        assert!(text.contains("IFCBUILDINGELEMENTPROXYTYPE("));
        assert!(text.contains("'Kiosk'"));
        assert!(text.contains("IFCSURFACESTYLE('Red'"));
        // Same model, same file.
        assert_eq!(to_ifc(&m, &opts).main, files.main);
        let (back, report) = import(&files.main, &IfcOptions::default()).unwrap();
        assert!((report.meters_per_unit - 0.001).abs() < 1e-12);
        for d in back.definitions.values() {
            assert!(validate(&d.entities, Checks::TOPOLOGY).is_empty());
        }
        let mut a = (0.0, DVec3::ZERO);
        let mut b = (0.0, DVec3::ZERO);
        world_area(&m, &m.root, DMat4::IDENTITY, &mut a);
        world_area(&back, &back.root, DMat4::IDENTITY, &mut b);
        assert!((a.0 - b.0).abs() < 1e-6 * a.0, "{} {}", a.0, b.0);
        let (ca, cb) = (a.1 / a.0, b.1 / b.0);
        assert!((ca - cb).length() < 1e-4, "{ca} {cb}");
        // The component kept its name; the red top kept its material.
        assert!(
            back.definitions
                .values()
                .any(|d| d.name == "Box" && !d.is_group)
        );
        assert!(back.materials.iter().any(|x| x.name == "Red"));
        let kiosk = back
            .definitions
            .values()
            .flat_map(|d| &d.entities.instances)
            .any(|i| i.name == "Kiosk");
        assert!(kiosk);
    }

    #[test]
    fn guids_are_22_chars() {
        let g = compress_guid(u128::MAX);
        assert_eq!(g.len(), 22);
        assert!(g.starts_with('3'));
        assert_eq!(compress_guid(0), "0".repeat(22));
    }
}
