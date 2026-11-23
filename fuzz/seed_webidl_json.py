#!/usr/bin/env python3
"""Generate original synthetic WebIDL toJSON and JSON-type seeds."""

from pathlib import Path

SEEDS = {
    "json-bigint-result": "interface I { bigint toJSON(); };",
    "json-optional-argument": "interface I { object toJSON(optional long value=0); };",
    "json-static-operation": "interface I { static bigint toJSON(long ignored); };",
    "json-named-getter": "interface I { getter long toJSON(DOMString key); };",
    "json-dictionary-partial": "dictionary D {}; partial dictionary D { any opaque; }; interface I { D toJSON(); };",
    "json-dictionary-parent": "dictionary Base { long count; }; dictionary D : Base { sequence<DOMString> names; }; interface I { D toJSON(); };",
    "json-inherited-interface": "interface Base { object toJSON(); }; interface Child : Base {}; interface I { Child toJSON(); };",
    "json-mixin-interface": "interface mixin M { object toJSON(); }; interface Host {}; Host includes M; interface I { Host toJSON(); };",
    "json-interface-self": "interface I { I toJSON(); };",
    "json-promise-field": "dictionary D { Promise<D> future; }; interface I { D toJSON(); };",
}


def generate(root):
    root.mkdir(parents=True, exist_ok=True)
    for name, source in SEEDS.items():
        (root / name).write_bytes(source.encode("utf-8"))


if __name__ == "__main__":
    generate(Path(__file__).resolve().parent / "seeds" / "webidl")
