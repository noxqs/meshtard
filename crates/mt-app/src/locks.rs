//! Modifier-key inference locks of the drawing and modify tools (FEATURES.md 70; help
//! page "Introducing Drawing Basics and Concepts"):
//!
//! - Shift held, after the first point: locks the direction under the cursor. An axis,
//!   parallel or perpendicular direction locks as it is; a point inference (endpoint,
//!   midpoint, guide point, on edge, intersection, center, component origin...) locks
//!   the line from the anchor through that point ([`mt_infer::Kind::ThroughPoint`]).
//!   The cursor then slides on that line. While Shift is held and nothing is locked yet,
//!   the lock is taken as soon as the cursor reaches something lockable.
//! - Shift held over a face, in a tool with a plane (Rectangle, Circle, Polygon, Arc,
//!   2 Point Arc, Pie, Rotate, Protractor): locks the tool's plane to the face's.
//! - Shift+Alt over a face: also holds the cursor to that plane (through the anchor);
//!   in the Line tool this is the face-plane lock. Rotate and Protractor always hold
//!   the cursor to the Shift-locked plane, and Alt frees the center point while the
//!   plane stays locked.
//! - Alt alone in the Line tool after the first click cycles the inference filter
//!   ([`mt_infer::Filter`]); in the other tools Alt keeps its own meaning (Paint
//!   Bucket sample, Tag sample, Flip axes...).
//!
//! Release rules (unverified, TRIAL_CHECKLIST.md 1e): releasing Shift drops what Shift
//! locked; an arrow key pressed again toggles its lock off; Escape cancels the tool
//! and drops every lock and the filter; a click that completes a segment drops the
//! direction lock (taken again from the new point while Shift is still held).

use glam::DVec3;
use mt_infer::{Engine, Inference, Kind};

/// What the tool under the keys offers.
pub trait LockTarget {
    /// See `DrawTool::lock_face_plane`.
    fn lock_face_plane(&mut self, normal: Option<DVec3>) -> bool;
    /// See `DrawTool::alt_frees_center`.
    fn alt_frees_center(&self) -> bool;
}

/// A drawing tool as a [`LockTarget`].
pub struct Draw<'a>(pub &'a mut dyn crate::tools::DrawTool);

impl LockTarget for Draw<'_> {
    fn lock_face_plane(&mut self, normal: Option<DVec3>) -> bool {
        self.0.lock_face_plane(normal)
    }
    fn alt_frees_center(&self) -> bool {
        self.0.alt_frees_center()
    }
}

/// A modify tool as a [`LockTarget`].
pub struct Modify<'a>(pub &'a mut dyn crate::modify::ModifyTool);

impl LockTarget for Modify<'_> {
    fn lock_face_plane(&mut self, normal: Option<DVec3>) -> bool {
        self.0.lock_face_plane(normal)
    }
    fn alt_frees_center(&self) -> bool {
        self.0.alt_frees_center()
    }
}

/// The keys held this frame and what they may do.
#[derive(Debug, Clone, Copy, Default)]
pub struct Keys {
    pub shift: bool,
    pub alt: bool,
    /// Shift locks directions and planes in this tool.
    pub shift_locks: bool,
    /// Alt cycles the inference filter (the Line tool after its first click).
    pub alt_cycles: bool,
}

/// The Shift and Alt lock state carried between frames.
#[derive(Debug, Clone, Copy, Default)]
pub struct ModifierLocks {
    /// Shift locked a direction.
    dir: bool,
    /// Shift locked a plane (the tool's, the engine's or both).
    plane: bool,
    /// The tool's plane is locked (to unlock it on release).
    tool_plane: bool,
    /// The engine holds the cursor to the plane.
    engine_plane: bool,
    alt_was: bool,
}

impl ModifierLocks {
    /// Applies the keys of this frame; `inf` is the inference under the cursor (world
    /// space) and `anchor` the tool's last point (world space).
    pub fn update(
        &mut self,
        engine: &mut Engine,
        tool: &mut dyn LockTarget,
        inf: Option<&Inference>,
        anchor: Option<DVec3>,
        keys: Keys,
    ) {
        let alt_pressed = keys.alt && !self.alt_was;
        self.alt_was = keys.alt;
        let shift = keys.shift && keys.shift_locks;
        if shift && !self.dir && !self.plane {
            if let Some(i) = inf {
                match (i.kind, i.normal) {
                    (Kind::OnFace, Some(n)) => {
                        self.tool_plane = tool.lock_face_plane(Some(n));
                        // Rotate and Protractor hold the center to the plane (Alt
                        // frees it); the other tools only with Shift+Alt.
                        self.engine_plane = keys.alt || tool.alt_frees_center();
                        if self.engine_plane {
                            engine.lock_plane(i.point, n);
                        }
                        self.plane = self.tool_plane || self.engine_plane;
                    }
                    _ => {
                        if let Some(a) = anchor {
                            self.dir = engine.lock_current(i, a);
                        }
                    }
                }
            }
        } else if !shift {
            self.release(engine, tool);
        }
        // Alt frees the center point of Rotate and Protractor, the plane stays.
        engine.plane_free = keys.alt && self.engine_plane && tool.alt_frees_center();
        if alt_pressed && !keys.shift && keys.alt_cycles {
            engine.filter = engine.filter.next();
        }
    }

    /// Drops what Shift locked.
    pub fn release(&mut self, engine: &mut Engine, tool: &mut dyn LockTarget) {
        if self.dir {
            engine.unlock();
        }
        if self.tool_plane {
            tool.lock_face_plane(None);
        }
        if self.engine_plane {
            engine.unlock_plane();
        }
        *self = Self {
            alt_was: self.alt_was,
            ..Self::default()
        };
    }

    /// A click changed the model: the direction lock goes (re-taken from the new
    /// anchor while Shift is held); a plane lock stays while Shift is held.
    pub fn after_change(&mut self, engine: &mut Engine) {
        engine.unlock();
        self.dir = false;
    }

    /// Escape or a tool change: every lock and the filter go (the tool's own plane
    /// lock is reset by its cancel or replacement).
    pub fn reset(&mut self, engine: &mut Engine) {
        engine.unlock();
        engine.unlock_plane();
        engine.filter = mt_infer::Filter::All;
        *self = Self {
            alt_was: self.alt_was,
            ..Self::default()
        };
    }

    /// Shift locked something.
    #[cfg(test)]
    fn active(&self) -> bool {
        self.dir || self.plane
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{DrawTool, for_command};
    use mt_infer::{Filter, Kind};

    fn inf(kind: Kind, point: DVec3, normal: Option<DVec3>) -> Inference {
        Inference {
            point,
            kind,
            edge: None,
            guide: None,
            locked: false,
            normal,
            segment: None,
            container: None,
        }
    }

    const SHIFT: Keys = Keys {
        shift: true,
        alt: false,
        shift_locks: true,
        alt_cycles: false,
    };
    const NONE: Keys = Keys {
        shift: false,
        alt: false,
        shift_locks: true,
        alt_cycles: false,
    };

    #[test]
    fn line_shift_locks_through_a_point_and_release_unlocks() {
        let mut e = Engine::default();
        let mut l = ModifierLocks::default();
        let mut line = for_command("tool.line").unwrap();
        let a = DVec3::ZERO;
        let end = inf(Kind::Endpoint, DVec3::new(10.0, 5.0, 0.0), None);
        // Shift with nothing lockable under the cursor: nothing yet.
        let free = inf(Kind::Free, DVec3::new(3.0, 1.0, 0.0), None);
        l.update(
            &mut e,
            &mut Draw(line.as_mut()),
            Some(&free),
            Some(a),
            SHIFT,
        );
        assert!(!e.is_locked() && !l.active());
        // Still holding Shift, the cursor reaches an endpoint: locked through it.
        l.update(&mut e, &mut Draw(line.as_mut()), Some(&end), Some(a), SHIFT);
        assert!(e.is_locked() && l.active());
        // Released.
        l.update(&mut e, &mut Draw(line.as_mut()), Some(&end), Some(a), NONE);
        assert!(!e.is_locked() && !l.active());
        // Shift over a face in the Line tool: no plane (the tool has none) ...
        let face = inf(Kind::OnFace, DVec3::new(1.0, 1.0, 0.0), Some(DVec3::Z));
        l.update(
            &mut e,
            &mut Draw(line.as_mut()),
            Some(&face),
            Some(a),
            SHIFT,
        );
        assert!(e.locked_plane().is_none());
        l.update(&mut e, &mut Draw(line.as_mut()), Some(&face), Some(a), NONE);
        // ... but Shift+Alt locks the cursor to the face's plane.
        let shift_alt = Keys { alt: true, ..SHIFT };
        l.update(
            &mut e,
            &mut Draw(line.as_mut()),
            Some(&face),
            Some(a),
            shift_alt,
        );
        assert_eq!(e.locked_plane().map(|p| p.1), Some(DVec3::Z));
        assert!(!e.plane_free);
        l.update(&mut e, &mut Draw(line.as_mut()), Some(&face), Some(a), NONE);
        assert!(e.locked_plane().is_none());
    }

    #[test]
    fn line_alt_cycles_the_filter_only_where_allowed() {
        let mut e = Engine::default();
        let mut l = ModifierLocks::default();
        let mut line = for_command("tool.line").unwrap();
        let cycle = Keys {
            alt: true,
            alt_cycles: true,
            ..NONE
        };
        let up = Keys {
            alt_cycles: true,
            ..NONE
        };
        let mut press = |e: &mut Engine| {
            l.update(e, &mut Draw(line.as_mut()), None, None, cycle);
            l.update(e, &mut Draw(line.as_mut()), None, None, up);
        };
        press(&mut e);
        assert_eq!(e.filter, Filter::PointsOnly);
        press(&mut e);
        assert_eq!(e.filter, Filter::ParallelPerpendicular);
        press(&mut e);
        assert_eq!(e.filter, Filter::All);
        // Held Alt does not repeat; tools that do not cycle leave the filter alone.
        let mut l = ModifierLocks::default();
        let mut line = for_command("tool.line").unwrap();
        l.update(&mut e, &mut Draw(line.as_mut()), None, None, cycle);
        l.update(&mut e, &mut Draw(line.as_mut()), None, None, cycle);
        assert_eq!(e.filter, Filter::PointsOnly);
        let mut l = ModifierLocks::default();
        l.update(
            &mut e,
            &mut Draw(line.as_mut()),
            None,
            None,
            Keys { alt: true, ..NONE },
        );
        assert_eq!(e.filter, Filter::PointsOnly);
        l.reset(&mut e);
        assert_eq!(e.filter, Filter::All);
    }

    #[test]
    fn rectangle_shift_on_a_face_locks_its_plane() {
        let mut e = Engine::default();
        let mut l = ModifierLocks::default();
        let mut rect = for_command("tool.rectangle").unwrap();
        // A wall facing red, hovered with Shift: the plane locks before the first click.
        let wall = inf(Kind::OnFace, DVec3::new(0.0, 5.0, 5.0), Some(DVec3::X));
        l.update(&mut e, &mut Draw(rect.as_mut()), Some(&wall), None, SHIFT);
        assert!(l.active());
        assert!(
            e.locked_plane().is_none(),
            "Shift alone: the tool's plane only"
        );
        // First corner on the ground (a point there, no face normal): the rectangle
        // still stands in the wall's plane.
        let mut ents = mt_kernel::Entities::default();
        let ground = inf(Kind::Free, DVec3::new(50.0, 0.0, 0.0), None);
        rect.click(&mut ents, &ground);
        // Shift released after the first corner: the plane stays.
        let anchor = rect.anchor();
        l.update(
            &mut e,
            &mut Draw(rect.as_mut()),
            Some(&ground),
            anchor,
            NONE,
        );
        let c = inf(Kind::Free, DVec3::new(50.0, 20.0, 10.0), None);
        rect.click(&mut ents, &c);
        assert_eq!(ents.edges.len(), 4);
        for v in ents.vertices.values() {
            assert!((v.pos.x - 50.0).abs() < 1e-9, "{:?}", v.pos);
        }
        // Shift+Alt: also the engine's plane.
        let shift_alt = Keys { alt: true, ..SHIFT };
        l.update(
            &mut e,
            &mut Draw(rect.as_mut()),
            Some(&wall),
            None,
            shift_alt,
        );
        assert_eq!(e.locked_plane().map(|p| p.1), Some(DVec3::X));
        l.update(&mut e, &mut Draw(rect.as_mut()), Some(&wall), None, NONE);
        assert!(e.locked_plane().is_none());
    }

    #[test]
    fn protractor_and_rotate_alt_frees_the_center() {
        for id in ["tool.protractor", "tool.rotate"] {
            let mut e = Engine::default();
            let mut l = ModifierLocks::default();
            let wall = inf(Kind::OnFace, DVec3::new(0.0, 5.0, 5.0), Some(DVec3::X));
            let mut draw = for_command(id);
            let mut modify = crate::modify::for_command(id);
            let (mut d, mut m);
            let tool: &mut dyn LockTarget = match (&mut draw, &mut modify) {
                (Some(t), _) => {
                    d = Draw(t.as_mut());
                    &mut d
                }
                (_, Some(t)) => {
                    m = Modify(t.as_mut());
                    &mut m
                }
                _ => unreachable!(),
            };
            l.update(&mut e, tool, Some(&wall), None, SHIFT);
            assert!(e.locked_plane().is_some(), "{id}: center held to the plane");
            assert!(!e.plane_free);
            // Alt with Shift still held: the center is free, the plane stays.
            l.update(&mut e, tool, Some(&wall), None, Keys { alt: true, ..SHIFT });
            assert!(e.plane_free && e.locked_plane().is_some(), "{id}");
            l.update(&mut e, tool, Some(&wall), None, SHIFT);
            assert!(!e.plane_free);
            l.update(&mut e, tool, Some(&wall), None, NONE);
            assert!(e.locked_plane().is_none());
        }
    }

    #[test]
    fn rotate_in_a_locked_plane_turns_about_its_normal() {
        let mut rot = crate::modify::for_command("tool.rotate").unwrap();
        assert!(rot.direction_locks());
        assert!(rot.lock_face_plane(Some(DVec3::X)));
        let mut ents = mt_kernel::Entities::default();
        mt_kernel::add_line(
            &mut ents,
            DVec3::new(0.0, 10.0, 0.0),
            DVec3::new(0.0, 20.0, 0.0),
        );
        let edge = ents.edges.keys().next().unwrap();
        let sel = [crate::document::Owner::Edge(edge)];
        let mut ctx = crate::modify::Ctx {
            ents: &mut ents,
            picked: None,
            selection: &sel,
            ctrl: false,
            eye: DVec3::new(100.0, 0.0, 0.0),
        };
        // Center at the origin, start along green, end up blue (points off the plane
        // are projected): a quarter turn about red.
        rot.click(&mut ctx, &inf(Kind::Free, DVec3::ZERO, None));
        rot.lock_face_plane(None);
        rot.click(&mut ctx, &inf(Kind::Free, DVec3::new(5.0, 10.0, 0.0), None));
        let (_, v) = rot.measurement(
            Some(&inf(Kind::Free, DVec3::new(-3.0, 0.0, 10.0), None)),
            mt_infer::units::LengthUnit::Inch,
        );
        assert_eq!(v, "90.0");
        assert!(rot.click(
            &mut ctx,
            &inf(Kind::Free, DVec3::new(-3.0, 0.0, 10.0), None)
        ));
        let e = ents.edges.values().next().unwrap();
        let (a, b) = (ents.pos(e.start), ents.pos(e.end));
        assert!((a - DVec3::new(0.0, 0.0, 10.0)).length() < 1e-9, "{a:?}");
        assert!((b - DVec3::new(0.0, 0.0, 20.0)).length() < 1e-9, "{b:?}");
    }

    #[test]
    fn modify_tools_take_direction_locks() {
        for id in [
            "tool.move",
            "tool.rotate",
            "tool.scale",
            "tool.offset",
            "tool.push_pull",
            "tool.follow_me",
            "tool.flip",
        ] {
            let t = crate::modify::for_command(id).unwrap();
            assert!(t.direction_locks(), "{id}");
            assert_eq!(t.shift_is_modifier(), id == "tool.scale", "{id}");
        }
        for id in ["tool.paint_bucket", "tool.section_plane"] {
            assert!(!crate::modify::for_command(id).unwrap().direction_locks());
        }
        // Tools with a plane take the Shift face lock.
        for id in [
            "tool.rectangle",
            "tool.circle",
            "tool.polygon",
            "tool.arc",
            "tool.arc_2pt",
            "tool.pie",
            "tool.protractor",
        ] {
            let mut t = for_command(id).unwrap();
            assert!(
                DrawTool::lock_face_plane(t.as_mut(), Some(DVec3::X)),
                "{id}"
            );
        }
        let mut line = for_command("tool.line").unwrap();
        assert!(!DrawTool::lock_face_plane(line.as_mut(), Some(DVec3::X)));
    }
}
