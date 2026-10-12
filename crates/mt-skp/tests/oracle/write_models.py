"""Builds the reference models for `tests/write.rs` with OpenSKP's Python writer.

    python3 -I write_models.py OUTDIR

Writes `OUTDIR/<model>.skp` for every model below, plus the two texture images
(`tex.png`, `tex.jpg`) the textured models embed. `tests/write.rs` builds the
same models through `mt_skp::write` and requires byte-identical output.

OpenSKP stamps definitions, instances, groups and images with `uuid.uuid4()`
and definitions with `time.time()`. Both are replaced here, inside
`openskp.create` only, by the same deterministic sources the Rust side uses
(`SkpBuilder::deterministic(SEED, TIMESTAMP)`): GUIDs from SplitMix64 seeded
with SEED (each GUID = two consecutive outputs, little-endian), and a fixed
timestamp. Both are reset before each model.

One deliberate difference from OpenSKP is patched in below: `CArcCurve`'s last three
doubles are the arc's y axis (`normal x xaxis`, as OpenSKP's own reader decodes them
and SketchUp writes them), where OpenSKP's writer stores `(0, radius, 0)`, right only
for arcs about +Z.
"""

import importlib
import math
import struct
import sys
import types
import uuid
import zlib
from pathlib import Path

create_mod = importlib.import_module("openskp.create")
Point3d = create_mod.Point3d
Vector3d = create_mod.Vector3d
Length = create_mod.Length
Timestamp = create_mod.Timestamp

def _write_arc_curve(self, center, normal, xaxis, start_angle, end_angle, radius, num_segments):
    if not (0 <= num_segments <= 0xFF):
        raise create_mod.SkpWriteError(f"num_segments must be between 0 and 255, got {num_segments}")
    slot = self._new_of_known_class("CArcCurve", schema=create_mod._ARCCURVE_SCHEMA)
    self._preamble()
    self.buf += bytes([0, num_segments]) + bytes(3)
    yaxis = (normal[1] * xaxis[2] - normal[2] * xaxis[1],
             normal[2] * xaxis[0] - normal[0] * xaxis[2],
             normal[0] * xaxis[1] - normal[1] * xaxis[0])
    for v in (*center, *normal, *xaxis, start_angle, end_angle, *yaxis):
        self.buf += struct.pack("<d", v)
    return slot


create_mod._ArchiveWriter.write_arc_curve = _write_arc_curve

SEED = 0x5EED
TIMESTAMP = 1_700_000_000
M64 = (1 << 64) - 1


def splitmix_guids(seed):
    state = seed

    def nxt():
        nonlocal state
        state = (state + 0x9E3779B97F4A7C15) & M64
        z = state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & M64
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M64
        return z ^ (z >> 31)

    def uuid4():
        return uuid.UUID(bytes=nxt().to_bytes(8, "little") + nxt().to_bytes(8, "little"))

    return uuid4


def deterministic_builder():
    create_mod.uuid = types.SimpleNamespace(uuid4=splitmix_guids(SEED))
    create_mod.time = types.SimpleNamespace(time=lambda: float(TIMESTAMP))
    return create_mod.SkpBuilder()


def png_bytes():
    """A 2x2 RGB PNG."""
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    raw = b"\x00\xff\x00\x00\x00\xff\x00" + b"\x00\x00\x00\xff\xff\xff\xff"
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 2, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )


def jpg_bytes():
    """Just the JPEG magic and a stub: the writer embeds images verbatim."""
    return b"\xff\xd8\xff\xe0" + bytes(range(32)) + b"\xff\xd9"


QUAD = [(0.0, 0.0, 0.0), (100.0, 0.0, 0.0), (100.0, 50.0, 0.0), (0.0, 50.0, 0.0)]


def single_face(b, ctx):
    b.add_face(QUAD)


def box(b, ctx):
    x, y, z = 40.0, 30.0, 20.0
    p = [(0, 0, 0), (x, 0, 0), (x, y, 0), (0, y, 0), (0, 0, z), (x, 0, z), (x, y, z), (0, y, z)]
    for f in [(0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)]:
        b.add_face([p[i] for i in f])


def holes(b, ctx):
    b.add_face(
        [(0, 0, 0), (200, 0, 0), (200, 0, 100), (0, 0, 100)],
        holes=[
            [(20, 0, 20), (60, 0, 20), (60, 0, 70), (20, 0, 70)],
            [(100, 0, 30), (150, 0, 30), (125, 0, 80)],
        ],
    )
    b.add_face([(200, 0, 0), (200, 50, 0), (200, 50, 100), (200, 0, 100)])


def materials_layers(b, ctx):
    red = b.add_material("Red", (255, 0, 0))
    glass = b.add_material("Glass", (100, 150, 255, 128), opacity=0.4)
    again = b.add_material("Red", (0, 0, 0))
    roof = b.add_layer("Roof", color=(200, 10, 10))
    hidden = b.add_layer("Hidden", hidden=True)
    plain = b.add_layer("Plain")
    b.add_face(QUAD, material=red, back_material=glass, layer=roof)
    b.add_face([(0, 0, 10), (10, 0, 10), (10, 10, 10)], material=again, layer=hidden, hidden=True)
    b.add_face(
        [(0, 0, 0), (0, 50, 0), (0, 50, 30), (0, 0, 30)],
        layer=plain, soft_edges=True, smooth_edges=True, hidden_edges=True,
    )


def components(b, ctx):
    red = b.add_material("Red", (255, 0, 0))
    furniture = b.add_layer("Furniture")
    with b.add_component_definition("Chair") as chair:
        chair.add_face([(0, 0, 0), (20, 0, 0), (20, 20, 0), (0, 20, 0)])
        chair.add_face([(0, 0, 0), (0, 20, 0), (0, 20, 30), (0, 0, 30)])
    b.add_instance(chair, translation=(100, 0, 0))
    b.add_instance(chair, name="Rotated", translation=(0, 100, 0), rotation=((0, 0, 1), math.pi / 4))
    b.add_instance(
        chair, name="", matrix3x3=(2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0),
        material=red, layer=furniture, hidden=True,
    )


def groups(b, ctx):
    with b.add_group("Table", translation=(50, 0, 0)) as t:
        t.add_face([(0, 0, 0), (30, 0, 0), (30, 30, 0), (0, 30, 0)])
    with b.add_group(rotation=((1, 0, 0), math.pi / 2), attributes={"k": "v"}) as g:
        g.add_face([(0, 0, 0), (10, 0, 0), (10, 10, 0)])
    b.add_face(QUAD)


def groups_only(b, ctx):
    with b.add_group("Lonely", translation=(1, 2, 3), hidden=True) as g:
        g.add_face([(0, 0, 0), (10, 0, 0), (10, 10, 0)])


def nested(b, ctx):
    with b.add_component_definition("Wheel") as wheel:
        wheel.add_circle((0, 0, 0), (0, 1, 0), 10, num_segments=12)
    with b.add_component_definition("Engine") as engine:
        engine.add_face([(0, 0, 0), (30, 0, 0), (30, 30, 0), (0, 30, 0)])
    with b.add_component_definition("Car", always_faces_camera=True, shadows_face_sun=True) as car:
        car.add_face([(0, 0, 0), (150, 0, 0), (150, 60, 0), (0, 60, 0)])
        car.add_instance(wheel, translation=(20, 0, 0))
        car.add_instance(wheel, name="RearWheel", translation=(130, 0, 0))
        car.add_group_instance(engine, translation=(50, 10, 10))
        car.add_group_instance(engine, name="", translation=(90, 10, 10))
        car.add_text("Car", (0, 0, 0))
        car.add_construction_point((75, 30, 0))
        car.add_arc((0, 0, 0), (0, 0, 1), 5, 0, math.pi)
        car.add_polyline([(0, 0, 5), (5, 0, 5), (5, 5, 5)])
    b.add_instance(car)
    b.add_instance(car, translation=(0, 200, 0), rotation=((0, 0, 1), math.pi))
    b.add_text("Root", (10, 10, 10))


def circles(b, ctx):
    b.add_circle((50, 50, 0), (0, 0, 1), 40)
    b.add_circle((0, 0, 50), (1, 1, 1), 10, num_segments=8)
    b.add_circle((0, 0, -50), (0, 0, -1), 5, num_segments=3)


def arcs(b, ctx):
    b.add_arc((0, 0, 0), (0, 0, 1), 40, 0, math.pi / 2)
    b.add_arc((100, 0, 0), (0, 1, 0), 25, math.pi, -math.pi / 3, num_segments=7, soft_edges=True)


def polylines(b, ctx):
    b.add_polyline([(0, 0, 0), (10, 10, 0), (20, 0, 0), (30, 10, 0)])
    b.add_polyline([(0, 0, 10), (10, 0, 10), (10, 10, 10)], closed=True, hidden_edges=True)
    b.add_face([(0, 0, 0), (10, 10, 0), (0, 10, 0)])


def guides(b, ctx):
    b.add_construction_line((0, 0, 0), point2=(100, 50, 25))
    b.add_construction_line((10, 10, 10), direction=(0, 0, 2))
    b.add_construction_point((5, 5, 5))
    b.add_section_plane((0, 0, 30), (0, 0, 1))
    b.add_section_plane((10, 0, 0), (1, 2, 0))


def annotations(b, ctx):
    b.add_face(QUAD)
    b.add_text("Hello", (0, 0, 0))
    b.add_dimension((0, 0, 0), (100, 0, 0), offset=-12.5)
    b.add_text("Ünïcode ✓", (100, 50, 0), leader=(5, -5, 10))
    b.add_dimension((0, 0, 0), (0, 50, 0))


def triangulate(b, ctx):
    b.add_face([(0, 0, 0), (10, 0, 0), (10, 10, 3), (0, 10, 0)], auto_triangulate=True)
    b.add_face([(20, 0, 0), (30, 0, 0), (30, 10, 0), (20, 10, 0)], auto_triangulate=True)
    b.add_face(
        [(40, 0, 0), (50, 0, 0), (55, 5, 2), (50, 10, 0), (40, 10, 1)],
        auto_triangulate=True, attributes={"tri": True},
    )


def attributes(b, ctx):
    with b.add_component_definition(
        "Attr", attribute_dicts=[("dynamic_attributes", {"a": 1, "b": 2.5}), ("other", {"s": "x"})],
    ) as d:
        d.add_face(QUAD, attributes={
            "name": "front", "n": None, "flag": True, "flag2": False,
            "p": Point3d(1, 2, 3), "v": Vector3d(0, 0, 1), "len": Length(12.0),
            "t": Timestamp(1700000000), "arr": [1, "two", [3.0, None]], "neg": -5,
        })
    b.add_instance(d, attributes={"price": 9.99})
    b.add_instance(d, attribute_dict_name="custom", attributes={"x": 1})


def textures(b, ctx):
    brick = b.add_texture_material("Brick", ctx["png"], applied_width=8.0, applied_height=16.0)
    wood = b.add_texture_material("Wood", ctx["jpg"], opacity=0.5)
    b.add_image(ctx["png"], 48, 36, translation=(0, 0, 40), rotation=((1, 0, 0), math.pi / 2))
    b.add_face(QUAD, material=brick, front_uv=[((0, 0, 0), (0, 0)), ((50, 0, 0), (1, 0)), ((0, 50, 0), (0, 1))])
    b.add_face(
        [(0, 0, 0), (0, 0, 30), (0, 50, 30), (0, 50, 0)], material=wood, back_material=brick,
        back_uv=[((0, 0, 0), (0, 0)), ((0, 0, 30), (2, 0)), ((0, 50, 0), (0, 3))],
    )
    b.add_circle((200, 0, 0), (0, 0, 1), 20, material=brick,
                 front_uv=[((200, 0, 0), (0, 0)), ((210, 0, 0), (1, 0)), ((200, 10, 0), (0.5, 1))])


def large_grid(b, ctx):
    """Enough archive slots (> 0x7fff) to force every escape form."""
    n = 70
    for i in range(n):
        for j in range(n):
            b.add_face([(i, j, 0), (i + 1, j, 0), (i + 1, j + 1, 0), (i, j + 1, 0)])


MODELS = [
    single_face, box, holes, materials_layers, components, groups, groups_only, nested,
    circles, arcs, polylines, guides, annotations, triangulate, attributes, textures, large_grid,
]


def main(out):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    ctx = {"png": str(out / "tex.png"), "jpg": str(out / "tex.jpg")}
    Path(ctx["png"]).write_bytes(png_bytes())
    Path(ctx["jpg"]).write_bytes(jpg_bytes())
    for model in MODELS:
        b = deterministic_builder()
        model(b, ctx)
        b.save(str(out / f"{model.__name__}.skp"))


if __name__ == "__main__":
    main(sys.argv[1])
