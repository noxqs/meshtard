//! IFC4 export (Reference View style): one site and building, a storey per storey
//! group, every other visible group or component instance an element named after
//! the instance (or its definition), placed by the translation and rotation of its
//! transform, nested ones aggregated under their parent (`IfcRelAggregates`).
//!
//! - Spatial structure: groups whose Entity Info "Type" (the IFC class the importer
//!   sets) is `IfcBuildingStorey`, found at the root or inside `IfcProject`,
//!   `IfcSite` and `IfcBuilding` groups (and groups or components without faces
//!   holding such groups, like the component an IFC import is placed as), become
//!   storeys (named after the group, elevation from its height); their contents
//!   are contained in them. Elements outside storeys go to the building; without
//!   any storey group there is one storey, "Level 1", holding everything. The
//!   project, site and building take the names and GlobalIds of such groups when
//!   present.
//! - Element classes: the definition's Type when it is one of the building element
//!   classes written (walls, slabs, beams, columns, members, plates, roofs, stairs,
//!   ramps, railings, coverings, curtain walls, footings, piles, chimneys, shading
//!   devices, doors, windows, furniture, furnishing elements, a few distribution
//!   elements; `...StandardCase` as the parent class), else
//!   `IfcBuildingElementProxy`; component types are the class's type entity.
//! - Names and GlobalIds: instance names; a group's definition description is its
//!   GlobalId when it reads as one (the importer puts it there) and is not used yet.
//! - Properties: `Pset_<Class>Common.Reference` = the component's name; quantities
//!   `Qto_<Class>BaseQuantities.NetVolume` for elements whose faces form a solid
//!   (walls, slabs, beams, columns, members, plates, footings, piles, proxies).
//! - Geo-location: a located model writes the site's RefLatitude / RefLongitude;
//!   a map origin or EPSG code (from an IFC import) writes an `IfcMapConversion`
//!   (meters, north as the x axis direction) and north as the context's TrueNorth.
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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

use glam::{DMat3, DMat4, DVec3};

use super::ifc::split_rigid;
use super::step::{quote, real};
use super::{ExportOptions, Files};
use crate::entities::{DefinitionId, Instance, MaterialId};
use crate::mesh_export::{Triangle, Units, face_triangles};
use crate::model::Model;
use crate::solid;

/// A component's representation map and type, per (definition, inherited material).
type Maps = HashMap<(DefinitionId, Option<MaterialId>), Option<(u64, Option<u64>)>>;

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
    /// Type -> elements typed by it.
    typed: BTreeMap<u64, Vec<u64>>,
    /// An SI metre unit, when written.
    metre: Option<u64>,
    /// GlobalIds written.
    used: HashSet<String>,
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
        loop {
            self.guids += 1;
            let a = splitmix(self.seed ^ self.guids);
            let b = splitmix(a);
            let g = compress_guid((u128::from(a) << 64) | u128::from(b));
            if self.used.insert(g.clone()) {
                return quote(&g);
            }
        }
    }

    /// `kept` when it reads as a GlobalId not written yet, else a new one.
    fn guid_or(&mut self, kept: &str) -> String {
        if is_guid(kept) && self.used.insert(kept.to_owned()) {
            quote(kept)
        } else {
            self.guid()
        }
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

    /// The representation map and type (of `class`, when it has one) of a component
    /// (faces of the definition itself, default faces painted `inherited`).
    fn map(
        &mut self,
        def: DefinitionId,
        inherited: Option<MaterialId>,
        class: &ElementClass,
    ) -> Option<(u64, Option<u64>)> {
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
            let ty = class.ty.as_ref().map(|t| {
                let g = self.guid();
                let tail = if class.ty_tail.is_empty() {
                    String::new()
                } else {
                    format!(",{}", class.ty_tail)
                };
                self.add(format!(
                    "{t}({g},$,{},$,$,$,(#{map}),$,${tail})",
                    quote(&d.name)
                ))
            });
            Some((map, ty))
        };
        self.maps.insert((def, inherited), result);
        result
    }

    /// An element of `class` (a proxy when `None`) with GlobalId `guid` (a new one
    /// when it is not usable).
    #[allow(clippy::too_many_arguments)]
    fn element(
        &mut self,
        class: Option<&ElementClass>,
        guid: &str,
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
        let g = self.guid_or(guid);
        let ot = if object_type.is_empty() {
            "$".to_owned()
        } else {
            quote(object_type)
        };
        let proxy;
        let class = match class {
            Some(c) => c,
            None => {
                proxy = ElementClass::proxy();
                &proxy
            }
        };
        let tail = if class.tail.is_empty() {
            String::new()
        } else {
            format!(",{}", class.tail)
        };
        self.add(format!(
            "{}({g},$,{},$,{ot},#{placement},{rep},${tail})",
            class.entity,
            quote(name)
        ))
    }

    /// `Pset_<Class>Common` with Reference `reference` and, when `volume` (cubic
    /// meters) is given, the class's `Qto_` set with NetVolume, on `element`.
    fn properties(
        &mut self,
        element: u64,
        class: &ElementClass,
        reference: Option<&str>,
        volume: Option<f64>,
    ) {
        if let Some(r) = reference.filter(|_| class.has_common_pset()) {
            let prop = self.add(format!(
                "IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER({}),$)",
                quote(r)
            ));
            let g = self.guid();
            let set = self.add(format!(
                "IFCPROPERTYSET({g},$,'Pset_{}Common',$,(#{prop}))",
                class.short
            ));
            let g = self.guid();
            self.add(format!(
                "IFCRELDEFINESBYPROPERTIES({g},$,$,$,(#{element}),#{set})"
            ));
        }
        if let (Some(v), Some(qto)) = (volume, class.volume_qto()) {
            let q = self.add(format!("IFCQUANTITYVOLUME('NetVolume',$,$,{},$)", real(v)));
            let g = self.guid();
            let set = self.add(format!("IFCELEMENTQUANTITY({g},$,'{qto}',$,$,(#{q}))"));
            let g = self.guid();
            self.add(format!(
                "IFCRELDEFINESBYPROPERTIES({g},$,$,$,(#{element}),#{set})"
            ));
        }
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
        let class = element_class(&def.attributes.kind).unwrap_or_else(ElementClass::proxy);
        let mut ty = None;
        // An IFC element's unnamed, untyped components (its mapped items on import)
        // are part of its body, not elements of their own.
        let inline: Vec<&Instance> = if def.is_group && !def.attributes.kind.is_empty() {
            def.entities
                .instances
                .iter()
                .filter(|k| {
                    !k.hidden
                        && layer_visible(model, k.layer)
                        && k.name.is_empty()
                        && model.definitions.get(k.definition).is_some_and(|d| {
                            !d.is_group
                                && !d.is_image
                                && d.attributes.kind.is_empty()
                                && d.entities.instances.is_empty()
                        })
                })
                .collect()
        } else {
            Vec::new()
        };
        let (items, mapped) = if def.is_group || def.is_image {
            let mut tris = Vec::new();
            face_triangles(model, &def.entities, local, material, &mut tris);
            if tris.is_empty() && !inline.is_empty() {
                // Mapped items only; typed by the type of a single one.
                let mut items = Vec::new();
                for k in &inline {
                    let m = k.material.or(material);
                    if let Some((map, t)) = self.map(k.definition, m, &class) {
                        if inline.len() == 1 {
                            ty = t;
                        }
                        let op = self.operator(local * k.transform);
                        items.push(self.add(format!("IFCMAPPEDITEM(#{map},#{op})")));
                    }
                }
                (items, true)
            } else {
                // With faces of its own: one tessellation.
                for k in &inline {
                    let d = &model.definitions[k.definition];
                    let m = k.material.or(material);
                    face_triangles(model, &d.entities, local * k.transform, m, &mut tris);
                }
                (self.face_sets(tris), false)
            }
        } else {
            match self.map(inst.definition, material, &class) {
                Some((map, t)) => {
                    ty = t;
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
        let guid = if def.is_group {
            def.description.as_str()
        } else {
            ""
        };
        let object_type = if def.is_group { "" } else { def.name.as_str() };
        let proxy = self.element(
            Some(&class),
            guid,
            name,
            object_type,
            placement,
            &items,
            mapped,
        );
        if let Some(t) = ty {
            self.typed.entry(t).or_default().push(proxy);
        }
        // Properties: the component's name; the volume of a solid body.
        let volume = (!items.is_empty()
            && def.entities.instances.is_empty()
            && solid::is_solid(&def.entities))
        .then(|| {
            let det = DMat3::from_mat4(local).determinant().abs();
            solid::volume(&def.entities) * det * 0.0254_f64.powi(3)
        });
        // Reference: the component's name, or the type of an IFC element's mapped body.
        let reference = if def.is_group || def.is_image {
            inline
                .first()
                .map(|k| model.definitions[k.definition].name.as_str())
        } else {
            Some(def.name.as_str())
        };
        self.properties(proxy, &class, reference, volume);
        stack.push(inst.definition);
        let kids: Vec<u64> = def
            .entities
            .instances
            .iter()
            .filter(|k| !inline.iter().any(|i| std::ptr::eq(*i, *k)))
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
                self.metre = Some(metre);
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
        if units == Units::Meters {
            self.metre = Some(length);
        }
        let area = self.add("IFCSIUNIT(*,.AREAUNIT.,$,.SQUARE_METRE.)");
        let volume = self.add("IFCSIUNIT(*,.VOLUMEUNIT.,$,.CUBIC_METRE.)");
        let angle = self.add("IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.)");
        self.add(format!(
            "IFCUNITASSIGNMENT((#{length},#{area},#{volume},#{angle}))"
        ))
    }
}

/// IFC4 entity name and the attributes after `Tag` for an element class (Entity
/// Info "Type" as the IFC importer sets it), with its type entity and the
/// attributes after `ElementType`; `None` for classes written as proxies.
/// `...StandardCase` classes are written as their parent class.
fn element_class(kind: &str) -> Option<ElementClass> {
    let kind = kind.strip_suffix("StandardCase").unwrap_or(kind);
    let nine = ".NOTDEFINED.";
    let (entity, tail, ty, ty_tail): (&str, &str, Option<&str>, &str) = match kind {
        "IfcWall" => ("IFCWALL", nine, Some("IFCWALLTYPE"), nine),
        "IfcSlab" => ("IFCSLAB", nine, Some("IFCSLABTYPE"), nine),
        "IfcBeam" => ("IFCBEAM", nine, Some("IFCBEAMTYPE"), nine),
        "IfcColumn" => ("IFCCOLUMN", nine, Some("IFCCOLUMNTYPE"), nine),
        "IfcMember" => ("IFCMEMBER", nine, Some("IFCMEMBERTYPE"), nine),
        "IfcPlate" => ("IFCPLATE", nine, Some("IFCPLATETYPE"), nine),
        "IfcRoof" => ("IFCROOF", nine, Some("IFCROOFTYPE"), nine),
        "IfcStair" => ("IFCSTAIR", nine, Some("IFCSTAIRTYPE"), nine),
        "IfcRamp" => ("IFCRAMP", nine, Some("IFCRAMPTYPE"), nine),
        "IfcRailing" => ("IFCRAILING", nine, Some("IFCRAILINGTYPE"), nine),
        "IfcCovering" => ("IFCCOVERING", nine, Some("IFCCOVERINGTYPE"), nine),
        "IfcCurtainWall" => ("IFCCURTAINWALL", nine, Some("IFCCURTAINWALLTYPE"), nine),
        "IfcFooting" => ("IFCFOOTING", nine, Some("IFCFOOTINGTYPE"), nine),
        "IfcPile" => ("IFCPILE", ".NOTDEFINED.,$", Some("IFCPILETYPE"), nine),
        "IfcChimney" => ("IFCCHIMNEY", nine, Some("IFCCHIMNEYTYPE"), nine),
        "IfcShadingDevice" => ("IFCSHADINGDEVICE", nine, Some("IFCSHADINGDEVICETYPE"), nine),
        "IfcBuildingElementProxy" => (
            "IFCBUILDINGELEMENTPROXY",
            nine,
            Some("IFCBUILDINGELEMENTPROXYTYPE"),
            nine,
        ),
        "IfcDoor" => (
            "IFCDOOR",
            "$,$,.NOTDEFINED.,.NOTDEFINED.,$",
            Some("IFCDOORTYPE"),
            ".NOTDEFINED.,.NOTDEFINED.,$,$",
        ),
        "IfcWindow" => (
            "IFCWINDOW",
            "$,$,.NOTDEFINED.,.NOTDEFINED.,$",
            Some("IFCWINDOWTYPE"),
            ".NOTDEFINED.,.NOTDEFINED.,$,$",
        ),
        "IfcFurniture" => (
            "IFCFURNITURE",
            nine,
            Some("IFCFURNITURETYPE"),
            ".NOTDEFINED.,.NOTDEFINED.",
        ),
        "IfcFurnishingElement" => (
            "IFCFURNISHINGELEMENT",
            "",
            Some("IFCFURNISHINGELEMENTTYPE"),
            "",
        ),
        "IfcSanitaryTerminal" => (
            "IFCSANITARYTERMINAL",
            nine,
            Some("IFCSANITARYTERMINALTYPE"),
            nine,
        ),
        "IfcLightFixture" => ("IFCLIGHTFIXTURE", nine, Some("IFCLIGHTFIXTURETYPE"), nine),
        "IfcFlowTerminal"
        | "IfcFlowSegment"
        | "IfcFlowFitting"
        | "IfcFlowController"
        | "IfcDistributionElement" => {
            // IFC2X3 classes still in IFC4 (no predefined type, abstract types).
            let upper = kind.to_ascii_uppercase();
            return Some(ElementClass {
                entity: upper,
                tail: String::new(),
                ty: None,
                ty_tail: String::new(),
                short: kind[3..].to_owned(),
            });
        }
        _ => return None,
    };
    let short = kind[3..].to_owned();
    Some(ElementClass {
        entity: entity.to_owned(),
        tail: tail.to_owned(),
        ty: ty.map(str::to_owned),
        ty_tail: ty_tail.to_owned(),
        short,
    })
}

/// How an element class is written (see [`element_class`]).
#[derive(Debug, Clone, PartialEq)]
struct ElementClass {
    entity: String,
    /// Attributes after `Tag`, comma separated (empty: none).
    tail: String,
    ty: Option<String>,
    /// Attributes after the type's `ElementType`.
    ty_tail: String,
    /// The class without `Ifc`, e.g. `Wall`.
    short: String,
}

impl ElementClass {
    fn proxy() -> Self {
        element_class("IfcBuildingElementProxy").expect("proxy class")
    }

    /// `Pset_<class>Common` exists with a `Reference` property.
    fn has_common_pset(&self) -> bool {
        !matches!(
            self.short.as_str(),
            "Furniture"
                | "FurnishingElement"
                | "FlowTerminal"
                | "FlowSegment"
                | "FlowFitting"
                | "FlowController"
                | "DistributionElement"
                | "LightFixture"
        )
    }

    /// The `Qto_` set with a `NetVolume` quantity, when there is one.
    fn volume_qto(&self) -> Option<String> {
        match self.short.as_str() {
            "BuildingElementProxy" => Some("Qto_BuildingElementProxyQuantities".into()),
            "Wall" | "Slab" | "Beam" | "Column" | "Member" | "Plate" | "Footing" | "Pile" => {
                Some(format!("Qto_{}BaseQuantities", self.short))
            }
            _ => None,
        }
    }
}

/// Whether `s` reads as an IFC GlobalId (22 characters of its base 64 alphabet,
/// the first 0..3).
fn is_guid(s: &str) -> bool {
    s.len() == 22
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'$')
        && matches!(s.as_bytes()[0], b'0'..=b'3')
}

/// Decimal degrees as an `IfcCompoundPlaneAngleMeasure` (degrees, minutes, seconds,
/// millionths of a second, all with the angle's sign).
fn compound_angle(deg: f64) -> String {
    let sign = if deg < 0.0 { -1 } else { 1 };
    let micro_total = (deg.abs() * 3_600_000_000.0).round() as i64;
    let d = micro_total / 3_600_000_000;
    let m = micro_total / 60_000_000 % 60;
    let s = micro_total / 1_000_000 % 60;
    let u = micro_total % 1_000_000;
    format!("({},{},{},{})", sign * d, sign * m, sign * s, sign * u)
}

/// The spatial structure found in a model (see [`to_ifc`]).
#[derive(Default)]
struct Spatial<'m> {
    /// Name and GlobalId (definition description) of the project, site and
    /// building groups found, when any.
    project: Option<(&'m str, &'m str)>,
    site: Option<(&'m str, &'m str)>,
    building: Option<(&'m str, &'m str)>,
    /// Storey groups: instance, world transform of its parent, inherited material.
    storeys: Vec<(&'m Instance, DMat4, Option<MaterialId>)>,
    /// Other instances outside storeys.
    elements: Vec<(&'m Instance, DMat4, Option<MaterialId>)>,
    /// Loose faces of the root and the project / site / building groups (world).
    loose: Vec<Triangle>,
}

const ABOVE_STOREY: [&str; 3] = ["IfcProject", "IfcSite", "IfcBuilding"];

/// The instances of `ents` (placed by `xf`) with space groups replaced by their
/// contents (spaces are not written; what they hold is contained in the storey).
fn expand_spaces<'m>(
    model: &'m Model,
    ents: &'m crate::entities::Entities,
    xf: DMat4,
    inherited: Option<MaterialId>,
    out: &mut Vec<(&'m Instance, DMat4, Option<MaterialId>)>,
    depth: usize,
) {
    for k in &ents.instances {
        match model.definitions.get(k.definition) {
            Some(d) if d.attributes.kind == "IfcSpace" && !k.hidden && depth < 8 => {
                let m = k.material.or(inherited);
                expand_spaces(model, &d.entities, xf * k.transform, m, out, depth + 1);
            }
            _ => out.push((k, xf, inherited)),
        }
    }
}

/// Walks `ents` (placed by `xf`) for storey groups: project, site and building
/// groups, and groups or components without faces of their own holding one, are
/// looked into; everything else is an element.
fn find_spatial<'m>(
    model: &'m Model,
    ents: &'m crate::entities::Entities,
    xf: DMat4,
    inherited: Option<MaterialId>,
    out: &mut Spatial<'m>,
    depth: usize,
) {
    for inst in &ents.instances {
        if inst.hidden || !layer_visible(model, inst.layer) {
            continue;
        }
        let Some(def) = model.definitions.get(inst.definition) else {
            continue;
        };
        let kind = def.attributes.kind.as_str();
        let material = inst.material.or(inherited);
        let world = xf * inst.transform;
        let label = |d: &'m crate::model::Definition| -> (&'m str, &'m str) {
            let name = if inst.name.is_empty() {
                d.name.as_str()
            } else {
                inst.name.as_str()
            };
            (name, d.description.as_str())
        };
        let holds_spatial = def.entities.faces.is_empty()
            && def.entities.instances.iter().any(|k| {
                model.definitions.get(k.definition).is_some_and(|d| {
                    let k = d.attributes.kind.as_str();
                    ABOVE_STOREY.contains(&k) || k == "IfcBuildingStorey"
                })
            });
        if kind == "IfcBuildingStorey" {
            out.storeys.push((inst, xf, inherited));
        } else if depth < 16
            && (ABOVE_STOREY.contains(&kind)
                || kind == "IfcSpace"
                || (kind.is_empty() && holds_spatial))
        {
            let slot = match kind {
                "IfcProject" => Some(&mut out.project),
                "IfcSite" => Some(&mut out.site),
                "IfcBuilding" => Some(&mut out.building),
                _ => None,
            };
            if let Some(slot) = slot
                && slot.is_none()
            {
                *slot = Some(label(def));
            }
            face_triangles(model, &def.entities, world, material, &mut out.loose);
            find_spatial(model, &def.entities, world, material, out, depth + 1);
        } else {
            out.elements.push((inst, xf, inherited));
        }
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
        used: HashSet::new(),
        metre: None,
    };
    let mut spatial = Spatial::default();
    find_spatial(model, &model.root, DMat4::IDENTITY, None, &mut spatial, 0);
    let origin = w.point(DVec3::ZERO);
    w.wcs = w.add(format!("IFCAXIS2PLACEMENT3D(#{origin},$,$)"));
    let geo = model.geo;
    let true_north = if geo.north_angle != 0.0 {
        let a = geo.north_angle.to_radians();
        let d = w.add(format!(
            "IFCDIRECTION(({},{}))",
            real(a.sin()),
            real(a.cos())
        ));
        format!("#{d}")
    } else {
        "$".to_owned()
    };
    let ctx = w.add(format!(
        "IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#{},{true_north})",
        w.wcs
    ));
    w.body = w.add(format!(
        "IFCGEOMETRICREPRESENTATIONSUBCONTEXT('Body','Model',*,*,*,*,#{ctx},$,.MODEL_VIEW.,$)"
    ));
    let units = w.units(opts.units);
    let (name, kept) = spatial.project.unwrap_or((&opts.base_name, ""));
    let g = w.guid_or(kept);
    let project = w.add(format!(
        "IFCPROJECT({g},$,{},$,$,$,$,(#{ctx}),#{units})",
        quote(name)
    ));
    if geo.map_origin != DVec3::ZERO || geo.epsg != 0 {
        let metre = match w.metre {
            Some(m) => m,
            None => w.add("IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.)"),
        };
        let crs_name = if geo.epsg != 0 {
            format!("EPSG:{}", geo.epsg)
        } else {
            "Unknown".to_owned()
        };
        let crs = w.add(format!(
            "IFCPROJECTEDCRS({},$,$,$,$,$,#{metre})",
            quote(&crs_name)
        ));
        let a = geo.north_angle.to_radians();
        let o = geo.map_origin;
        w.add(format!(
            "IFCMAPCONVERSION(#{ctx},#{crs},{},{},{},{},{},{})",
            real(o.x),
            real(o.y),
            real(o.z),
            real(a.cos()),
            real(a.sin()),
            real(0.0254 / opts.units.per_inch())
        ));
    }
    let site_pl = w.add(format!("IFCLOCALPLACEMENT($,#{})", w.wcs));
    let (name, kept) = spatial.site.unwrap_or(("Site", ""));
    let g = w.guid_or(kept);
    let (lat, lon) = if geo.located {
        (
            compound_angle(model.shadows.latitude),
            compound_angle(model.shadows.longitude),
        )
    } else {
        ("$".to_owned(), "$".to_owned())
    };
    let site = w.add(format!(
        "IFCSITE({g},$,{},$,$,#{site_pl},$,$,.ELEMENT.,{lat},{lon},$,$,$)",
        quote(name)
    ));
    let building_pl = w.add(format!("IFCLOCALPLACEMENT(#{site_pl},#{})", w.wcs));
    let (name, kept) = spatial.building.unwrap_or(("Building", ""));
    let g = w.guid_or(kept);
    let building = w.add(format!(
        "IFCBUILDING({g},$,{},$,$,#{building_pl},$,$,.ELEMENT.,$,$,$)",
        quote(name)
    ));
    for (whole, part) in [(project, site), (site, building)] {
        let g = w.guid();
        w.add(format!("IFCRELAGGREGATES({g},$,$,$,#{whole},(#{part}))"));
    }
    let mut storeys = Vec::new();
    let mut stack = Vec::new();
    // Storey groups and their contents.
    for &(inst, parent, inherited) in &spatial.storeys {
        let def = &model.definitions[inst.definition];
        let world = parent * inst.transform;
        let (rigid, _) = split_rigid(world);
        let axis = w.axis2(rigid);
        let pl = w.add(format!("IFCLOCALPLACEMENT(#{building_pl},#{axis})"));
        let name = if inst.name.is_empty() {
            &def.name
        } else {
            &inst.name
        };
        let g = w.guid_or(&def.description);
        let storey = w.add(format!(
            "IFCBUILDINGSTOREY({g},$,{},$,$,#{pl},$,$,.ELEMENT.,{})",
            quote(name),
            real(rigid.w_axis.z * w.scale)
        ));
        storeys.push(storey);
        let material = inst.material.or(inherited);
        let mut contained = Vec::new();
        let mut tris = Vec::new();
        face_triangles(model, &def.entities, DMat4::IDENTITY, material, &mut tris);
        if !tris.is_empty() {
            let sets = w.face_sets(tris);
            let ep = w.add(format!("IFCLOCALPLACEMENT(#{pl},#{})", w.wcs));
            contained.push(w.element(None, "", name, "", ep, &sets, false));
        }
        stack.push(inst.definition);
        let mut members = Vec::new();
        expand_spaces(model, &def.entities, world, material, &mut members, 0);
        for (k, parent, inherited) in members {
            if let Some(e) = w.instance(k, parent, rigid, pl, inherited, &mut stack) {
                contained.push(e);
            }
        }
        stack.pop();
        if !contained.is_empty() {
            let g = w.guid();
            w.add(format!(
                "IFCRELCONTAINEDINSPATIALSTRUCTURE({g},$,$,$,({}),#{storey})",
                refs(&contained)
            ));
        }
    }
    // Everything else: in the one storey, or in the building next to storeys.
    let (container, container_pl) = if storeys.is_empty() {
        let pl = w.add(format!("IFCLOCALPLACEMENT(#{building_pl},#{})", w.wcs));
        let g = w.guid();
        let storey = w.add(format!(
            "IFCBUILDINGSTOREY({g},$,'Level 1',$,$,#{pl},$,$,.ELEMENT.,0.)"
        ));
        storeys.push(storey);
        (storey, pl)
    } else {
        (building, building_pl)
    };
    let g = w.guid();
    w.add(format!(
        "IFCRELAGGREGATES({g},$,$,$,#{building},({}))",
        refs(&storeys)
    ));
    let mut top = Vec::new();
    // Loose faces of the root and of the project, site and building groups.
    let mut tris = std::mem::take(&mut spatial.loose);
    face_triangles(model, &model.root, DMat4::IDENTITY, None, &mut tris);
    if !tris.is_empty() {
        let sets = w.face_sets(tris);
        let pl = w.add(format!("IFCLOCALPLACEMENT(#{container_pl},#{})", w.wcs));
        top.push(w.element(None, "", &opts.base_name, "", pl, &sets, false));
    }
    for &(inst, parent, inherited) in &spatial.elements {
        if let Some(p) = w.instance(
            inst,
            parent,
            DMat4::IDENTITY,
            container_pl,
            inherited,
            &mut stack,
        ) {
            top.push(p);
        }
    }
    if !top.is_empty() {
        let g = w.guid();
        w.add(format!(
            "IFCRELCONTAINEDINSPATIALSTRUCTURE({g},$,$,$,({}),#{container})",
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
    fn imported_classes_storeys_and_georeference_round_trip() {
        let body = "\
#7=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#6,$);
#8=IFCPROJECTEDCRS('EPSG:25832',$,$,$,$,$,#3);
#9=IFCMAPCONVERSION(#7,#8,1000.,2000.,0.,0.,1.,$);
#33=IFCBUILDINGSTOREY('2StoreyGuid00000000002',$,'Level 2',$,$,#34,$,$,.ELEMENT.,6000.);
#34=IFCLOCALPLACEMENT(#21,#35);
#35=IFCAXIS2PLACEMENT3D(#36,$,$);
#36=IFCCARTESIANPOINT((0.,0.,6000.));
#37=IFCRELAGGREGATES('r4',$,$,$,#20,(#33));
#58=IFCDIRECTION((0.,0.,1.));
#50=IFCWALL('2WallGuid0000000000001',$,'Wall A',$,$,#100,#52,$,$);
#52=IFCPRODUCTDEFINITIONSHAPE($,$,(#55));
#55=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#56));
#56=IFCEXTRUDEDAREASOLID(#57,$,#58,2500.);
#57=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,4000.,200.);
#60=IFCDOOR('2DoorGuid0000000000001',$,'Door 1',$,$,#34,#52,$,$,$,$,$,$);
#61=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,300.,300.);
#62=IFCEXTRUDEDAREASOLID(#61,$,#58,3000.);
#63=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#62));
#64=IFCREPRESENTATIONMAP(#6,#63);
#65=IFCCOLUMNTYPE('t',$,'Col Type',$,$,$,(#64),$,$,.COLUMN.);
#66=IFCCARTESIANTRANSFORMATIONOPERATOR3D($,$,#5,$,$);
#67=IFCMAPPEDITEM(#64,#66);
#68=IFCSHAPEREPRESENTATION($,'Body','MappedRepresentation',(#67));
#69=IFCPRODUCTDEFINITIONSHAPE($,$,(#68));
#70=IFCCOLUMN('2ColGuid00000000000001',$,'C1',$,$,#34,#69,$,$);
#71=IFCRELCONTAINEDINSPATIALSTRUCTURE('c',$,$,$,(#50),#30);
#72=IFCRELCONTAINEDINSPATIALSTRUCTURE('c2',$,$,$,(#60,#70),#33);
";
        let text = crate::formats::ifc::tests::file(body)
            .replace(
                "IFCBUILDINGSTOREY('st'",
                "IFCBUILDINGSTOREY('2StoreyGuid00000000001'",
            )
            .replace(
                "#10=IFCSITE('s',$,'Site',$,$,#11,$,$,.ELEMENT.,$,$,$,$,$);",
                "#10=IFCSITE('s',$,'Site',$,$,#11,$,$,.ELEMENT.,(48,8,0,0),(11,34,30,0),$,$,$);",
            );
        let (m, _) = import(text.as_bytes(), &IfcOptions::default()).unwrap();
        assert!(m.geo.located && m.geo.epsg == 25832);
        let opts = ExportOptions {
            units: Units::Millimeters,
            base_name: "house".into(),
            ..Default::default()
        };
        let out = String::from_utf8(to_ifc(&m, &opts).main).unwrap();
        assert_eq!(to_ifc(&m, &opts).main, out.as_bytes());
        assert!(out.contains("IFCWALL('2WallGuid0000000000001',$,'Wall A'"));
        assert!(out.contains("IFCDOOR('2DoorGuid0000000000001',$,'Door 1'"));
        assert!(out.contains("IFCCOLUMNTYPE("));
        assert!(out.contains("IFCBUILDINGSTOREY('2StoreyGuid00000000001',$,'Level 1'"));
        assert!(out.contains("IFCBUILDINGSTOREY('2StoreyGuid00000000002',$,'Level 2'"));
        assert!(!out.contains("IFCBUILDINGELEMENTPROXY("));
        assert!(out.contains("'Pset_ColumnCommon'"));
        assert!(out.contains("IFCIDENTIFIER('Col Type')"));
        assert!(out.contains("'Qto_WallBaseQuantities'"));
        // 4000 x 200 x 2500 mm.
        assert!(out.contains(&format!("'NetVolume',$,$,{},$", real(2.0))));
        assert!(out.contains("(48,8,0,0),(11,34,30,0)"));
        assert!(out.contains("IFCPROJECTEDCRS('EPSG:25832'"));
        assert!(out.contains("IFCMAPCONVERSION("));
        // Read back: the same classes, storeys, names, ids and location.
        let (back, _) = import(out.as_bytes(), &IfcOptions::default()).unwrap();
        let kinds = |m: &Model, kind: &str| {
            m.definitions
                .values()
                .filter(|d| d.attributes.kind == kind)
                .map(|d| d.description.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(kinds(&back, "IfcWall"), ["2WallGuid0000000000001"]);
        assert_eq!(kinds(&back, "IfcDoor"), ["2DoorGuid0000000000001"]);
        assert_eq!(kinds(&back, "IfcColumn"), ["2ColGuid00000000000001"]);
        let mut storeys = kinds(&back, "IfcBuildingStorey");
        storeys.sort();
        assert_eq!(
            storeys,
            ["2StoreyGuid00000000001", "2StoreyGuid00000000002"]
        );
        assert!(back.geo.located);
        assert!((back.shadows.latitude - m.shadows.latitude).abs() < 1e-9);
        assert!((back.geo.north_angle - 90.0).abs() < 1e-9);
        assert!((back.geo.map_origin - m.geo.map_origin).length() < 1e-6);
        let mut a = (0.0, DVec3::ZERO);
        let mut b = (0.0, DVec3::ZERO);
        world_area(&m, &m.root, DMat4::IDENTITY, &mut a);
        world_area(&back, &back.root, DMat4::IDENTITY, &mut b);
        assert!((a.0 - b.0).abs() < 1e-6 * a.0, "{} {}", a.0, b.0);
        assert!((a.1 / a.0 - b.1 / b.0).length() < 1e-4);
    }

    #[test]
    fn guids_are_22_chars() {
        let g = compress_guid(u128::MAX);
        assert_eq!(g.len(), 22);
        assert!(g.starts_with('3'));
        assert_eq!(compress_guid(0), "0".repeat(22));
        assert!(is_guid(&g) && !is_guid("w") && !is_guid(&"4".repeat(22)));
        assert_eq!(compound_angle(-87.6394), "(-87,-38,-21,-840000)");
    }
}
