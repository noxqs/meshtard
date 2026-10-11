//! SketchUp's Push/Pull tool.

use glam::{DMat4, DVec3};

use crate::TOL;
use crate::entities::{Entities, FaceId};
use crate::geom::Plane;
use crate::ops::insert_line;
use crate::transform::{
    Desired, drop_unsound_faces, face_edges, face_sample, fill, flip_face, transform_vertices,
    vertices_of,
};

/// SketchUp's Push/Pull of `face` by `distance` along its normal (negative pushes in).
/// - If every boundary edge has other faces and they are all perpendicular to the face,
///   the face moves and its neighbors stretch (no new faces).
/// - Otherwise (or when `new_faces`, Ctrl in SketchUp) side faces are created for every
///   loop edge (holes too) and a new face with the face's attributes is created at the
///   offset; the original face stays as the base only if it was free-standing (no other
///   faces on its edges), oriented to point out of the new solid.
/// - A new face landing on a coplanar face looking the other way cancels with it (both
///   are removed): pushing a rectangle drawn on a box top down by the full height cuts a
///   hole through.
///
/// Returns the face at the pushed position (None if nothing happened, e.g. distance ~0,
/// or when the pushed face cancelled out).
pub fn push_pull(
    ents: &mut Entities,
    face: FaceId,
    distance: f64,
    new_faces: bool,
) -> Option<FaceId> {
    if !ents.faces.contains_key(face) || distance.abs() <= TOL {
        return None;
    }
    ents.ensure_index();
    let n = ents.faces[face].normal;
    let t = n * distance;
    let edges = face_edges(ents, face);
    let others = |e| ents.edges[e].faces.iter().filter(move |&&g| g != face);
    let free = edges.iter().all(|&e| others(e).next().is_none());
    let stretch = !new_faces
        && edges.iter().all(|&e| {
            others(e).next().is_some()
                && others(e).all(|&g| ents.faces[g].normal.dot(n).abs() < 1e-6)
        });

    if stretch {
        let target = face_sample(ents, face) + t;
        let vs = vertices_of(ents, &[], &[face]);
        transform_vertices(ents, &vs, DMat4::from_translation(t));
        if ents.faces.contains_key(face) {
            return Some(face);
        }
        return ents
            .faces
            .keys()
            .find(|&g| ents.faces[g].normal.dot(n) > 0.0 && ents.face_contains(g, target));
    }

    let f = &ents.faces[face];
    let loops: Vec<Vec<DVec3>> = f.loops().map(|l| ents.loop_points(l)).collect();
    let attrs = f.attrs.clone();
    let sign = distance.signum();
    // Free-standing faces make a solid between the two positions, facing out; faces on a
    // solid make a bump or a pocket whose faces look at the open side.
    let (top_normal, side_sign) = if free { (n * sign, 1.0) } else { (n, sign) };
    if free {
        if distance > 0.0 {
            flip_face(ents, face);
        }
    } else {
        ents.remove_face(face);
    }

    for l in &loops {
        for (i, &p) in l.iter().enumerate() {
            let q = l[(i + 1) % l.len()];
            insert_line(ents, p, p + t, false);
            insert_line(ents, p + t, q + t, false);
        }
    }

    let moved = |l: &Vec<DVec3>| -> Vec<DVec3> { l.iter().map(|p| *p + t).collect() };
    let mut desired = vec![Desired {
        plane: Plane::new(top_normal, loops[0][0] + t),
        polys: vec![(moved(&loops[0]), loops[1..].iter().map(moved).collect())],
        attrs,
    }];
    for l in &loops {
        for (i, &a) in l.iter().enumerate() {
            let b = l[(i + 1) % l.len()];
            let normal = (b - a).cross(n) * side_sign;
            if normal.length() <= 1e-12 {
                continue;
            }
            desired.push(Desired {
                plane: Plane::new(normal, a),
                polys: vec![(vec![a, b, b + t, a + t], Vec::new())],
                attrs: Default::default(),
            });
        }
    }
    let made = fill(ents, &desired, true);
    let region = crate::spatial::Aabb::of(loops.iter().flatten().flat_map(|&p| [p, p + t]));
    drop_unsound_faces(ents, &region);
    made[0]
        .first()
        .copied()
        .filter(|f| ents.faces.contains_key(*f))
}
