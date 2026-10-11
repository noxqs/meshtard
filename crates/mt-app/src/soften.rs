//! Window > Soften Edges: softens (and optionally smooths) the selected edges whose
//! faces meet at less than the panel's angle, live as the settings change.
//!
//! The edges are the selected ones plus the edges of selected faces (unverified that
//! SketchUp includes face edges). An edge between two faces is softened when the
//! angle between the faces' normals is at most the slider's angle; edges between
//! coplanar faces only with Soften coplanar; the others are unsoftened. Edges without
//! exactly two faces are left alone. Consecutive changes on the same selection replace
//! each other's undo step, so a slider drag is one undo step.

use mt_kernel::{EdgeId, Entities};

use crate::App;
use crate::document::Owner;

/// Normals closer than this (radians) count as coplanar.
const COPLANAR: f64 = 1e-3;

/// The Soften Edges panel's settings and the undo step its last change made.
#[derive(Debug, Default)]
pub struct SoftenPanel {
    pub settings: mt_ui::SoftenSettings,
    /// The document state after the last change and the selection it applied to.
    step: Option<(u64, Vec<Owner>)>,
}

/// Angle in radians between the normals of the two faces of edge `e`, with the faces
/// taken as consistently oriented across the edge (0 for coplanar faces); `None` unless
/// the edge has exactly two faces.
pub fn normal_angle(ents: &Entities, e: EdgeId) -> Option<f64> {
    let edge = ents.edges.get(e)?;
    let [f, g] = edge.faces[..] else {
        return None;
    };
    let dir = |face| {
        ents.faces
            .get(face)?
            .loops()
            .find_map(|l| l.coedges.iter().find(|c| c.edge == e).map(|c| c.reversed))
    };
    let (nf, mut ng) = (ents.faces.get(f)?.normal, ents.faces.get(g)?.normal);
    // Consistently oriented faces run a shared edge in opposite directions.
    if dir(f)? == dir(g)? {
        ng = -ng;
    }
    Some(nf.dot(ng).clamp(-1.0, 1.0).acos())
}

/// The selected edges and the edges of selected faces.
pub fn edges_of(ents: &Entities, items: &[Owner]) -> Vec<EdgeId> {
    let mut out = Vec::new();
    for o in items {
        match *o {
            Owner::Edge(e) if ents.edges.contains_key(e) => out.push(e),
            Owner::Face(f) => {
                if let Some(face) = ents.faces.get(f) {
                    out.extend(face.loops().flat_map(|l| l.coedges.iter().map(|c| c.edge)));
                }
            }
            _ => {}
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Softens or unsoftens `edges` for `s`; returns whether any edge changed.
pub fn apply(ents: &mut Entities, edges: &[EdgeId], s: &mt_ui::SoftenSettings) -> bool {
    let limit = f64::from(s.angle).to_radians();
    let mut changed = false;
    for &e in edges {
        let Some(angle) = normal_angle(ents, e) else {
            continue;
        };
        let soft = if angle < COPLANAR {
            s.coplanar
        } else {
            angle <= limit + 1e-9
        };
        let smooth = soft && s.smooth;
        let attrs = &mut ents.edges[e].attrs;
        if (attrs.soft, attrs.smooth) != (soft, smooth) {
            attrs.soft = soft;
            attrs.smooth = smooth;
            changed = true;
        }
    }
    changed
}

impl App {
    /// The panel's settings changed: applies them to the selection. Returns whether
    /// the scene must be rebuilt.
    pub(crate) fn soften_changed(&mut self, s: mt_ui::SoftenSettings) -> bool {
        self.soften.settings = s;
        let items = self.selection.items().to_vec();
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        let mut undone = false;
        if let Some((state, sel)) = &self.soften.step
            && *state == doc.state()
            && *sel == items
        {
            // The previous change of this drag: start again from before it.
            undone = doc.undo();
        }
        let edges = edges_of(doc.active(), &items);
        let changed = !edges.is_empty() && doc.edit(|ents| apply(ents, &edges, &s));
        self.soften.step = changed.then(|| (doc.state(), items));
        changed || undone
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    /// Two squares meeting along the x axis at `deg` degrees (0 = flat).
    fn fold(deg: f64) -> (Entities, EdgeId) {
        let mut ents = Entities::new();
        let (c, s) = (deg.to_radians().cos(), deg.to_radians().sin());
        let a = [DVec3::ZERO, DVec3::X * 10.0];
        let up = DVec3::new(0.0, 10.0 * c, 10.0 * s);
        let down = DVec3::new(0.0, -10.0, 0.0);
        for (p, q) in [
            (a[0], a[1]),
            (a[1], a[1] + up),
            (a[1] + up, a[0] + up),
            (a[0] + up, a[0]),
            (a[1], a[1] + down),
            (a[1] + down, a[0] + down),
            (a[0] + down, a[0]),
        ] {
            mt_kernel::add_line(&mut ents, p, q);
        }
        let e = ents
            .edges
            .iter()
            .find(|(_, x)| x.faces.len() == 2)
            .map(|(id, _)| id)
            .expect("shared edge");
        (ents, e)
    }

    #[test]
    fn angle_threshold_and_coplanar() {
        let (mut ents, e) = fold(15.0);
        let a = normal_angle(&ents, e).unwrap().to_degrees();
        assert!((a - 15.0).abs() < 1e-6, "{a}");
        let mut s = mt_ui::SoftenSettings::default();
        assert!(apply(&mut ents, &[e], &s), "15 <= 20");
        assert!(ents.edges[e].attrs.soft && ents.edges[e].attrs.smooth);
        s.angle = 10;
        assert!(apply(&mut ents, &[e], &s), "unsoftened below the angle");
        assert!(!ents.edges[e].attrs.soft);
        assert!(!apply(&mut ents, &[e], &s), "no change");

        // A sharp fold (165 degrees between normals) stays hard at 20.
        let (ents2, e2) = fold(165.0);
        assert!((normal_angle(&ents2, e2).unwrap().to_degrees() - 165.0).abs() < 1e-6);

        // The flat case, only with Soften coplanar: make the faces' normals agree
        // (coplanar squares drawn this way would merge into one face).
        let (mut flat, e3) = fold(15.0);
        let [f, g] = flat.edges[e3].faces[..] else {
            unreachable!()
        };
        let nf = flat.faces[f].normal;
        flat.faces[g].normal = nf;
        if normal_angle(&flat, e3).unwrap() > 1.0 {
            flat.faces[g].normal = -nf;
        }
        assert!(normal_angle(&flat, e3).unwrap() < COPLANAR);
        s = mt_ui::SoftenSettings::default();
        assert!(!apply(&mut flat, &[e3], &s));
        s.coplanar = true;
        assert!(apply(&mut flat, &[e3], &s));
        // Edges without two faces are left alone.
        let border = flat
            .edges
            .iter()
            .find(|(_, x)| x.faces.len() == 1)
            .unwrap()
            .0;
        assert!(!apply(&mut flat, &[border], &s));
    }

    #[test]
    fn faces_bring_their_edges() {
        let (ents, e) = fold(30.0);
        let f = ents.edges[e].faces[0];
        let edges = edges_of(&ents, &[Owner::Face(f)]);
        assert_eq!(edges.len(), 4);
        assert!(edges.contains(&e));
    }
}
