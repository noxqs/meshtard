//! JSON dump of a [`Model`] in the shape of `tools/oracle_dump.py` (OpenSKP Python's
//! dataclasses), so the two readers can be compared field by field.
//!
//! Empty strings become `null`, as in the oracle. Byte buffers become `{"bytes": len}`.

use serde_json::{Map, Value, json};

use crate::model::*;

fn s(v: &str) -> Value {
    if v.is_empty() { Value::Null } else { json!(v) }
}

fn opt<T: Into<Value> + Clone>(v: &Option<T>) -> Value {
    v.clone().map_or(Value::Null, Into::into)
}

fn vec3(v: &Option<Vec3>) -> Value {
    v.map_or(Value::Null, |v| json!(v))
}

fn bytes(v: &Option<Vec<u8>>) -> Value {
    v.as_ref()
        .map_or(Value::Null, |b| json!({ "bytes": b.len() }))
}

fn attr(v: &AttrValue) -> Value {
    match v {
        AttrValue::Null => Value::Null,
        AttrValue::String(x) => s(x),
        AttrValue::Integer(n) => json!(n),
        AttrValue::Float(x) | AttrValue::Length(x) => json!(x),
        AttrValue::Boolean(b) => json!(b),
        AttrValue::Vec(v) => json!(v),
        AttrValue::Array(items) => Value::Array(items.iter().map(attr).collect()),
    }
}

fn dicts(d: &AttrDicts) -> Value {
    Value::Object(
        d.iter()
            .map(|(name, entries)| {
                let e: Map<String, Value> =
                    entries.iter().map(|(k, v)| (k.clone(), attr(v))).collect();
                (name.clone(), Value::Object(e))
            })
            .collect(),
    )
}

fn definition(d: &Definition) -> Value {
    let vertices: Map<String, Value> = d
        .vertices
        .values()
        .map(|v| {
            (
                v.id.to_string(),
                json!({ "id": v.id, "x": v.x, "y": v.y, "z": v.z }),
            )
        })
        .collect();
    let edges: Map<String, Value> = d
        .edges
        .values()
        .map(|e| {
            let mut v = json!({
                "id": e.id, "v1_id": e.v1_id, "v2_id": e.v2_id,
                "soft": e.soft, "smooth": e.smooth, "hidden": e.hidden,
            });
            // Only legacy files carry curve ids so far; absent means "not read".
            if let Some(c) = e.curve_id {
                v["curve_id"] = json!(c);
            }
            // The edge's own tag, only when the file gives one.
            if !e.layer.is_empty() {
                v["layer"] = json!(e.layer);
            }
            (e.id.to_string(), v)
        })
        .collect();
    let faces: Map<String, Value> = d
        .faces
        .values()
        .map(|f| {
            let loops: Vec<Vec<[i64; 2]>> = f
                .loops
                .iter()
                .map(|l| l.iter().map(|c| [c.edge_id, c.orientation]).collect())
                .collect();
            let v = json!({
                "id": f.id,
                "loops": loops,
                "normal": vec3(&f.normal),
                "material_id": opt(&f.material_id),
                "back_material_id": opt(&f.back_material_id),
                "uv_transform": f.uv_transform.map_or(Value::Null, |m| json!(m)),
                "uv_transform_back": f.uv_transform_back.map_or(Value::Null, |m| json!(m)),
                "uv_projected": f.uv_projected,
                "uv_projected_back": f.uv_projected_back,
                "hidden": f.hidden,
                "layer": s(&f.layer),
            });
            (f.id.to_string(), v)
        })
        .collect();
    let instances: Vec<Value> = d
        .instances
        .iter()
        .map(|i| {
            json!({
                "name": s(&i.name),
                "ref_idx": opt(&i.ref_idx),
                "guid": s(&i.guid),
                "matrix": if i.matrix.is_empty() { Value::Null } else { json!(i.matrix) },
                "layer": s(&i.layer),
                "properties": i.properties,
                "attribute_dictionaries": dicts(&i.attribute_dictionaries),
                "material_id": opt(&i.material_id),
                "hidden": i.hidden,
            })
        })
        .collect();
    json!({
        "id": d.id,
        "guid": s(&d.guid),
        "name": s(&d.name),
        "vertices": vertices,
        "edges": edges,
        "faces": faces,
        "instances": instances,
        "section_planes": d.section_planes.iter().map(|p| json!({
            "plane": p.plane, "name": s(&p.name), "label": s(&p.label), "hidden": p.hidden,
        })).collect::<Vec<_>>(),
        "texts": d.texts.iter().map(|t| json!({
            "text": s(&t.text), "hidden": t.hidden, "point": vec3(&t.point),
            "label_point": vec3(&t.label_point),
            "screen_position": t.screen_position.map_or(Value::Null, |p| json!(p)),
        })).collect::<Vec<_>>(),
        "dimensions": d.dimensions.iter().map(dimension).collect::<Vec<_>>(),
        "construction_lines": d.construction_lines.iter().map(|c| json!({
            "point": c.point, "direction": c.direction, "start": vec3(&c.start), "end": vec3(&c.end),
        })).collect::<Vec<_>>(),
        "construction_points": d.construction_points.iter().map(|c| json!({ "position": c.position })).collect::<Vec<_>>(),
        "always_faces_camera": d.always_faces_camera,
        "shadows_face_sun": d.shadows_face_sun,
        "is_image": d.is_image,
    })
}

fn dimension(d: &Dimension) -> Value {
    json!({
        "text": s(&d.text), "hidden": d.hidden, "a": vec3(&d.a), "b": vec3(&d.b),
        "offset": d.offset, "plane_x": vec3(&d.plane_x), "normal": vec3(&d.normal),
        "offset_direction": vec3(&d.offset_direction),
    })
}

/// The model as JSON in the oracle's shape.
pub fn to_json(m: &Model) -> Value {
    let definitions: Map<String, Value> = m
        .definitions
        .iter()
        .map(|(id, d)| (id.to_string(), definition(d)))
        .collect();
    json!({
        "version": s(&m.version),
        "units": opt(&m.units),
        "definitions": definitions,
        "root": definition(&m.root),
        "layers": m.layers.iter().map(|l| json!({
            "name": s(&l.name), "color_r": l.color[0], "color_g": l.color[1], "color_b": l.color[2],
            "hidden": l.hidden,
        })).collect::<Vec<_>>(),
        "pages": m.pages.iter().map(|p| json!({
            "name": s(&p.name), "eye": vec3(&p.eye), "target": vec3(&p.target), "up": vec3(&p.up),
            "fov": p.fov, "parallel": p.parallel, "ortho_height": p.ortho_height,
            "hidden_layers": p.hidden_layers,
        })).collect::<Vec<_>>(),
        "dimensions": m.dimensions.iter().map(dimension).collect::<Vec<_>>(),
        "materials": m.materials.iter().map(|x| json!({
            "name": s(&x.name),
            "color": x.color,
            "transparency": x.transparency,
            "id": opt(&x.id),
            "texture": x.texture.as_ref().map_or(Value::Null, |t| json!({
                "filename": s(&t.filename), "width": t.width, "height": t.height, "data": bytes(&t.data),
            })),
            "colorized": x.colorized,
            "colorize_type": x.colorize_type,
        })).collect::<Vec<_>>(),
        "styles": m.styles.iter().map(|st| json!({
            "name": s(&st.name), "front_color": st.front_color, "back_color": st.back_color,
        })).collect::<Vec<_>>(),
    })
}
