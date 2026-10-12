//! IFC (ISO 16739) import, geometry only: no Classifier, property sets or
//! quantities. Reads IFC2X3, IFC4 and IFC4X3 STEP files ([`super::step`]) and
//! `.ifczip` archives. Export is in [`super::ifc_export`].
//!
//! - The spatial structure and the products under it (`IfcRelAggregates`,
//!   `IfcRelContainedInSpatialStructure`, `IfcRelNests`) become nested groups: the
//!   instance is named after the entity's Name (its IFC class when it has none), the
//!   definition has the IFC class as its Entity Info "Type" and the GlobalId as its
//!   description. Groups with nothing below them are left out; products in no
//!   relationship go under the project. Spaces come in hidden.
//! - Placements: `IfcLocalPlacement` chains (`IfcAxis2Placement3D` / `2D`); group
//!   transforms are the placements relative to the parent's. Grid and linear
//!   placements count as skipped and place at the parent.
//! - Body geometry (the `Body` representation, else the first 3D one):
//!   `IfcFacetedBrep` (with voids), `IfcShellBasedSurfaceModel`,
//!   `IfcFaceBasedSurfaceModel`, `IfcTriangulatedFaceSet`, `IfcPolygonalFaceSet`
//!   (faces with voids), `IfcExtrudedAreaSolid` (rectangle, rounded and hollow
//!   rectangle, circle, hollow circle, ellipse, I / L / T / U shapes, arbitrary closed
//!   profiles with voids from polylines, indexed poly curves with arcs and composite
//!   curves of polylines and trimmed circles, derived profiles), `IfcSweptDiskSolid`
//!   (tube along a polyline, mitered), `IfcMappedItem` (a component per
//!   `IfcRepresentationMap`, named after the type that owns it) and
//!   `IfcBooleanResult` (the first operand only: openings and clippings are not
//!   cut; unions keep both). Curves are 24 segments per full circle.
//! - Colors: `IfcStyledItem` surface styles (shading or rendering color and
//!   transparency) on items, `IfcIndexedColourMap` per-face colors, and the color of
//!   the product's (or its type's) material through `IfcRelAssociatesMaterial` and
//!   `IfcMaterialDefinitionRepresentation`, which paints the group (faces without a
//!   style of their own).
//! - Units: the project's `IfcUnitAssignment` length unit (SI prefixes and conversion
//!   based units such as feet), and its plane angle unit for trimmed circles.
//! - Openings (`IfcOpeningElement`) are not imported. Everything else that is not
//!   read is counted by class in [`IfcReport::skipped`].
//! - Faces become kernel faces as in mesh import ([`mesh_import::to_entities`]):
//!   welded, coplanar neighbors merged when asked. Faces with holes are triangulated
//!   first; merging joins them again.
//! - Determinism: entities are visited in instance-number order; maps keyed by
//!   number are only looked up, never iterated for output.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::f64::consts::TAU;
use std::io::Read;

use glam::{DMat3, DMat4, DVec2, DVec3};

use super::step::{self, Entity, File, Value};
use crate::entities::{DefinitionId, Entities, Instance};
use crate::mesh_import::{self, Mesh, MeshOptions, Polygon};
use crate::model::{Definition, Material, Model};

/// Segments of a full circle.
const CIRCLE_SEGMENTS: usize = 24;
/// Segments around a swept disk tube.
const TUBE_SEGMENTS: usize = 12;
const MAX_DEPTH: usize = 32;

/// IFC import choices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IfcOptions {
    pub merge_coplanar: bool,
}

impl Default for IfcOptions {
    fn default() -> Self {
        Self {
            merge_coplanar: true,
        }
    }
}

/// What an import read and left out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IfcReport {
    /// `FILE_SCHEMA`, e.g. `IFC4`.
    pub schema: String,
    /// The file's length unit in meters.
    pub meters_per_unit: f64,
    /// Products (spatial elements included) read.
    pub products: usize,
    /// Products that brought geometry.
    pub with_geometry: usize,
    /// Representation items read, by IFC class.
    pub items: BTreeMap<String, usize>,
    /// Entities not read, by IFC class (representation items, profiles, curves,
    /// placements, boolean second operands, openings).
    pub skipped: BTreeMap<String, usize>,
}

impl IfcReport {
    /// One line for the status bar: products and what was skipped.
    pub fn summary(&self) -> String {
        let mut s = format!(
            "{} of {} objects with geometry",
            self.with_geometry, self.products
        );
        if !self.skipped.is_empty() {
            let mut list: Vec<(&String, &usize)> = self.skipped.iter().collect();
            list.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let parts: Vec<String> = list
                .iter()
                .take(4)
                .map(|(k, n)| format!("{n} {k}"))
                .collect();
            s += &format!("; skipped {}", parts.join(", "));
            if list.len() > 4 {
                s += ", ...";
            }
        }
        s
    }
}

/// The IFC file inside `bytes`: the bytes themselves, or the first `.ifc` entry of
/// an `.ifczip` archive.
fn unzip(bytes: &[u8]) -> Result<std::borrow::Cow<'_, [u8]>, String> {
    if !bytes.starts_with(b"PK") {
        return Ok(bytes.into());
    }
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let name = zip
        .file_names()
        .filter_map(Result::ok)
        .map(|n| n.into_owned())
        .filter(|n| n.to_ascii_lowercase().ends_with(".ifc"))
        .min()
        .ok_or("no .ifc file in the archive")?;
    let mut out = Vec::new();
    zip.by_name(&name)
        .map_err(|e| e.to_string())?
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    Ok(out.into())
}

/// Reads an IFC (or IFCZIP) file into a model: the project's groups at its root.
pub fn import(bytes: &[u8], opts: &IfcOptions) -> Result<(Model, IfcReport), String> {
    let data = unzip(bytes)?;
    let file = step::parse(&data)?;
    if !file.schema.iter().any(|s| s.starts_with("IFC")) && !file.schema.is_empty() {
        return Err(format!(
            "not an IFC file (schema {})",
            file.schema.join(", ")
        ));
    }
    Ok(Reader::new(&file, opts).run())
}

/// `IFCWALLSTANDARDCASE` as `IfcWallStandardCase` (known words; the rest
/// capitalized as one word).
pub fn class_name(upper: &str) -> String {
    const WORDS: &[&str] = &[
        "ACCESSORY",
        "ACTUATOR",
        "ADVANCED",
        "AIR",
        "ALARM",
        "ALIGNMENT",
        "ANCHOR",
        "ANNOTATION",
        "APPLIANCE",
        "ARBITRARY",
        "AREA",
        "ASSEMBLY",
        "AUDIO",
        "BAR",
        "BASED",
        "BEAM",
        "BEARING",
        "BLOCK",
        "BOILER",
        "BOOLEAN",
        "BOUNDED",
        "BOUNDING",
        "BOX",
        "BREP",
        "BRIDGE",
        "BSPLINE",
        "BUILDING",
        "BURNER",
        "CABLE",
        "CARRIER",
        "CARTESIAN",
        "CASE",
        "CHAMBER",
        "CHILLER",
        "CHIMNEY",
        "CIRCLE",
        "CIRCULAR",
        "CIVIL",
        "CLIPPING",
        "CLOSED",
        "COIL",
        "COLUMN",
        "COMMUNICATIONS",
        "COMPOSITE",
        "COMPRESSOR",
        "CONDENSER",
        "CONE",
        "CONTROL",
        "CONTROLLER",
        "CONVERSION",
        "COOLER",
        "COOLING",
        "COURSE",
        "COVERING",
        "CURTAIN",
        "CURVE",
        "CYLINDER",
        "DAMPER",
        "DEEP",
        "DEF",
        "DERIVED",
        "DEVICE",
        "DISCRETE",
        "DISK",
        "DISTRIBUTION",
        "DOOR",
        "DUCT",
        "EARTHWORKS",
        "ELECTRIC",
        "ELEMENT",
        "ELEMENTED",
        "ELLIPSE",
        "ENERGY",
        "ENGINE",
        "EQUIPMENT",
        "EVAPORATIVE",
        "EXCHANGER",
        "EXTERNAL",
        "EXTRUDED",
        "FACE",
        "FACETED",
        "FACILITY",
        "FAN",
        "FASTENER",
        "FEATURE",
        "FILL",
        "FILTER",
        "FIRE",
        "FITTING",
        "FIXED",
        "FIXTURE",
        "FLIGHT",
        "FLOW",
        "FOOTING",
        "FOUNDATION",
        "FURNISHING",
        "FURNITURE",
        "GENERATOR",
        "GEOGRAPHIC",
        "GEOMETRIC",
        "GRID",
        "HALF",
        "HEAT",
        "HEATER",
        "HOLLOW",
        "HUMIDIFIER",
        "IMPACT",
        "INDEXED",
        "INTERCEPTOR",
        "IRREGULAR",
        "ITEM",
        "JUNCTION",
        "KERB",
        "KNOTS",
        "LAMP",
        "LIGHT",
        "LINE",
        "LINEAR",
        "LOCAL",
        "LOOP",
        "MAPPED",
        "MARINE",
        "MECHANICAL",
        "MEDICAL",
        "MEMBER",
        "MESH",
        "METER",
        "MODEL",
        "MOORING",
        "MOTOR",
        "MOVING",
        "NETWORK",
        "OPEN",
        "OPENING",
        "OUTLET",
        "PARAMETER",
        "PART",
        "PAVEMENT",
        "PILE",
        "PIPE",
        "PLACEMENT",
        "PLANE",
        "PLATE",
        "POINT",
        "POLYGONAL",
        "POLYLINE",
        "POLYLOOP",
        "PORT",
        "PROFILE",
        "PROJECT",
        "PROTECTIVE",
        "PROXY",
        "PUMP",
        "PYRAMID",
        "RAIL",
        "RAILING",
        "RAILWAY",
        "RAMP",
        "RATIONAL",
        "RECTANGLE",
        "REFERENCE",
        "REINFORCING",
        "RESULT",
        "REVOLVED",
        "RIGHT",
        "ROAD",
        "ROOF",
        "ROUNDED",
        "SANITARY",
        "SECTIONED",
        "SEGMENT",
        "SENSOR",
        "SET",
        "SHADING",
        "SHAPE",
        "SHELL",
        "SIGN",
        "SIGNAL",
        "SILENCER",
        "SITE",
        "SLAB",
        "SOLAR",
        "SOLID",
        "SPACE",
        "SPATIAL",
        "SPHERE",
        "SPINE",
        "STACK",
        "STAIR",
        "STANDARD",
        "STORAGE",
        "STOREY",
        "SUPPRESSION",
        "SURFACE",
        "SWEPT",
        "SWITCHING",
        "SYSTEM",
        "TANK",
        "TAPERED",
        "TENDON",
        "TERMINAL",
        "TESSELLATED",
        "TOWER",
        "TRACK",
        "TRANSFORMER",
        "TRANSPORT",
        "TREATMENT",
        "TRIANGULATED",
        "TRIMMED",
        "TUBE",
        "TUNNEL",
        "UNIT",
        "UNITARY",
        "VALUE",
        "VALVE",
        "VEHICLE",
        "VIRTUAL",
        "VISUAL",
        "VOIDS",
        "WALL",
        "WASTE",
        "WINDOW",
        "WITH",
        "ZONE",
    ];
    let Some(mut rest) = upper.strip_prefix("IFC") else {
        return upper.to_owned();
    };
    let mut out = String::from("Ifc");
    let cap = |w: &str| {
        let mut s = w[..1].to_owned();
        s.push_str(&w[1..].to_ascii_lowercase());
        s
    };
    let mut tail = String::new();
    while !rest.is_empty() {
        let best = WORDS
            .iter()
            .filter(|w| rest.starts_with(**w))
            .max_by_key(|w| w.len());
        match best {
            Some(w) => {
                if !tail.is_empty() {
                    out += &cap(&tail);
                    tail.clear();
                }
                out += &cap(w);
                rest = &rest[w.len()..];
            }
            None => {
                let c = rest.chars().next().unwrap_or('_');
                tail.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    if !tail.is_empty() {
        out += &cap(&tail);
    }
    out
}

/// Geometry gathered for one group or component, in file units.
#[derive(Default)]
struct Geo {
    mesh: Mesh,
    /// Mapped items: definition, transform (model units), styled material.
    instances: Vec<(DefinitionId, DMat4, Option<usize>)>,
}

impl Geo {
    /// Adds a planar face; with holes it is triangulated (merging joins it again).
    fn face(&mut self, outer: &[DVec3], holes: &[Vec<DVec3>], material: Option<usize>) {
        let outer = dedup(outer);
        if outer.len() < 3 {
            return;
        }
        let holes: Vec<Vec<DVec3>> = holes
            .iter()
            .map(|h| dedup(h))
            .filter(|h| h.len() >= 3)
            .collect();
        if holes.is_empty() {
            self.polygon(&outer, material);
            return;
        }
        let normal = newell(&outer);
        let Some(n) = normal.try_normalize() else {
            return;
        };
        let u = n.any_orthonormal_vector();
        let v = n.cross(u);
        let mut all: Vec<DVec3> = outer.clone();
        let mut starts = Vec::new();
        for h in &holes {
            starts.push(all.len());
            all.extend(h.iter().copied());
        }
        let flat: Vec<f64> = all.iter().flat_map(|p| [p.dot(u), p.dot(v)]).collect();
        let Ok(idx) = earcutr::earcut(&flat, &starts, 2) else {
            self.polygon(&outer, material);
            return;
        };
        for t in idx.as_chunks::<3>().0 {
            let [a, b, c] = t.map(|k| all[k]);
            let tri = if (b - a).cross(c - a).dot(n) < 0.0 {
                [a, c, b]
            } else {
                [a, b, c]
            };
            self.polygon(&tri, material);
        }
    }

    fn polygon(&mut self, points: &[DVec3], material: Option<usize>) {
        let first = self.mesh.positions.len();
        self.mesh.positions.extend_from_slice(points);
        self.mesh.polygons.push(Polygon {
            vertices: (first..first + points.len()).collect(),
            material,
            uvs: Vec::new(),
        });
    }
}

/// `points` without consecutive repeats or a closing repeat of the first.
fn dedup(points: &[DVec3]) -> Vec<DVec3> {
    let mut out: Vec<DVec3> = Vec::with_capacity(points.len());
    for &p in points {
        if out.last().is_none_or(|q| (*q - p).length_squared() > 1e-18) {
            out.push(p);
        }
    }
    while out.len() > 1 && (out[0] - out[out.len() - 1]).length_squared() <= 1e-18 {
        out.pop();
    }
    out
}

fn newell(points: &[DVec3]) -> DVec3 {
    let mut n = DVec3::ZERO;
    for (i, p) in points.iter().enumerate() {
        let q = points[(i + 1) % points.len()];
        n += DVec3::new(
            (p.y - q.y) * (p.z + q.z),
            (p.z - q.z) * (p.x + q.x),
            (p.x - q.x) * (p.y + q.y),
        );
    }
    n
}

fn area2(points: &[DVec2]) -> f64 {
    let mut a = 0.0;
    for (i, p) in points.iter().enumerate() {
        let q = points[(i + 1) % points.len()];
        a += p.perp_dot(q);
    }
    a
}

/// Points on a circle arc of radius `r` from angle `a0` to `a1` (both included),
/// 24 segments per full turn.
fn arc(center: DVec2, rx: f64, ry: f64, a0: f64, a1: f64) -> Vec<DVec2> {
    let n = ((a1 - a0).abs() / TAU * CIRCLE_SEGMENTS as f64)
        .ceil()
        .max(1.0) as usize;
    (0..=n)
        .map(|k| {
            let a = a0 + (a1 - a0) * k as f64 / n as f64;
            center + DVec2::new(rx * a.cos(), ry * a.sin())
        })
        .collect()
}

/// A full circle (or ellipse) without the closing point.
fn circle(rx: f64, ry: f64) -> Vec<DVec2> {
    let mut c = arc(DVec2::ZERO, rx, ry, 0.0, TAU);
    c.pop();
    c
}

/// The arc through three points (2D or 3D), tessellated; a straight line when they
/// are collinear.
fn arc3(a: DVec3, b: DVec3, c: DVec3) -> Vec<DVec3> {
    let n = (b - a).cross(c - a);
    if n.length_squared() < 1e-20 {
        return vec![a, c];
    }
    // Circumcenter.
    let ab = b - a;
    let ac = c - a;
    let center = a
        + (ac.length_squared() * n.cross(ab) + ab.length_squared() * ac.cross(n))
            / (2.0 * n.length_squared());
    let nz = n.normalize();
    let u = (a - center).normalize();
    let v = nz.cross(u);
    let r = (a - center).length();
    let angle = |p: DVec3| {
        let d = p - center;
        let t = d.dot(v).atan2(d.dot(u));
        if t < 0.0 { t + TAU } else { t }
    };
    let end = angle(c);
    let pts = arc(DVec2::ZERO, r, r, 0.0, end);
    pts.into_iter()
        .map(|p| center + u * p.x + v * p.y)
        .collect()
}

fn mat_from_axes(x: DVec3, y: DVec3, z: DVec3, origin: DVec3) -> DMat4 {
    DMat4::from_cols(
        x.extend(0.0),
        y.extend(0.0),
        z.extend(0.0),
        origin.extend(1.0),
    )
}

/// IFC's FirstProjAxis: `arg` (or X, or Y when `z` is X) made perpendicular to `z`.
fn first_proj_axis(z: DVec3, arg: Option<DVec3>) -> DVec3 {
    let v = arg.unwrap_or(if (z - DVec3::X).length() > 1e-9 {
        DVec3::X
    } else {
        DVec3::Y
    });
    (v - z * v.dot(z))
        .try_normalize()
        .unwrap_or_else(|| z.any_orthonormal_vector())
}

/// IFC's SecondProjAxis: `arg` (or Y) made perpendicular to `z` and `x`; keeps a
/// mirrored frame mirrored.
fn second_proj_axis(z: DVec3, x: DVec3, arg: Option<DVec3>) -> DVec3 {
    let v = arg.unwrap_or(DVec3::Y);
    let t = v - z * v.dot(z);
    (t - x * t.dot(x))
        .try_normalize()
        .unwrap_or_else(|| z.cross(x))
}

struct Reader<'a> {
    f: &'a File,
    opts: MeshOptions,
    /// Radians per file plane angle unit.
    angle: f64,
    model: Model,
    report: IfcReport,
    materials_by_key: HashMap<(String, [u8; 3], u64), usize>,
    /// Representation item -> styled material.
    item_style: HashMap<u64, usize>,
    /// Tessellated face set -> its `IfcIndexedColourMap`.
    colour_maps: HashMap<u64, u64>,
    /// `IfcMaterial` -> its styled representation items.
    material_reps: HashMap<u64, Vec<u64>>,
    /// Object or type -> relating material.
    product_material: HashMap<u64, u64>,
    /// Object -> its type.
    product_type: HashMap<u64, u64>,
    /// Representation map -> name of the type that owns it.
    map_names: HashMap<u64, String>,
    maps: HashMap<u64, Option<DefinitionId>>,
    placements: HashMap<u64, DMat4>,
    groups: usize,
}

impl<'a> Reader<'a> {
    fn new(f: &'a File, opts: &IfcOptions) -> Self {
        let mut r = Reader {
            f,
            opts: MeshOptions {
                inches_per_unit: 1.0 / 0.0254,
                y_up: false,
                merge_coplanar: opts.merge_coplanar,
            },
            angle: 1.0,
            model: Model::default(),
            report: IfcReport {
                schema: f.schema.join(", "),
                meters_per_unit: 1.0,
                ..Default::default()
            },
            materials_by_key: HashMap::new(),
            item_style: HashMap::new(),
            colour_maps: HashMap::new(),
            material_reps: HashMap::new(),
            product_material: HashMap::new(),
            product_type: HashMap::new(),
            map_names: HashMap::new(),
            maps: HashMap::new(),
            placements: HashMap::new(),
            groups: 0,
        };
        r.units();
        r.index();
        r
    }

    fn skip(&mut self, class: &str) {
        *self.report.skipped.entry(class_name(class)).or_default() += 1;
    }

    fn read(&mut self, class: &str) {
        *self.report.items.entry(class_name(class)).or_default() += 1;
    }

    fn get(&self, v: &Value) -> Option<(u64, &'a Entity)> {
        let f: &'a File = self.f;
        f.deref(v)
    }

    // ---- units ----

    /// Meters (length) or radians (plane angle) per unit of a unit entity.
    fn unit_factor(&self, v: &Value, depth: usize) -> Option<(String, f64)> {
        let (_, e) = self.get(v)?;
        match e.class.as_str() {
            "IFCSIUNIT" => {
                let kind = e.arg(1).as_enum()?.to_owned();
                let prefix = match e.arg(2).as_enum() {
                    Some("EXA") => 1e18,
                    Some("PETA") => 1e15,
                    Some("TERA") => 1e12,
                    Some("GIGA") => 1e9,
                    Some("MEGA") => 1e6,
                    Some("KILO") => 1e3,
                    Some("HECTO") => 1e2,
                    Some("DECA") => 1e1,
                    Some("DECI") => 1e-1,
                    Some("CENTI") => 1e-2,
                    Some("MILLI") => 1e-3,
                    Some("MICRO") => 1e-6,
                    Some("NANO") => 1e-9,
                    Some("PICO") => 1e-12,
                    _ => 1.0,
                };
                Some((kind, prefix))
            }
            "IFCCONVERSIONBASEDUNIT" | "IFCCONVERSIONBASEDUNITWITHOFFSET" if depth < 4 => {
                let kind = e.arg(1).as_enum()?.to_owned();
                let (_, m) = self.get(e.arg(3))?;
                let value = m.arg(0).as_f64()?;
                let base = self.unit_factor(m.arg(1), depth + 1).map_or(1.0, |u| u.1);
                Some((kind, value * base))
            }
            _ => None,
        }
    }

    fn units(&mut self) {
        let f = self.f;
        let Some(project) = f.entities.values().find(|e| e.class == "IFCPROJECT") else {
            return;
        };
        let Some((_, ua)) = self.get(project.arg(8)) else {
            return;
        };
        for u in ua.arg(0).list() {
            match self.unit_factor(u, 0) {
                Some((k, m)) if k == "LENGTHUNIT" && m > 0.0 => {
                    self.report.meters_per_unit = m;
                    self.opts.inches_per_unit = m / 0.0254;
                }
                Some((k, a)) if k == "PLANEANGLEUNIT" && a > 0.0 => self.angle = a,
                _ => {}
            }
        }
    }

    // ---- indexes ----

    fn index(&mut self) {
        let f = self.f;
        for (&id, e) in &f.entities {
            match e.class.as_str() {
                "IFCSTYLEDITEM" => {
                    if let Some(item) = e.arg(0).as_ref_id()
                        && let Some(m) = self.style_material(e.arg(1), 0)
                    {
                        self.item_style.insert(item, m);
                    }
                }
                "IFCINDEXEDCOLOURMAP" => {
                    if let Some(set) = e.arg(0).as_ref_id() {
                        self.colour_maps.insert(set, id);
                    }
                }
                "IFCMATERIALDEFINITIONREPRESENTATION" => {
                    if let Some(m) = e.arg(3).as_ref_id() {
                        let items: Vec<u64> = e
                            .arg(2)
                            .refs()
                            .into_iter()
                            .filter_map(|r| f.get(r))
                            .flat_map(|r| r.arg(3).refs())
                            .collect();
                        self.material_reps.entry(m).or_default().extend(items);
                    }
                }
                "IFCRELASSOCIATESMATERIAL" => {
                    if let Some(m) = e.arg(5).as_ref_id() {
                        for o in e.arg(4).refs() {
                            self.product_material.entry(o).or_insert(m);
                        }
                    }
                }
                "IFCRELDEFINESBYTYPE" => {
                    if let Some(t) = e.arg(5).as_ref_id() {
                        for o in e.arg(4).refs() {
                            self.product_type.entry(o).or_insert(t);
                        }
                    }
                }
                _ => {}
            }
            // Types own representation maps (IfcTypeProduct.RepresentationMaps).
            if e.class.ends_with("TYPE") || e.class.ends_with("STYLE") {
                let name = e.arg(2).as_str().unwrap_or("").trim();
                for m in e.arg(6).refs() {
                    if f.get(m).is_some_and(|x| x.class == "IFCREPRESENTATIONMAP") {
                        let n = if name.is_empty() {
                            class_name(&e.class)
                        } else {
                            name.to_owned()
                        };
                        self.map_names.entry(m).or_insert(n);
                    }
                }
            }
        }
    }

    // ---- styles and materials ----

    /// The model material of a color, made once per name, color and opacity.
    fn material(&mut self, name: &str, color: [u8; 3], opacity: f64) -> usize {
        let name = name.trim();
        let opacity = opacity.clamp(0.0, 1.0);
        let key = (name.to_owned(), color, opacity.to_bits());
        if let Some(&m) = self.materials_by_key.get(&key) {
            return m;
        }
        let base = if name.is_empty() {
            format!("Color_{:02X}{:02X}{:02X}", color[0], color[1], color[2])
        } else {
            name.to_owned()
        };
        let mut unique = base.clone();
        let mut n = 1;
        while self.model.materials.iter().any(|m| m.name == unique) {
            n += 1;
            unique = format!("{base} {n}");
        }
        self.model.materials.push(Material {
            name: unique,
            color,
            opacity,
            texture: None,
        });
        let id = self.model.materials.len() - 1;
        self.materials_by_key.insert(key, id);
        id
    }

    fn colour(&self, v: &Value) -> Option<[u8; 3]> {
        let (_, c) = self.get(v)?;
        (c.class == "IFCCOLOURRGB")
            .then(|| [1, 2, 3].map(|k| super::unit_byte(c.arg(k).as_f64().unwrap_or(0.8))))
    }

    /// The material of a styled item's styles (presentation style assignments or
    /// surface styles).
    fn style_material(&mut self, styles: &Value, depth: usize) -> Option<usize> {
        if depth > 4 {
            return None;
        }
        for s in styles.refs() {
            let Some(e) = self.f.get(s) else { continue };
            match e.class.as_str() {
                "IFCPRESENTATIONSTYLEASSIGNMENT" => {
                    if let Some(m) = self.style_material(e.arg(0), depth + 1) {
                        return Some(m);
                    }
                }
                "IFCSURFACESTYLE" => {
                    let name = e.arg(0).as_str().unwrap_or("").to_owned();
                    for el in e.arg(2).refs() {
                        let Some(x) = self.f.get(el) else { continue };
                        if matches!(
                            x.class.as_str(),
                            "IFCSURFACESTYLESHADING" | "IFCSURFACESTYLERENDERING"
                        ) && let Some(color) = self.colour(x.arg(0))
                        {
                            let transparency = x.arg(1).as_f64().unwrap_or(0.0);
                            return Some(self.material(&name, color, 1.0 - transparency));
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// The color of an `IfcMaterial` or the first colored material of a layer,
    /// profile or constituent set or list.
    fn material_of(&mut self, id: u64, depth: usize) -> Option<usize> {
        if depth > 6 {
            return None;
        }
        let e = self.f.get(id)?;
        let next: Vec<u64> = match e.class.as_str() {
            "IFCMATERIAL" => {
                let items = self.material_reps.get(&id).cloned().unwrap_or_default();
                for it in items {
                    if let Some(si) = self.f.get(it).filter(|x| x.class == "IFCSTYLEDITEM")
                        && let Some(m) = self.style_material(si.arg(1), 0)
                    {
                        return Some(m);
                    }
                }
                return None;
            }
            "IFCMATERIALLAYERSETUSAGE" | "IFCMATERIALPROFILESETUSAGE" => e.arg(0).refs(),
            "IFCMATERIALPROFILESETUSAGETAPERING" => e.arg(0).refs(),
            "IFCMATERIALLAYERSET" | "IFCMATERIALLIST" => e.arg(0).refs(),
            "IFCMATERIALLAYER" | "IFCMATERIALLAYERWITHOFFSETS" => e.arg(0).refs(),
            "IFCMATERIALPROFILESET" | "IFCMATERIALCONSTITUENTSET" => e.arg(2).refs(),
            "IFCMATERIALPROFILE" | "IFCMATERIALCONSTITUENT" => e.arg(2).refs(),
            _ => Vec::new(),
        };
        next.into_iter()
            .find_map(|n| self.material_of(n, depth + 1))
    }

    /// The material painting a product: its own association, else its type's.
    fn product_paint(&mut self, id: u64) -> Option<usize> {
        let m = self.product_material.get(&id).copied().or_else(|| {
            let t = self.product_type.get(&id)?;
            self.product_material.get(t).copied()
        })?;
        self.material_of(m, 0)
    }

    // ---- points and placements ----

    fn point(&self, v: &Value) -> Option<DVec3> {
        let (_, e) = self.get(v)?;
        let c = e.arg(0).numbers();
        match (e.class.as_str(), c.len()) {
            ("IFCCARTESIANPOINT", 2) => Some(DVec3::new(c[0], c[1], 0.0)),
            ("IFCCARTESIANPOINT", 3) => Some(DVec3::new(c[0], c[1], c[2])),
            _ => None,
        }
    }

    fn direction(&self, v: &Value) -> Option<DVec3> {
        let (_, e) = self.get(v)?;
        let c = e.arg(0).numbers();
        let d = match (e.class.as_str(), c.len()) {
            ("IFCDIRECTION", 2) => DVec3::new(c[0], c[1], 0.0),
            ("IFCDIRECTION", 3) => DVec3::new(c[0], c[1], c[2]),
            _ => return None,
        };
        d.try_normalize()
    }

    /// An `IfcAxis2Placement3D` / `2D` as a matrix (identity when missing).
    fn axis2(&mut self, v: &Value) -> DMat4 {
        let Some((_, e)) = self.get(v) else {
            return DMat4::IDENTITY;
        };
        let origin = self.point(e.arg(0)).unwrap_or(DVec3::ZERO);
        match e.class.as_str() {
            "IFCAXIS2PLACEMENT3D" => {
                let z = self.direction(e.arg(1)).unwrap_or(DVec3::Z);
                let x = first_proj_axis(z, self.direction(e.arg(2)));
                mat_from_axes(x, z.cross(x), z, origin)
            }
            "IFCAXIS2PLACEMENT2D" => {
                let x = self
                    .direction(e.arg(1))
                    .map(|d| DVec3::new(d.x, d.y, 0.0))
                    .and_then(DVec3::try_normalize)
                    .unwrap_or(DVec3::X);
                mat_from_axes(x, DVec3::Z.cross(x), DVec3::Z, origin)
            }
            other => {
                let other = other.to_owned();
                self.skip(&other);
                DMat4::IDENTITY
            }
        }
    }

    /// An object placement in file world coordinates.
    fn placement(&mut self, v: &Value, depth: usize) -> DMat4 {
        let Some((id, e)) = self.get(v) else {
            return DMat4::IDENTITY;
        };
        if let Some(m) = self.placements.get(&id) {
            return *m;
        }
        let m = match e.class.as_str() {
            "IFCLOCALPLACEMENT" if depth < MAX_DEPTH => {
                let rel = self.placement(e.arg(0), depth + 1);
                rel * self.axis2(e.arg(1))
            }
            other => {
                let other = other.to_owned();
                self.skip(&other);
                DMat4::IDENTITY
            }
        };
        self.placements.insert(id, m);
        m
    }

    /// An `IfcCartesianTransformationOperator3D` (uniform or not) or `2D`.
    fn operator(&self, v: &Value) -> DMat4 {
        let Some((_, e)) = self.get(v) else {
            return DMat4::IDENTITY;
        };
        let origin = self.point(e.arg(2)).unwrap_or(DVec3::ZERO);
        let s = e.arg(3).as_f64().unwrap_or(1.0);
        match e.class.as_str() {
            "IFCCARTESIANTRANSFORMATIONOPERATOR3D"
            | "IFCCARTESIANTRANSFORMATIONOPERATOR3DNONUNIFORM" => {
                let z = self.direction(e.arg(4)).unwrap_or(DVec3::Z);
                let x = first_proj_axis(z, self.direction(e.arg(0)));
                let y = second_proj_axis(z, x, self.direction(e.arg(1)));
                let (s2, s3) = if e.class.ends_with("NONUNIFORM") {
                    (
                        e.arg(5).as_f64().unwrap_or(s),
                        e.arg(6).as_f64().unwrap_or(s),
                    )
                } else {
                    (s, s)
                };
                mat_from_axes(x * s, y * s2, z * s3, origin)
            }
            "IFCCARTESIANTRANSFORMATIONOPERATOR2D"
            | "IFCCARTESIANTRANSFORMATIONOPERATOR2DNONUNIFORM" => {
                let x = self
                    .direction(e.arg(0))
                    .map(|d| DVec3::new(d.x, d.y, 0.0))
                    .and_then(DVec3::try_normalize)
                    .unwrap_or(DVec3::X);
                let y0 = self.direction(e.arg(1)).unwrap_or(DVec3::Z.cross(x));
                let y = second_proj_axis(DVec3::Z, x, Some(y0));
                let s2 = if e.class.ends_with("NONUNIFORM") {
                    e.arg(4).as_f64().unwrap_or(s)
                } else {
                    s
                };
                mat_from_axes(x * s, y * s2, DVec3::Z, origin)
            }
            _ => DMat4::IDENTITY,
        }
    }

    /// A point list (`IfcCartesianPointList2D` / `3D`).
    fn point_list(&self, v: &Value) -> Vec<DVec3> {
        let Some((_, e)) = self.get(v) else {
            return Vec::new();
        };
        e.arg(0)
            .list()
            .iter()
            .map(|p| {
                let c = p.numbers();
                DVec3::new(
                    c.first().copied().unwrap_or(0.0),
                    c.get(1).copied().unwrap_or(0.0),
                    c.get(2).copied().unwrap_or(0.0),
                )
            })
            .collect()
    }

    // ---- curves and profiles ----

    /// A curve as points (2D curves at z = 0).
    fn curve(&mut self, v: &Value, depth: usize) -> Option<Vec<DVec3>> {
        let (_, e) = self.get(v)?;
        if depth > MAX_DEPTH {
            return None;
        }
        let out = match e.class.as_str() {
            "IFCPOLYLINE" => e
                .arg(0)
                .list()
                .iter()
                .filter_map(|p| self.point(p))
                .collect(),
            "IFCINDEXEDPOLYCURVE" => {
                let pts = self.point_list(e.arg(0));
                let at = |i: &Value| {
                    i.as_i64()
                        .and_then(|k| usize::try_from(k - 1).ok())
                        .and_then(|k| pts.get(k).copied())
                };
                match e.arg(1) {
                    Value::Null => pts.clone(),
                    segs => {
                        let mut out: Vec<DVec3> = Vec::new();
                        for s in segs.list() {
                            let idx: Vec<DVec3> = s.list().iter().filter_map(at).collect();
                            let is_arc = matches!(s, Value::Typed(n, _) if n == "IFCARCINDEX");
                            let piece = if is_arc && idx.len() == 3 {
                                arc3(idx[0], idx[1], idx[2])
                            } else {
                                idx
                            };
                            out.extend(piece);
                        }
                        out
                    }
                }
            }
            "IFCCOMPOSITECURVE" | "IFCCOMPOSITECURVEONSURFACE" => {
                let mut out: Vec<DVec3> = Vec::new();
                for s in e.arg(0).refs() {
                    let Some(seg) = self.f.get(s) else { continue };
                    if seg.class != "IFCCOMPOSITECURVESEGMENT"
                        && seg.class != "IFCREPARAMETRISEDCOMPOSITECURVESEGMENT"
                    {
                        let c = seg.class.clone();
                        self.skip(&c);
                        continue;
                    }
                    let same = seg.arg(1).as_enum() != Some("F");
                    let Some(mut piece) = self.curve(seg.arg(2), depth + 1) else {
                        continue;
                    };
                    if !same {
                        piece.reverse();
                    }
                    out.extend(piece);
                }
                out
            }
            "IFCTRIMMEDCURVE" => self.trimmed(e, depth)?,
            "IFCCIRCLE" | "IFCELLIPSE" => {
                let m = self.axis2(e.arg(0));
                let rx = e.arg(1).as_f64()?;
                let ry = if e.class == "IFCELLIPSE" {
                    e.arg(2).as_f64()?
                } else {
                    rx
                };
                circle(rx, ry)
                    .into_iter()
                    .map(|p| m.transform_point3(p.extend(0.0)))
                    .collect()
            }
            other => {
                let other = other.to_owned();
                self.skip(&other);
                return None;
            }
        };
        let out = dedup(&out);
        (out.len() >= 2).then_some(out)
    }

    fn trimmed(&mut self, e: &Entity, depth: usize) -> Option<Vec<DVec3>> {
        let (_, basis) = self.get(e.arg(0))?;
        let sense = e.arg(3).as_enum() != Some("F");
        let cartesian_first = e.arg(4).as_enum() == Some("CARTESIAN");
        let trim = |r: &Self, v: &Value| -> (Option<DVec3>, Option<f64>) {
            let mut p = None;
            let mut t = None;
            for x in v.list() {
                match x {
                    Value::Typed(n, inner) if n == "IFCPARAMETERVALUE" => t = inner.as_f64(),
                    Value::Real(_) | Value::Int(_) => t = x.as_f64(),
                    other => p = p.or_else(|| r.point(other)),
                }
            }
            (p, t)
        };
        let (p1, t1) = trim(self, e.arg(1));
        let (p2, t2) = trim(self, e.arg(2));
        match basis.class.as_str() {
            "IFCCIRCLE" | "IFCELLIPSE" => {
                let m = self.axis2(basis.arg(0));
                let rx = basis.arg(1).as_f64()?;
                let ry = if basis.class == "IFCELLIPSE" {
                    basis.arg(2).as_f64()?
                } else {
                    rx
                };
                let inv = m.inverse();
                let ang = |p: Option<DVec3>, t: Option<f64>| -> Option<f64> {
                    let from_point = p.map(|p| {
                        let l = inv.transform_point3(p);
                        (l.y / ry).atan2(l.x / rx)
                    });
                    let from_param = t.map(|t| t * self.angle);
                    if cartesian_first {
                        from_point.or(from_param)
                    } else {
                        from_param.or(from_point)
                    }
                };
                let a0 = ang(p1, t1)?;
                let mut a1 = ang(p2, t2)?;
                if sense {
                    while a1 <= a0 + 1e-12 {
                        a1 += TAU;
                    }
                } else {
                    while a1 >= a0 - 1e-12 {
                        a1 -= TAU;
                    }
                }
                Some(
                    arc(DVec2::ZERO, rx, ry, a0, a1)
                        .into_iter()
                        .map(|p| m.transform_point3(p.extend(0.0)))
                        .collect(),
                )
            }
            "IFCLINE" => {
                let origin = self.point(basis.arg(0))?;
                let (_, vec) = self.get(basis.arg(1))?;
                let dir = self.direction(vec.arg(0))? * vec.arg(1).as_f64().unwrap_or(1.0);
                let at = |p: Option<DVec3>, t: Option<f64>| {
                    if cartesian_first {
                        p.or(t.map(|t| origin + dir * t))
                    } else {
                        t.map(|t| origin + dir * t).or(p)
                    }
                };
                let (a, b) = (at(p1, t1)?, at(p2, t2)?);
                Some(if sense { vec![a, b] } else { vec![b, a] })
            }
            "IFCPOLYLINE" => {
                let pts = self.curve(e.arg(0), depth + 1)?;
                let at = |p: Option<DVec3>, t: Option<f64>| -> Option<f64> {
                    t.or_else(|| {
                        let p = p?;
                        (0..pts.len())
                            .min_by(|&i, &j| {
                                (pts[i] - p).length().total_cmp(&(pts[j] - p).length())
                            })
                            .map(|i| i as f64)
                    })
                };
                let (a, b) = (at(p1, t1)?, at(p2, t2)?);
                let (lo, hi) = (a.min(b), a.max(b));
                let point_at = |t: f64| {
                    let i = (t.floor().max(0.0) as usize).min(pts.len() - 1);
                    let j = (i + 1).min(pts.len() - 1);
                    pts[i].lerp(pts[j], t - i as f64)
                };
                let mut out = vec![point_at(lo)];
                out.extend(
                    pts.iter()
                        .enumerate()
                        .filter(|(k, _)| (*k as f64) > lo && (*k as f64) < hi)
                        .map(|(_, p)| *p),
                );
                out.push(point_at(hi));
                if (a > b) == sense {
                    out.reverse();
                }
                Some(out)
            }
            other => {
                let other = other.to_owned();
                self.skip(&other);
                None
            }
        }
    }

    /// A closed curve as a loop (no repeated closing point), in the xy plane.
    fn closed_loop(&mut self, v: &Value) -> Option<Vec<DVec2>> {
        let pts = self.curve(v, 0)?;
        let pts: Vec<DVec2> = dedup(&pts).into_iter().map(|p| p.truncate()).collect();
        (pts.len() >= 3).then_some(pts)
    }

    /// A profile's loops in its plane: the outline counter-clockwise first, then
    /// holes clockwise.
    fn profile(&mut self, v: &Value, depth: usize) -> Option<Vec<Vec<DVec2>>> {
        let (_, e) = self.get(v)?;
        let class = e.class.as_str();
        let num = |i: usize| e.arg(i).as_f64();
        let rect = |w: f64, h: f64| {
            vec![
                DVec2::new(-w / 2.0, -h / 2.0),
                DVec2::new(w / 2.0, -h / 2.0),
                DVec2::new(w / 2.0, h / 2.0),
                DVec2::new(-w / 2.0, h / 2.0),
            ]
        };
        let pts = |list: &[(f64, f64)]| list.iter().map(|&(x, y)| DVec2::new(x, y)).collect();
        let mut loops: Vec<Vec<DVec2>> = match class {
            "IFCRECTANGLEPROFILEDEF" => vec![rect(num(3)?, num(4)?)],
            "IFCROUNDEDRECTANGLEPROFILEDEF" => {
                let (w, h) = (num(3)?, num(4)?);
                let r = num(5).unwrap_or(0.0).clamp(0.0, w.min(h) / 2.0);
                if r <= 0.0 {
                    vec![rect(w, h)]
                } else {
                    let (cx, cy) = (w / 2.0 - r, h / 2.0 - r);
                    let q = TAU / 4.0;
                    let mut l = Vec::new();
                    for (k, c) in [(cx, -cy), (cx, cy), (-cx, cy), (-cx, -cy)]
                        .into_iter()
                        .enumerate()
                    {
                        let a0 = -q + q * k as f64;
                        l.extend(arc(DVec2::new(c.0, c.1), r, r, a0, a0 + q));
                    }
                    vec![l]
                }
            }
            "IFCRECTANGLEHOLLOWPROFILEDEF" => {
                let (w, h, t) = (num(3)?, num(4)?, num(5)?);
                vec![rect(w, h), rect(w - 2.0 * t, h - 2.0 * t)]
            }
            "IFCCIRCLEPROFILEDEF" => vec![circle(num(3)?, num(3)?)],
            "IFCCIRCLEHOLLOWPROFILEDEF" => {
                let (r, t) = (num(3)?, num(4)?);
                vec![circle(r, r), circle(r - t, r - t)]
            }
            "IFCELLIPSEPROFILEDEF" => vec![circle(num(3)?, num(4)?)],
            "IFCISHAPEPROFILEDEF" => {
                let (b, h, tw, tf) = (num(3)?, num(4)?, num(5)?, num(6)?);
                let (x, y, w) = (b / 2.0, h / 2.0, tw / 2.0);
                vec![pts(&[
                    (-x, -y),
                    (x, -y),
                    (x, -y + tf),
                    (w, -y + tf),
                    (w, y - tf),
                    (x, y - tf),
                    (x, y),
                    (-x, y),
                    (-x, y - tf),
                    (-w, y - tf),
                    (-w, -y + tf),
                    (-x, -y + tf),
                ])]
            }
            "IFCLSHAPEPROFILEDEF" => {
                let d = num(3)?;
                let w = num(4).unwrap_or(d);
                let t = num(5)?;
                let (x, y) = (w / 2.0, d / 2.0);
                vec![pts(&[
                    (-x, -y),
                    (x, -y),
                    (x, -y + t),
                    (-x + t, -y + t),
                    (-x + t, y),
                    (-x, y),
                ])]
            }
            "IFCUSHAPEPROFILEDEF" => {
                let (d, w, tw, tf) = (num(3)?, num(4)?, num(5)?, num(6)?);
                let (x, y) = (w / 2.0, d / 2.0);
                vec![pts(&[
                    (-x, -y),
                    (x, -y),
                    (x, -y + tf),
                    (-x + tw, -y + tf),
                    (-x + tw, y - tf),
                    (x, y - tf),
                    (x, y),
                    (-x, y),
                ])]
            }
            "IFCTSHAPEPROFILEDEF" => {
                let (d, w, tw, tf) = (num(3)?, num(4)?, num(5)?, num(6)?);
                let (x, y, h) = (w / 2.0, d / 2.0, tw / 2.0);
                vec![pts(&[
                    (-h, -y),
                    (h, -y),
                    (h, y - tf),
                    (x, y - tf),
                    (x, y),
                    (-x, y),
                    (-x, y - tf),
                    (-h, y - tf),
                ])]
            }
            "IFCARBITRARYCLOSEDPROFILEDEF" | "IFCARBITRARYPROFILEDEFWITHVOIDS" => {
                let mut l = vec![self.closed_loop(e.arg(2))?];
                if class == "IFCARBITRARYPROFILEDEFWITHVOIDS" {
                    for h in e.arg(3).list() {
                        if let Some(h) = self.closed_loop(h) {
                            l.push(h);
                        }
                    }
                }
                // No position: the curves are in the profile plane.
                return Some(orient(l));
            }
            "IFCDERIVEDPROFILEDEF" if depth < 8 => {
                let parent = self.profile(e.arg(2), depth + 1)?;
                let m = self.operator(e.arg(3));
                let l = parent
                    .into_iter()
                    .map(|l| {
                        l.into_iter()
                            .map(|p| m.transform_point3(p.extend(0.0)).truncate())
                            .collect()
                    })
                    .collect();
                return Some(orient(l));
            }
            other => {
                let other = other.to_owned();
                self.skip(&other);
                return None;
            }
        };
        // Parameterized profiles: placed by Position (optional in IFC4).
        let m = self.axis2(e.arg(2));
        for l in &mut loops {
            for p in l.iter_mut() {
                *p = m.transform_point3(p.extend(0.0)).truncate();
            }
        }
        Some(orient(loops))
    }

    // ---- representation items ----

    /// Adds item `id` (in the coordinates `xf` maps to the group's) to `geo`.
    fn item(&mut self, id: u64, xf: DMat4, inherited: Option<usize>, geo: &mut Geo, depth: usize) {
        let Some(e) = self.f.get(id) else { return };
        if depth > MAX_DEPTH {
            return;
        }
        let style = self.item_style.get(&id).copied().or(inherited);
        let class = e.class.as_str();
        let done = match class {
            "IFCFACETEDBREP" | "IFCFACETEDBREPWITHVOIDS" => {
                let mut shells = e.arg(0).refs();
                shells.extend(e.arg(1).refs());
                for s in shells {
                    self.shell(s, xf, style, geo);
                }
                true
            }
            "IFCSHELLBASEDSURFACEMODEL" | "IFCFACEBASEDSURFACEMODEL" => {
                for s in e.arg(0).refs() {
                    self.shell(s, xf, style, geo);
                }
                true
            }
            "IFCCLOSEDSHELL" | "IFCOPENSHELL" | "IFCCONNECTEDFACESET" => {
                self.shell(id, xf, style, geo);
                true
            }
            "IFCTRIANGULATEDFACESET" | "IFCTRIANGULATEDIRREGULARNETWORK" => {
                self.tessellated(id, e, xf, style, geo, true)
            }
            "IFCPOLYGONALFACESET" => self.tessellated(id, e, xf, style, geo, false),
            "IFCEXTRUDEDAREASOLID" | "IFCEXTRUDEDAREASOLIDTAPERED" => {
                self.extrusion(e, xf, style, geo)
            }
            "IFCSWEPTDISKSOLID" | "IFCSWEPTDISKSOLIDPOLYGONAL" => self.tube(e, xf, style, geo),
            "IFCMAPPEDITEM" => self.mapped(e, xf, style, geo, depth),
            "IFCBOOLEANRESULT" | "IFCBOOLEANCLIPPINGRESULT" => {
                let union = e.arg(0).as_enum() == Some("UNION");
                let mut ops = vec![e.arg(1)];
                if union {
                    ops.push(e.arg(2));
                } else if let Some((_, second)) = self.get(e.arg(2)) {
                    let c = second.class.clone();
                    self.skip(&c);
                }
                for op in ops {
                    if let Some(o) = op.as_ref_id() {
                        self.item(o, xf, style, geo, depth + 1);
                    }
                }
                true
            }
            _ => false,
        };
        if done {
            self.read(class);
        } else {
            self.skip(class);
        }
    }

    /// The faces of a shell or connected face set.
    fn shell(&mut self, id: u64, xf: DMat4, style: Option<usize>, geo: &mut Geo) {
        let Some(e) = self.f.get(id) else { return };
        let style = self.item_style.get(&id).copied().or(style);
        for face in e.arg(0).refs() {
            let Some(fe) = self.f.get(face) else { continue };
            let fstyle = self.item_style.get(&face).copied().or(style);
            let mut outer: Option<Vec<DVec3>> = None;
            let mut bounds: Vec<Vec<DVec3>> = Vec::new();
            for b in fe.arg(0).refs() {
                let Some(be) = self.f.get(b) else { continue };
                let Some((_, lp)) = self.get(be.arg(0)) else {
                    continue;
                };
                if lp.class != "IFCPOLYLOOP" {
                    let c = lp.class.clone();
                    self.skip(&c);
                    continue;
                }
                let mut pts: Vec<DVec3> = lp
                    .arg(0)
                    .list()
                    .iter()
                    .filter_map(|p| self.point(p))
                    .map(|p| xf.transform_point3(p))
                    .collect();
                if be.arg(1).as_enum() == Some("F") {
                    pts.reverse();
                }
                if be.class == "IFCFACEOUTERBOUND" && outer.is_none() {
                    outer = Some(pts);
                } else {
                    bounds.push(pts);
                }
            }
            let outer = match outer {
                Some(o) => o,
                None => {
                    // No outer bound marked: the largest loop.
                    let Some(k) = (0..bounds.len()).max_by(|&a, &b| {
                        newell(&bounds[a])
                            .length()
                            .total_cmp(&newell(&bounds[b]).length())
                    }) else {
                        continue;
                    };
                    bounds.remove(k)
                }
            };
            geo.face(&outer, &bounds, fstyle);
        }
    }

    /// `IfcTriangulatedFaceSet` (`triangles`) or `IfcPolygonalFaceSet`.
    fn tessellated(
        &mut self,
        id: u64,
        e: &Entity,
        xf: DMat4,
        style: Option<usize>,
        geo: &mut Geo,
        triangles: bool,
    ) -> bool {
        let coords: Vec<DVec3> = self
            .point_list(e.arg(0))
            .into_iter()
            .map(|p| xf.transform_point3(p))
            .collect();
        // PnIndex: the face indices point into it, it into the coordinates.
        let pn: Vec<i64> = e
            .arg(if triangles { 4 } else { 3 })
            .list()
            .iter()
            .filter_map(Value::as_i64)
            .collect();
        let at = |k: &Value| -> Option<DVec3> {
            let k = k.as_i64()?;
            let k = if pn.is_empty() {
                k
            } else {
                *pn.get(usize::try_from(k - 1).ok()?)?
            };
            coords.get(usize::try_from(k - 1).ok()?).copied()
        };
        let loop_of = |l: &Value| -> Vec<DVec3> { l.list().iter().filter_map(at).collect() };
        // Per-face colors.
        let colours: Vec<Option<usize>> = match self.colour_maps.get(&id).copied() {
            Some(cm) => self.face_colours(cm),
            None => Vec::new(),
        };
        let face_style = |k: usize| colours.get(k).copied().flatten().or(style);
        if triangles {
            for (k, t) in e.arg(3).list().iter().enumerate() {
                let pts = loop_of(t);
                if pts.len() == 3 {
                    geo.polygon(&pts, face_style(k));
                }
            }
        } else {
            for (k, fr) in e.arg(2).refs().into_iter().enumerate() {
                let Some(fe) = self.f.get(fr) else { continue };
                let outer = loop_of(fe.arg(0));
                let holes: Vec<Vec<DVec3>> = if fe.class == "IFCINDEXEDPOLYGONALFACEWITHVOIDS" {
                    fe.arg(1).list().iter().map(loop_of).collect()
                } else {
                    Vec::new()
                };
                geo.face(&outer, &holes, face_style(k));
            }
        }
        true
    }

    /// Materials of an `IfcIndexedColourMap`'s faces.
    fn face_colours(&mut self, cm: u64) -> Vec<Option<usize>> {
        let Some(e) = self.f.get(cm) else {
            return Vec::new();
        };
        let opacity = e.arg(1).as_f64().unwrap_or(1.0);
        let list: Vec<[u8; 3]> = self
            .get(e.arg(2))
            .map(|(_, l)| {
                l.arg(0)
                    .list()
                    .iter()
                    .map(|c| {
                        let n = c.numbers();
                        [0, 1, 2].map(|k| super::unit_byte(n.get(k).copied().unwrap_or(0.8)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut cache: BTreeMap<usize, usize> = BTreeMap::new();
        e.arg(3)
            .list()
            .iter()
            .map(|i| {
                let k = usize::try_from(i.as_i64()? - 1).ok()?;
                let c = *list.get(k)?;
                Some(
                    *cache
                        .entry(k)
                        .or_insert_with(|| self.material("", c, opacity)),
                )
            })
            .collect()
    }

    fn extrusion(&mut self, e: &Entity, xf: DMat4, style: Option<usize>, geo: &mut Geo) -> bool {
        let Some(loops) = self.profile(e.arg(0), 0) else {
            return false;
        };
        let pos = self.axis2(e.arg(1));
        let Some(dir) = self.direction(e.arg(2)) else {
            return false;
        };
        let Some(depth) = e.arg(3).as_f64() else {
            return false;
        };
        let d = dir * depth;
        if d.z.abs() < 1e-12 {
            return false;
        }
        let m = xf * pos;
        let flip = d.z < 0.0;
        let put = |p: DVec2, h: DVec3| m.transform_point3(p.extend(0.0) + h);
        let face = |pts: Vec<DVec3>, holes: Vec<Vec<DVec3>>, geo: &mut Geo| {
            if flip {
                let rev = |mut v: Vec<DVec3>| {
                    v.reverse();
                    v
                };
                geo.face(
                    &rev(pts),
                    &holes.into_iter().map(rev).collect::<Vec<_>>(),
                    style,
                );
            } else {
                geo.face(&pts, &holes, style);
            }
        };
        let Some((outer, holes)) = loops.split_first() else {
            return false;
        };
        // Bottom (facing -z), top (facing +z).
        let bottom = |l: &Vec<DVec2>| -> Vec<DVec3> {
            l.iter().rev().map(|&p| put(p, DVec3::ZERO)).collect()
        };
        let top = |l: &Vec<DVec2>| -> Vec<DVec3> { l.iter().map(|&p| put(p, d)).collect() };
        face(bottom(outer), holes.iter().map(bottom).collect(), geo);
        face(top(outer), holes.iter().map(top).collect(), geo);
        for l in &loops {
            for k in 0..l.len() {
                let (a, b) = (l[k], l[(k + 1) % l.len()]);
                face(
                    vec![
                        put(a, DVec3::ZERO),
                        put(b, DVec3::ZERO),
                        put(b, d),
                        put(a, d),
                    ],
                    Vec::new(),
                    geo,
                );
            }
        }
        true
    }

    /// `IfcSweptDiskSolid`: a tube along the directrix, mitered at its corners.
    fn tube(&mut self, e: &Entity, xf: DMat4, style: Option<usize>, geo: &mut Geo) -> bool {
        let Some(path) = self.curve(e.arg(0), 0) else {
            return false;
        };
        let Some(r) = e.arg(1).as_f64().filter(|r| *r > 0.0) else {
            return false;
        };
        let path = dedup(&path);
        if path.len() < 2 {
            return false;
        }
        let dirs: Vec<DVec3> = path
            .windows(2)
            .map(|w| (w[1] - w[0]).normalize_or_zero())
            .collect();
        let t0 = dirs[0];
        let u = t0.any_orthonormal_vector();
        let v = t0.cross(u);
        let mut rings: Vec<Vec<DVec3>> = vec![
            (0..TUBE_SEGMENTS)
                .map(|k| {
                    let a = TAU * k as f64 / TUBE_SEGMENTS as f64;
                    path[0] + (u * a.cos() + v * a.sin()) * r
                })
                .collect(),
        ];
        for i in 1..path.len() {
            let d_in = dirs[i - 1];
            let plane_n = match dirs.get(i) {
                Some(&d_out) => (d_in + d_out).try_normalize().unwrap_or(d_in),
                None => d_in,
            };
            let denom = d_in.dot(plane_n);
            let prev = rings.last().expect("a ring");
            let ring: Vec<DVec3> = prev
                .iter()
                .map(|&q| {
                    let s = if denom.abs() > 0.2 {
                        (path[i] - q).dot(plane_n) / denom
                    } else {
                        (path[i] - path[i - 1]).length()
                    };
                    q + d_in * s
                })
                .collect();
            rings.push(ring);
        }
        let place = |p: DVec3| xf.transform_point3(p);
        for w in rings.windows(2) {
            for k in 0..TUBE_SEGMENTS {
                let k1 = (k + 1) % TUBE_SEGMENTS;
                geo.face(
                    &[w[0][k], w[0][k1], w[1][k1], w[1][k]].map(place),
                    &[],
                    style,
                );
            }
        }
        let first: Vec<DVec3> = rings[0].iter().rev().map(|&p| place(p)).collect();
        let last: Vec<DVec3> = rings[rings.len() - 1].iter().map(|&p| place(p)).collect();
        geo.face(&first, &[], style);
        geo.face(&last, &[], style);
        true
    }

    fn mapped(
        &mut self,
        e: &Entity,
        xf: DMat4,
        style: Option<usize>,
        geo: &mut Geo,
        depth: usize,
    ) -> bool {
        let Some((map_id, map)) = self.get(e.arg(0)) else {
            return false;
        };
        let origin = self.axis2(map.arg(0));
        let target = self.operator(e.arg(1));
        let Some(def) = self.map_definition(map_id, map, depth) else {
            return false;
        };
        let c = self.conversion();
        geo.instances
            .push((def, c * xf * target * origin * c.inverse(), style));
        true
    }

    /// The component of a representation map, made once.
    fn map_definition(&mut self, map_id: u64, map: &Entity, depth: usize) -> Option<DefinitionId> {
        if let Some(d) = self.maps.get(&map_id) {
            return *d;
        }
        self.maps.insert(map_id, None);
        let mut geo = Geo::default();
        if let Some((_, rep)) = self.get(map.arg(1)) {
            for it in rep.arg(3).refs() {
                self.item(it, DMat4::IDENTITY, None, &mut geo, depth + 1);
            }
        }
        let ents = self.build(geo);
        if is_empty(&ents) {
            return None;
        }
        let base = self
            .map_names
            .get(&map_id)
            .cloned()
            .unwrap_or_else(|| format!("IfcRepresentationMap #{map_id}"));
        let name = self.model.unique_definition_name(&base);
        let def = self.model.definitions.insert(Definition {
            name,
            entities: ents,
            ..Default::default()
        });
        self.maps.insert(map_id, Some(def));
        Some(def)
    }

    /// File to model units as a matrix.
    fn conversion(&self) -> DMat4 {
        DMat4::from_scale(DVec3::splat(self.opts.inches_per_unit))
    }

    fn build(&self, geo: Geo) -> Entities {
        let ids: Vec<usize> = (0..self.model.materials.len()).collect();
        let mut ents = mesh_import::to_entities(&geo.mesh, &self.opts, &ids);
        for (definition, transform, material) in geo.instances {
            ents.instances
                .push(instance(definition, transform, "", material));
        }
        ents
    }

    // ---- products ----

    /// The body representation items of a product shape.
    fn body_items(&self, shape: &Value) -> Vec<u64> {
        let Some((_, pds)) = self.get(shape) else {
            return Vec::new();
        };
        let reps: Vec<&Entity> = pds
            .arg(2)
            .refs()
            .into_iter()
            .filter_map(|r| self.f.get(r))
            .filter(|r| r.class == "IFCSHAPEREPRESENTATION")
            .collect();
        let ident = |r: &&Entity| r.arg(1).as_str().unwrap_or("").to_ascii_lowercase();
        let pick = reps
            .iter()
            .find(|r| ident(r) == "body")
            .or_else(|| reps.iter().find(|r| ident(r) == "facetation"))
            .or_else(|| {
                reps.iter().find(|r| {
                    let i = ident(r);
                    let t = r.arg(2).as_str().unwrap_or("").to_ascii_lowercase();
                    !matches!(
                        i.as_str(),
                        "axis"
                            | "box"
                            | "footprint"
                            | "annotation"
                            | "profile"
                            | "plan"
                            | "reference"
                            | "clearance"
                            | "lighting"
                            | "cog"
                            | "surveypoints"
                    ) && !matches!(
                        t.as_str(),
                        "curve2d"
                            | "curve3d"
                            | "curve"
                            | "geometriccurveset"
                            | "geometricset"
                            | "annotation2d"
                            | "boundingbox"
                            | "point"
                            | "pointcloud"
                    )
                })
            });
        pick.map(|r| r.arg(3).refs()).unwrap_or_default()
    }

    fn run(mut self) -> (Model, IfcReport) {
        let f = self.f;
        // Parent of each object: aggregation first, then containment, then nesting.
        let mut parent: HashMap<u64, u64> = HashMap::new();
        let mut children: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
        for rel in [
            "IFCRELAGGREGATES",
            "IFCRELCONTAINEDINSPATIALSTRUCTURE",
            "IFCRELNESTS",
        ] {
            for e in f.entities.values().filter(|e| e.class == rel) {
                let (whole, parts) = if rel == "IFCRELCONTAINEDINSPATIALSTRUCTURE" {
                    (e.arg(5).as_ref_id(), e.arg(4).refs())
                } else {
                    (e.arg(4).as_ref_id(), e.arg(5).refs())
                };
                let Some(whole) = whole else { continue };
                for p in parts {
                    if p != whole && !parent.contains_key(&p) {
                        parent.insert(p, whole);
                        children.entry(whole).or_default().insert(p);
                    }
                }
            }
        }
        let is_product = |e: &Entity| {
            e.args.len() >= 7
                && f.deref(e.arg(6))
                    .is_some_and(|(_, s)| s.class == "IFCPRODUCTDEFINITIONSHAPE")
        };
        let mut roots: Vec<u64> = f
            .entities
            .iter()
            .filter(|(_, e)| e.class == "IFCPROJECT")
            .map(|(&id, _)| id)
            .collect();
        let project = roots.first().copied();
        // Objects in no relationship: under the project.
        for (&id, e) in &f.entities {
            if parent.contains_key(&id) || e.class == "IFCPROJECT" || !is_product(e) {
                continue;
            }
            match project {
                Some(p) => {
                    children.entry(p).or_default().insert(id);
                }
                None => roots.push(id),
            }
        }
        let mut seen = BTreeSet::new();
        for r in roots {
            if let Some(inst) = self.node(r, DMat4::IDENTITY, &children, &mut seen, 0) {
                self.model.root.instances.push(inst);
            }
        }
        (self.model, self.report)
    }

    /// The group of object `id` and the objects under it, placed relative to
    /// `parent_world` (file units).
    fn node(
        &mut self,
        id: u64,
        parent_world: DMat4,
        children: &BTreeMap<u64, BTreeSet<u64>>,
        seen: &mut BTreeSet<u64>,
        depth: usize,
    ) -> Option<Instance> {
        if depth > 64 || !seen.insert(id) {
            return None;
        }
        let e = self.f.get(id)?;
        if matches!(
            e.class.as_str(),
            "IFCOPENINGELEMENT" | "IFCOPENINGSTANDARDCASE" | "IFCVOIDINGFEATURE"
        ) {
            let c = e.class.clone();
            self.skip(&c);
            return None;
        }
        let is_project = e.class == "IFCPROJECT";
        if !is_project {
            self.report.products += 1;
        }
        let world = if is_project {
            DMat4::IDENTITY
        } else {
            match e.arg(5) {
                Value::Null => parent_world,
                v => self.placement(v, 0),
            }
        };
        let mut geo = Geo::default();
        if !is_project && e.args.len() >= 7 {
            for it in self.body_items(e.arg(6)) {
                self.item(it, DMat4::IDENTITY, None, &mut geo, 0);
            }
        }
        let has_geometry = !geo.mesh.polygons.is_empty() || !geo.instances.is_empty();
        let mut ents = self.build(geo);
        if has_geometry && !is_empty(&ents) {
            self.report.with_geometry += 1;
        }
        if let Some(kids) = children.get(&id) {
            for &k in kids {
                if let Some(inst) = self.node(k, world, children, seen, depth + 1) {
                    ents.instances.push(inst);
                }
            }
        }
        if is_empty(&ents) {
            return None;
        }
        let class = class_name(&e.class);
        self.groups += 1;
        let name = format!("Group#{}", self.groups);
        let definition = self.model.definitions.insert(Definition {
            name,
            entities: ents,
            is_group: true,
            description: e.arg(0).as_str().unwrap_or("").to_owned(),
            attributes: crate::model::DefinitionAttrs {
                kind: class.clone(),
                ..Default::default()
            },
            ..Default::default()
        });
        let label = e.arg(2).as_str().unwrap_or("").trim();
        let label = if label.is_empty() { &class } else { label };
        let c = self.conversion();
        let rel = parent_world.inverse() * world;
        let material = self.product_paint(id);
        let mut inst = instance(definition, c * rel * c.inverse(), label, material);
        inst.hidden = e.class == "IFCSPACE";
        Some(inst)
    }
}

/// Profile loops with the outline counter-clockwise and holes clockwise.
fn orient(mut loops: Vec<Vec<DVec2>>) -> Vec<Vec<DVec2>> {
    loops.retain(|l| l.len() >= 3);
    for (k, l) in loops.iter_mut().enumerate() {
        let a = area2(l);
        if (k == 0) == (a < 0.0) {
            l.reverse();
        }
    }
    loops
}

fn is_empty(ents: &Entities) -> bool {
    ents.faces.is_empty() && ents.instances.is_empty() && ents.edges.is_empty()
}

fn instance(
    definition: DefinitionId,
    transform: DMat4,
    name: &str,
    material: Option<usize>,
) -> Instance {
    Instance {
        definition,
        transform,
        name: name.to_owned(),
        hidden: false,
        locked: false,
        layer: None,
        material,
        attrs: Default::default(),
    }
}

/// The translation and rotation of `xf` with its axes made orthonormal (Gram-Schmidt
/// from X), and what remains (`rigid * rest = xf`).
pub(crate) fn split_rigid(xf: DMat4) -> (DMat4, DMat4) {
    let m3 = DMat3::from_mat4(xf);
    let x = m3.x_axis.try_normalize().unwrap_or(DVec3::X);
    let y = (m3.y_axis - x * m3.y_axis.dot(x))
        .try_normalize()
        .unwrap_or_else(|| x.any_orthonormal_vector());
    let z = x.cross(y);
    let rigid = DMat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), xf.w_axis);
    let rest = rigid.inverse() * xf;
    (rigid, rest)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{Checks, validate};

    pub(crate) const HEADER: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\n\
        FILE_NAME('t.ifc','',(''),(''),'','','');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n";

    /// A project in millimeters with a site, building and storey; `body` lines are
    /// added and `#100` is the storey's placement.
    pub(crate) fn file(body: &str) -> String {
        format!(
            "{HEADER}\
#1=IFCPROJECT('p',$,'Proj',$,$,$,$,$,#2);
#2=IFCUNITASSIGNMENT((#3,#4));
#3=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#4=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);
#5=IFCCARTESIANPOINT((0.,0.,0.));
#6=IFCAXIS2PLACEMENT3D(#5,$,$);
#10=IFCSITE('s',$,'Site',$,$,#11,$,$,.ELEMENT.,$,$,$,$,$);
#11=IFCLOCALPLACEMENT($,#6);
#20=IFCBUILDING('b',$,'Bldg',$,$,#21,$,$,.ELEMENT.,$,$,$);
#21=IFCLOCALPLACEMENT(#11,#6);
#30=IFCBUILDINGSTOREY('st',$,'Level 1',$,$,#100,$,$,.ELEMENT.,0.);
#31=IFCCARTESIANPOINT((0.,0.,3000.));
#32=IFCAXIS2PLACEMENT3D(#31,$,$);
#100=IFCLOCALPLACEMENT(#21,#32);
#40=IFCRELAGGREGATES('r1',$,$,$,#1,(#10));
#41=IFCRELAGGREGATES('r2',$,$,$,#10,(#20));
#42=IFCRELAGGREGATES('r3',$,$,$,#20,(#30));
{body}
ENDSEC;
END-ISO-10303-21;
"
        )
    }

    fn valid(model: &Model) {
        assert!(validate(&model.root, Checks::TOPOLOGY).is_empty());
        for d in model.definitions.values() {
            let errs = validate(&d.entities, Checks::TOPOLOGY);
            assert!(errs.is_empty(), "{}: {errs:?}", d.name);
        }
    }

    /// World positions of every vertex under `ents`.
    fn world_points(model: &Model, ents: &Entities, xf: DMat4, out: &mut Vec<DVec3>) {
        out.extend(ents.vertices.values().map(|v| xf.transform_point3(v.pos)));
        for i in &ents.instances {
            world_points(
                model,
                &model.definitions[i.definition].entities,
                xf * i.transform,
                out,
            );
        }
    }

    fn bounds(model: &Model) -> (DVec3, DVec3) {
        let mut pts = Vec::new();
        world_points(model, &model.root, DMat4::IDENTITY, &mut pts);
        let lo = pts.iter().copied().fold(DVec3::splat(f64::MAX), DVec3::min);
        let hi = pts.iter().copied().fold(DVec3::splat(f64::MIN), DVec3::max);
        (lo, hi)
    }

    /// The chain of instance names from the root down to the first faces.
    fn names(model: &Model) -> Vec<String> {
        let mut out = Vec::new();
        let mut ents = &model.root;
        while let Some(i) = ents.instances.first() {
            out.push(i.name.clone());
            ents = &model.definitions[i.definition].entities;
        }
        out
    }

    #[test]
    fn extruded_wall_with_style_in_hierarchy() {
        let body = "\
#50=IFCWALL('w',$,'Wall A',$,$,#51,#52,$,$);
#51=IFCLOCALPLACEMENT(#100,#53);
#53=IFCAXIS2PLACEMENT3D(#54,$,$);
#54=IFCCARTESIANPOINT((1000.,0.,0.));
#52=IFCPRODUCTDEFINITIONSHAPE($,$,(#55,#59));
#59=IFCSHAPEREPRESENTATION($,'Axis','Curve2D',(#60));
#60=IFCPOLYLINE((#5,#54));
#55=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#56));
#56=IFCEXTRUDEDAREASOLID(#57,$,#58,2500.);
#57=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,4000.,200.);
#58=IFCDIRECTION((0.,0.,1.));
#61=IFCSTYLEDITEM(#56,(#62),$);
#62=IFCSURFACESTYLE('Brick',.BOTH.,(#63));
#63=IFCSURFACESTYLESHADING(#64,0.25);
#64=IFCCOLOURRGB($,1.,0.,0.);
#70=IFCRELCONTAINEDINSPATIALSTRUCTURE('c',$,$,$,(#50),#30);
";
        let (m, report) = import(file(body).as_bytes(), &IfcOptions::default()).unwrap();
        valid(&m);
        assert_eq!(report.schema, "IFC4");
        assert!((report.meters_per_unit - 0.001).abs() < 1e-12);
        assert_eq!(names(&m), ["Proj", "Site", "Bldg", "Level 1", "Wall A"]);
        let wall = m.root.instances[0].definition;
        let mut d = &m.definitions[wall];
        while d.entities.faces.is_empty() {
            d = &m.definitions[d.entities.instances[0].definition];
        }
        assert_eq!(d.attributes.kind, "IfcWall");
        assert_eq!(d.entities.faces.len(), 6);
        assert_eq!(m.materials.len(), 1);
        assert_eq!(m.materials[0].name, "Brick");
        assert_eq!(m.materials[0].color, [255, 0, 0]);
        assert!((m.materials[0].opacity - 0.75).abs() < 1e-9);
        assert!(
            d.entities
                .faces
                .values()
                .all(|f| f.attrs.material == Some(0))
        );
        // 4000 x 200 x 2500 mm centered on x = 1000 mm, on a storey at 3000 mm.
        let (lo, hi) = bounds(&m);
        let mm = 1.0 / 25.4;
        assert!(
            (lo - DVec3::new(-1000.0, -100.0, 3000.0) * mm).length() < 1e-6,
            "{lo}"
        );
        assert!(
            (hi - DVec3::new(3000.0, 100.0, 5500.0) * mm).length() < 1e-6,
            "{hi}"
        );
        assert_eq!(report.with_geometry, 1);
        assert_eq!(report.products, 4);
    }

    #[test]
    fn breps_tessellations_and_mapped_items() {
        // A unit cube as a faceted brep in a type's representation map, placed twice;
        // a triangle set and a polygonal face set with a hole; a boolean whose
        // second operand is skipped.
        let mut body = String::from(
            "\
#200=IFCCARTESIANPOINT((0.,0.,0.));
#201=IFCCARTESIANPOINT((1.,0.,0.));
#202=IFCCARTESIANPOINT((1.,1.,0.));
#203=IFCCARTESIANPOINT((0.,1.,0.));
#204=IFCCARTESIANPOINT((0.,0.,1.));
#205=IFCCARTESIANPOINT((1.,0.,1.));
#206=IFCCARTESIANPOINT((1.,1.,1.));
#207=IFCCARTESIANPOINT((0.,1.,1.));
",
        );
        let quads = [
            [0, 3, 2, 1],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ];
        let mut faces = Vec::new();
        for (k, q) in quads.iter().enumerate() {
            let id = 210 + 3 * k;
            body += &format!(
                "#{id}=IFCPOLYLOOP((#{},#{},#{},#{}));\n#{}=IFCFACEOUTERBOUND(#{id},.T.);\n#{}=IFCFACE((#{}));\n",
                200 + q[0],
                200 + q[1],
                200 + q[2],
                200 + q[3],
                id + 1,
                id + 2,
                id + 1
            );
            faces.push(format!("#{}", id + 2));
        }
        body += &format!(
            "\
#240=IFCCLOSEDSHELL(({}));
#241=IFCFACETEDBREP(#240);
#242=IFCSHAPEREPRESENTATION($,'Body','Brep',(#241));
#243=IFCREPRESENTATIONMAP(#6,#242);
#244=IFCFURNITURETYPE('t',$,'Box Type',$,$,$,(#243),$,$,.NOTDEFINED.,$);
#245=IFCCARTESIANTRANSFORMATIONOPERATOR3D($,$,#5,1000.,$);
#246=IFCMAPPEDITEM(#243,#245);
#247=IFCCARTESIANPOINT((5000.,0.,0.));
#248=IFCCARTESIANTRANSFORMATIONOPERATOR3D($,$,#247,1000.,$);
#249=IFCMAPPEDITEM(#243,#248);
#250=IFCSHAPEREPRESENTATION($,'Body','MappedRepresentation',(#246,#249));
#251=IFCPRODUCTDEFINITIONSHAPE($,$,(#250));
#252=IFCFURNISHINGELEMENT('f',$,$,$,$,#100,#251,$);
#260=IFCCARTESIANPOINTLIST3D(((0.,0.,500.),(1000.,0.,500.),(0.,1000.,500.),(10.,10.,10.)));
#261=IFCTRIANGULATEDFACESET(#260,$,$,((1,2,3)),$);
#262=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(3000.,0.,0.),(3000.,3000.,0.),(0.,3000.,0.),(1000.,1000.,0.),(2000.,1000.,0.),(2000.,2000.,0.),(1000.,2000.,0.)));
#263=IFCINDEXEDPOLYGONALFACEWITHVOIDS((1,2,3,4),((8,7,6,5)));
#264=IFCPOLYGONALFACESET(#262,$,(#263),$);
#265=IFCBOOLEANCLIPPINGRESULT(.DIFFERENCE.,#264,#266);
#266=IFCHALFSPACESOLID(#267,.F.);
#267=IFCPLANE(#6);
#268=IFCSHAPEREPRESENTATION($,'Body','Tessellation',(#261,#265));
#269=IFCPRODUCTDEFINITIONSHAPE($,$,(#268));
#270=IFCSLAB('sl',$,'Floor',$,$,$,#269,$,$);
#271=IFCRELCONTAINEDINSPATIALSTRUCTURE('c',$,$,$,(#252,#270),#30);
",
            faces.join(",")
        );
        let (m, report) = import(file(&body).as_bytes(), &IfcOptions::default()).unwrap();
        valid(&m);
        let comp = m
            .definitions
            .values()
            .find(|d| d.name == "Box Type")
            .expect("component");
        assert!(!comp.is_group);
        assert_eq!(comp.entities.faces.len(), 6);
        // Two placements of the component.
        let placed: usize = m
            .definitions
            .values()
            .flat_map(|d| &d.entities.instances)
            .filter(|i| m.definitions[i.definition].name == "Box Type")
            .count();
        assert_eq!(placed, 2);
        // The holed square merged back into one face with a hole.
        let slab = m
            .definitions
            .values()
            .find(|d| d.attributes.kind == "IfcSlab")
            .unwrap();
        assert_eq!(slab.entities.faces.len(), 2);
        assert!(slab.entities.faces.values().any(|f| f.holes.len() == 1));
        assert_eq!(report.skipped.get("IfcHalfSpaceSolid"), Some(&1));
        assert_eq!(report.items.get("IfcMappedItem"), Some(&2));
        let (lo, hi) = bounds(&m);
        let mm = 1.0 / 25.4;
        assert!((hi.x - 6000.0 * mm).abs() < 1e-6 && lo.x.abs() < 1e-6);
        assert!(report.summary().contains("skipped 1 IfcHalfSpaceSolid"));
    }

    #[test]
    fn profiles_and_curves() {
        let body = "\
#300=IFCCARTESIANPOINT((0.,0.));
#301=IFCCARTESIANPOINT((2000.,0.));
#302=IFCCARTESIANPOINT((2000.,1000.));
#303=IFCCARTESIANPOINT((0.,1000.));
#304=IFCPOLYLINE((#300,#301,#302,#303,#300));
#305=IFCCIRCLE(#306,200.);
#306=IFCAXIS2PLACEMENT2D(#307,$);
#307=IFCCARTESIANPOINT((1000.,500.));
#308=IFCARBITRARYPROFILEDEFWITHVOIDS(.AREA.,$,#304,(#305));
#309=IFCEXTRUDEDAREASOLID(#308,$,#58,100.);
#58=IFCDIRECTION((0.,0.,1.));
#310=IFCCARTESIANPOINTLIST2D(((0.,0.),(1000.,0.),(1500.,500.),(1000.,1000.),(0.,1000.)));
#311=IFCINDEXEDPOLYCURVE(#310,(IFCLINEINDEX((1,2)),IFCARCINDEX((2,3,4)),IFCLINEINDEX((4,5,1))),$);
#312=IFCARBITRARYCLOSEDPROFILEDEF(.AREA.,$,#311);
#313=IFCEXTRUDEDAREASOLID(#312,$,#314,200.);
#314=IFCDIRECTION((0.,0.,-1.));
#315=IFCCIRCLEPROFILEDEF(.AREA.,$,$,50.);
#316=IFCEXTRUDEDAREASOLID(#315,$,#58,500.);
#317=IFCTRIMMEDCURVE(#318,(IFCPARAMETERVALUE(0.)),(IFCPARAMETERVALUE(3.14159265358979)),.T.,.PARAMETER.);
#318=IFCCIRCLE(#319,500.);
#319=IFCAXIS2PLACEMENT2D(#300,$);
#320=IFCCOMPOSITECURVE((#321,#322),.F.);
#321=IFCCOMPOSITECURVESEGMENT(.CONTINUOUS.,.T.,#317);
#322=IFCCOMPOSITECURVESEGMENT(.CONTINUOUS.,.T.,#323);
#323=IFCPOLYLINE((#324,#325));
#324=IFCCARTESIANPOINT((-500.,0.));
#325=IFCCARTESIANPOINT((500.,0.));
#326=IFCARBITRARYCLOSEDPROFILEDEF(.AREA.,$,#320);
#327=IFCEXTRUDEDAREASOLID(#326,$,#58,10.);
#328=IFCPOLYLINE((#329,#330,#331));
#329=IFCCARTESIANPOINT((0.,0.,0.));
#330=IFCCARTESIANPOINT((1000.,0.,0.));
#331=IFCCARTESIANPOINT((1000.,1000.,0.));
#332=IFCSWEPTDISKSOLID(#328,20.,$,$,$);
#333=IFCISHAPEPROFILEDEF(.AREA.,$,$,200.,400.,10.,20.,$,$,$,$);
#334=IFCEXTRUDEDAREASOLID(#333,$,#58,1000.);
#335=IFCADVANCEDBREP(#5);
#340=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#309,#313,#316,#327,#332,#334,#335));
#341=IFCPRODUCTDEFINITIONSHAPE($,$,(#340));
#342=IFCBUILDINGELEMENTPROXY('x',$,$,$,$,#100,#341,$,$);
#343=IFCRELCONTAINEDINSPATIALSTRUCTURE('c',$,$,$,(#342),#30);
";
        let (m, report) = import(file(body).as_bytes(), &IfcOptions::default()).unwrap();
        valid(&m);
        assert_eq!(report.items.get("IfcExtrudedAreaSolid"), Some(&5));
        assert_eq!(report.items.get("IfcSweptDiskSolid"), Some(&1));
        assert_eq!(report.skipped.get("IfcAdvancedBrep"), Some(&1));
        let proxy = m
            .definitions
            .values()
            .find(|d| d.attributes.kind == "IfcBuildingElementProxy")
            .unwrap();
        // The plate with a round hole: top and bottom faces have one hole each.
        let holed = proxy
            .entities
            .faces
            .values()
            .filter(|f| f.holes.len() == 1)
            .count();
        assert!(holed >= 2, "{holed}");
        assert_eq!(proxy.attributes.kind, "IfcBuildingElementProxy");
    }

    #[test]
    fn units_material_association_and_zip() {
        let body = "\
#50=IFCCOLUMN('c',$,$,$,$,#100,#52,$,$);
#52=IFCPRODUCTDEFINITIONSHAPE($,$,(#55));
#55=IFCSHAPEREPRESENTATION($,$,'SweptSolid',(#56));
#56=IFCEXTRUDEDAREASOLID(#57,$,#58,10.);
#57=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,1.,1.);
#58=IFCDIRECTION((0.,0.,1.));
#80=IFCMATERIAL('Concrete',$,$);
#81=IFCRELASSOCIATESMATERIAL('m',$,$,$,(#50),#82);
#82=IFCMATERIALLAYERSETUSAGE(#83,.AXIS2.,.POSITIVE.,0.,$);
#83=IFCMATERIALLAYERSET((#84),$,$);
#84=IFCMATERIALLAYER(#80,1.,$,$,$,$,$);
#85=IFCMATERIALDEFINITIONREPRESENTATION($,$,(#86),#80);
#86=IFCSTYLEDREPRESENTATION($,$,$,(#87));
#87=IFCSTYLEDITEM($,(#88),$);
#88=IFCPRESENTATIONSTYLEASSIGNMENT((#89));
#89=IFCSURFACESTYLE('Concrete',.BOTH.,(#90));
#90=IFCSURFACESTYLERENDERING(#91,$,$,$,$,$,$,$,.NOTDEFINED.);
#91=IFCCOLOURRGB($,0.5,0.5,0.5);
#70=IFCRELCONTAINEDINSPATIALSTRUCTURE('c',$,$,$,(#50),#30);
";
        // Feet instead of millimeters.
        let text = file(body).replace(
            "#3=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);",
            "#3=IFCCONVERSIONBASEDUNIT(#7,.LENGTHUNIT.,'FOOT',#8);\n#7=IFCDIMENSIONALEXPONENTS(1,0,0,0,0,0,0);\n#8=IFCMEASUREWITHUNIT(IFCLENGTHMEASURE(0.3048),#9);\n#9=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);",
        );
        let (m, report) = import(text.as_bytes(), &IfcOptions::default()).unwrap();
        valid(&m);
        assert!((report.meters_per_unit - 0.3048).abs() < 1e-12);
        let (lo, hi) = bounds(&m);
        // Storey at 3000 ft, column 10 ft tall.
        assert!((lo.z - 36000.0).abs() < 1e-6 && (hi.z - 36120.0).abs() < 1e-6);
        let col = m
            .definitions
            .values()
            .flat_map(|d| &d.entities.instances)
            .find(|i| m.definitions[i.definition].attributes.kind == "IfcColumn")
            .unwrap();
        assert_eq!(col.name, "IfcColumn");
        assert_eq!(m.materials[col.material.unwrap()].name, "Concrete");
        // The same file zipped.
        let mut buf = Vec::new();
        {
            let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            z.start_file("model.ifc", zip::write::SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut z, text.as_bytes()).unwrap();
            z.finish().unwrap();
        }
        let (mz, _) = import(&buf, &IfcOptions::default()).unwrap();
        assert_eq!(mz.definitions.len(), m.definitions.len());
        assert!(
            import(
                b"ISO-10303-21;HEADER;FILE_SCHEMA(('AP214'));ENDSEC;DATA;ENDSEC;END-ISO-10303-21;",
                &IfcOptions::default()
            )
            .is_err()
        );
    }

    #[test]
    fn class_names() {
        assert_eq!(class_name("IFCWALLSTANDARDCASE"), "IfcWallStandardCase");
        assert_eq!(class_name("IFCBUILDINGSTOREY"), "IfcBuildingStorey");
        assert_eq!(class_name("IFCFLOWTERMINAL"), "IfcFlowTerminal");
        assert_eq!(class_name("IFCXYZTHING"), "IfcXyzthing");
        assert_eq!(class_name("IFCHALFSPACESOLID"), "IfcHalfSpaceSolid");
        assert_eq!(
            class_name("IFCRECTANGLEPROFILEDEF"),
            "IfcRectangleProfileDef"
        );
        assert_eq!(class_name("IFCPOLYLINE"), "IfcPolyline");
    }

    #[test]
    fn rigid_split_keeps_mirroring_in_rest() {
        let xf = DMat4::from_translation(DVec3::new(1.0, 2.0, 3.0))
            * DMat4::from_rotation_z(0.3)
            * DMat4::from_scale(DVec3::new(2.0, -1.0, 3.0));
        let (r, rest) = split_rigid(xf);
        assert!((r * rest - xf).abs_diff_eq(DMat4::ZERO, 1e-12));
        assert!((DMat3::from_mat4(r).determinant() - 1.0).abs() < 1e-12);
    }
}
