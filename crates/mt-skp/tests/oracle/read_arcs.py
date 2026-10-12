"""Prints OpenSKP's decoded `CArcCurve` frames of a legacy .skp as JSON, by slot.

    python3 -I read_arcs.py FILE.skp

`tools/oracle_dump.py` leaves them out: OpenSKP keeps them on its internal geometry
builder (`builder.arc_curves`), not on the public model. `tests/write.rs` compares them
with `mt_skp`'s `Definition::arc_curves`.
"""

import json
import sys

from openskp import legacy


def main(path):
    with open(path, "rb") as f:
        data = f.read()
    ar = legacy._walk(data)[0]
    arcs = {
        str(slot): ent[2]["params"]
        for slot, ent in ar.slots.items()
        if ent[0] == "obj" and isinstance(ent[2], dict) and ent[2].get("k") == "arccurve"
    }
    json.dump(arcs, sys.stdout, indent=1, sort_keys=True)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main(sys.argv[1])
