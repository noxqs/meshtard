//! Geometry kernel: a non-manifold boundary representation (vertices, edges, faces with
//! holes; an edge may border any number of faces) with SketchUp's modeling behavior.
//!
//! Units are inches, Z up, as in `.skp` files.

pub mod annotation;
pub mod arrangement;
pub mod bvh;
pub mod clipboard;
pub mod entities;
pub mod export;
pub mod follow_me;
pub mod geom;
pub mod group;
pub mod import;
pub mod intersect;
pub mod mesh_export;
pub mod mesh_import;
pub mod model;
pub mod offset;
pub mod ops;
pub mod push_pull;
pub mod solid;
pub mod spatial;
pub mod tessellate;
pub mod texture;
pub mod transform;
pub mod validate;
pub mod visibility;

pub use annotation::{Dimension, DimensionKind, Text};
pub use entities::*;
pub use follow_me::follow_me;
pub use intersect::intersect_faces;
pub use model::{
    Definition, FaceStyle, FogSettings, GlueTo, Layer, Material, Model, Page, ShadowSettings,
    StyleSettings,
};
pub use offset::offset_face;
pub use ops::{add_curve, add_line, erase_curve, erase_edge};
pub use push_pull::push_pull;
pub use solid::{SolidError, SolidInput, is_solid};
pub use texture::{FaceUv, Texture};
pub use transform::{copy_geometry, copy_geometry_from, transform_vertices, vertices_of};
pub use validate::{Checks, validate};

/// Distance below which two points are the same, in inches. SketchUp's internal tolerance
/// is believed to be about a thousandth of an inch (unverified).
pub const TOL: f64 = 1e-3;
