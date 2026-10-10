//! Geometry kernel: a non-manifold boundary representation (vertices, edges, faces with
//! holes; an edge may border any number of faces) with SketchUp's modeling behavior.
//!
//! Units are inches, Z up, as in `.skp` files.

pub mod arrangement;
pub mod entities;
pub mod geom;
pub mod import;
pub mod model;
pub mod ops;
pub mod tessellate;
pub mod validate;

pub use entities::*;
pub use model::{Definition, Layer, Material, Model};
pub use ops::{add_line, erase_edge};
pub use validate::{Checks, validate};

/// Distance below which two points are the same, in inches. SketchUp's internal tolerance
/// is believed to be about a thousandth of an inch (unverified).
pub const TOL: f64 = 1e-3;
