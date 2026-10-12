"""Prints the layers OpenSKP decodes for the texts and dimensions of a legacy .skp.

    python3 -I read_annotation_layers.py FILE.skp

`tools/oracle_dump.py` leaves them out: OpenSKP reads every drawbase's layer id
(`db.layer`, the slot of the `CLayer` record) but keeps it on texts and dimensions
only in its internal archive. This script resolves each id to the layer's name
through the same archive. Output: `{"text": [...], "dimension": [...]}`, each a list
of `[text, layer name or null]` in file order. `tests/write.rs` and
`tests/oracle.rs` compare them with `mt_skp`'s `TextEntity::layer` /
`Dimension::layer`.
"""

import json
import sys

from openskp import legacy


def main(path):
    with open(path, "rb") as f:
        data = f.read()
    ar = legacy._walk(data)[0]

    def obj(slot):
        ent = ar.slots.get(slot)
        if ent and ent[0] == "obj" and isinstance(ent[2], dict):
            return ent[2]
        return None

    out = {"text": [], "dimension": []}
    for slot in sorted(ar.slots):
        v = obj(slot)
        if v is None or v.get("k") not in out:
            continue
        lid = v.get("db", {}).get("layer") or 0
        layer = obj(lid) if lid else None
        name = layer.get("name") if layer and layer.get("k") == "layer" else None
        if name == "Layer0":
            name = None
        out[v["k"]].append([v.get("text", ""), name])
    json.dump(out, sys.stdout, indent=1, sort_keys=True)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main(sys.argv[1])
