//! Reading glTF accessors from the JSON and buffers with every offset, size and enum
//! checked, for [`crate::gltf_io`]: the `gltf` crate's readers index, unwrap and hit
//! `unreachable!` on inputs its validation lets through (index accessors of float
//! type, strides smaller than an element, empty sparse accessors), and its validation
//! itself indexes the `POSITION` accessor before checking it exists.

use gltf::json::accessor::{ComponentType, Type};
use gltf::json::validation::Checked;
use gltf::json::{self, Root};

/// Values (components) one import may read over all its accessors at least, however
/// small its buffers: about 11 million positions.
pub(crate) const MIN_VALUES: usize = 1 << 25;

/// Most elements of an accessor without a buffer view (zeros, before sparse values).
const MAX_ZERO_COUNT: usize = 1 << 16;

/// What [`check`] and the readers found wrong.
pub(crate) type Error = String;

/// Checks what the `gltf` crate's validation reads without checking: each mesh
/// primitive's attribute, index and morph target accessors exist.
pub(crate) fn check(root: &Root) -> Result<(), Error> {
    let n = root.accessors.len();
    for (mi, m) in root.meshes.iter().enumerate() {
        for p in &m.primitives {
            let targets = p
                .targets
                .iter()
                .flatten()
                .flat_map(|t| [t.positions, t.normals, t.tangents].into_iter().flatten());
            if p.attributes
                .values()
                .copied()
                .chain(p.indices)
                .chain(targets)
                .any(|i| i.value() >= n)
            {
                return Err(format!("mesh {mi} names a missing accessor"));
            }
        }
    }
    Ok(())
}

/// Bytes, element stride and element size of an accessor's data in its view.
struct Layout<'a> {
    data: &'a [u8],
    stride: usize,
    elem: usize,
}

fn component_size(c: ComponentType) -> usize {
    match c {
        ComponentType::I8 | ComponentType::U8 => 1,
        ComponentType::I16 | ComponentType::U16 => 2,
        ComponentType::U32 | ComponentType::F32 => 4,
    }
}

fn multiplicity(t: Type) -> usize {
    match t {
        Type::Scalar => 1,
        Type::Vec2 => 2,
        Type::Vec3 => 3,
        Type::Vec4 | Type::Mat2 => 4,
        Type::Mat3 => 9,
        Type::Mat4 => 16,
    }
}

/// The bytes `count` elements of `elem` bytes at `offset` in view `view` take, with
/// the view's stride (or `elem`).
fn layout<'a>(
    root: &Root,
    buffers: &'a [Vec<u8>],
    view: json::Index<json::buffer::View>,
    offset: u64,
    count: usize,
    elem: usize,
    strided: bool,
) -> Result<Layout<'a>, Error> {
    let bad = || format!("buffer view {} is out of range", view.value());
    let v = root.buffer_views.get(view.value()).ok_or_else(bad)?;
    let buf = buffers.get(v.buffer.value()).ok_or_else(bad)?;
    let start = usize::try_from(v.byte_offset.map_or(0, |o| o.0)).map_err(|_| bad())?;
    let len = usize::try_from(v.byte_length.0).map_err(|_| bad())?;
    let view_data = buf
        .get(start..start.checked_add(len).ok_or_else(bad)?)
        .ok_or_else(bad)?;
    let stride = match v.byte_stride {
        Some(s) if strided => s.0,
        _ => elem,
    };
    if stride < elem {
        return Err(format!(
            "buffer view {} has a stride too small",
            view.value()
        ));
    }
    let offset = usize::try_from(offset).map_err(|_| bad())?;
    let need = match count {
        0 => 0,
        n => (n - 1)
            .checked_mul(stride)
            .and_then(|x| x.checked_add(elem))
            .and_then(|x| x.checked_add(offset))
            .ok_or_else(bad)?,
    };
    let data = view_data.get(offset.min(need)..need).ok_or_else(bad)?;
    Ok(Layout { data, stride, elem })
}

/// One component at `b` (its size bytes) as a number: normalized integers in 0..1
/// (or -1..1) when `normalized`.
fn component(b: &[u8], c: ComponentType, normalized: bool) -> f64 {
    // As the `gltf` crate normalizes: in f32.
    let n = |v: f32, max: f32| f64::from(if normalized { (v / max).max(-1.0) } else { v });
    match c {
        ComponentType::I8 => n(f32::from(b[0] as i8), 127.0),
        ComponentType::U8 => n(f32::from(b[0]), 255.0),
        ComponentType::I16 => n(f32::from(i16::from_le_bytes([b[0], b[1]])), 32767.0),
        ComponentType::U16 => n(f32::from(u16::from_le_bytes([b[0], b[1]])), 65535.0),
        // Indices: exact.
        ComponentType::U32 => f64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        ComponentType::F32 => f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
    }
}

/// Reads accessor `index` as `count × dims` numbers, when its type has `dims`
/// components and its component type is one of `allowed`. `budget` counts down the
/// values all reads of one import may take.
pub(crate) fn read(
    root: &Root,
    buffers: &[Vec<u8>],
    index: usize,
    dims: usize,
    allowed: &[ComponentType],
    budget: &mut usize,
) -> Result<Vec<f64>, Error> {
    let bad = |what: &str| format!("accessor {index}: {what}");
    let a = root.accessors.get(index).ok_or_else(|| bad("missing"))?;
    let (Checked::Valid(c), Checked::Valid(t)) = (a.component_type, a.type_) else {
        return Err(bad("unknown type"));
    };
    let c = c.0;
    if multiplicity(t) != dims || !allowed.contains(&c) {
        return Err(bad("unexpected type"));
    }
    let count = usize::try_from(a.count.0).map_err(|_| bad("too large"))?;
    let values = count.checked_mul(dims).ok_or_else(|| bad("too large"))?;
    if values > *budget {
        return Err("the glTF file is too large".into());
    }
    *budget -= values;
    let size = component_size(c);
    let elem = size * dims;
    let mut out = match a.buffer_view {
        Some(view) => {
            let l = layout(
                root,
                buffers,
                view,
                a.byte_offset.map_or(0, |o| o.0),
                count,
                elem,
                true,
            )?;
            let mut out = Vec::with_capacity(values);
            for i in 0..count {
                let e = &l.data[i * l.stride..i * l.stride + l.elem];
                out.extend(e.chunks_exact(size).map(|b| component(b, c, a.normalized)));
            }
            out
        }
        None if count <= MAX_ZERO_COUNT && a.sparse.is_some() => vec![0.0; values],
        None => return Err(bad("no data")),
    };
    if let Some(s) = &a.sparse {
        let n = usize::try_from(s.count.0).map_err(|_| bad("sparse count too large"))?;
        let Checked::Valid(ic) = s.indices.component_type else {
            return Err(bad("unknown sparse index type"));
        };
        let ic = ic.0;
        if !matches!(
            ic,
            ComponentType::U8 | ComponentType::U16 | ComponentType::U32
        ) {
            return Err(bad("sparse index type"));
        }
        let isize = component_size(ic);
        let idx = layout(
            root,
            buffers,
            s.indices.buffer_view,
            s.indices.byte_offset.0,
            n,
            isize,
            false,
        )?;
        let vals = layout(
            root,
            buffers,
            s.values.buffer_view,
            s.values.byte_offset.0,
            n,
            elem,
            false,
        )?;
        for k in 0..n {
            let at = component(&idx.data[k * isize..(k + 1) * isize], ic, false) as usize;
            if at >= count {
                return Err(bad("sparse index out of range"));
            }
            let e = &vals.data[k * elem..(k + 1) * elem];
            for (d, b) in e.chunks_exact(size).enumerate() {
                out[at * dims + d] = component(b, c, a.normalized);
            }
        }
    }
    Ok(out)
}

/// Floating point (or normalized integer) components.
pub(crate) const FLOATS: &[ComponentType] = &[
    ComponentType::F32,
    ComponentType::I8,
    ComponentType::U8,
    ComponentType::I16,
    ComponentType::U16,
];

/// Index component types.
pub(crate) const INDICES: &[ComponentType] =
    &[ComponentType::U8, ComponentType::U16, ComponentType::U32];

/// Whether accessor `index` is normalized or float, as texture coordinates must be.
pub(crate) fn float_or_normalized(root: &Root, index: usize) -> bool {
    root.accessors.get(index).is_some_and(|a| {
        a.normalized || matches!(a.component_type, Checked::Valid(c) if c.0 == ComponentType::F32)
    })
}
