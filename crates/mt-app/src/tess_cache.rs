//! Faces' triangles kept from one scene build to the next (spec/PERFORMANCE.md): after
//! an edit only the faces it changed are tessellated again.
//!
//! Each entities collection (the root, a definition) keeps its faces' triangles in
//! the order the scene walk reaches them, in flat lists. The next build walks the faces
//! in the same order and reuses an entry only when the face still has the same loops
//! (vertices in order), positions and normal, the only inputs of
//! `tessellate::triangulate`; anything else is tessellated again. The lists are read
//! front to back, which keeps the check cheaper than the tessellation it saves.

use std::ops::Range;

use glam::DVec3;
use mt_kernel::slot_store::SlotStore;
use mt_kernel::spatial::FxMap;
use mt_kernel::{Entities, Face, FaceId, VertexId, tessellate};

use crate::undo::Ctx;

/// One face's entry in a [`Coll`].
struct Entry {
    face: FaceId,
    normal: DVec3,
    /// Range in `Coll::from`.
    from: Range<u32>,
    /// Range in `Coll::tris`.
    tris: Range<u32>,
}

/// The kept triangles of one collection.
#[derive(Default)]
struct Coll {
    /// In the order the faces were reached (the collection's face order).
    entries: Vec<Entry>,
    /// Each entry's loops: vertices and positions in order, loops separated by a
    /// null id.
    from: Vec<(VertexId, DVec3)>,
    tris: Vec<[DVec3; 3]>,
    /// The build that made it.
    build: u64,
}

/// What `tessellate::triangulate` reads of a face, as stored in `Coll::from`.
fn inputs<'a>(ents: &'a Entities, face: &'a Face) -> impl Iterator<Item = (VertexId, DVec3)> + 'a {
    face.loops().enumerate().flat_map(move |(i, l)| {
        let gap = (i > 0).then_some((VertexId::default(), DVec3::ZERO));
        gap.into_iter().chain(l.coedges.iter().map(move |c| {
            let v = ents.coedge_ends(*c).0;
            (v, ents.pos(v))
        }))
    })
}

fn range(r: &Range<u32>) -> Range<usize> {
    r.start as usize..r.end as usize
}

/// See the module documentation.
#[derive(Default)]
pub struct TessCache {
    colls: FxMap<Ctx, Coll>,
    /// Scene builds so far.
    build: u64,
}

impl TessCache {
    /// Starts a scene build.
    pub fn begin(&mut self) {
        self.build += 1;
    }

    /// Ends a scene build: collections it did not reach are forgotten.
    pub fn end(&mut self) {
        let build = self.build;
        self.colls.retain(|_, c| c.build == build);
    }

    /// Starts reaching the faces of collection `ctx`; [`TessCache::finish`] ends it.
    pub fn visit(&mut self, ctx: Ctx) -> Visit {
        let old = self.colls.remove(&ctx).unwrap_or_default();
        if old.build == self.build {
            // Reached again in this build (another instance): read only.
            return Visit {
                ctx,
                old,
                new: None,
                cursor: 0,
            };
        }
        let new = Coll {
            entries: Vec::with_capacity(old.entries.len()),
            from: Vec::with_capacity(old.from.len()),
            tris: Vec::with_capacity(old.tris.len()),
            build: self.build,
        };
        Visit {
            ctx,
            old,
            new: Some(new),
            cursor: 0,
        }
    }

    /// Ends a [`TessCache::visit`].
    pub fn finish(&mut self, visit: Visit) {
        let Visit { ctx, old, new, .. } = visit;
        self.colls.insert(ctx, new.unwrap_or(old));
    }

    /// Keeps collection `ctx`'s triangles for this build without visiting it: the
    /// caller knows its faces unchanged (`scene_cache` kept what it drew).
    pub fn keep(&mut self, ctx: Ctx) {
        if let Some(c) = self.colls.get_mut(&ctx) {
            c.build = self.build;
        }
    }
}

/// The faces of one collection being reached ([`TessCache::visit`]).
pub struct Visit {
    ctx: Ctx,
    /// The previous build's triangles, or this build's when reached again.
    old: Coll,
    /// This build's, when reached the first time.
    new: Option<Coll>,
    /// Position in `old.entries`.
    cursor: usize,
}

/// Faces with fewer vertices (and no holes) are not kept: checking them costs about
/// what tessellating them does.
const KEEP_MIN_VERTICES: usize = 4;

/// Collections with fewer faces are not kept: the bookkeeping would cost more than it
/// saves.
pub const KEEP_MIN_FACES: usize = 16;

impl Visit {
    /// Keeps the entries of the faces in slot chunk `chunk` as they are, without
    /// checking them: the caller knows those faces unchanged (`scene_cache`). Chunks
    /// are given in order, between the faces asked for with [`Visit::triangles`].
    pub fn keep_chunk(&mut self, chunk: usize) {
        let of = SlotStore::<FaceId, Face>::chunk_of;
        let old = &self.old;
        while self.cursor < old.entries.len() && of(old.entries[self.cursor].face) < chunk {
            self.cursor += 1;
        }
        let Some(new) = &mut self.new else {
            return;
        };
        while let Some(e) = old.entries.get(self.cursor).filter(|e| of(e.face) == chunk) {
            let from_start = new.from.len() as u32;
            let tris_start = new.tris.len() as u32;
            new.from.extend_from_slice(&old.from[range(&e.from)]);
            new.tris.extend_from_slice(&old.tris[range(&e.tris)]);
            new.entries.push(Entry {
                face: e.face,
                normal: e.normal,
                from: from_start..new.from.len() as u32,
                tris: tris_start..new.tris.len() as u32,
            });
            self.cursor += 1;
        }
    }

    /// The triangles of face `f` of `ents`; `None` for a face not kept (the caller
    /// tessellates it). Faces must be asked for in the collection's order (some may be
    /// left out).
    pub fn triangles(&mut self, ents: &Entities, f: FaceId) -> Option<&[[DVec3; 3]]> {
        let face = &ents.faces[f];
        if face.holes.is_empty() && face.outer.coedges.len() < KEEP_MIN_VERTICES {
            return None;
        }
        let old = &self.old;
        while self.cursor < old.entries.len() && old.entries[self.cursor].face < f {
            self.cursor += 1;
        }
        let found = old.entries.get(self.cursor).filter(|e| e.face == f);
        let Some(new) = &mut self.new else {
            // Reached again in this build: nothing changed since.
            return found.map(|e| &old.tris[range(&e.tris)]);
        };
        let same = found.filter(|e| {
            e.normal == face.normal
                && inputs(ents, face).eq(old.from[range(&e.from)].iter().copied())
        });
        let from_start = new.from.len() as u32;
        let tris_start = new.tris.len() as u32;
        match same {
            Some(e) => {
                new.from.extend_from_slice(&old.from[range(&e.from)]);
                new.tris.extend_from_slice(&old.tris[range(&e.tris)]);
            }
            None => {
                new.from.extend(inputs(ents, face));
                new.tris.extend(tessellate::triangulate(ents, f));
            }
        }
        let entry = Entry {
            face: f,
            normal: face.normal,
            from: from_start..new.from.len() as u32,
            tris: tris_start..new.tris.len() as u32,
        };
        let tris = range(&entry.tris);
        new.entries.push(entry);
        Some(&new.tris[tris])
    }
}
