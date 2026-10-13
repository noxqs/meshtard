//! A slot map that knows which parts of it changed since its last snapshot
//! (spec/PERFORMANCE.md, undo steps of huge contexts).
//!
//! [`SlotStore`] behaves exactly as `slotmap::SlotMap` does: the same flat storage,
//! keys, versions, free list and iteration order, and the same `Debug` output. On top
//! of that it splits its slots into chunks of [`CHUNK`] and marks a chunk dirty on any
//! `&mut` access to a slot in it (inserting, removing, `get_mut`, `iter_mut`...).
//! [`SlotStore::freeze`] returns a [`Snapshot`]: the slots as shared, read-only chunks.
//! Chunks that are not dirty since the previous snapshot are taken from it as they are
//! (a reference count), so a snapshot after a small edit costs only the chunks the
//! edit touched. [`SlotStore::thaw`] makes the map equal to a snapshot again,
//! rewriting only the chunks that differ.
//!
//! Reads go straight to the flat storage, as with `SlotMap`; writes also set a flag.

use std::fmt;
use std::marker::PhantomData;
use std::ops::{Index, IndexMut, Range};
use std::sync::Arc;

use slotmap::{Key, KeyData};

const SHIFT: usize = 8;
/// Slots per chunk.
pub const CHUNK: usize = 1 << SHIFT;

#[derive(Clone, PartialEq)]
enum Content<V> {
    Occupied(V),
    /// The next free slot (the free list), as `slotmap` keeps it.
    Vacant(u32),
}

/// A slot and its version: even when vacant, odd when occupied (as in `slotmap`).
#[derive(Clone, PartialEq)]
struct Slot<V> {
    version: u32,
    content: Content<V>,
}

/// `slotmap`'s format, so a model's dump is the same as with a `SlotMap`.
impl<V: fmt::Debug> fmt::Debug for Slot<V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut b = f.debug_struct("Slot");
        b.field("version", &self.version);
        match &self.content {
            Content::Occupied(v) => b.field("value", v).finish(),
            Content::Vacant(next) => b.field("next_free", next).finish(),
        }
    }
}

struct Frozen<V> {
    /// Every chunk but the last holds exactly [`CHUNK`] slots.
    chunks: Vec<Arc<[Slot<V>]>>,
    slots: usize,
    free_head: u32,
    num_elems: u32,
}

/// The state of a [`SlotStore`] at [`SlotStore::freeze`]: immutable, cheap to clone,
/// sharing its chunks with other snapshots of the same store.
pub struct Snapshot<V>(Arc<Frozen<V>>);

impl<V> Clone for Snapshot<V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<V> Snapshot<V> {
    /// Entries held.
    pub fn len(&self) -> usize {
        self.0.num_elems as usize
    }

    pub fn is_empty(&self) -> bool {
        self.0.num_elems == 0
    }

    /// Whether both are the same snapshot (not just equal ones).
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// How many of its chunks this snapshot shares with `other`, and how many it has.
    pub fn chunks_shared_with(&self, other: &Self) -> (usize, usize) {
        let shared = (self.0.chunks.iter().zip(&other.0.chunks))
            .filter(|(a, b)| Arc::ptr_eq(a, b))
            .count();
        (shared, self.0.chunks.len())
    }
}

impl<V: Clone> Snapshot<V> {
    /// A store holding this snapshot's state (a full copy).
    pub fn to_store<K: Key>(&self) -> SlotStore<K, V> {
        let f = &*self.0;
        let mut slots = Vec::with_capacity(f.slots);
        for c in &f.chunks {
            slots.extend_from_slice(c);
        }
        SlotStore {
            slots,
            free_head: f.free_head,
            num_elems: f.num_elems,
            dirty: vec![false; f.chunks.len()],
            base: Some(self.clone()),
            _k: PhantomData,
        }
    }
}

/// See the module documentation.
pub struct SlotStore<K: Key, V> {
    slots: Vec<Slot<V>>,
    free_head: u32,
    num_elems: u32,
    /// Per chunk of `slots`: may differ from the same chunk of `base` (chunks past its
    /// end count as changed).
    dirty: Vec<bool>,
    /// The last snapshot taken or restored. Every chunk not marked in `dirty` equals
    /// its chunk exactly (slot for slot, vacant ones included).
    base: Option<Snapshot<V>>,
    _k: PhantomData<fn(K) -> K>,
}

#[inline]
fn key<K: Key>(idx: usize, version: u32) -> K {
    KeyData::from_ffi(u64::from(version) << 32 | idx as u64).into()
}

/// A key's slot index and version.
#[inline]
fn parts<K: Key>(k: K) -> (usize, u32) {
    let ffi = k.data().as_ffi();
    ((ffi & 0xffff_ffff) as usize, (ffi >> 32) as u32)
}

/// Slots of chunk `c` in a store of `len` slots.
fn chunk_range(c: usize, len: usize) -> Range<usize> {
    c << SHIFT..((c + 1) << SHIFT).min(len)
}

impl<K: Key, V> Default for SlotStore<K, V> {
    fn default() -> Self {
        Self::with_key()
    }
}

impl<K: Key, V> SlotStore<K, V> {
    /// An empty store (with `slotmap`'s sentinel slot at index 0).
    pub fn with_key() -> Self {
        Self {
            slots: vec![Slot {
                version: 0,
                content: Content::Vacant(0),
            }],
            free_head: 1,
            num_elems: 0,
            dirty: Vec::new(),
            base: None,
            _k: PhantomData,
        }
    }

    pub fn new() -> Self {
        Self::with_key()
    }

    pub fn len(&self) -> usize {
        self.num_elems as usize
    }

    pub fn is_empty(&self) -> bool {
        self.num_elems == 0
    }

    #[inline]
    pub fn contains_key(&self, k: K) -> bool {
        self.get(k).is_some()
    }

    #[inline]
    pub fn get(&self, k: K) -> Option<&V> {
        let (idx, version) = parts(k);
        match self.slots.get(idx) {
            Some(Slot {
                version: v,
                content: Content::Occupied(x),
            }) if *v == version => Some(x),
            _ => None,
        }
    }

    /// Marks the chunk of slot `idx` changed.
    #[inline]
    fn touch(&mut self, idx: usize) {
        if let Some(d) = self.dirty.get_mut(idx >> SHIFT) {
            *d = true;
        }
    }

    fn touch_all(&mut self) {
        self.dirty.fill(true);
    }

    #[inline]
    pub fn get_mut(&mut self, k: K) -> Option<&mut V> {
        let (idx, version) = parts(k);
        if let Some(d) = self.dirty.get_mut(idx >> SHIFT) {
            *d = true;
        }
        match self.slots.get_mut(idx) {
            Some(Slot {
                version: v,
                content: Content::Occupied(x),
            }) if *v == version => Some(x),
            _ => None,
        }
    }

    #[inline]
    pub fn insert(&mut self, value: V) -> K {
        self.insert_with_key(|_| value)
    }

    #[inline]
    pub fn insert_with_key(&mut self, f: impl FnOnce(K) -> V) -> K {
        let idx = self.free_head as usize;
        if let Some(slot) = self.slots.get_mut(idx) {
            let version = slot.version | 1;
            let k = key(idx, version);
            let value = f(k);
            let Content::Vacant(next) = slot.content else {
                unreachable!("free list on an occupied slot")
            };
            *slot = Slot {
                version,
                content: Content::Occupied(value),
            };
            self.free_head = next;
            self.num_elems += 1;
            self.touch(idx);
            return k;
        }
        assert!(self.slots.len() < u32::MAX as usize, "SlotMap is full");
        let k = key(idx, 1);
        self.slots.push(Slot {
            version: 1,
            content: Content::Occupied(f(k)),
        });
        match self.dirty.get_mut(idx >> SHIFT) {
            Some(d) => *d = true,
            None => self.dirty.push(true),
        }
        self.free_head = idx as u32 + 1;
        self.num_elems += 1;
        k
    }

    #[inline]
    pub fn remove(&mut self, k: K) -> Option<V> {
        self.get(k)?;
        let (idx, _) = parts(k);
        self.touch(idx);
        let slot = &mut self.slots[idx];
        let Content::Occupied(value) =
            std::mem::replace(&mut slot.content, Content::Vacant(self.free_head))
        else {
            unreachable!()
        };
        slot.version = slot.version.wrapping_add(1);
        self.free_head = idx as u32;
        self.num_elems -= 1;
        Some(value)
    }

    /// Keeps the entries `f` returns true for, in key order (as `SlotMap::retain`).
    pub fn retain(&mut self, mut f: impl FnMut(K, &mut V) -> bool) {
        self.touch_all();
        for idx in 1..self.slots.len() {
            let slot = &mut self.slots[idx];
            let version = slot.version;
            let keep = match &mut slot.content {
                Content::Occupied(v) => f(key(idx, version), v),
                Content::Vacant(_) => true,
            };
            if !keep {
                self.remove(key(idx, version));
            }
        }
    }

    /// Removes every entry, in key order (as `SlotMap::clear`).
    pub fn clear(&mut self) {
        self.retain(|_, _| false);
    }

    #[inline]
    pub fn iter(&self) -> Iter<'_, K, V> {
        Iter {
            slots: self.slots.iter(),
            idx: 0,
            remaining: self.len(),
            _k: PhantomData,
        }
    }

    pub fn iter_mut(&mut self) -> IterMut<'_, K, V> {
        self.touch_all();
        IterMut {
            remaining: self.len(),
            slots: self.slots.iter_mut(),
            idx: 0,
            _k: PhantomData,
        }
    }

    #[inline]
    pub fn keys(&self) -> Keys<'_, K, V> {
        Keys(self.iter())
    }

    #[inline]
    pub fn values(&self) -> Values<'_, K, V> {
        Values(self.iter())
    }

    pub fn values_mut(&mut self) -> ValuesMut<'_, K, V> {
        ValuesMut(self.iter_mut())
    }

    /// Whether both are exactly the same, slot for slot (vacant slots and the free list
    /// included).
    pub fn identical(&self, other: &Self) -> bool
    where
        V: PartialEq,
    {
        self.free_head == other.free_head
            && self.num_elems == other.num_elems
            && self.slots == other.slots
    }

    /// Whether chunk `c` is known to equal chunk `c` of `base`.
    fn clean(&self, c: usize) -> bool {
        !self.dirty.get(c).copied().unwrap_or(true)
            && self.base.as_ref().is_some_and(|b| c < b.0.chunks.len())
    }

    /// Whether both hold the same values under the same keys (free slots are not
    /// compared). Chunks both have unchanged from one same snapshot are not looked into.
    pub fn same_entries(&self, other: &Self) -> bool
    where
        V: PartialEq,
    {
        /// The occupied slots in `range`.
        fn occupied<V>(
            slots: &[Slot<V>],
            range: Range<usize>,
        ) -> impl Iterator<Item = (usize, u32, &V)> {
            let start = range.start;
            slots
                .get(range)
                .unwrap_or(&[])
                .iter()
                .enumerate()
                .filter_map(move |(i, s)| match &s.content {
                    Content::Occupied(v) => Some((start + i, s.version, v)),
                    Content::Vacant(_) => None,
                })
        }
        if self.len() != other.len() {
            return false;
        }
        let n = self.slots.len().max(other.slots.len()).div_ceil(CHUNK);
        (0..n).all(|c| {
            let shared = self.clean(c)
                && other.clean(c)
                && match (&self.base, &other.base) {
                    (Some(a), Some(b)) => Arc::ptr_eq(&a.0.chunks[c], &b.0.chunks[c]),
                    _ => false,
                };
            shared || {
                let r = c << SHIFT..(c + 1) << SHIFT;
                let a = occupied(&self.slots, r.start..r.end.min(self.slots.len()));
                let b = occupied(&other.slots, r.start..r.end.min(other.slots.len()));
                a.eq(b)
            }
        })
    }
}

impl<K: Key, V: Clone + PartialEq> SlotStore<K, V> {
    /// The current state as a snapshot, which becomes the base the next one starts
    /// from. Chunks unchanged since the last snapshot (or equal to it, slot for slot)
    /// are shared with it; the others are copied.
    pub fn freeze(&mut self) -> Snapshot<V> {
        let n = self.slots.len().div_ceil(CHUNK);
        if let Some(b) = &self.base
            && b.0.slots == self.slots.len()
            && b.0.free_head == self.free_head
            && b.0.num_elems == self.num_elems
            && !self.dirty.contains(&true)
        {
            return b.clone();
        }
        let chunks = (0..n)
            .map(|c| {
                let r = chunk_range(c, self.slots.len());
                let old = (self.base.as_ref()).and_then(|b| b.0.chunks.get(c));
                match old {
                    Some(old) if self.clean(c) => old.clone(),
                    Some(old) if **old == self.slots[r.clone()] => old.clone(),
                    _ => Arc::from(&self.slots[r]),
                }
            })
            .collect();
        let s = Snapshot(Arc::new(Frozen {
            chunks,
            slots: self.slots.len(),
            free_head: self.free_head,
            num_elems: self.num_elems,
        }));
        self.base = Some(s.clone());
        self.dirty.clear();
        self.dirty.resize(n, false);
        s
    }

    /// Makes this store hold `s`'s state, which becomes its base. Rewrites only the
    /// chunks that differ from it; returns whether anything did.
    pub fn thaw(&mut self, s: &Snapshot<V>) -> bool {
        let f = &*s.0;
        let mut changed = self.slots.len() != f.slots
            || self.free_head != f.free_head
            || self.num_elems != f.num_elems;
        self.slots.truncate(f.slots);
        for (c, chunk) in f.chunks.iter().enumerate() {
            let r = chunk_range(c, f.slots);
            let same = self.clean(c)
                && (self.base.as_ref()).is_some_and(|b| Arc::ptr_eq(&b.0.chunks[c], chunk))
                && self.slots.len() >= r.end;
            if same {
                continue;
            }
            changed = true;
            if self.slots.len() >= r.end {
                self.slots[r].clone_from_slice(chunk);
            } else {
                self.slots.truncate(r.start);
                self.slots.extend_from_slice(chunk);
            }
        }
        self.free_head = f.free_head;
        self.num_elems = f.num_elems;
        self.dirty.clear();
        self.dirty.resize(f.chunks.len(), false);
        self.base = Some(s.clone());
        changed
    }
}

/// A full copy (keeping the base snapshot it shares unchanged chunks with).
impl<K: Key, V: Clone> Clone for SlotStore<K, V> {
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
            free_head: self.free_head,
            num_elems: self.num_elems,
            dirty: self.dirty.clone(),
            base: self.base.clone(),
            _k: PhantomData,
        }
    }
}

/// `SlotMap`'s derived format, field for field (the change tracking is left out).
impl<K: Key, V: fmt::Debug> fmt::Debug for SlotStore<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("SlotMap")
            .field("slots", &self.slots)
            .field("free_head", &self.free_head)
            .field("num_elems", &self.num_elems)
            .field("_k", &self._k)
            .finish()
    }
}

impl<K: Key, V> Index<K> for SlotStore<K, V> {
    type Output = V;

    #[inline]
    fn index(&self, k: K) -> &V {
        match self.get(k) {
            Some(v) => v,
            None => panic!("invalid SlotMap key used"),
        }
    }
}

impl<K: Key, V> IndexMut<K> for SlotStore<K, V> {
    #[inline]
    fn index_mut(&mut self, k: K) -> &mut V {
        match self.get_mut(k) {
            Some(v) => v,
            None => panic!("invalid SlotMap key used"),
        }
    }
}

/// Entries in key order.
pub struct Iter<'a, K, V> {
    slots: std::slice::Iter<'a, Slot<V>>,
    /// Index of the slot `slots` yields next.
    idx: usize,
    remaining: usize,
    _k: PhantomData<fn(K) -> K>,
}

impl<K, V> Clone for Iter<'_, K, V> {
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
            idx: self.idx,
            remaining: self.remaining,
            _k: PhantomData,
        }
    }
}

impl<'a, K: Key, V> Iterator for Iter<'a, K, V> {
    type Item = (K, &'a V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while self.remaining > 0 {
            let s = self.slots.next()?;
            self.idx += 1;
            if let Content::Occupied(v) = &s.content {
                self.remaining -= 1;
                return Some((key(self.idx - 1, s.version), v));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

/// Entries in key order, writable.
pub struct IterMut<'a, K, V> {
    slots: std::slice::IterMut<'a, Slot<V>>,
    idx: usize,
    remaining: usize,
    _k: PhantomData<fn(K) -> K>,
}

impl<'a, K: Key, V> Iterator for IterMut<'a, K, V> {
    type Item = (K, &'a mut V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while self.remaining > 0 {
            let s = self.slots.next()?;
            self.idx += 1;
            if let Content::Occupied(v) = &mut s.content {
                self.remaining -= 1;
                return Some((key(self.idx - 1, s.version), v));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

/// Keys in order.
pub struct Keys<'a, K, V>(Iter<'a, K, V>);

/// Values in key order.
pub struct Values<'a, K, V>(Iter<'a, K, V>);

/// Values in key order, writable.
pub struct ValuesMut<'a, K, V>(IterMut<'a, K, V>);

impl<K, V> Clone for Keys<'_, K, V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<K, V> Clone for Values<'_, K, V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<K: Key, V> Iterator for Keys<'_, K, V> {
    type Item = K;

    #[inline]
    fn next(&mut self) -> Option<K> {
        self.0.next().map(|(k, _)| k)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<'a, K: Key, V> Iterator for Values<'a, K, V> {
    type Item = &'a V;

    #[inline]
    fn next(&mut self) -> Option<&'a V> {
        self.0.next().map(|(_, v)| v)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<'a, K: Key, V> Iterator for ValuesMut<'a, K, V> {
    type Item = &'a mut V;

    #[inline]
    fn next(&mut self) -> Option<&'a mut V> {
        self.0.next().map(|(_, v)| v)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<K: Key, V> ExactSizeIterator for Iter<'_, K, V> {}
impl<K: Key, V> ExactSizeIterator for IterMut<'_, K, V> {}
impl<K: Key, V> ExactSizeIterator for Keys<'_, K, V> {}
impl<K: Key, V> ExactSizeIterator for Values<'_, K, V> {}
impl<K: Key, V> ExactSizeIterator for ValuesMut<'_, K, V> {}
impl<K: Key, V> std::iter::FusedIterator for Iter<'_, K, V> {}
impl<K: Key, V> std::iter::FusedIterator for IterMut<'_, K, V> {}
impl<K: Key, V> std::iter::FusedIterator for Keys<'_, K, V> {}
impl<K: Key, V> std::iter::FusedIterator for Values<'_, K, V> {}
impl<K: Key, V> std::iter::FusedIterator for ValuesMut<'_, K, V> {}

impl<'a, K: Key, V> IntoIterator for &'a SlotStore<K, V> {
    type Item = (K, &'a V);
    type IntoIter = Iter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, K: Key, V> IntoIterator for &'a mut SlotStore<K, V> {
    type Item = (K, &'a mut V);
    type IntoIter = IterMut<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    slotmap::new_key_type! { struct Id; }

    /// Random inserts, removes and changes give the same keys, iteration and dump as
    /// `SlotMap`; snapshots keep their state and restore it exactly.
    #[test]
    fn behaves_as_slotmap() {
        let mut a: SlotMap<Id, u64> = SlotMap::with_key();
        let mut b: SlotStore<Id, u64> = SlotStore::with_key();
        let mut keys: Vec<Id> = Vec::new();
        let mut snaps = Vec::new();
        let mut seed = 12345u64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for step in 0..20_000u64 {
            let r = rand();
            if r % 3 != 0 || keys.is_empty() {
                let ka = a.insert(step);
                let kb = b.insert(step);
                assert_eq!(ka, kb);
                keys.push(ka);
            } else {
                let k = keys.swap_remove((r / 3) as usize % keys.len());
                assert_eq!(a.remove(k), b.remove(k));
                assert_eq!(a.remove(k), b.remove(k));
            }
            if step % 997 == 0 {
                snaps.push((format!("{a:?}"), b.freeze()));
                let k = keys[(r as usize / 7) % keys.len()];
                *a.get_mut(k).unwrap() += 1;
                b[k] += 1;
            }
            if step % 4001 == 0 {
                a.retain(|k, v| (k.data().as_ffi() + *v) % 5 != 0);
                b.retain(|k, v| (k.data().as_ffi() + *v) % 5 != 0);
                keys.retain(|k| a.contains_key(*k));
            }
        }
        let dump = format!("{a:?}");
        assert_eq!(dump, format!("{b:?}"));
        assert!(a.iter().eq(b.iter()));
        assert_eq!(a.len(), b.len());
        // Every snapshot kept the state it was taken in, and restores it.
        let last = b.freeze();
        for (d, s) in snaps.iter().rev() {
            assert_eq!(d, &format!("{:?}", s.to_store::<Id>()));
            b.thaw(s);
            assert_eq!(d, &format!("{b:?}"));
        }
        assert!(b.thaw(&last));
        assert_eq!(dump, format!("{b:?}"));
        assert!(!b.thaw(&last), "nothing to rewrite");
        // A small change shares all but one chunk with the snapshot before.
        let k = keys[keys.len() / 2];
        b[k] += 1;
        let next = b.freeze();
        let (same, all) = next.chunks_shared_with(&last);
        assert_eq!(same + 1, all);
        let c = b.clone();
        assert!(c.same_entries(&b));
        let mut d = last.to_store::<Id>();
        assert!(!d.same_entries(&b));
        d.thaw(&next);
        assert!(d.same_entries(&b));
        b.clear();
        a.clear();
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
    }
}
