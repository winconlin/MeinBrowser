#!/usr/bin/env python3
"""Generate original synthetic WebIDL member-attribute conformance seeds."""
from pathlib import Path
SEEDS = {
    "member-attrs-same-object": "interface Value {}; interface I { [SameObject] readonly attribute Value value; };",
    "member-attrs-nullable": "typedef object? Value; interface I { [SameObject] readonly attribute Value value; };",
    "member-attrs-new-promise": "interface I { [NewObject] Promise<long> create(); };",
    "member-attrs-named-getter": "interface Value {}; interface I { [NewObject] getter Value item(DOMString key); };",
    "member-attrs-anonymous-getter": "interface Value {}; interface I { [NewObject] getter Value (DOMString key); };",
    "member-attrs-replaceable-namespace": "namespace N { [Replaceable] readonly attribute object value; };",
    "member-attrs-setter-conflict": "interface I { [Replaceable,LegacyLenientSetter] readonly attribute object value; };",
    "member-attrs-forward-conflict": "interface I { [PutForwards=text,Replaceable] readonly attribute object value; };",
    "member-attrs-forward-promise": "interface I { [PutForwards=text] readonly attribute Promise<long> value; };",
    "member-attrs-argument": "interface I { undefined run([NewObject] object value); };",
    "member-attrs-form": "interface I { [SameObject=Value] readonly attribute object value; };",
    "member-attrs-opaque": "interface I { [Custom=(SameObject,NewObject)] readonly attribute long value; };",
}


def generate(root):
    root.mkdir(parents=True, exist_ok=True)
    for name, source in SEEDS.items():
        (root / name).write_bytes(source.encode("utf-8"))


if __name__ == "__main__":
    generate(Path(__file__).resolve().parent / "seeds" / "webidl")
