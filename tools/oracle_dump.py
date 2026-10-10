"""Dump an .skp file as canonical JSON using the OpenSKP Python package (the oracle).

    python3 -I tools/oracle_dump.py FILE.skp > FILE.json

The Rust reader (`cargo run -p mt-skp --example dump FILE.skp`) emits the same shape;
`crates/mt-skp/tests/oracle.rs` compares the two.
"""

import dataclasses
import json
import sys

import openskp


def canon(v):
    if dataclasses.is_dataclass(v):
        return {f.name: canon(getattr(v, f.name)) for f in dataclasses.fields(v)}
    if isinstance(v, (bytes, bytearray)):
        return {"bytes": len(v)}
    if isinstance(v, dict):
        return {str(k): canon(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [canon(x) for x in v]
    if v == "":
        return None
    return v


def main(path):
    model = openskp.SkpFile(path).parse()
    out = canon(model)
    out["root"] = canon(model.root)
    out.pop("materials_by_id", None)
    json.dump(out, sys.stdout, indent=1, sort_keys=True)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main(sys.argv[1])
