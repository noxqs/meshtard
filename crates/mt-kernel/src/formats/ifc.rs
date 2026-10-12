//! IFC (ISO 16739) import, geometry only: no Classifier, property sets or
//! quantities. Reads IFC2X3, IFC4 and IFC4X3 STEP files ([`super::step`]) and
//! `.ifczip` archives. Export is in [`super::ifc_export`].
//!
//! - The spatial structure and the products under it (`IfcRelAggregates`,
//!   `IfcRelContainedInSpatialStructure`, `IfcRelNests`) become nested groups: the
//!   instance is named after the entity's Name (its IFC class when it has none), the
//!   definition has the IFC class as its Entity Info "Type" and the GlobalId as its
//!   description. Groups with nothing below them are left out; products in no
//!   relationship go under the project. Spaces come in hidden (only their own faces
//!   and edges when objects are contained in them).
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
//!   / `IfcSweptDiskSolidPolygonal` (tube along the directrix, hollow with an inner
//!   radius, mitered; closed directrices make a ring), `IfcRevolvedAreaSolid`,
//!   `IfcSurfaceCurveSweptAreaSolid` / `IfcFixedReferenceSweptAreaSolid` (the profile
//!   carried along the directrix, mitered, see [`ifc_geom::sweep_rings`]), the CSG
//!   primitives `IfcBlock`, `IfcRightCircularCylinder`, `IfcRightCircularCone`,
//!   `IfcSphere` (12 bands) and `IfcRectangularPyramid`, `IfcCsgSolid`,
//!   `IfcAdvancedBrep` (with voids; planar faces from their edge loops, curved faces
//!   on B-spline, cylindrical, conical, spherical and toroidal surfaces tessellated
//!   inside their edge loops, [`ifc_geom::surface_triangles`]; edges are lines,
//!   circles, ellipses, polylines or B-spline curves, shared by both faces),
//!   `IfcMappedItem` (a component per `IfcRepresentationMap`, named after the type
//!   that owns it) and `IfcBooleanResult` / `IfcBooleanClippingResult` (see below).
//!   Curves are 24 segments per full circle.
//! - Colors: `IfcStyledItem` surface styles (shading or rendering color and
//!   transparency) on items, `IfcIndexedColourMap` per-face colors, and the color of
//!   the product's (or its type's) material through `IfcRelAssociatesMaterial` and
//!   `IfcMaterialDefinitionRepresentation`, which paints the group (faces without a
//!   style of their own).
//! - Textures: `IfcIndexedTriangleTextureMap` texture coordinates on triangulated
//!   face sets, with the image of an `IfcBlobTexture` (PNG / JPEG), an
//!   `IfcPixelTexture` (made a PNG) or an `IfcImageTexture` (read by its URL through
//!   [`import_with`]'s callback); the material is the item's style with the image.
//! - Units: the project's `IfcUnitAssignment` length unit (SI prefixes and conversion
//!   based units such as feet), and its plane angle unit for trimmed circles.
//! - Georeferencing: `IfcSite` RefLatitude / RefLongitude become the model's
//!   location (Shadows latitude and longitude, geo-located), the `IfcMapConversion`
//!   x axis (else the context's TrueNorth) its north angle, and the conversion's
//!   eastings, northings and height (map units, with Scale; IfcOpenShell's reading
//!   when no map unit is given) its map origin and EPSG code. A model farther than
//!   1 km from the origin (its middle, per axis) is moved near it by whole meters,
//!   [`IfcReport::moved`], the map origin following; an element whose points are far
//!   from its own origin has them moved near it the same way first, so definitions
//!   hold small coordinates.
//! - Booleans, with the Solid Tools: `.UNION.` and `.INTERSECTION.` join their
//!   operands ([`solid::union`], [`solid::intersect`]; counted in
//!   [`IfcReport::joined`]) when both are closed solids, else a union keeps both
//!   operands as they are and an intersection the first.
//! - Cutting, with the Solid Tools' [`solid::subtract`]: the openings of an element
//!   (`IfcRelVoidsElement` to an `IfcOpeningElement`, placed by its own placement) are
//!   cut out of the element's body, and a boolean `.DIFFERENCE.` cuts its second
//!   operand out of its first. Second operands may be solids or half spaces
//!   (`IfcHalfSpaceSolid`, `IfcBoxedHalfSpace`: a box on the material side of the
//!   plane, larger than the first operand; `IfcPolygonalBoundedHalfSpace`: that box
//!   intersected with the boundary's prism). A cut happens only when both operands are
//!   closed solids ([`solid::is_solid`]: an element with mapped items is not) and the
//!   result passes the topology checks; otherwise the first operand is kept whole as
//!   before and the opening or second operand is counted in [`IfcReport::skipped`].
//!   Cuts made are counted by class in [`IfcReport::cut`]. Openings are not imported
//!   as objects. Everything else that is not read is counted by class in
//!   [`IfcReport::skipped`].
//! - Faces become kernel faces as in mesh import ([`mesh_import::to_entities`]):
//!   welded, coplanar neighbors merged when asked. Faces with holes are triangulated
//!   first; merging joins them again.
//! - Determinism: entities are visited in instance-number order; maps keyed by
//!   number are only looked up, never iterated for output.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::f64::consts::TAU;
use std::io::Read;

use glam::{DMat3, DMat4, DVec2, DVec3};

use super::ifc_geom;
use super::step::{self, Entity, File, Value};
use crate::entities::{DefinitionId, Entities, Instance};
use crate::mesh_import::{self, Mesh, MeshOptions, Polygon};
use crate::model::{Definition, Material, Model};
use crate::solid::{self, SolidInput};
use crate::validate::{Checks, validate};

/// Segments of a full circle.
pub(crate) const CIRCLE_SEGMENTS: usize = 24;
/// Bands of a sphere from pole to pole.
const SPHERE_STACKS: usize = 12;
/// Geometry farther than this from the origin (meters, per axis) is moved near it.
const FAR: f64 = 1000.0;
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
    /// placements, boolean second operands and openings that were not cut).
    pub skipped: BTreeMap<String, usize>,
    /// Openings and boolean difference operands cut out, by IFC class.
    pub cut: BTreeMap<String, usize>,
    /// Boolean union and intersection operands joined with the Solid Tools, by IFC
    /// class.
    pub joined: BTreeMap<String, usize>,
    /// How far the model was moved toward the origin (meters, file axes): the site
    /// was farther than 1 km away. The model's geo-location keeps the offset.
    pub moved: DVec3,
}

impl IfcReport {
    /// One line for the status bar: products and what was skipped.
    pub fn summary(&self) -> String {
        let mut s = format!(
            "{} of {} objects with geometry",
            self.with_geometry, self.products
        );
        let cut: usize = self.cut.values().sum();
        if cut > 0 {
            s += &format!("; {cut} openings and booleans cut");
        }
        if self.moved != DVec3::ZERO {
            s += &format!("; moved {:.0} m to the origin", self.moved.length());
        }
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
/// Texture images the file names by URL are not read (see [`import_with`]).
pub fn import(bytes: &[u8], opts: &IfcOptions) -> Result<(Model, IfcReport), String> {
    import_with(bytes, opts, &|_| None)
}

/// [`import`], reading the texture images the file names (`IfcImageTexture` URLs)
/// through `read`.
pub fn import_with(
    bytes: &[u8],
    opts: &IfcOptions,
    read: super::ReadFile,
) -> Result<(Model, IfcReport), String> {
    let data = unzip(bytes)?;
    let file = step::parse(&data)?;
    if !file.schema.iter().any(|s| s.starts_with("IFC")) && !file.schema.is_empty() {
        return Err(format!(
            "not an IFC file (schema {})",
            file.schema.join(", ")
        ));
    }
    Ok(Reader::new(&file, opts, read).run())
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
        "BEZIER",
        "BLOB",
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
        "CONICAL",
        "CONTROL",
        "CONTROLLER",
        "CONVERSION",
        "COOLER",
        "COOLING",
        "COURSE",
        "COVERING",
        "CRS",
        "CSG",
        "CURTAIN",
        "CURVE",
        "CYLINDER",
        "CYLINDRICAL",
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
        "EDGE",
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
        "EXTRUSION",
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
        "IMAGE",
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
        "OF",
        "OFFSET",
        "OPEN",
        "OPENING",
        "ORIENTED",
        "OUTLET",
        "PARAMETER",
        "PART",
        "PAVEMENT",
        "PILE",
        "PIPE",
        "PIXEL",
        "PLACEMENT",
        "PLANE",
        "PLATE",
        "POINT",
        "POLYGONAL",
        "POLYLINE",
        "POLYLOOP",
        "PORT",
        "PRIMITIVE",
        "PROFILE",
        "PROJECT",
        "PROJECTED",
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
        "RECTANGULAR",
        "REFERENCE",
        "REINFORCING",
        "RESULT",
        "REVOLUTION",
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
        "SPHERICAL",
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
        "TEXTURE",
        "TOROIDAL",
        "TOWER",
        "TRACK",
        "TRANSFORMER",
        "TRANSPORT",
        "TREATMENT",
        "TRIANGLE",
        "TRIANGULATED",
        "TRIMMED",
        "TUBE",
        "TUNNEL",
        "UNIT",
        "UNITARY",
        "VALUE",
        "VALVE",
        "VEHICLE",
        "VERTEX",
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

    /// Adds `other`'s polygons and placements.
    fn append(&mut self, other: Geo) {
        let base = self.mesh.positions.len();
        self.mesh.positions.extend(other.mesh.positions);
        self.mesh
            .polygons
            .extend(other.mesh.polygons.into_iter().map(|mut p| {
                for v in &mut p.vertices {
                    *v += base;
                }
                p
            }));
        self.instances.extend(other.instances);
    }

    /// Bounding box of the polygons' corners.
    fn bounds(&self) -> Option<(DVec3, DVec3)> {
        let mut it = self.mesh.positions.iter().copied();
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
    }

    /// A box, faces outward: `m` maps the unit-free box `lo..hi` into place.
    fn cuboid(&mut self, m: DMat4, lo: DVec3, hi: DVec3, material: Option<usize>) {
        let c = |i: usize| {
            m.transform_point3(DVec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            ))
        };
        let flip = m.determinant() < 0.0;
        for q in [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [1, 3, 7, 5],
            [3, 2, 6, 7],
            [2, 0, 4, 6],
        ] {
            let mut pts = q.map(c).to_vec();
            if flip {
                pts.reverse();
            }
            self.polygon(&pts, material);
        }
    }

    /// The prism of the counter-clockwise loop `l` from `z0` to `z1` (`z0 < z1`),
    /// faces outward, placed by `m`.
    fn prism(&mut self, m: DMat4, l: &[DVec2], z0: f64, z1: f64, material: Option<usize>) {
        let flip = m.determinant() < 0.0;
        let put = |p: DVec2, z: f64| m.transform_point3(p.extend(z));
        let add = |mut pts: Vec<DVec3>, geo: &mut Geo| {
            if flip {
                pts.reverse();
            }
            geo.face(&pts, &[], material);
        };
        add(l.iter().rev().map(|&p| put(p, z0)).collect(), self);
        add(l.iter().map(|&p| put(p, z1)).collect(), self);
        for k in 0..l.len() {
            let (a, b) = (l[k], l[(k + 1) % l.len()]);
            add(vec![put(a, z0), put(b, z0), put(b, z1), put(a, z1)], self);
        }
    }

    /// A quad, split into two triangles unless it is flat.
    fn quad(&mut self, q: [DVec3; 4], material: Option<usize>) {
        let n = (q[1] - q[0]).cross(q[2] - q[0]);
        let size = (q[2] - q[0]).length().max((q[3] - q[1]).length());
        let off = match n.try_normalize() {
            Some(n) => (q[3] - q[0]).dot(n).abs(),
            None => 0.0,
        };
        if off <= 1e-9 * size.max(1e-300) {
            self.face(&q, &[], material);
        } else {
            self.face(&[q[0], q[1], q[2]], &[], material);
            self.face(&[q[0], q[2], q[3]], &[], material);
        }
    }

    /// The solid swept through `rings` (`rings[s][l]`: loop `l` at station `s`,
    /// loop 0 the outline, the others holes, the same number of points at every
    /// station): sides between stations and, unless `closed` (the last station is
    /// the first), caps at both ends. Faces turn outward when the outline runs
    /// counter-clockwise seen from where the sweep goes.
    fn sweep(&mut self, rings: &[Vec<Vec<DVec3>>], closed: bool, material: Option<usize>) {
        if rings.len() < 2 || rings[0].is_empty() {
            return;
        }
        let centroid = |l: &[DVec3]| l.iter().copied().sum::<DVec3>() / l.len().max(1) as f64;
        // Which way the outline turns relative to the motion.
        let mut flip = false;
        for s in 1..rings.len() {
            let motion = centroid(&rings[s][0]) - centroid(&rings[s - 1][0]);
            let along = newell(&rings[s - 1][0]).dot(motion);
            if along.abs() > 1e-12 * motion.length() {
                flip = along < 0.0;
                break;
            }
        }
        let rev = |mut v: Vec<DVec3>| {
            v.reverse();
            v
        };
        for w in rings.windows(2) {
            for (a, b) in w[0].iter().zip(&w[1]) {
                let n = a.len().min(b.len());
                for k in 0..n {
                    let k1 = (k + 1) % n;
                    let q = [a[k], a[k1], b[k1], b[k]];
                    self.quad(if flip { [q[3], q[2], q[1], q[0]] } else { q }, material);
                }
            }
        }
        if !closed {
            let (first, last) = (&rings[0], &rings[rings.len() - 1]);
            let cap = |loops: &Vec<Vec<DVec3>>, reverse: bool| -> (Vec<DVec3>, Vec<Vec<DVec3>>) {
                let f = |l: &Vec<DVec3>| if reverse { rev(l.clone()) } else { l.clone() };
                (f(&loops[0]), loops[1..].iter().map(f).collect())
            };
            let (o, h) = cap(first, !flip);
            self.face(&o, &h, material);
            let (o, h) = cap(last, flip);
            self.face(&o, &h, material);
        }
    }

    fn polygon_uv(&mut self, points: &[DVec3], material: Option<usize>, uvs: Vec<DVec2>) {
        self.polygon(points, material);
        if let Some(p) = self.mesh.polygons.last_mut() {
            p.uvs = uvs;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Difference,
    Union,
    Intersection,
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
    /// Element -> its openings (`IfcRelVoidsElement`), in number order.
    voids: BTreeMap<u64, BTreeSet<u64>>,
    groups: usize,
    /// Reads texture images named by URL.
    read: super::ReadFile<'a>,
    /// `IfcEdgeCurve` -> its points from start to end vertex (item coordinates).
    edges: HashMap<u64, Vec<DVec3>>,
    /// Tessellated face set -> its `IfcIndexedTriangleTextureMap`.
    texture_maps: HashMap<u64, u64>,
    /// (style material, surface texture) -> textured material.
    textured: HashMap<(Option<usize>, u64), Option<usize>>,
}

impl<'a> Reader<'a> {
    fn new(f: &'a File, opts: &IfcOptions, read: super::ReadFile<'a>) -> Self {
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
            voids: BTreeMap::new(),
            groups: 0,
            read,
            edges: HashMap::new(),
            texture_maps: HashMap::new(),
            textured: HashMap::new(),
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
                "IFCINDEXEDTRIANGLETEXTUREMAP" => {
                    if let Some(set) = e.arg(1).as_ref_id() {
                        self.texture_maps.entry(set).or_insert(id);
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
                "IFCRELVOIDSELEMENT" => {
                    if let (Some(el), Some(op)) = (e.arg(4).as_ref_id(), e.arg(5).as_ref_id()) {
                        self.voids.entry(el).or_default().insert(op);
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
            "IFCBSPLINECURVEWITHKNOTS" | "IFCRATIONALBSPLINECURVEWITHKNOTS" => {
                let points: Vec<DVec3> = e
                    .arg(1)
                    .list()
                    .iter()
                    .filter_map(|p| self.point(p))
                    .collect();
                let mults: Vec<i64> = e.arg(5).list().iter().filter_map(Value::as_i64).collect();
                let degree = e.arg(0).as_i64().and_then(|d| usize::try_from(d).ok())?;
                let knots =
                    ifc_geom::Knots::new(degree, &mults, &e.arg(6).numbers(), points.len())?;
                let weights = if e.class.starts_with("IFCRATIONAL") {
                    e.arg(8).numbers()
                } else {
                    Vec::new()
                };
                ifc_geom::BSplineCurve {
                    knots,
                    points,
                    weights,
                }
                .sample()
            }
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
            "IFCREVOLVEDAREASOLID" | "IFCREVOLVEDAREASOLIDTAPERED" => {
                self.revolved(e, xf, style, geo)
            }
            "IFCSURFACECURVESWEPTAREASOLID" | "IFCFIXEDREFERENCESWEPTAREASOLID" => {
                self.swept_along(e, xf, style, geo)
            }
            "IFCBLOCK"
            | "IFCRIGHTCIRCULARCYLINDER"
            | "IFCRIGHTCIRCULARCONE"
            | "IFCSPHERE"
            | "IFCRECTANGULARPYRAMID" => self.primitive(e, xf, style, geo),
            "IFCCSGSOLID" => match e.arg(0).as_ref_id() {
                Some(root) => {
                    self.item(root, xf, style, geo, depth + 1);
                    true
                }
                None => false,
            },
            "IFCADVANCEDBREP" | "IFCADVANCEDBREPWITHVOIDS" => {
                let mut shells = e.arg(0).refs();
                shells.extend(e.arg(1).refs());
                for s in shells {
                    self.shell(s, xf, style, geo);
                }
                true
            }
            "IFCMAPPEDITEM" => self.mapped(e, xf, style, geo, depth),
            "IFCBOOLEANRESULT" | "IFCBOOLEANCLIPPINGRESULT" => {
                self.boolean(e, xf, style, geo, depth);
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

    // ---- cutting ----

    /// `geo` as kernel entities (model units, coplanar faces merged) when it is a
    /// closed solid.
    fn solid_ents(&self, geo: &Geo) -> Option<Entities> {
        if !geo.instances.is_empty() || geo.mesh.polygons.is_empty() {
            return None;
        }
        let opts = MeshOptions {
            merge_coplanar: true,
            ..self.opts
        };
        let ids: Vec<usize> = (0..self.model.materials.len()).collect();
        let ents = mesh_import::to_entities(&geo.mesh, &opts, &ids);
        solid::is_solid(&ents).then_some(ents)
    }

    /// `body - cutter`, when it comes out sound.
    fn subtract(body: &Entities, cutter: &Entities) -> Option<Entities> {
        let out = solid::subtract(
            SolidInput::new(cutter, DMat4::IDENTITY),
            SolidInput::new(body, DMat4::IDENTITY),
        )
        .ok()?;
        validate(&out, Checks::TOPOLOGY).is_empty().then_some(out)
    }

    /// Adds the faces of `ents` (model units) to `geo` (file units).
    fn add_ents(&self, geo: &mut Geo, ents: &Entities) {
        let k = 1.0 / self.opts.inches_per_unit;
        for f in ents.faces.values() {
            let outer: Vec<DVec3> = ents.loop_points(&f.outer).iter().map(|p| *p * k).collect();
            let holes: Vec<Vec<DVec3>> = f
                .holes
                .iter()
                .map(|h| ents.loop_points(h).iter().map(|p| *p * k).collect())
                .collect();
            geo.face(&outer, &holes, f.attrs.material);
        }
    }

    /// A half space as a closed solid (file units) covering `lo..hi` (the first
    /// operand's bounds, group coordinates): `None` when `e` is not a half space.
    fn half_space(&mut self, e: &Entity, xf: DMat4, lo: DVec3, hi: DVec3) -> Option<Entities> {
        let class = e.class.as_str();
        if !matches!(
            class,
            "IFCHALFSPACESOLID" | "IFCBOXEDHALFSPACE" | "IFCPOLYGONALBOUNDEDHALFSPACE"
        ) {
            return None;
        }
        let (_, plane) = self.get(e.arg(0))?;
        if plane.class != "IFCPLANE" {
            return None;
        }
        let pm = xf * self.axis2(plane.arg(0));
        // The agreement flag is true when the plane's normal points away from the
        // material (ISO 16739).
        let agree = e.arg(1).as_enum() != Some("F");
        let size = (hi - lo).length() + 1.0;
        let reach = |m: DMat4| {
            let c = m.inverse().transform_point3((lo + hi) / 2.0);
            (c, 2.0 * size + c.length())
        };
        let (c, r) = reach(pm);
        let (z0, z1) = if agree { (-r, 0.0) } else { (0.0, r) };
        let mut boxed = Geo::default();
        boxed.cuboid(
            pm,
            DVec3::new(c.x - r, c.y - r, z0),
            DVec3::new(c.x + r, c.y + r, z1),
            None,
        );
        let half = self.solid_ents(&boxed)?;
        if class != "IFCPOLYGONALBOUNDEDHALFSPACE" {
            return Some(half);
        }
        let m = xf * self.axis2(e.arg(2));
        let outline = orient(vec![self.closed_loop(e.arg(3))?]).pop()?;
        let (c, r) = reach(m);
        let mut prism = Geo::default();
        prism.prism(m, &outline, c.z - r, c.z + r, None);
        let prism = self.solid_ents(&prism)?;
        let out = solid::intersect(
            SolidInput::new(&half, DMat4::IDENTITY),
            SolidInput::new(&prism, DMat4::IDENTITY),
        )
        .ok()?;
        solid::is_solid(&out).then_some(out)
    }

    /// A boolean result with the Solid Tools when both operands are closed solids:
    /// `.DIFFERENCE.` cuts the second operand out of the first, `.UNION.` and
    /// `.INTERSECTION.` join them. Otherwise a union keeps both operands as they are,
    /// a difference or intersection the first one (the second counted as skipped).
    fn boolean(
        &mut self,
        e: &Entity,
        xf: DMat4,
        style: Option<usize>,
        geo: &mut Geo,
        depth: usize,
    ) {
        let op = match e.arg(0).as_enum() {
            Some("UNION") => Op::Union,
            Some("INTERSECTION") => Op::Intersection,
            _ => Op::Difference,
        };
        let mut first = Geo::default();
        if let Some(o) = e.arg(1).as_ref_id() {
            self.item(o, xf, style, &mut first, depth + 1);
        }
        let Some((second_id, second)) = self.get(e.arg(2)) else {
            geo.append(first);
            return;
        };
        let class = second.class.clone();
        let items = self.report.items.clone();
        let body = self.solid_ents(&first);
        // Cut faces take the element's paint; joined ones keep their style.
        let second_style = if op == Op::Difference { None } else { style };
        let cutter = match (&body, first.bounds()) {
            (Some(_), Some((lo, hi))) => self.half_space(second, xf, lo, hi).or_else(|| {
                let mut g = Geo::default();
                self.item(second_id, xf, second_style, &mut g, depth + 1);
                self.solid_ents(&g)
            }),
            _ => None,
        };
        let result = body.zip(cutter).and_then(|(b, c)| match op {
            Op::Difference => Self::subtract(&b, &c),
            Op::Union | Op::Intersection => Self::join(&b, &c, op == Op::Union),
        });
        match result {
            Some(out) => {
                if class.contains("HALFSPACE") {
                    self.read(&class);
                }
                let counts = if op == Op::Difference {
                    &mut self.report.cut
                } else {
                    &mut self.report.joined
                };
                *counts.entry(class_name(&class)).or_default() += 1;
                self.add_ents(geo, &out);
            }
            None => {
                self.report.items = items;
                geo.append(first);
                if op == Op::Union {
                    self.item(second_id, xf, style, geo, depth + 1);
                } else {
                    self.skip(&class);
                }
            }
        }
    }

    /// The union (or intersection) of two solids, when it comes out sound.
    fn join(a: &Entities, b: &Entities, union: bool) -> Option<Entities> {
        let f = if union {
            solid::union
        } else {
            solid::intersect
        };
        let out = f(
            SolidInput::new(a, DMat4::IDENTITY),
            SolidInput::new(b, DMat4::IDENTITY),
        )
        .ok()?;
        validate(&out, Checks::TOPOLOGY).is_empty().then_some(out)
    }

    /// Cuts the openings of an element out of its body `ents` (model units, in the
    /// element's coordinates; `world` is the element's placement, file units).
    fn cut_openings(&mut self, ents: &mut Entities, openings: &BTreeSet<u64>, world: DMat4) {
        let solid = solid::is_solid(ents);
        for &op in openings {
            let Some(o) = self.f.get(op) else { continue };
            let class = o.class.clone();
            if !solid || o.args.len() < 7 {
                self.skip(&class);
                continue;
            }
            let rel = world.inverse() * self.placement(o.arg(5), 0);
            let items = self.report.items.clone();
            // Each body item is cut on its own: openings are often several touching
            // extrusions, whose faces together are not one solid.
            let mut cut = Some(ents.clone());
            for it in self.body_items(o.arg(6)) {
                let mut g = Geo::default();
                self.item(it, rel, None, &mut g, 0);
                cut = cut.and_then(|body| {
                    let c = self.solid_ents(&g)?;
                    Self::subtract(&body, &c)
                });
            }
            match cut {
                Some(out) => {
                    *ents = out;
                    *self.report.cut.entry(class_name(&class)).or_default() += 1;
                }
                None => {
                    self.report.items = items;
                    self.skip(&class);
                }
            }
        }
    }

    /// The faces of a shell or connected face set: faces bounded by poly loops or
    /// edge loops; `IfcAdvancedFace` / `IfcFaceSurface` on curved surfaces are
    /// tessellated (see [`ifc_geom::surface_triangles`]).
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
                let pts: Vec<DVec3> = match lp.class.as_str() {
                    "IFCPOLYLOOP" => lp
                        .arg(0)
                        .list()
                        .iter()
                        .filter_map(|p| self.point(p))
                        .collect(),
                    "IFCEDGELOOP" => self.edge_loop(lp),
                    other => {
                        let c = other.to_owned();
                        self.skip(&c);
                        continue;
                    }
                };
                let mut pts: Vec<DVec3> = pts.into_iter().map(|p| xf.transform_point3(p)).collect();
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
            let mut outer = outer;
            if matches!(fe.class.as_str(), "IFCADVANCEDFACE" | "IFCFACESURFACE") {
                // The face's normal is the surface's, reversed when not same sense
                // (and by a mirroring placement).
                let sense = (fe.arg(2).as_enum() != Some("F")) != (xf.determinant() < 0.0);
                match self.surface(fe.arg(1), xf) {
                    Some(Ok(surface)) => {
                        let mut loops = vec![dedup(&outer)];
                        loops.extend(bounds.iter().map(|b| dedup(b)));
                        if let Some(tris) = ifc_geom::surface_triangles(&surface, &loops, sense) {
                            for t in tris {
                                geo.polygon(&t, fstyle);
                            }
                            continue;
                        }
                    }
                    Some(Err(class)) => self.skip(&class),
                    None => {
                        // A plane: loops turned to face along its normal.
                        if let Some((_, plane)) = self.get(fe.arg(1)) {
                            let m = xf * self.axis2(plane.arg(0));
                            let n = m.x_axis.truncate().cross(m.y_axis.truncate());
                            let n = if sense { n } else { -n };
                            if newell(&outer).dot(n) < 0.0 {
                                outer.reverse();
                                for b in &mut bounds {
                                    b.reverse();
                                }
                            }
                        }
                    }
                }
            }
            geo.face(&outer, &bounds, fstyle);
        }
    }

    /// The curved surface of a face (`Ok`), or the class of one not read (`Err`);
    /// `None` for a plane.
    fn surface(&mut self, v: &Value, xf: DMat4) -> Option<Result<ifc_geom::Surface, String>> {
        use ifc_geom::Surface;
        let (_, e) = self.get(v)?;
        let num = |i: usize| e.arg(i).as_f64().unwrap_or(0.0);
        let class = e.class.as_str();
        let s = match class {
            "IFCPLANE" => return None,
            "IFCCYLINDRICALSURFACE" => Surface::Cylinder {
                m: xf * self.axis2(e.arg(0)),
                r: num(1),
            },
            "IFCCONICALSURFACE" => Surface::Cone {
                m: xf * self.axis2(e.arg(0)),
                r: num(1),
                tan: (num(2) * self.angle).tan(),
            },
            "IFCSPHERICALSURFACE" => Surface::Sphere {
                m: xf * self.axis2(e.arg(0)),
                r: num(1),
            },
            "IFCTOROIDALSURFACE" => Surface::Torus {
                m: xf * self.axis2(e.arg(0)),
                big: num(1),
                r: num(2),
            },
            "IFCBSPLINESURFACEWITHKNOTS" | "IFCRATIONALBSPLINESURFACEWITHKNOTS" => {
                let points: Vec<Vec<DVec3>> = e
                    .arg(2)
                    .list()
                    .iter()
                    .map(|row| {
                        row.list()
                            .iter()
                            .filter_map(|p| self.point(p))
                            .map(|p| xf.transform_point3(p))
                            .collect()
                    })
                    .collect();
                let nu = points.len();
                let nv = points.first().map_or(0, Vec::len);
                let ints = |i: usize| -> Vec<i64> {
                    e.arg(i).list().iter().filter_map(Value::as_i64).collect()
                };
                let deg = |i: usize| e.arg(i).as_i64().and_then(|d| usize::try_from(d).ok());
                let weights: Vec<Vec<f64>> = if class.starts_with("IFCRATIONAL") {
                    e.arg(12).list().iter().map(Value::numbers).collect()
                } else {
                    Vec::new()
                };
                let u =
                    deg(0).and_then(|d| ifc_geom::Knots::new(d, &ints(7), &e.arg(9).numbers(), nu));
                let v = deg(1)
                    .and_then(|d| ifc_geom::Knots::new(d, &ints(8), &e.arg(10).numbers(), nv));
                match (u, v) {
                    (Some(u), Some(v)) if points.iter().all(|r| r.len() == nv) => {
                        Surface::BSpline(ifc_geom::BSplineSurface {
                            u,
                            v,
                            points,
                            weights,
                        })
                    }
                    _ => return Some(Err(class.to_owned())),
                }
            }
            other => return Some(Err(other.to_owned())),
        };
        Some(Ok(s))
    }

    /// The points of an `IfcEdgeLoop` (item coordinates), each edge once.
    fn edge_loop(&mut self, lp: &Entity) -> Vec<DVec3> {
        let mut out: Vec<DVec3> = Vec::new();
        for oe in lp.arg(0).refs() {
            let Some(o) = self.f.get(oe) else { continue };
            let mut pts = self.edge_points(oe, o, 0);
            if o.class == "IFCORIENTEDEDGE" && o.arg(3).as_enum() == Some("F") {
                pts.reverse();
            }
            for p in pts {
                if out.last().is_none_or(|q| (*q - p).length_squared() > 1e-18) {
                    out.push(p);
                }
            }
        }
        out
    }

    /// The points of an edge from its start to its end vertex: oriented edges by
    /// their edge element, edge curves along their geometry (lines, circles and
    /// ellipses between the vertices, other curves cut at the points nearest the
    /// vertices). Edges are tessellated once so the faces on both sides share points.
    fn edge_points(&mut self, id: u64, e: &Entity, depth: usize) -> Vec<DVec3> {
        if let Some(p) = self.edges.get(&id) {
            return p.clone();
        }
        let vertex = |r: &Self, v: &Value| -> Option<DVec3> {
            let (_, ve) = r.get(v)?;
            r.point(ve.arg(0))
        };
        let pts = match e.class.as_str() {
            "IFCORIENTEDEDGE" if depth < 4 => match self.get(e.arg(2)) {
                Some((eid, inner)) => self.edge_points(eid, inner, depth + 1),
                None => Vec::new(),
            },
            "IFCEDGECURVE" => {
                let (s, t) = (vertex(self, e.arg(0)), vertex(self, e.arg(1)));
                let same = e.arg(3).as_enum() != Some("F");
                self.edge_curve(e.arg(2), same, s, t)
                    .unwrap_or_else(|| s.into_iter().chain(t).collect())
            }
            _ => vertex(self, e.arg(0))
                .into_iter()
                .chain(vertex(self, e.arg(1)))
                .collect(),
        };
        self.edges.insert(id, pts.clone());
        pts
    }

    fn edge_curve(
        &mut self,
        geom: &Value,
        same: bool,
        s: Option<DVec3>,
        t: Option<DVec3>,
    ) -> Option<Vec<DVec3>> {
        let (s, t) = (s?, t?);
        let (_, g) = self.get(geom)?;
        match g.class.as_str() {
            "IFCLINE" => Some(vec![s, t]),
            "IFCCIRCLE" | "IFCELLIPSE" => {
                let m = self.axis2(g.arg(0));
                let rx = g.arg(1).as_f64()?;
                let ry = if g.class == "IFCELLIPSE" {
                    g.arg(2).as_f64()?
                } else {
                    rx
                };
                let inv = m.inverse();
                let ang = |p: DVec3| {
                    let l = inv.transform_point3(p);
                    (l.y / ry).atan2(l.x / rx)
                };
                let a0 = ang(s);
                let mut a1 = ang(t);
                let closed = (s - t).length_squared() <= 1e-18;
                if same {
                    while a1 <= a0 + if closed { 1e-9 } else { 1e-12 } {
                        a1 += TAU;
                    }
                } else {
                    while a1 >= a0 - if closed { 1e-9 } else { 1e-12 } {
                        a1 -= TAU;
                    }
                }
                let mut pts: Vec<DVec3> = arc(DVec2::ZERO, rx, ry, a0, a1)
                    .into_iter()
                    .map(|p| m.transform_point3(p.extend(0.0)))
                    .collect();
                let n = pts.len();
                pts[0] = s;
                pts[n - 1] = t;
                Some(pts)
            }
            _ => {
                let mut pts = self.curve(geom, 0)?;
                if !same {
                    pts.reverse();
                }
                let near = |p: DVec3, from: usize, pts: &[DVec3]| {
                    (from..pts.len()).min_by(|&i, &j| {
                        (pts[i] - p)
                            .length_squared()
                            .total_cmp(&(pts[j] - p).length_squared())
                    })
                };
                let i0 = near(s, 0, &pts)?;
                let i1 = near(t, i0 + 1, &pts).unwrap_or(pts.len() - 1);
                let mut out: Vec<DVec3> = pts[i0..=i1.max(i0)].to_vec();
                if out.len() < 2 {
                    out = vec![s, t];
                }
                let n = out.len();
                out[0] = s;
                out[n - 1] = t;
                Some(out)
            }
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
        let texture = match self.texture_maps.get(&id).copied() {
            Some(tm) if triangles => self.texture_map(tm, style),
            _ => None,
        };
        if triangles {
            for (k, t) in e.arg(3).list().iter().enumerate() {
                let pts = loop_of(t);
                if pts.len() != 3 {
                    continue;
                }
                let uvs = texture.as_ref().and_then(|(_, coords, index)| {
                    let idx: Vec<i64> = match index.get(k) {
                        Some(i) => i.to_vec(),
                        None if index.is_empty() => {
                            t.list().iter().filter_map(Value::as_i64).collect()
                        }
                        None => return None,
                    };
                    idx.iter()
                        .map(|&i| coords.get(usize::try_from(i - 1).ok()?).copied())
                        .collect::<Option<Vec<DVec2>>>()
                        .filter(|u| u.len() == 3)
                });
                match (uvs, &texture) {
                    (Some(uvs), Some((m, _, _))) => geo.polygon_uv(&pts, Some(*m), uvs),
                    _ => geo.polygon(&pts, face_style(k)),
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

    /// The textured material, texture coordinates and per-triangle coordinate
    /// indices (empty: the face set's own indices) of an
    /// `IfcIndexedTriangleTextureMap`; `None` when its image cannot be read.
    fn texture_map(
        &mut self,
        tm: u64,
        style: Option<usize>,
    ) -> Option<(usize, Vec<DVec2>, Vec<[i64; 3]>)> {
        let e = self.f.get(tm)?;
        let texture = e.arg(0).refs().into_iter().next()?;
        let material = self.texture_material(style, texture)?;
        let coords: Vec<DVec2> = self
            .get(e.arg(2))
            .map(|(_, l)| {
                l.arg(0)
                    .list()
                    .iter()
                    .map(|c| {
                        let n = c.numbers();
                        DVec2::new(
                            n.first().copied().unwrap_or(0.0),
                            n.get(1).copied().unwrap_or(0.0),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let index: Vec<[i64; 3]> = e
            .arg(3)
            .list()
            .iter()
            .map(|t| {
                let n: Vec<i64> = t.list().iter().filter_map(Value::as_i64).collect();
                [0, 1, 2].map(|k| n.get(k).copied().unwrap_or(0))
            })
            .collect();
        Some((material, coords, index))
    }

    /// The material of a style (or a grey one) with a surface texture's image:
    /// `IfcImageTexture` (read by its URL), `IfcBlobTexture` (PNG or JPEG) or
    /// `IfcPixelTexture` (made a PNG). Made once per style and texture.
    fn texture_material(&mut self, style: Option<usize>, texture: u64) -> Option<usize> {
        if let Some(m) = self.textured.get(&(style, texture)) {
            return *m;
        }
        let image = self.texture_image(texture);
        if image.is_none() {
            let class = self
                .f
                .get(texture)
                .map(|t| t.class.clone())
                .unwrap_or_default();
            self.skip(&class);
        }
        let m = image.map(|(filename, data)| {
            let base = style
                .and_then(|m| self.model.materials.get(m))
                .cloned()
                .unwrap_or(Material {
                    name: String::new(),
                    color: [204, 204, 204],
                    opacity: 1.0,
                    texture: None,
                });
            let name = if base.name.is_empty() {
                filename
                    .rsplit_once('.')
                    .map_or(filename.as_str(), |(stem, _)| stem)
                    .to_owned()
            } else {
                base.name.clone()
            };
            self.push_material(Material {
                name,
                texture: Some(crate::Texture {
                    filename,
                    data,
                    width: super::TEXTURE_TILE[0],
                    height: super::TEXTURE_TILE[1],
                    colorize: false,
                }),
                ..base
            })
        });
        self.textured.insert((style, texture), m);
        m
    }

    fn texture_image(&self, texture: u64) -> Option<(String, std::sync::Arc<[u8]>)> {
        let t = self.f.get(texture)?;
        match t.class.as_str() {
            "IFCIMAGETEXTURE" => {
                // IFC4: URLReference is the 6th attribute, IFC2X3 the 5th.
                let url = t.args.iter().skip(4).rev().find_map(Value::as_str)?;
                let tex = super::find_texture(url, self.read)?;
                Some((tex.filename, tex.data))
            }
            "IFCBLOBTEXTURE" => {
                let data = match t.arg(6) {
                    Value::Binary(h) => ifc_geom::binary_bytes(h),
                    _ => return None,
                };
                let ext = if data.starts_with(b"\x89PNG") {
                    "png"
                } else {
                    "jpg"
                };
                super::is_image(&data).then(|| (format!("texture{texture}.{ext}"), data.into()))
            }
            "IFCPIXELTEXTURE" => {
                let w = u32::try_from(t.arg(5).as_i64()?).ok()?;
                let h = u32::try_from(t.arg(6).as_i64()?).ok()?;
                let c = u8::try_from(t.arg(7).as_i64()?).ok()?;
                let mut rows: Vec<Vec<u8>> = Vec::new();
                let pixels: Vec<Vec<u8>> = t
                    .arg(8)
                    .list()
                    .iter()
                    .map(|p| match p {
                        Value::Binary(hex) => ifc_geom::binary_bytes(hex),
                        _ => Vec::new(),
                    })
                    .collect();
                if pixels.len() != (w as usize) * (h as usize)
                    || pixels.iter().any(|p| p.len() != c as usize)
                {
                    return None;
                }
                // The pixels run left to right from the bottom row up.
                for row in pixels.chunks(w as usize) {
                    rows.push(row.concat());
                }
                rows.reverse();
                let png = ifc_geom::png(w, h, c, &rows.concat())?;
                Some((format!("texture{texture}.png"), png.into()))
            }
            _ => None,
        }
    }

    /// Adds a material, its name made unique.
    fn push_material(&mut self, mut m: Material) -> usize {
        let base = m.name.clone();
        let mut n = 1;
        while self.model.materials.iter().any(|x| x.name == m.name) {
            n += 1;
            m.name = format!("{base} {n}");
        }
        self.model.materials.push(m);
        self.model.materials.len() - 1
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

    /// `IfcSweptDiskSolid` / `IfcSweptDiskSolidPolygonal`: a tube (hollow with an
    /// inner radius) along the directrix, mitered at its corners; start and end
    /// parameters are not read (the whole directrix, as IfcOpenShell does), nor the
    /// polygonal kind's fillet radius.
    fn tube(&mut self, e: &Entity, xf: DMat4, style: Option<usize>, geo: &mut Geo) -> bool {
        let Some((path, closed)) = self.directrix(e.arg(0), None, None) else {
            return false;
        };
        let Some(r) = e.arg(1).as_f64().filter(|r| *r > 0.0) else {
            return false;
        };
        let mut loops = vec![ifc_geom::circle_n(r, TUBE_SEGMENTS)];
        if let Some(inner) = e.arg(2).as_f64().filter(|i| *i > 0.0 && *i < r) {
            let mut h = ifc_geom::circle_n(inner, TUBE_SEGMENTS);
            h.reverse();
            loops.push(h);
        }
        let x0 = path[1] - path[0];
        let Some(rings) = ifc_geom::sweep_rings(&path, x0.any_orthonormal_vector(), &loops, closed)
        else {
            return false;
        };
        geo.sweep(&place_rings(rings, xf), closed, style);
        true
    }

    /// A sweep's directrix as points, and whether it is closed. `start` and `end`
    /// trim a bare circle or ellipse (plane angle units); other curves are taken
    /// whole.
    fn directrix(
        &mut self,
        v: &Value,
        start: Option<f64>,
        end: Option<f64>,
    ) -> Option<(Vec<DVec3>, bool)> {
        let (_, e) = self.get(v)?;
        let (pts, closed) = match e.class.as_str() {
            "IFCCIRCLE" | "IFCELLIPSE" => {
                let m = self.axis2(e.arg(0));
                let rx = e.arg(1).as_f64()?;
                let ry = if e.class == "IFCELLIPSE" {
                    e.arg(2).as_f64()?
                } else {
                    rx
                };
                let (pts, closed) = match (start, end) {
                    (Some(a), Some(b)) if ((b - a) * self.angle).abs() < TAU - 1e-9 => (
                        arc(DVec2::ZERO, rx, ry, a * self.angle, b * self.angle),
                        false,
                    ),
                    _ => (circle(rx, ry), true),
                };
                let pts = pts
                    .into_iter()
                    .map(|p| m.transform_point3(p.extend(0.0)))
                    .collect();
                (pts, closed)
            }
            "IFCPOLYLINE" => {
                let raw: Vec<DVec3> = e
                    .arg(0)
                    .list()
                    .iter()
                    .filter_map(|p| self.point(p))
                    .collect();
                let closed =
                    raw.len() > 3 && (raw[0] - raw[raw.len() - 1]).length_squared() <= 1e-18;
                (dedup(&raw), closed)
            }
            _ => (self.curve(v, 0)?, false),
        };
        let pts = dedup(&pts);
        let closed = closed && pts.len() >= 3;
        (pts.len() >= 2).then_some((pts, closed))
    }

    /// `IfcRevolvedAreaSolid`: the profile turned about the axis (24 steps per full
    /// turn); the tapered kind's end profile is not read.
    fn revolved(&mut self, e: &Entity, xf: DMat4, style: Option<usize>, geo: &mut Geo) -> bool {
        let Some(loops) = self.profile(e.arg(0), 0) else {
            return false;
        };
        let pos = self.axis2(e.arg(1));
        let Some((_, axis)) = self.get(e.arg(2)) else {
            return false;
        };
        let origin = self.point(axis.arg(0)).unwrap_or(DVec3::ZERO);
        let dir = self.direction(axis.arg(1)).unwrap_or(DVec3::Z);
        let Some(angle) = e.arg(3).as_f64().map(|a| a * self.angle) else {
            return false;
        };
        if angle.abs() < 1e-12 {
            return false;
        }
        let full = angle.abs() >= TAU - 1e-9;
        let angle = angle.clamp(-TAU, TAU);
        let n = (angle.abs() / TAU * CIRCLE_SEGMENTS as f64).ceil().max(1.0) as usize;
        let m = xf * pos;
        let mut rings: Vec<Vec<Vec<DVec3>>> = (0..=n)
            .map(|k| {
                let turn = DMat4::from_translation(origin)
                    * DMat4::from_axis_angle(dir, angle * k as f64 / n as f64)
                    * DMat4::from_translation(-origin);
                let t = m * turn;
                loops
                    .iter()
                    .map(|l| {
                        l.iter()
                            .map(|p| t.transform_point3(p.extend(0.0)))
                            .collect()
                    })
                    .collect()
            })
            .collect();
        if full {
            rings[n] = rings[0].clone();
        }
        geo.sweep(&rings, full, style);
        true
    }

    /// `IfcSurfaceCurveSweptAreaSolid` and `IfcFixedReferenceSweptAreaSolid`: the
    /// profile swept along the directrix (in the Position's coordinates), mitered at
    /// its corners. Its x axis starts along the reference surface's normal (a plane's
    /// normal, or the extrusion direction crossed with the directrix for a surface of
    /// linear extrusion; this matches IfcOpenShell) or along the fixed reference, and
    /// is carried along the directrix without twisting.
    fn swept_along(&mut self, e: &Entity, xf: DMat4, style: Option<usize>, geo: &mut Geo) -> bool {
        let Some(loops) = self.profile(e.arg(0), 0) else {
            return false;
        };
        let pos = self.axis2(e.arg(1));
        let Some((path, closed)) = self.directrix(e.arg(2), e.arg(3).as_f64(), e.arg(4).as_f64())
        else {
            return false;
        };
        let t0 = (path[1] - path[0]).normalize_or_zero();
        let x0 = if e.class == "IFCFIXEDREFERENCESWEPTAREASOLID" {
            self.direction(e.arg(5)).unwrap_or(DVec3::X)
        } else {
            match self.get(e.arg(5)) {
                Some((_, s)) if s.class == "IFCPLANE" => self.axis2(s.arg(0)).z_axis.truncate(),
                Some((_, s)) if s.class == "IFCSURFACEOFLINEAREXTRUSION" => {
                    let m = self.axis2(s.arg(1));
                    let d = self.direction(s.arg(2)).unwrap_or(DVec3::Z);
                    t0.cross(m.transform_vector3(d))
                }
                Some((_, s)) => {
                    let c = s.class.clone();
                    self.skip(&c);
                    DVec3::Z
                }
                None => DVec3::Z,
            }
        };
        let Some(rings) = ifc_geom::sweep_rings(&path, x0, &loops, closed) else {
            return false;
        };
        geo.sweep(&place_rings(rings, xf * pos), closed, style);
        true
    }

    /// CSG primitives (`IfcBlock`, `IfcRightCircularCylinder`, `IfcRightCircularCone`,
    /// `IfcSphere`, `IfcRectangularPyramid`) at their Position.
    fn primitive(&mut self, e: &Entity, xf: DMat4, style: Option<usize>, geo: &mut Geo) -> bool {
        let m = xf * self.axis2(e.arg(0));
        let num = |i: usize| e.arg(i).as_f64().filter(|x| *x > 0.0);
        let lift = |l: &[DVec2], z: f64| -> Vec<Vec<DVec3>> {
            vec![l.iter().map(|p| m.transform_point3(p.extend(z))).collect()]
        };
        let rings: Vec<Vec<Vec<DVec3>>> = match e.class.as_str() {
            "IFCBLOCK" => {
                let (Some(x), Some(y), Some(z)) = (num(1), num(2), num(3)) else {
                    return false;
                };
                geo.cuboid(m, DVec3::ZERO, DVec3::new(x, y, z), style);
                return true;
            }
            "IFCRIGHTCIRCULARCYLINDER" => {
                let (Some(h), Some(r)) = (num(1), num(2)) else {
                    return false;
                };
                let c = circle(r, r);
                vec![lift(&c, 0.0), lift(&c, h)]
            }
            "IFCRIGHTCIRCULARCONE" => {
                let (Some(h), Some(r)) = (num(1), num(2)) else {
                    return false;
                };
                let c = circle(r, r);
                vec![lift(&c, 0.0), lift(&vec![DVec2::ZERO; c.len()], h)]
            }
            "IFCRECTANGULARPYRAMID" => {
                let (Some(x), Some(y), Some(h)) = (num(1), num(2), num(3)) else {
                    return false;
                };
                let base = [
                    DVec2::ZERO,
                    DVec2::new(x, 0.0),
                    DVec2::new(x, y),
                    DVec2::new(0.0, y),
                ];
                vec![lift(&base, 0.0), lift(&[DVec2::new(x, y) / 2.0; 4], h)]
            }
            "IFCSPHERE" => {
                let Some(r) = num(1) else { return false };
                ifc_geom::sphere_rings(r, SPHERE_STACKS, CIRCLE_SEGMENTS)
                    .into_iter()
                    .map(|l| vec![l.into_iter().map(|p| m.transform_point3(p)).collect()])
                    .collect()
            }
            _ => return false,
        };
        geo.sweep(&rings, false, style);
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

    fn build(&self, mut geo: Geo) -> Entities {
        let ids: Vec<usize> = (0..self.model.materials.len()).collect();
        if geo.mesh.polygons.iter().any(|p| !p.uvs.is_empty()) {
            // Texture sizes for the faces' texture positions.
            geo.mesh.materials = self
                .model
                .materials
                .iter()
                .map(|m| mesh_import::MeshMaterial {
                    name: m.name.clone(),
                    color: m.color,
                    opacity: m.opacity,
                    texture: m.texture.as_ref().map(|t| mesh_import::MeshTexture {
                        filename: t.filename.clone(),
                        data: t.data.clone(),
                        size: [t.width, t.height],
                    }),
                })
                .collect();
        }
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
        self.georeference();
        (self.model, self.report)
    }

    /// Whole meters (file units) to move `geo` by when its middle is farther than
    /// [`FAR`] from the origin along an axis; zero otherwise.
    fn far_shift(&self, geo: &Geo) -> DVec3 {
        let Some((lo, hi)) = geo.bounds() else {
            return DVec3::ZERO;
        };
        let m = self.report.meters_per_unit;
        let c = (lo + hi) / 2.0 * m;
        DVec3::from_array(
            c.to_array()
                .map(|x| if x.abs() > FAR { x.round() } else { 0.0 }),
        ) / m
    }

    /// Moves a model far from the origin near it, and reads the site's latitude
    /// and longitude (`IfcSite` RefLatitude / RefLongitude), north
    /// (`IfcMapConversion` axis, else the context's TrueNorth) and where the model
    /// origin lies in map coordinates (the map conversion's eastings, northings and
    /// height plus the move, else the move alone) into the model's geo-location.
    fn georeference(&mut self) {
        let f = self.f;
        // Move the whole model near the origin.
        let mut b = [DVec3::splat(f64::MAX), DVec3::splat(f64::MIN)];
        for i in &self.model.root.instances {
            world_bounds(&self.model, i, DMat4::IDENTITY, 0, &mut b);
        }
        let mut moved = DVec3::ZERO;
        if b[0].x <= b[1].x {
            let to_m = 0.0254;
            let c = (b[0] + b[1]) / 2.0 * to_m;
            moved = DVec3::from_array(
                c.to_array()
                    .map(|x| if x.abs() > FAR { x.round() } else { 0.0 }),
            );
            if moved != DVec3::ZERO {
                let t = DMat4::from_translation(-moved / to_m);
                for i in &mut self.model.root.instances {
                    i.transform = t * i.transform;
                }
            }
        }
        self.report.moved = moved;
        let mut geo = self.model.geo;
        // North.
        let conversion = f
            .entities
            .values()
            .find(|e| e.class.starts_with("IFCMAPCONVERSION"));
        let true_north = f
            .entities
            .values()
            .find(|e| {
                e.class == "IFCGEOMETRICREPRESENTATIONCONTEXT" && !matches!(e.arg(5), Value::Null)
            })
            .and_then(|e| f.deref(e.arg(5)))
            .map(|(_, d)| d.arg(0).numbers())
            .filter(|n| n.len() >= 2 && (n[0] != 0.0 || n[1] != 0.0));
        let mut north = None;
        if let Some(c) = conversion {
            let (xa, xo) = (
                c.arg(5).as_f64().unwrap_or(1.0),
                c.arg(6).as_f64().unwrap_or(0.0),
            );
            let theta = xo.atan2(xa);
            north = Some(theta.to_degrees());
            // The target CRS: its EPSG code and map unit (meters per map unit).
            let crs = f.deref(c.arg(1)).map(|(_, crs)| crs);
            geo.epsg = crs
                .and_then(|crs| crs.arg(0).as_str())
                .and_then(|n| {
                    let n = n.trim().to_ascii_uppercase();
                    n.strip_prefix("EPSG:")?.trim().parse().ok()
                })
                .unwrap_or(0);
            // Scale: map units per file unit (1 when not given); without a map unit
            // the map unit is the file unit times the scale (as IfcOpenShell reads it).
            let scale = c.arg(7).as_f64().filter(|s| *s > 0.0).unwrap_or(1.0);
            let map_unit = crs
                .and_then(|crs| self.unit_factor(crs.arg(6), 0))
                .map(|u| u.1)
                .filter(|m| *m > 0.0)
                .unwrap_or(self.report.meters_per_unit / scale);
            let d = moved / self.report.meters_per_unit;
            let (sin, cos) = theta.sin_cos();
            let e = c.arg(2).as_f64().unwrap_or(0.0) + scale * (cos * d.x - sin * d.y);
            let n = c.arg(3).as_f64().unwrap_or(0.0) + scale * (sin * d.x + cos * d.y);
            let h = c.arg(4).as_f64().unwrap_or(0.0) + scale * d.z;
            geo.map_origin = DVec3::new(e, n, h) * map_unit;
        } else {
            geo.map_origin = moved;
            if let Some(n) = true_north {
                north = Some(n[0].atan2(n[1]).to_degrees());
            }
        }
        if let Some(a) = north {
            geo.north_angle = a.rem_euclid(360.0);
            if geo.north_angle < 1e-9 || geo.north_angle > 360.0 - 1e-9 {
                geo.north_angle = 0.0;
            }
        }
        self.model.geo = geo;
        // Latitude and longitude.
        let site = f.entities.values().find(|e| e.class == "IFCSITE");
        let angle = |v: &Value| -> Option<f64> {
            let n = v.numbers();
            let parts = [1.0, 60.0, 3600.0, 3_600_000_000.0];
            (!n.is_empty()).then(|| n.iter().zip(parts).map(|(x, p)| x / p).sum())
        };
        if let Some(site) = site
            && let (Some(lat), Some(lon)) = (angle(site.arg(9)), angle(site.arg(10)))
            && (lat != 0.0 || lon != 0.0)
            && lat.abs() <= 90.0
            && lon.abs() <= 180.0
        {
            self.model.geo.located = true;
            self.model.shadows.latitude = lat;
            self.model.shadows.longitude = lon;
        }
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
            // Openings of an element are counted where they are cut.
            if !self.voids.values().any(|v| v.contains(&id)) {
                let c = e.class.clone();
                self.skip(&c);
            }
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
        // Geometry far from the element's own origin (coordinates written in map
        // units): moved near it, the element's origin moved the other way.
        let shift = self.far_shift(&geo);
        let world = if shift == DVec3::ZERO {
            world
        } else {
            for p in &mut geo.mesh.positions {
                *p -= shift;
            }
            let back = DMat4::from_translation(-shift * self.opts.inches_per_unit);
            for (_, xf, _) in &mut geo.instances {
                *xf = back * *xf;
            }
            world * DMat4::from_translation(shift)
        };
        let mut ents = self.build(geo);
        if let Some(openings) = self.voids.get(&id).cloned() {
            self.cut_openings(&mut ents, &openings, world);
        }
        if has_geometry && !is_empty(&ents) {
            self.report.with_geometry += 1;
        }
        let own_geometry = !ents.faces.is_empty();
        if let Some(kids) = children.get(&id) {
            for &k in kids {
                if let Some(inst) = self.node(k, world, children, seen, depth + 1) {
                    ents.instances.push(inst);
                }
            }
        }
        // Spaces come in hidden; one holding objects (furniture contained in a
        // room) hides only its own faces and edges.
        let is_space = e.class == "IFCSPACE";
        let holds = !ents.instances.is_empty();
        if is_space && holds && own_geometry {
            for f in ents.faces.values_mut() {
                f.attrs.hidden = true;
            }
            for ed in ents.edges.values_mut() {
                ed.attrs.hidden = true;
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
        inst.hidden = is_space && !holds;
        Some(inst)
    }
}

/// Rings of [`ifc_geom::sweep_rings`] placed by `m`.
fn place_rings(rings: Vec<Vec<Vec<DVec3>>>, m: DMat4) -> Vec<Vec<Vec<DVec3>>> {
    rings
        .into_iter()
        .map(|r| {
            r.into_iter()
                .map(|l| l.into_iter().map(|p| m.transform_point3(p)).collect())
                .collect()
        })
        .collect()
}

/// Grows `b` by the world positions under instance `i` placed by `xf` (inches).
fn world_bounds(model: &Model, i: &Instance, xf: DMat4, depth: usize, b: &mut [DVec3; 2]) {
    let Some(d) = model.definitions.get(i.definition) else {
        return;
    };
    let xf = xf * i.transform;
    for v in d.entities.vertices.values() {
        let p = xf.transform_point3(v.pos);
        b[0] = b[0].min(p);
        b[1] = b[1].max(p);
    }
    if depth < MAX_DEPTH {
        for k in &d.entities.instances {
            world_bounds(model, k, xf, depth + 1, b);
        }
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

    /// The first definition of IFC class `kind` with faces.
    fn body_of<'m>(m: &'m Model, kind: &str) -> &'m Entities {
        &m.definitions
            .values()
            .find(|d| d.attributes.kind == kind && !d.entities.faces.is_empty())
            .unwrap_or_else(|| panic!("no {kind}"))
            .entities
    }

    const MM3: f64 = 1.0 / (25.4 * 25.4 * 25.4);

    #[test]
    fn wall_with_window_opening_is_cut() {
        // A 4000 x 200 x 2500 wall; a 1000 x 1200 window opening (400 deep, through
        // the wall) placed relative to the wall, 900 above its base.
        let body = "\
#50=IFCWALL('w',$,'Wall A',$,$,#51,#52,$,$);
#51=IFCLOCALPLACEMENT(#100,#53);
#53=IFCAXIS2PLACEMENT3D(#54,$,$);
#54=IFCCARTESIANPOINT((1000.,0.,0.));
#52=IFCPRODUCTDEFINITIONSHAPE($,$,(#55));
#55=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#56));
#56=IFCEXTRUDEDAREASOLID(#57,$,#58,2500.);
#57=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,4000.,200.);
#58=IFCDIRECTION((0.,0.,1.));
#80=IFCOPENINGELEMENT('o',$,'Window opening',$,$,#81,#82,$,.OPENING.);
#81=IFCLOCALPLACEMENT(#51,#83);
#83=IFCAXIS2PLACEMENT3D(#84,$,$);
#84=IFCCARTESIANPOINT((500.,0.,900.));
#82=IFCPRODUCTDEFINITIONSHAPE($,$,(#85));
#85=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#86));
#86=IFCEXTRUDEDAREASOLID(#87,$,#58,1200.);
#87=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,1000.,400.);
#88=IFCRELVOIDSELEMENT('v',$,$,$,#50,#80);
#70=IFCRELCONTAINEDINSPATIALSTRUCTURE('c',$,$,$,(#50),#30);
";
        let (m, report) = import(file(body).as_bytes(), &IfcOptions::default()).unwrap();
        valid(&m);
        assert_eq!(report.cut.get("IfcOpeningElement"), Some(&1));
        assert_eq!(report.skipped.get("IfcOpeningElement"), None);
        assert!(report.summary().contains("1 openings and booleans cut"));
        let wall = body_of(&m, "IfcWall");
        assert!(crate::is_solid(wall));
        // Six sides and four reveals; the two big sides have the hole.
        assert_eq!(wall.faces.len(), 10);
        assert_eq!(
            wall.faces.values().filter(|f| f.holes.len() == 1).count(),
            2
        );
        let v = crate::solid::volume(wall);
        let want = (4000.0 * 200.0 * 2500.0 - 1000.0 * 200.0 * 1200.0) * MM3;
        assert!((v - want).abs() < 1e-6 * want, "{v} {want}");
        // No opening object comes in.
        assert!(
            m.definitions
                .values()
                .all(|d| d.attributes.kind != "IfcOpeningElement")
        );
        // Deterministic.
        let (again, _) = import(file(body).as_bytes(), &IfcOptions::default()).unwrap();
        let pts = |e: &Entities| -> Vec<[u64; 3]> {
            e.vertices
                .values()
                .map(|v| v.pos.to_array().map(f64::to_bits))
                .collect()
        };
        assert_eq!(pts(wall), pts(body_of(&again, "IfcWall")));
    }

    #[test]
    fn boolean_differences_and_clippings_are_cut() {
        // Column 1: a 1000 x 1000 x 3000 box clipped by a half space above z = 2000
        // (agreement false: the normal +z points into the material).
        // Column 2: the same box minus a 400 x 400 x 3000 box at its corner
        // (IfcBooleanResult with two solids).
        // Column 3: clipped by a polygonal bounded half space: the half space above
        // z = 2000 only over the square x, y in 0..500 (one corner removed).
        // Column 4: an open face set clipped: not cut, kept whole.
        let body = "\
#58=IFCDIRECTION((0.,0.,1.));
#57=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,1000.,1000.);
#56=IFCEXTRUDEDAREASOLID(#57,$,#58,3000.);
#90=IFCCARTESIANPOINT((0.,0.,2000.));
#91=IFCAXIS2PLACEMENT3D(#90,$,$);
#92=IFCPLANE(#91);
#93=IFCHALFSPACESOLID(#92,.F.);
#94=IFCBOOLEANCLIPPINGRESULT(.DIFFERENCE.,#56,#93);
#95=IFCSHAPEREPRESENTATION($,'Body','Clipping',(#94));
#96=IFCPRODUCTDEFINITIONSHAPE($,$,(#95));
#97=IFCCOLUMN('c1',$,'C1',$,$,$,#96,$,$);
#110=IFCCARTESIANPOINT((400.,400.,-10.));
#111=IFCAXIS2PLACEMENT3D(#110,$,$);
#112=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,400.,400.);
#113=IFCEXTRUDEDAREASOLID(#112,#111,#58,3020.);
#114=IFCBOOLEANRESULT(.DIFFERENCE.,#56,#113);
#115=IFCSHAPEREPRESENTATION($,'Body','CSG',(#114));
#116=IFCPRODUCTDEFINITIONSHAPE($,$,(#115));
#117=IFCBEAM('c2',$,'C2',$,$,$,#116,$,$);
#120=IFCPOLYLINE((#121,#122,#123,#124,#121));
#121=IFCCARTESIANPOINT((0.,0.));
#122=IFCCARTESIANPOINT((600.,0.));
#123=IFCCARTESIANPOINT((600.,600.));
#124=IFCCARTESIANPOINT((0.,600.));
#125=IFCPOLYGONALBOUNDEDHALFSPACE(#92,.F.,#6,#120);
#126=IFCBOOLEANCLIPPINGRESULT(.DIFFERENCE.,#56,#125);
#127=IFCSHAPEREPRESENTATION($,'Body','Clipping',(#126));
#128=IFCPRODUCTDEFINITIONSHAPE($,$,(#127));
#129=IFCMEMBER('c3',$,'C3',$,$,$,#128,$,$);
#130=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(3000.,0.,0.),(0.,3000.,3000.)));
#131=IFCTRIANGULATEDFACESET(#130,$,$,((1,2,3)),$);
#132=IFCBOOLEANCLIPPINGRESULT(.DIFFERENCE.,#131,#93);
#133=IFCSHAPEREPRESENTATION($,'Body','Clipping',(#132));
#134=IFCPRODUCTDEFINITIONSHAPE($,$,(#133));
#135=IFCPLATE('c4',$,'C4',$,$,$,#134,$,$);
#140=IFCRELCONTAINEDINSPATIALSTRUCTURE('c',$,$,$,(#97,#117,#129,#135),#30);
";
        let (m, report) = import(file(body).as_bytes(), &IfcOptions::default()).unwrap();
        valid(&m);
        let full = 1000.0 * 1000.0 * 3000.0 * MM3;
        let vol = |kind: &str| {
            let e = body_of(&m, kind);
            assert!(crate::is_solid(e), "{kind}");
            crate::solid::volume(e)
        };
        let near = |a: f64, b: f64| (a - b).abs() < 1e-6 * full;
        assert!(near(vol("IfcColumn"), full * 2.0 / 3.0));
        let corner = 100.0 * 100.0 * 3000.0 * MM3;
        assert!(near(vol("IfcBeam"), full - 9.0 * corner));
        let removed = 500.0 * 500.0 * 1000.0 * MM3;
        assert!(near(vol("IfcMember"), full - removed));
        assert_eq!(report.cut.get("IfcHalfSpaceSolid"), Some(&1));
        assert_eq!(report.cut.get("IfcExtrudedAreaSolid"), Some(&1));
        assert_eq!(report.cut.get("IfcPolygonalBoundedHalfSpace"), Some(&1));
        // The plate's open triangle is kept, its half space counted as skipped.
        assert_eq!(body_of(&m, "IfcPlate").faces.len(), 1);
        assert_eq!(report.skipped.get("IfcHalfSpaceSolid"), Some(&1));
    }

    #[test]
    fn openings_of_mapped_elements_are_not_cut() {
        // The element's body is a mapped item: not a solid in the kernel's sense
        // (nested instance), so the opening is counted as skipped.
        let body = "\
#58=IFCDIRECTION((0.,0.,1.));
#57=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,1000.,1000.);
#56=IFCEXTRUDEDAREASOLID(#57,$,#58,1000.);
#60=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#56));
#61=IFCREPRESENTATIONMAP(#6,#60);
#62=IFCCARTESIANTRANSFORMATIONOPERATOR3D($,$,#5,$,$);
#63=IFCMAPPEDITEM(#61,#62);
#64=IFCSHAPEREPRESENTATION($,'Body','MappedRepresentation',(#63));
#65=IFCPRODUCTDEFINITIONSHAPE($,$,(#64));
#66=IFCWALL('w',$,'W',$,$,#100,#65,$,$);
#80=IFCOPENINGELEMENT('o',$,$,$,$,#100,#82,$,.OPENING.);
#82=IFCPRODUCTDEFINITIONSHAPE($,$,(#60));
#88=IFCRELVOIDSELEMENT('v',$,$,$,#66,#80);
#70=IFCRELCONTAINEDINSPATIALSTRUCTURE('c',$,$,$,(#66),#30);
";
        let (m, report) = import(file(body).as_bytes(), &IfcOptions::default()).unwrap();
        valid(&m);
        assert_eq!(report.skipped.get("IfcOpeningElement"), Some(&1));
        assert!(report.cut.is_empty());
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
#335=IFCSECTIONEDSOLIDHORIZONTAL(#5);
#340=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#309,#313,#316,#327,#332,#334,#335));
#341=IFCPRODUCTDEFINITIONSHAPE($,$,(#340));
#342=IFCBUILDINGELEMENTPROXY('x',$,$,$,$,#100,#341,$,$);
#343=IFCRELCONTAINEDINSPATIALSTRUCTURE('c',$,$,$,(#342),#30);
";
        let (m, report) = import(file(body).as_bytes(), &IfcOptions::default()).unwrap();
        valid(&m);
        assert_eq!(report.items.get("IfcExtrudedAreaSolid"), Some(&5));
        assert_eq!(report.items.get("IfcSweptDiskSolid"), Some(&1));
        assert_eq!(report.skipped.get("IfcSectionedSolidHorizontal"), Some(&1));
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

    /// Volume (mm3) of the first solid body of class `kind`.
    fn solid_volume(m: &Model, kind: &str) -> f64 {
        let e = body_of(m, kind);
        assert!(crate::is_solid(e), "{kind} is not a solid");
        crate::solid::volume(e) / MM3
    }

    fn near(a: f64, b: f64, rel: f64) -> bool {
        (a - b).abs() <= rel * b.abs()
    }

    #[test]
    fn csg_primitives_revolutions_and_sweeps() {
        // One element per item (classes only tell them apart), millimeters.
        let element = |id: u64, class: &str, item: u64| {
            format!(
                "#{id}=IFCSHAPEREPRESENTATION($,'Body','CSG',(#{item}));\n\
                 #{}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{id}));\n\
                 #{}={class}('e{id}',$,$,$,$,#100,#{},$,$);\n",
                id + 1,
                id + 2,
                id + 1
            )
        };
        let mut body = String::from(
            "\
#58=IFCDIRECTION((0.,0.,1.));
#59=IFCDIRECTION((0.,1.,0.));
#200=IFCBLOCK(#6,1000.,2000.,3000.);
#201=IFCRIGHTCIRCULARCYLINDER(#6,1000.,500.);
#202=IFCRIGHTCIRCULARCONE(#6,1000.,500.);
#203=IFCSPHERE(#6,1000.);
#204=IFCRECTANGULARPYRAMID(#6,1000.,2000.,3000.);
#210=IFCCARTESIANPOINT((3000.,0.));
#211=IFCAXIS2PLACEMENT2D(#210,$);
#212=IFCRECTANGLEPROFILEDEF(.AREA.,$,#211,2000.,1000.);
#213=IFCAXIS1PLACEMENT(#5,#59);
#205=IFCREVOLVEDAREASOLID(#212,#6,#213,1.5707963267948966);
#220=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,100.,100.);
#221=IFCCARTESIANPOINT((1000.,0.,0.));
#222=IFCCARTESIANPOINT((1000.,1000.,0.));
#223=IFCPOLYLINE((#5,#221,#222));
#206=IFCFIXEDREFERENCESWEPTAREASOLID(#220,#6,#223,$,$,#58);
#230=IFCCARTESIANPOINT((500.,500.,500.));
#231=IFCAXIS2PLACEMENT3D(#230,$,$);
#232=IFCBLOCK(#6,1000.,1000.,1000.);
#233=IFCBLOCK(#231,1000.,1000.,1000.);
#234=IFCBOOLEANRESULT(.UNION.,#232,#233);
#207=IFCCSGSOLID(#234);
#240=IFCPLANE(#6);
#208=IFCSURFACECURVESWEPTAREASOLID(#220,#6,#223,$,$,#240);
",
        );
        let classes = [
            "IFCCOLUMN",
            "IFCBEAM",
            "IFCMEMBER",
            "IFCPLATE",
            "IFCFOOTING",
            "IFCRAILING",
            "IFCPILE",
            "IFCCOVERING",
            "IFCROOF",
        ];
        let mut ids = Vec::new();
        for (k, class) in classes.iter().enumerate() {
            let id = 300 + 10 * k as u64;
            body += &element(id, class, 200 + k as u64);
            ids.push(format!("#{}", id + 2));
        }
        body += &format!(
            "#400=IFCRELCONTAINEDINSPATIALSTRUCTURE('c',$,$,$,({}),#30);\n",
            ids.join(",")
        );
        let (m, report) = import(file(&body).as_bytes(), &IfcOptions::default()).unwrap();
        valid(&m);
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
        assert!(near(solid_volume(&m, "IfcColumn"), 6e9, 1e-9));
        // A 24-sided polygon of radius 500 instead of the circle.
        let disk = 12.0 * 500.0 * 500.0 * (TAU / 24.0).sin();
        assert!(near(solid_volume(&m, "IfcBeam"), disk * 1000.0, 1e-9));
        assert!(near(
            solid_volume(&m, "IfcMember"),
            disk * 1000.0 / 3.0,
            1e-9
        ));
        let ball = 4.0 / 3.0 * std::f64::consts::PI * 1e9;
        let v = solid_volume(&m, "IfcPlate");
        assert!(v < ball && v > 0.95 * ball, "{v}");
        assert!(near(solid_volume(&m, "IfcFooting"), 2e9, 1e-9));
        // Pappus: area times the centroid's path (a quarter turn at 3000 mm).
        let pappus = 2000.0 * 1000.0 * 3000.0 * TAU / 4.0;
        let v = solid_volume(&m, "IfcRailing");
        assert!(v < pappus && v > 0.98 * pappus, "{v} {pappus}");
        // A mitered square tube along two straight legs.
        assert!(near(
            solid_volume(&m, "IfcPile"),
            100.0 * 100.0 * 2000.0,
            1e-9
        ));
        assert!(near(
            solid_volume(&m, "IfcRoof"),
            100.0 * 100.0 * 2000.0,
            1e-9
        ));
        // The union of two overlapping cubes.
        assert!(near(solid_volume(&m, "IfcCovering"), 2e9 - 0.125e9, 1e-9));
        assert_eq!(report.joined.get("IfcBlock"), Some(&1));
        // The fixed reference (z) is the profile's x axis: the tube is 100 mm tall
        // around the storey's height.
        let pile = body_of(&m, "IfcPile");
        let zs: Vec<f64> = pile.vertices.values().map(|v| v.pos.z * 25.4).collect();
        let (z0, z1) = zs
            .iter()
            .fold((f64::MAX, f64::MIN), |(a, b), z| (a.min(*z), b.max(*z)));
        assert!(
            (z0 + 50.0).abs() < 1e-6 && (z1 - 50.0).abs() < 1e-6,
            "{z0} {z1}"
        );
    }

    #[test]
    fn advanced_brep_cylinder() {
        // A cylinder of radius 1000 and height 1000: two planar faces bounded by
        // circle edges, a cylindrical face bounded by the circles and a seam.
        let body = "\
#58=IFCDIRECTION((0.,0.,1.));
#59=IFCDIRECTION((0.,0.,-1.));
#500=IFCCARTESIANPOINT((1000.,0.,0.));
#501=IFCCARTESIANPOINT((1000.,0.,1000.));
#502=IFCVERTEXPOINT(#500);
#503=IFCVERTEXPOINT(#501);
#504=IFCCIRCLE(#6,1000.);
#505=IFCCARTESIANPOINT((0.,0.,1000.));
#506=IFCAXIS2PLACEMENT3D(#505,$,$);
#507=IFCCIRCLE(#506,1000.);
#508=IFCEDGECURVE(#502,#502,#504,.T.);
#509=IFCEDGECURVE(#503,#503,#507,.T.);
#510=IFCPOLYLINE((#500,#501));
#511=IFCEDGECURVE(#502,#503,#510,.T.);
#512=IFCORIENTEDEDGE(*,*,#508,.T.);
#513=IFCORIENTEDEDGE(*,*,#511,.T.);
#514=IFCORIENTEDEDGE(*,*,#509,.F.);
#515=IFCORIENTEDEDGE(*,*,#511,.F.);
#516=IFCEDGELOOP((#512,#513,#514,#515));
#517=IFCFACEOUTERBOUND(#516,.T.);
#518=IFCCYLINDRICALSURFACE(#6,1000.);
#519=IFCADVANCEDFACE((#517),#518,.T.);
#520=IFCEDGELOOP((#512));
#521=IFCFACEOUTERBOUND(#520,.T.);
#522=IFCAXIS2PLACEMENT3D(#5,#59,$);
#523=IFCPLANE(#522);
#524=IFCADVANCEDFACE((#521),#523,.T.);
#525=IFCORIENTEDEDGE(*,*,#509,.T.);
#526=IFCEDGELOOP((#525));
#527=IFCFACEOUTERBOUND(#526,.T.);
#528=IFCPLANE(#506);
#529=IFCADVANCEDFACE((#527),#528,.T.);
#530=IFCCLOSEDSHELL((#519,#524,#529));
#531=IFCADVANCEDBREP(#530);
#532=IFCSHAPEREPRESENTATION($,'Body','AdvancedBrep',(#531));
#533=IFCPRODUCTDEFINITIONSHAPE($,$,(#532));
#534=IFCCOLUMN('c',$,$,$,$,#100,#533,$,$);
#535=IFCRELCONTAINEDINSPATIALSTRUCTURE('c',$,$,$,(#534),#30);
";
        let (m, report) = import(file(body).as_bytes(), &IfcOptions::default()).unwrap();
        valid(&m);
        assert_eq!(report.items.get("IfcAdvancedBrep"), Some(&1));
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
        let disk = 12.0 * 1000.0 * 1000.0 * (TAU / 24.0).sin();
        assert!(near(solid_volume(&m, "IfcColumn"), disk * 1000.0, 1e-6));
    }

    #[test]
    fn far_site_is_moved_and_georeferenced() {
        // The site 2500 km east and 5600 km north (millimeters), at 48°8' N 11°34'30" E,
        // with a map conversion turned a quarter turn; a slab's points far away too.
        let body = "\
#7=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#6,$);
#8=IFCPROJECTEDCRS('EPSG:25832',$,$,$,$,$,$);
#9=IFCMAPCONVERSION(#7,#8,100.,200.,10.,0.,1.,$);
#50=IFCWALL('w',$,'Wall',$,$,#100,#52,$,$);
#52=IFCPRODUCTDEFINITIONSHAPE($,$,(#55));
#55=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#56));
#56=IFCEXTRUDEDAREASOLID(#57,$,#58,2500.);
#57=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,4000.,200.);
#58=IFCDIRECTION((0.,0.,1.));
#60=IFCCARTESIANPOINTLIST3D(((9000000000.,0.,0.),(9000001000.,0.,0.),(9000000000.,1000.,0.)));
#61=IFCTRIANGULATEDFACESET(#60,$,$,((1,2,3)),$);
#62=IFCSHAPEREPRESENTATION($,'Body','Tessellation',(#61));
#63=IFCPRODUCTDEFINITIONSHAPE($,$,(#62));
#64=IFCSLAB('s',$,'Far slab',$,$,#65,#63,$,$);
#65=IFCLOCALPLACEMENT(#100,#66);
#66=IFCAXIS2PLACEMENT3D(#67,$,$);
#67=IFCCARTESIANPOINT((-9000000000.,0.,0.));
#70=IFCRELCONTAINEDINSPATIALSTRUCTURE('c',$,$,$,(#50,#64),#30);
";
        let text = file(body)
            .replace(
                "#10=IFCSITE('s',$,'Site',$,$,#11,$,$,.ELEMENT.,$,$,$,$,$);",
                "#10=IFCSITE('s',$,'Site',$,$,#11,$,$,.ELEMENT.,(48,8,0,0),(11,34,30,0),$,$,$);",
            )
            .replace(
                "#11=IFCLOCALPLACEMENT($,#6);",
                "#11=IFCLOCALPLACEMENT($,#12);\n#12=IFCAXIS2PLACEMENT3D(#13,$,$);\n#13=IFCCARTESIANPOINT((2500000000.,5600000000.,0.));",
            );
        let (m, report) = import(text.as_bytes(), &IfcOptions::default()).unwrap();
        valid(&m);
        assert_eq!(report.moved, DVec3::new(2_500_000.0, 5_600_000.0, 0.0));
        assert!(report.summary().contains("moved"));
        // The wall is near the origin now; every definition's points are small.
        let wall = body_of(&m, "IfcWall");
        assert!(wall.vertices.values().all(|v| v.pos.length() < 1e4));
        let slab = body_of(&m, "IfcSlab");
        assert!(slab.vertices.values().all(|v| v.pos.length() < 1e4));
        let mut pts = Vec::new();
        world_points(&m, &m.root, DMat4::IDENTITY, &mut pts);
        let wall_world = pts.iter().map(|p| p.length()).fold(f64::MAX, f64::min);
        assert!(wall_world < 1e4 / 25.4 * 1000.0, "{wall_world}");
        let g = m.geo;
        assert!(g.located);
        assert!((m.shadows.latitude - (48.0 + 8.0 / 60.0)).abs() < 1e-9);
        assert!((m.shadows.longitude - (11.0 + 34.5 / 60.0)).abs() < 1e-9);
        assert!((g.north_angle - 90.0).abs() < 1e-9);
        assert_eq!(g.epsg, 25832);
        // Map coordinates of the new origin (map units are the file's millimeters):
        // the move turned a quarter turn, plus the conversion's offset.
        let want = DVec3::new(100.0 - 5.6e9, 200.0 + 2.5e9, 10.0) * 0.001;
        assert!((g.map_origin - want).length() < 1e-6, "{}", g.map_origin);
    }

    #[test]
    fn texture_maps() {
        // A textured triangle: an embedded PNG (blob) and one read by URL.
        let png = ifc_geom::png(1, 1, 3, &[10, 200, 30]).unwrap();
        let hex: String = png.iter().map(|b| format!("{b:02X}")).collect();
        let body = format!(
            "\
#60=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(1000.,0.,0.),(0.,1000.,0.)));
#61=IFCTRIANGULATEDFACESET(#60,$,$,((1,2,3)),$);
#62=IFCTEXTUREVERTEXLIST(((0.,0.),(2.,0.),(0.,2.)));
#63=IFCBLOBTEXTURE(.T.,.T.,$,$,$,'PNG',\"0{hex}\");
#64=IFCINDEXEDTRIANGLETEXTUREMAP((#63),#61,#62,((1,2,3)));
#70=IFCCARTESIANPOINTLIST3D(((0.,0.,500.),(1000.,0.,500.),(0.,1000.,500.)));
#71=IFCTRIANGULATEDFACESET(#70,$,$,((1,2,3)),$);
#73=IFCIMAGETEXTURE(.T.,.T.,$,$,$,'maps/wood.png');
#74=IFCINDEXEDTRIANGLETEXTUREMAP((#73),#71,#62,$);
#80=IFCSHAPEREPRESENTATION($,'Body','Tessellation',(#61,#71));
#81=IFCPRODUCTDEFINITIONSHAPE($,$,(#80));
#82=IFCSLAB('s',$,$,$,$,#100,#81,$,$);
#83=IFCRELCONTAINEDINSPATIALSTRUCTURE('c',$,$,$,(#82),#30);
"
        );
        let read = |name: &str| (name == "maps/wood.png").then(|| png.clone());
        let (m, _) = import_with(file(&body).as_bytes(), &IfcOptions::default(), &read).unwrap();
        valid(&m);
        let textured: Vec<&Material> = m.materials.iter().filter(|x| x.texture.is_some()).collect();
        assert_eq!(textured.len(), 2);
        assert!(textured.iter().any(|x| x.name == "wood"));
        let slab = body_of(&m, "IfcSlab");
        assert_eq!(slab.faces.len(), 2);
        assert!(slab.faces.values().all(|f| f.attrs.uv.is_some()));
        // Without the file: the image texture is counted as skipped.
        let (_, report) = import(file(&body).as_bytes(), &IfcOptions::default()).unwrap();
        assert_eq!(report.skipped.get("IfcImageTexture"), Some(&1));
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
