"""Prints the texts and dimensions OpenSKP decodes in each definition of a legacy .skp.

    python3 -I read_definition_annotations.py FILE.skp

OpenSKP's public model keeps a definition's texts (text and points) but only the
text and hidden flag of its dimensions, and the layer of neither. This script reads
both from its internal archive, per entity list. Output: `{"root": {...},
"definitions": {name: {...}}}`, each `{"text": [...], "dimension": [...]}` in file
order: texts as `[text, layer, point, label]` (points `null` when not decoded, e.g.
for a screen text), dimensions as `[a, b, offset, layer]`; layer names are `null`
for Layer0. `crates/mt-kernel/tests/export.rs` compares them with what the kernel
saved.
"""

import json
import sys

from openskp import legacy


def main(path):
    with open(path, "rb") as f:
        data = f.read()
    ar, root, _layers, _materials = legacy._walk(data)

    def obj(slot):
        ent = ar.slots.get(slot)
        if ent and ent[0] == "obj" and isinstance(ent[2], dict):
            return ent[2]
        return None

    def layer_name(v):
        lid = v.get("db", {}).get("layer") or 0
        layer = obj(lid) if lid else None
        name = layer.get("name") if layer and layer.get("k") == "layer" else None
        return None if name == "Layer0" else name

    def point(p):
        return None if p is None else [float(c) for c in p]

    def annotations(ents):
        out = {"text": [], "dimension": []}
        for _s, _n, v in ents:
            if not isinstance(v, dict):
                continue
            if v.get("k") == "text":
                out["text"].append(
                    [v.get("text", ""), layer_name(v), point(v.get("p")), point(v.get("lp"))]
                )
            elif v.get("k") == "dimension":
                out["dimension"].append(
                    [point(v.get("a")), point(v.get("b")), v.get("offset", 0.0), layer_name(v)]
                )
        return out

    defs = {}
    for slot in sorted(ar.slots):
        ent = ar.slots[slot]
        if ent[0] == "obj" and ent[1] == "CComponentDefinition" and ent[2]:
            defs[ent[2]["name"]] = annotations(ent[2]["ents"])
    out = {"root": annotations(root), "definitions": defs}
    json.dump(out, sys.stdout, indent=1, sort_keys=True)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main(sys.argv[1])
