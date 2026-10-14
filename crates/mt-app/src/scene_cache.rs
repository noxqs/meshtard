//! Scene pieces kept from one build to the next (spec/PERFORMANCE.md, "Fifth round:
//! incremental rebuilds"): after an edit, only the parts of the scene made from what
//! the edit changed are drawn again; the rest is taken from the previous scene.
//!
//! A scene is walked collection by collection (`Document::collect`); each walk of a
//! collection (a *visit*: the root, or a definition reached through one instance)
//! draws its own edges and faces, then the instances it holds. A visit drawn flattened
//! is kept per chunk of its edges and of its faces (`mt_kernel::slot_store`'s chunks of
//! 256 slots): where the chunk's edges, triangles and vertices are in the scene, its
//! hidden marks, and the versions of every chunk it read (the chunk itself, the
//! vertices, edges and faces its entities reach). A visit that makes an instanced mesh
//! is kept whole, mesh included, and so is a placement of a mesh carried over: neither
//! depends on where it is placed (unless the mesh turned out flattened). The next build
//! finds each visit again by its collection, level and how many visits of both came
//! before; when its key matches (collection stores, transform for flattened visits,
//! inherited material, tag and fading, level, instancing role, cut openings) each
//! chunk whose versions all match is taken from the previous scene and the others are
//! drawn again. Anything else the walk reads (materials, tags, style, the open
//! context, the camera for face-camera components) is compared as a whole
//! ([`Settings`]): when it differs, everything is drawn again.
//!
//! The scene's lists are put together by [`List`]: ranges taken from the previous
//! scene that stay where they were are not copied; the new parts are written into the
//! previous lists in place (a small edit of a million-edge context writes a few
//! thousand items).
//!
//! Chunk versions change with every write and never go back (`SlotStore::chunk_version`),
//! so a match means the chunk is exactly what it was. The result is the scene a full
//! build makes, list for list (`tests::incremental_scenes_equal_full_builds`). The new
//! scene also says which of its ranges hold what the previous one held at the same
//! place ([`Changes`]), so the pick index, the snap geometry, the culling indexes and
//! the GPU buffers are updated only where it changed.

use std::ops::Range;

use glam::{DMat4, DVec3};
use mt_kernel::slot_store::SlotStore;
use mt_kernel::spatial::FxMap;
use mt_kernel::{DefinitionId, EdgeId, Entities, FaceId, MaterialId, VertexId};

use crate::document::{Owner, Scene, SceneBuild};
use crate::undo::Ctx;

/// One of a scene's lists while it is built ([`SceneBuild`]): what is drawn anew, in
/// order, and the ranges taken whole from the previous scene's list. When every range
/// taken stays where it was, [`List::finish`] writes the new parts into the previous
/// list in place instead of copying it all (a small edit of a huge context rewrites
/// little); otherwise the list is put together anew.
pub(crate) struct List<T> {
    new: Vec<T>,
    /// Ranges of the previous list taken: where they go, where they come from, how
    /// many (in order). New items fill the gaps, in order.
    old: Vec<(usize, usize, usize)>,
    len: usize,
}

impl<T> Default for List<T> {
    fn default() -> Self {
        Self {
            new: Vec::new(),
            old: Vec::new(),
            len: 0,
        }
    }
}

impl<T: Copy> List<T> {
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn push(&mut self, v: T) {
        self.new.push(v);
        self.len += 1;
    }

    pub fn extend(&mut self, items: impl IntoIterator<Item = T>) {
        let n = self.new.len();
        self.new.extend(items);
        self.len += self.new.len() - n;
    }

    /// Drops the last items (they must be new ones).
    pub fn truncate(&mut self, len: usize) {
        if len >= self.len {
            return;
        }
        let cut = self.len - len;
        assert!(
            self.old.last().is_none_or(|&(at, _, n)| at + n <= len),
            "only new items are dropped"
        );
        self.new.truncate(self.new.len() - cut);
        self.len = len;
    }

    /// Takes `range` of the previous list next.
    pub fn take(&mut self, range: Range<usize>) {
        let n = range.len();
        if n == 0 {
            return;
        }
        match self.old.last_mut() {
            Some((at, from, k)) if *at + *k == self.len && *from + *k == range.start => *k += n,
            _ => self.old.push((self.len, range.start, n)),
        }
        self.len += n;
    }

    /// The list, from `prev`, the previous scene's (taken: in place when possible).
    pub fn finish(self, prev: &mut Vec<T>) -> Vec<T> {
        let Self { new, old, len } = self;
        if old.is_empty() {
            return new;
        }
        let in_place = old.iter().all(|&(at, from, _)| at == from);
        let mut out = if in_place {
            std::mem::take(prev)
        } else {
            Vec::with_capacity(len)
        };
        // The new items go in the gaps between the ranges taken.
        let mut next = 0;
        let mut put = |out: &mut Vec<T>, at: usize, n: usize| {
            let items = &new[next..next + n];
            next += n;
            if in_place {
                let fit = out.len().saturating_sub(at).min(n);
                out[at..at + fit].copy_from_slice(&items[..fit]);
                out.extend_from_slice(&items[fit..]);
            } else {
                out.extend_from_slice(items);
            }
        };
        let mut at = 0;
        for &(to, from, n) in &old {
            put(&mut out, at, to - at);
            if !in_place {
                out.extend_from_slice(&prev[from..from + n]);
            }
            at = to + n;
        }
        put(&mut out, at, len - at);
        out.truncate(len);
        out
    }
}

/// Everything besides the geometry a scene build reads.
#[derive(Clone, PartialEq)]
pub(crate) struct Settings {
    pub materials: Vec<mt_kernel::Material>,
    pub layers: Vec<mt_kernel::Layer>,
    pub folders: Vec<mt_kernel::model_data::TagFolder>,
    pub colors: [[u8; 3]; 2],
    pub style: mt_kernel::StyleSettings,
    pub axes: DMat4,
    pub color_by_tag: bool,
    pub edit_fade: mt_kernel::model_data::EditFade,
    pub context: Vec<usize>,
    pub path: Option<Vec<DefinitionId>>,
    pub view: Option<crate::component_behavior::Facing>,
    pub instancing: bool,
}

/// Where a visit is relative to the open context (`document::Level`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum LevelKey {
    /// On the path; instances left to the context.
    Context(usize),
    Inside(Owner),
    Outside,
}

/// How a visit draws its own entities.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Role {
    /// Into the scene's lists.
    Flat,
    /// Makes the mesh of this key (instancing).
    Build(crate::instancing::Key),
    /// Places the mesh of this key, made by an earlier visit (its edges from the mesh).
    Placed(crate::instancing::Key),
}

/// What a visit's drawing depends on besides its collection's chunks.
#[derive(Clone, PartialEq)]
pub(crate) struct VisitKey {
    pub ctx: Ctx,
    pub xf: [u64; 16],
    /// `document::Inherited`, field by field.
    pub material: Option<MaterialId>,
    pub hidden: bool,
    pub container: Option<mt_infer::Container>,
    pub layer: Option<usize>,
    pub similar: bool,
    pub flat: bool,
    pub level: LevelKey,
    pub role: Role,
    /// Ids of the vertex, edge and face stores.
    pub uids: [u64; 3],
    pub openings: Vec<Vec<DVec3>>,
}

/// How visits are found again: by collection, level, and how many visits of both
/// came before in the walk (the same while the model's structure is).
type Slot = (Ctx, LevelKey, u32);

/// The bits of a transform (compared exactly).
pub(crate) fn bits(xf: &DMat4) -> [u64; 16] {
    xf.to_cols_array().map(f64::to_bits)
}

/// A slot map of a collection.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Map {
    Vertices,
    Edges,
    Faces,
}

fn version(ents: &Entities, map: Map, chunk: usize) -> Option<u64> {
    match map {
        Map::Vertices => ents.vertices.chunk_version(chunk),
        Map::Edges => ents.edges.chunk_version(chunk),
        Map::Faces => ents.faces.chunk_version(chunk),
    }
}

/// Where a piece's output is in its scene: edges (all, and stored), triangles (all, and
/// stored) and drawn vertices.
#[derive(Clone, Default, Debug)]
pub(crate) struct At {
    pub edges: Range<usize>,
    pub stored_edges: Range<usize>,
    pub tris: Range<usize>,
    pub stored_tris: Range<usize>,
    pub vertices: Range<usize>,
}

/// The current ends of the scene's lists.
pub(crate) fn ends(s: &SceneBuild) -> [usize; 5] {
    [
        s.edge_owner.len(),
        s.edge_list.len(),
        s.tri_owner.len(),
        s.tri_list.len(),
        s.vertices.len(),
    ]
}

impl At {
    /// From `start` (`ends` then) to the scene's ends now.
    pub fn since(start: [usize; 5], s: &SceneBuild) -> Self {
        let e = ends(s);
        Self {
            edges: start[0]..e[0],
            stored_edges: start[1]..e[1],
            tris: start[2]..e[2],
            stored_tris: start[3]..e[3],
            vertices: start[4]..e[4],
        }
    }
}

/// The chunks a chunk's drawing read (besides itself), while it is drawn.
#[derive(Default)]
pub(crate) struct Deps(Vec<(Map, u32)>);

impl Deps {
    #[inline]
    fn add(&mut self, map: Map, chunk: usize) {
        let d = (map, chunk as u32);
        if self.0.last() != Some(&d) {
            self.0.push(d);
        }
    }

    #[inline]
    fn vertex(&mut self, v: VertexId) {
        self.add(Map::Vertices, SlotStore::<VertexId, ()>::chunk_of(v));
    }

    /// What drawing edge `e` reads: its ends and the faces it takes normals and
    /// material from.
    #[inline]
    pub fn edge(&mut self, e: &mt_kernel::Edge) {
        self.vertex(e.start);
        self.vertex(e.end);
        for &f in e.faces.iter().take(2) {
            self.add(Map::Faces, SlotStore::<FaceId, ()>::chunk_of(f));
        }
    }

    /// What drawing face `face` of `ents` reads: its loops' edges and their ends.
    #[inline]
    pub fn face(&mut self, ents: &Entities, face: &mt_kernel::Face) {
        for l in face.loops() {
            for c in &l.coedges {
                self.add(Map::Edges, SlotStore::<EdgeId, ()>::chunk_of(c.edge));
                if let Some(e) = ents.edges.get(c.edge) {
                    self.vertex(e.start);
                    self.vertex(e.end);
                }
            }
        }
    }
}

/// What one chunk of a flat visit drew.
pub(crate) struct Piece {
    /// The chunk's version.
    version: u64,
    /// The versions of the other chunks it read.
    deps: Vec<(Map, u32, u64)>,
    at: At,
    /// Its edges drawn as hidden geometry (`Scene::hidden_edges`), relative to
    /// `at.edges`.
    hidden: Vec<u32>,
    /// Its first edges that lie in the visit's container (`Scene::edge_container`):
    /// the collection's edges, not hidden faces' outlines.
    contained: usize,
}

impl Piece {
    /// The piece of chunk `chunk` of `map` just drawn at `at`.
    pub fn new(
        ents: &Entities,
        map: Map,
        chunk: usize,
        deps: Deps,
        at: At,
        out: &SceneBuild,
        hidden: bool,
    ) -> Self {
        let mut d = deps.0;
        d.sort_unstable();
        d.dedup();
        let deps = d
            .into_iter()
            .map(|(m, c)| (m, c, version(ents, m, c as usize).unwrap_or(u64::MAX)))
            .collect();
        let hidden = if hidden {
            (at.edges.clone())
                .filter(|i| out.hidden_edges.contains(i))
                .map(|i| (i - at.edges.start) as u32)
                .collect()
        } else {
            Vec::new()
        };
        Self {
            version: version(ents, map, chunk).unwrap_or(u64::MAX),
            deps,
            contained: if map == Map::Edges { at.edges.len() } else { 0 },
            at,
            hidden,
        }
    }

    /// Whether chunk `chunk` of `map` and what it read are unchanged.
    fn valid(&self, ents: &Entities, map: Map, chunk: usize) -> bool {
        version(ents, map, chunk) == Some(self.version)
            && (self.deps.iter()).all(|&(m, c, v)| version(ents, m, c as usize) == Some(v))
    }
}

/// A visit that made a mesh (or found it must be drawn flattened), kept whole.
pub(crate) struct Whole {
    /// Every chunk version of the vertices, edges and faces.
    versions: [Vec<u64>; 3],
    pub at: At,
    hidden: Vec<u32>,
    /// See [`Piece`].
    contained: usize,
    /// The mesh made (`None`: drawn flattened), or placed.
    pub mesh: Option<u32>,
    pub lazy: bool,
    /// A placement of a mesh another visit made (which is not its to carry over).
    placed: bool,
    /// Where it was placed: what it drew depends on it unless it is a mesh (with its
    /// edges) or a placement of one.
    xf: [u64; 16],
}

fn all_versions(ents: &Entities) -> [Vec<u64>; 3] {
    let v =
        |n: usize, f: &dyn Fn(usize) -> Option<u64>| (0..n).map(|c| f(c).unwrap_or(0)).collect();
    [
        v(ents.vertices.chunk_count(), &|c| {
            ents.vertices.chunk_version(c)
        }),
        v(ents.edges.chunk_count(), &|c| ents.edges.chunk_version(c)),
        v(ents.faces.chunk_count(), &|c| ents.faces.chunk_version(c)),
    ]
}

impl Whole {
    /// The visit of `ents` just drawn at `at` by `xf`, the first `contained` of its
    /// edges in its container (`hidden`: some may be hidden geometry).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ents: &Entities,
        at: At,
        contained: usize,
        out: &SceneBuild,
        mesh: Option<u32>,
        lazy: bool,
        xf: &DMat4,
        hidden: bool,
    ) -> Self {
        Self {
            versions: all_versions(ents),
            hidden: Self::hidden(&at, out, hidden),
            at,
            contained,
            mesh,
            lazy,
            placed: false,
            xf: bits(xf),
        }
    }

    /// A placement of mesh `mesh` (made by another visit) by `xf` drawn at `at`. What
    /// it drew depends only on the mesh: it is valid while the mesh is carried over.
    pub fn placement(
        at: At,
        contained: usize,
        out: &SceneBuild,
        mesh: u32,
        xf: &DMat4,
        hidden: bool,
    ) -> Self {
        Self {
            versions: Default::default(),
            hidden: Self::hidden(&at, out, hidden),
            at,
            contained,
            mesh: Some(mesh),
            lazy: true,
            placed: true,
            xf: bits(xf),
        }
    }

    /// Its edges drawn as hidden geometry (looked for when there may be some).
    fn hidden(at: &At, out: &SceneBuild, any: bool) -> Vec<u32> {
        if !any {
            return Vec::new();
        }
        (at.edges.clone())
            .filter(|i| out.hidden_edges.contains(i))
            .map(|i| (i - at.edges.start) as u32)
            .collect()
    }

    /// Whether its collection is unchanged, and (when it drew anything in world
    /// space) it is placed by `xf` again. Placements are checked by their mesh
    /// ([`Pieces::valid_placed`]).
    fn valid(&self, ents: &Entities, xf: &DMat4) -> bool {
        if self.placed {
            return true;
        }
        let anywhere = self.mesh.is_some() && self.lazy;
        self.versions == all_versions(ents) && (anywhere || self.xf == bits(xf))
    }
}

/// What a visit drew, as the next build may reuse it.
pub(crate) struct VisitRec {
    key: VisitKey,
    slot: Slot,
    /// Per edge chunk and per face chunk (flat visits).
    pub edges: Vec<Option<Piece>>,
    pub faces: Vec<Option<Piece>>,
    pub whole: Option<Whole>,
}

impl VisitRec {
    pub fn new(key: VisitKey, slot: Slot) -> Self {
        Self {
            key,
            slot,
            edges: Vec::new(),
            faces: Vec::new(),
            whole: None,
        }
    }
}

/// The pieces of the last scene built with them, kept in the document.
#[derive(Default)]
pub(crate) struct SceneCache {
    settings: Option<Settings>,
    /// The scene the visits describe (`Scene::build`; 0: none).
    build: u64,
    visits: Vec<VisitRec>,
}

/// Ranges of a list that hold what the previous scene's held at the same place.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct Same(pub Vec<Range<usize>>);

impl Same {
    fn add(&mut self, r: Range<usize>) {
        if r.is_empty() {
            return;
        }
        match self.0.last_mut() {
            Some(last) if last.end == r.start => last.end = r.end,
            _ => self.0.push(r),
        }
    }

    /// The ranges of `0..len` not known to be the same, in order.
    pub fn changed(&self, len: usize) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let mut at = 0;
        for r in &self.0 {
            let end = r.start.min(len);
            if at < end {
                out.push(at..end);
            }
            at = at.max(r.end);
        }
        if at < len {
            out.push(at..len);
        }
        out
    }

    /// Whether every index in `r` is known to be the same.
    pub fn covers(&self, r: Range<usize>) -> bool {
        if r.is_empty() {
            return true;
        }
        let k = self.0.partition_point(|x| x.end <= r.start);
        self.0
            .get(k)
            .is_some_and(|x| x.start <= r.start && r.end <= x.end)
    }
}

/// What a scene kept of the one it was built from ([`Scene::changes`]).
#[derive(Default, Clone, Debug)]
pub struct Changes {
    /// The build this scene was made from (`None`: everything is new).
    pub from: Option<u64>,
    pub edges: Same,
    pub stored_edges: Same,
    pub tris: Same,
    pub stored_tris: Same,
    pub vertices: Same,
    /// The instanced meshes are the previous scene's, in the same order.
    pub meshes: bool,
}

/// The pieces of one build: the previous scene and its visits, and the visits of the
/// scene being built.
pub(crate) struct Pieces {
    prev: Scene,
    old: Vec<Option<VisitRec>>,
    index: FxMap<Slot, u32>,
    /// Visits of each collection and level so far in this walk.
    seen: FxMap<(Ctx, LevelKey), u32>,
    pub new: Vec<VisitRec>,
    pub changes: Changes,
    /// Meshes carried over: (index in the previous scene, in the new one).
    meshes: Vec<(u32, u32)>,
}

impl Pieces {
    /// Starts a build kept in `cache`, from `prev` when it is the scene the cache
    /// describes and was built with the same `settings`.
    pub fn begin(cache: &mut SceneCache, prev: Option<Scene>, settings: &Settings) -> Self {
        let usable = prev.as_ref().filter(|p| {
            cache.build != 0 && p.build == cache.build && cache.settings.as_ref() == Some(settings)
        });
        let from = usable.map(|p| p.build);
        let old: Vec<Option<VisitRec>> = if from.is_some() {
            std::mem::take(&mut cache.visits)
                .into_iter()
                .map(Some)
                .collect()
        } else {
            cache.visits.clear();
            Vec::new()
        };
        let index = (old.iter().enumerate())
            .filter_map(|(i, v)| Some((v.as_ref()?.slot, i as u32)))
            .collect();
        Self {
            prev: if from.is_some() {
                prev.unwrap_or_default()
            } else {
                Scene::default()
            },
            old,
            index,
            seen: FxMap::default(),
            new: Vec::new(),
            changes: Changes {
                from,
                ..Default::default()
            },
            meshes: Vec::new(),
        }
    }

    /// The previous scene, whose lists [`List::finish`] takes.
    pub fn prev(&mut self) -> &mut Scene {
        &mut self.prev
    }

    /// Ends the build of `scene`: the cache now describes it.
    pub fn end(mut self, cache: &mut SceneCache, scene: &mut Scene, settings: Settings) {
        cache.settings = Some(settings);
        cache.build = scene.build;
        cache.visits = std::mem::take(&mut self.new);
        let m = &self.meshes;
        self.changes.meshes = self.changes.from.is_some()
            && m.len() == scene.instancing.meshes.len()
            && m.len() == self.prev.instancing.meshes.len()
            && m.iter().all(|(a, b)| a == b);
        scene.changes = std::mem::take(&mut self.changes);
        // The previous scene is freed on another thread (a big one takes a while).
        let size = self.prev.tri_count() + self.prev.edge_count();
        crate::undo::discard(std::mem::take(&mut self.prev), size);
    }

    /// The record the visit `key` (the next one of its collection and level) keeps,
    /// and the previous build's record of it, taken.
    pub fn take(&mut self, key: VisitKey) -> (VisitRec, Option<VisitRec>) {
        let n = self.seen.entry((key.ctx, key.level)).or_insert(0);
        let slot = (key.ctx, key.level, *n);
        *n += 1;
        let old =
            (self.index.get(&slot)).and_then(|&i| self.old[i as usize].take_if(|v| v.key == key));
        (VisitRec::new(key, slot), old)
    }

    /// Piece `chunk` of `map` of the old record, when it is still valid.
    pub fn valid_piece(
        old: &mut Option<VisitRec>,
        ents: &Entities,
        map: Map,
        chunk: usize,
    ) -> Option<Piece> {
        let o = old.as_mut()?;
        let list = if map == Map::Edges {
            &mut o.edges
        } else {
            &mut o.faces
        };
        let p = list.get_mut(chunk)?.take()?;
        p.valid(ents, map, chunk).then_some(p)
    }

    /// The old record's placement of a mesh, when still valid: the mesh it placed
    /// is `mesh` now (carried over from the last build).
    pub fn valid_placed(
        &self,
        old: &mut Option<VisitRec>,
        ents: &Entities,
        mesh: u32,
    ) -> Option<Whole> {
        let w = Self::valid_whole(old, ents, &DMat4::IDENTITY)?;
        let was = w.mesh?;
        (w.placed && self.meshes.contains(&(was, mesh))).then_some(w)
    }

    /// The old record's whole visit, when still valid.
    pub fn valid_whole(old: &mut Option<VisitRec>, ents: &Entities, xf: &DMat4) -> Option<Whole> {
        let w = old.as_mut()?.whole.take()?;
        w.valid(ents, xf).then_some(w)
    }

    /// Copies what `at` (in the previous scene) holds to the end of `out`; returns
    /// where it is now.
    fn copy(
        &mut self,
        out: &mut SceneBuild,
        at: &At,
        hidden: &[u32],
        contained: usize,
        container: Option<mt_infer::Container>,
        moved: bool,
    ) -> At {
        let start = ends(out);
        out.edge_list.take(at.stored_edges.clone());
        out.edge_owner.take(at.edges.clone());
        out.tri_list.take(at.stored_tris.clone());
        out.tri_owner.take(at.tris.clone());
        out.tri_material.take(at.tris.clone());
        out.vertices.take(at.vertices.clone());
        let new = At::since(start, out);
        for &k in hidden {
            out.hidden_edges.insert(new.edges.start + k as usize);
        }
        if let Some(c) = container {
            let edges = new.edges.start..new.edges.start + contained;
            if !edges.is_empty() {
                crate::document::set_containers(&mut out.edge_container, edges, c);
            }
            if !new.tris.is_empty() {
                crate::document::set_containers(&mut out.tri_container, new.tris.clone(), c);
            }
        }
        let ch = &mut self.changes;
        let same = |s: &mut Same, old: &Range<usize>, new: &Range<usize>| {
            if old.start == new.start {
                s.add(new.clone());
            }
        };
        // Edges and triangles given by a placement's mesh move with it.
        if !moved {
            same(&mut ch.edges, &at.edges, &new.edges);
            same(&mut ch.tris, &at.tris, &new.tris);
        }
        same(&mut ch.stored_edges, &at.stored_edges, &new.stored_edges);
        same(&mut ch.stored_tris, &at.stored_tris, &new.stored_tris);
        same(&mut ch.vertices, &at.vertices, &new.vertices);
        new
    }

    /// Copies a kept chunk to the end of `out`.
    pub fn copy_piece(
        &mut self,
        out: &mut SceneBuild,
        mut p: Piece,
        container: Option<mt_infer::Container>,
    ) -> Piece {
        p.at = self.copy(out, &p.at, &p.hidden, p.contained, container, false);
        p
    }

    /// Copies a kept visit, now placed by `xf`, to the end of `out`, its mesh included;
    /// returns it with its new place and mesh number.
    pub fn copy_whole(
        &mut self,
        out: &mut SceneBuild,
        mut w: Whole,
        container: Option<mt_infer::Container>,
        xf: &DMat4,
    ) -> Whole {
        let moved = w.xf != bits(xf);
        w.at = self.copy(out, &w.at, &w.hidden, w.contained, container, moved);
        w.xf = bits(xf);
        if w.placed {
            // Its mesh is the one carried over for it.
            w.mesh = (w.mesh)
                .and_then(|m| self.meshes.iter().find(|x| x.0 == m))
                .map(|x| x.1);
        } else if let Some(m) = w.mesh {
            let mesh = std::mem::replace(
                &mut self.prev.instancing.meshes[m as usize],
                crate::instancing::Mesh::empty(),
            );
            let n = out.instancing.meshes.len() as u32;
            out.instancing.meshes.push(mesh);
            self.meshes.push((m, n));
            w.mesh = Some(n);
        }
        w
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::document::Document;

    fn bytes<T: bytemuck::Pod>(v: &[T]) -> &[u8] {
        bytemuck::cast_slice(v)
    }

    /// Container lists without their trailing `None`s (the same information).
    fn trimmed(v: &[Option<mt_infer::Container>]) -> &[Option<mt_infer::Container>] {
        let n = v.iter().rposition(Option::is_some).map_or(0, |i| i + 1);
        &v[..n]
    }

    /// Panics with the first difference of `a` and `b`, if any.
    fn same_list<T: PartialEq + std::fmt::Debug>(a: &[T], b: &[T], what: &str) {
        if let Some(i) = (0..a.len().min(b.len())).find(|&i| a[i] != b[i]) {
            panic!("{what}: [{i}] {:?} != {:?}", a[i], b[i]);
        }
        assert_eq!(a.len(), b.len(), "{what}: lengths");
    }

    /// `a` and `b` hold the same scene, list for list.
    pub(crate) fn assert_same(a: &Scene, b: &Scene, what: &str) {
        let tris = |s: &Scene| -> Vec<[[u64; 3]; 3]> {
            (s.tri_list.iter())
                .map(|t| t.map(|p| p.to_array().map(f64::to_bits)))
                .collect()
        };
        assert!(bytes(&a.edge_list) == bytes(&b.edge_list), "{what}: edges");
        same_list(
            &a.edge_owner,
            &b.edge_owner,
            &format!("{what}: edge owners"),
        );
        assert!(bytes(&a.vertices) == bytes(&b.vertices), "{what}: vertices");
        same_list(&a.indices, &b.indices, &format!("{what}: indices"));
        same_list(&tris(a), &tris(b), &format!("{what}: triangles"));
        same_list(
            &a.tri_owner,
            &b.tri_owner,
            &format!("{what}: triangle owners"),
        );
        same_list(
            &a.tri_material,
            &b.tri_material,
            &format!("{what}: materials"),
        );
        assert!(bytes(&a.guides) == bytes(&b.guides), "{what}: guides");
        assert_eq!(a.section, b.section, "{what}: section");
        assert_eq!(a.annotations, b.annotations, "{what}: annotations");
        assert_eq!(a.texture_of, b.texture_of, "{what}: textures");
        assert_eq!(a.textures.len(), b.textures.len(), "{what}: textures");
        let sorted = |s: &Scene| {
            let mut v: Vec<usize> = s.hidden_edges.iter().copied().collect();
            v.sort_unstable();
            v
        };
        same_list(&sorted(a), &sorted(b), &format!("{what}: hidden edges"));
        same_list(
            trimmed(&a.edge_container),
            trimmed(&b.edge_container),
            &format!("{what}: edge containers"),
        );
        same_list(
            trimmed(&a.tri_container),
            trimmed(&b.tri_container),
            &format!("{what}: triangle containers"),
        );
        let sun = |s: &Scene| -> Vec<_> {
            (s.sun_facing.iter())
                .map(|(r, m)| (r.clone(), m.to_cols_array().map(f64::to_bits)))
                .collect()
        };
        assert_eq!(sun(a), sun(b), "{what}: sun facing");
        assert!(
            a.instancing.exact() == b.instancing.exact(),
            "{what}: instancing"
        );
    }

    /// A scene's lists, to check what the next scene says it kept.
    pub(crate) struct Before {
        all_edges: Vec<mt_render::Segment>,
        all_tris: Vec<[DVec3; 3]>,
        edges: Vec<mt_render::Segment>,
        edge_owner: Vec<Option<Owner>>,
        tri_list: Vec<[DVec3; 3]>,
        tri_owner: Vec<Option<Owner>>,
        vertices: Vec<mt_render::FaceVertex>,
        meshes: String,
        build: u64,
    }

    impl Before {
        pub(crate) fn of(s: &Scene) -> Self {
            Self {
                all_edges: s.edges().to_vec(),
                all_tris: s.triangles().to_vec(),
                edges: s.edge_list.clone(),
                edge_owner: s.edge_owner.clone(),
                tri_list: s.tri_list.clone(),
                tri_owner: s.tri_owner.clone(),
                vertices: s.vertices.clone(),
                meshes: s.instancing.exact_meshes(),
                build: s.build,
            }
        }

        /// What `new`'s `changes` say it kept of this scene is so.
        pub(crate) fn check(&self, new: &Scene, what: &str) {
            let c = &new.changes;
            if c.from != Some(self.build) {
                return;
            }
            for r in &c.stored_edges.0 {
                assert!(
                    bytes(&new.edge_list[r.clone()]) == bytes(&self.edges[r.clone()]),
                    "{what}: kept edges {r:?}"
                );
            }
            for r in &c.edges.0 {
                let what = format!("{what}: kept edge owners");
                same_list(
                    &new.edge_owner[r.clone()],
                    &self.edge_owner[r.clone()],
                    &what,
                );
                let a = &new.edges()[r.clone()];
                assert!(
                    bytes(a) == bytes(&self.all_edges[r.clone()]),
                    "{what}: segments"
                );
            }
            for r in &c.stored_tris.0 {
                let what = format!("{what}: kept triangles");
                same_list(&new.tri_list[r.clone()], &self.tri_list[r.clone()], &what);
            }
            for r in &c.tris.0 {
                let what = format!("{what}: kept triangle owners");
                same_list(&new.tri_owner[r.clone()], &self.tri_owner[r.clone()], &what);
                let what = format!("{what}: kept triangles");
                same_list(
                    &new.triangles()[r.clone()],
                    &self.all_tris[r.clone()],
                    &what,
                );
            }
            for r in &c.vertices.0 {
                assert!(
                    bytes(&new.vertices[r.clone()]) == bytes(&self.vertices[r.clone()]),
                    "{what}: kept vertices {r:?}"
                );
            }
            if c.meshes {
                assert!(
                    new.instancing.exact_meshes() == self.meshes,
                    "{what}: meshes"
                );
            }
        }
    }

    /// Builds `doc`'s scene from `prev`, checks it against a full build and what it
    /// says it kept; returns it.
    pub(crate) fn step(doc: &Document, prev: Option<Scene>, what: &str) -> Scene {
        let before = prev.as_ref().map(Before::of);
        let scene = doc.scene_from(prev);
        assert_same(&scene, &doc.scene(), what);
        if let Some(b) = before {
            b.check(&scene, what);
        }
        scene
    }

    /// A list put together from new items and ranges of the previous list is the
    /// list made item by item, in place or not.
    #[test]
    fn lists_take_ranges_in_place_or_anew() {
        let mut seed = 5u64;
        let mut rnd = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for round in 0..300 {
            let prev: Vec<u32> = (0..rnd(200) as u32).map(|i| i + 1000).collect();
            let mut list = List::default();
            let mut want = Vec::new();
            let mut from = 0;
            // Same places (in place) in half of the rounds.
            let same = round % 2 == 0;
            while from < prev.len() {
                let n = 1 + rnd(20).min(prev.len() - from - 1);
                match rnd(4) {
                    0 if same => {
                        // A new part as long as the old one it replaces.
                        for k in 0..n {
                            list.push(7 + k as u32);
                            want.push(7 + k as u32);
                        }
                    }
                    0 => {
                        let k = rnd(5);
                        list.extend((0..k as u32).map(|i| i + 1));
                        want.extend((0..k as u32).map(|i| i + 1));
                    }
                    1 if !same => {}
                    _ => {
                        list.take(from..from + n);
                        want.extend_from_slice(&prev[from..from + n]);
                    }
                }
                from += n;
            }
            if rnd(3) == 0 {
                list.extend([1, 2, 3]);
                want.extend([1, 2, 3]);
                list.truncate(list.len() - 2);
                want.truncate(want.len() - 2);
            }
            assert_eq!(list.len(), want.len());
            let mut p = prev.clone();
            assert_eq!(list.finish(&mut p), want, "round {round}");
        }
    }

    fn square(ents: &mut Entities, o: DVec3, s: f64) {
        let p = [
            o,
            o + DVec3::X * s,
            o + DVec3::new(s, s, 0.0),
            o + DVec3::Y * s,
        ];
        for i in 0..4 {
            mt_kernel::add_line(ents, p[i], p[(i + 1) % 4]);
        }
    }

    /// A grid spanning many chunks at the root, a box component placed several times
    /// (instanced; one in a translucent material), groups of their
    /// own and nested components.
    pub(crate) fn model() -> Document {
        use crate::bench::{add_box, definition, instance};
        let mut doc = Document::untitled();
        doc.model = crate::bench::grid(24);
        let m = &mut doc.model;
        m.materials.push(mt_kernel::Material {
            name: "Red".into(),
            color: [200, 0, 0],
            opacity: 1.0,
            texture: None,
        });
        m.materials.push(mt_kernel::Material {
            name: "Glass".into(),
            color: [0, 0, 200],
            opacity: 0.4,
            texture: None,
        });
        m.layers.push(mt_kernel::Layer {
            name: "Tag".into(),
            color: [0, 150, 0],
            ..Default::default()
        });
        let mut ents = Entities::default();
        add_box(&mut ents, DVec3::ZERO, DVec3::splat(4.0));
        let leaf = m.definitions.insert(definition("Leaf", ents, false));
        for k in 0..6 {
            let xf = DMat4::from_translation(DVec3::new(10.0 * k as f64, -20.0, 0.0))
                * DMat4::from_rotation_z(0.3 * k as f64);
            let mut i = instance(leaf, xf);
            i.material = (k == 2).then_some(0);
            i.layer = (k == 3).then_some(1);
            m.root.instances.push(i);
        }
        let mut glass = instance(leaf, DMat4::from_translation(DVec3::Z * 30.0));
        glass.material = Some(1);
        m.root.instances.push(glass);
        for k in 0..3 {
            let mut ents = Entities::default();
            add_box(&mut ents, DVec3::ZERO, DVec3::new(5.0, 5.0, 2.0 + k as f64));
            let d = m.definitions.insert(definition("G", ents, true));
            let xf = DMat4::from_translation(DVec3::new(-30.0, 10.0 * k as f64, 0.0));
            m.root.instances.push(instance(d, xf));
        }
        let mut ents = Entities::default();
        for k in 0..2 {
            let xf = DMat4::from_translation(DVec3::X * 8.0 * k as f64);
            ents.instances.push(instance(leaf, xf));
        }
        let pair = m.definitions.insert(definition("Pair", ents, false));
        for k in 0..2 {
            let xf = DMat4::from_translation(DVec3::new(0.0, -60.0 - 20.0 * k as f64, 0.0));
            m.root.instances.push(instance(pair, xf));
        }
        // A tile placed three times: instanced faces, edges too few to instance (given
        // by the mesh for each placement).
        let mut ents = Entities::default();
        square(&mut ents, DVec3::ZERO, 3.0);
        let tile = m.definitions.insert(definition("Tile", ents, false));
        for k in 0..3 {
            let xf = DMat4::from_translation(DVec3::new(-50.0, -10.0 * k as f64, 0.0));
            m.root.instances.push(instance(tile, xf));
        }
        doc
    }

    fn move_by(d: &mut Document, targets: &[Owner], by: DVec3) {
        d.edit(|e| {
            crate::modify::apply(e, targets, DMat4::from_translation(by), false);
            true
        });
    }

    /// After many kinds of edit, changes without an undo step, style changes, opening
    /// groups and components, undo and redo, the scene built from the previous one is
    /// the one a full build makes, list for list, and what it says it kept is so.
    #[test]
    fn incremental_scenes_equal_full_builds() {
        type Action = (&'static str, fn(&mut Document));
        let actions: Vec<Action> = vec![
            ("nothing", |_| {}),
            ("move one face", |d| {
                let f = d.active().faces.keys().nth(500).unwrap();
                move_by(d, &[Owner::Face(f)], DVec3::Z);
            }),
            ("move a tenth", |d| {
                let some: Vec<Owner> = (d.active().faces.keys())
                    .step_by(10)
                    .map(Owner::Face)
                    .collect();
                move_by(d, &some, DVec3::new(1.0, 2.0, 3.0));
            }),
            ("paint a face", |d| {
                let f = d.active().faces.keys().nth(30).unwrap();
                d.edit(|e| {
                    e.faces[f].attrs.material = Some(0);
                    true
                });
            }),
            ("move an instance", |d| {
                move_by(d, &[Owner::Instance(1)], DVec3::Y);
            }),
            ("move a pair (instanced inside)", |d| {
                move_by(d, &[Owner::Instance(10)], DVec3::new(2.0, 0.0, 1.0));
            }),
            ("move a tile", |d| {
                move_by(d, &[Owner::Instance(13)], DVec3::new(0.0, 1.0, 0.0));
            }),
            ("erase edges", |d| {
                let edges: Vec<Owner> = (d.active().edges.keys())
                    .skip(200)
                    .step_by(13)
                    .take(30)
                    .map(Owner::Edge)
                    .collect();
                d.erase(&edges, false, false);
            }),
            ("draw into freed slots", |d| {
                d.edit(|e| {
                    square(e, DVec3::new(0.0, 300.0, 0.0), 10.0);
                    true
                });
            }),
            ("hide a face and an instance", |d| {
                let f = d.active().faces.keys().nth(40).unwrap();
                d.hide(&[Owner::Face(f), Owner::Instance(4)]);
            }),
            ("hidden geometry and objects (style)", |d| {
                d.model.style.hidden_geometry = true;
                d.model.style.hidden_objects = true;
            }),
            ("soften an edge, untracked", |d| {
                let e = d.model.root.edges.keys().nth(700).unwrap();
                d.model.root.edges[e].attrs.soft = true;
            }),
            ("move a hidden face", |d| {
                let f = d.active().faces.keys().nth(40).unwrap();
                move_by(d, &[Owner::Face(f)], DVec3::Z);
            }),
            ("open the leaf", |d| assert!(d.open_instance(0))),
            ("draw in the leaf", |d| {
                d.edit(|e| {
                    square(e, DVec3::new(0.0, 0.0, 9.0), 3.0);
                    true
                });
            }),
            ("close, change the leaf untracked", |d| {
                d.close_instance();
                let def = d.model.root.instances[0].definition;
                let ents = &mut d.model.definitions[def].entities;
                let v = ents.vertices.keys().next().unwrap();
                ents.vertices[v].pos.z -= 1.0;
            }),
            ("color by tag", |d| d.model.color_by_tag = true),
            ("color by tag off", |d| d.model.color_by_tag = false),
            ("open a group, hide the rest", |d| {
                d.model.style.hide_rest_of_model = true;
                assert!(d.open_instance(7));
            }),
            ("push/pull in the group", |d| {
                let f = d.active().faces.keys().next().unwrap();
                d.edit(|e| mt_kernel::push_pull::push_pull(e, f, 2.0, false).is_some());
            }),
            ("close", |d| {
                d.close_instance();
                d.model.style.hide_rest_of_model = false;
            }),
            ("delete an instance", |d| d.delete(&[Owner::Instance(2)])),
            ("lines in many chunks", |d| {
                d.edit(|e| {
                    for i in 0..600 {
                        let a = DVec3::new(i as f64 * 3.0, -200.0, 0.0);
                        mt_kernel::add_line(e, a, a + DVec3::new(0.0, -10.0, 1.0));
                    }
                    true
                });
            }),
            ("tag an edge", |d| {
                let e = d.active().edges.keys().nth(1500).unwrap();
                d.edit(|ents| {
                    ents.edges[e].attrs.layer = Some(1);
                    true
                });
            }),
            ("hide the tag", |d| d.model.layers[1].visible = false),
            ("group a few faces", |d| {
                d.edit_model(|model, ents| {
                    let sel = mt_kernel::group::Selection {
                        edges: vec![],
                        faces: ents.faces.keys().skip(100).take(20).collect(),
                        instances: vec![],
                    };
                    let g = mt_kernel::group::make_group(
                        &mut model.definitions,
                        ents,
                        &sel,
                        "G".into(),
                        true,
                    );
                    g.is_some()
                });
            }),
            ("explode a group", |d| {
                d.edit_model(|m, e| mt_kernel::group::explode(&m.definitions, e, 6));
            }),
        ];
        let mut doc = model();
        let mut scene = step(&doc, None, "first");
        assert!(scene.instancing.placements.len() >= 6, "instanced");
        for (name, action) in &actions {
            action(&mut doc);
            scene = step(&doc, Some(scene), name);
            if name.starts_with("move a") && !name.contains("hidden") {
                // Meshes do not depend on where they are placed.
                assert!(scene.changes.meshes, "{name}: meshes kept");
            }
            // Again with nothing changed: everything is kept, in place.
            scene = step(&doc, Some(scene), name);
            let c = &scene.changes;
            assert!(c.from.is_some(), "{name}");
            assert!(
                c.vertices.changed(scene.vertices.len()).is_empty(),
                "{name}"
            );
            assert!(
                c.stored_tris.changed(scene.tri_list.len()).is_empty(),
                "{name}"
            );
            assert!(
                c.edges.changed(scene.edge_count()).is_empty(),
                "{name}: edges"
            );
            assert!(
                c.tris.changed(scene.tri_count()).is_empty(),
                "{name}: triangles"
            );
            assert!(c.meshes, "{name}");
        }
        while doc.undo() {
            scene = step(&doc, Some(scene), "undo");
        }
        while doc.redo() {
            scene = step(&doc, Some(scene), "redo");
        }
        // A scene the cache does not describe: built in full.
        let other = doc.scene();
        let s = step(&doc, Some(other), "unknown previous scene");
        assert!(s.changes.from.is_none());
    }

    /// A small edit of a big flat context keeps all but a few chunks, in place.
    #[test]
    fn small_edits_keep_the_rest_in_place() {
        let mut doc = Document::untitled();
        doc.model = crate::bench::grid(120);
        let scene = step(&doc, None, "first");
        let f = doc.active().faces.keys().nth(3000).unwrap();
        move_by(&mut doc, &[Owner::Face(f)], DVec3::Z);
        let scene = step(&doc, Some(scene), "move one face");
        let c = &scene.changes;
        let changed: usize = (c.vertices.changed(scene.vertices.len()).iter())
            .map(|r| r.len())
            .sum();
        assert!(
            changed > 0 && changed < scene.vertices.len() / 10,
            "{changed}"
        );
        let changed: usize = (c.stored_edges.changed(scene.edge_list.len()).iter())
            .map(|r| r.len())
            .sum();
        assert!(
            changed > 0 && changed < scene.edge_list.len() / 10,
            "{changed}"
        );
    }
}
