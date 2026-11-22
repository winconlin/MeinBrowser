#!/usr/bin/env python3
"""Generate original synthetic special-operation/stringifier WebIDL seeds."""

from pathlib import Path

SEEDS = {
    "special-key-usvstring": "interface I { getter long get(USVString key); };",
    "special-optional-getter": "interface I { getter long get(optional DOMString key); };",
    "special-indexed-length": "interface I { getter long get(unsigned long index); };",
    "special-duplicate-getter": "interface I { getter long a(DOMString key); getter long b(DOMString key); };",
    "special-missing-getter": "interface I { setter undefined put(DOMString key, long value); };",
    "special-stringifier-bytestring": "interface I { stringifier attribute ByteString text; };",
    "special-mixin-stringifiers": "interface I { stringifier; }; interface mixin M { stringifier; }; I includes M;",
    "special-valid-aliases": "typedef [Tag] DOMString Key; interface I { getter long get(Key key); };",
    "special-inherited-getter": "interface Base { getter long get(DOMString key); }; interface I : Base { setter undefined put(DOMString key, long value); };",
    "special-annotated-length": "typedef [Tag] unsigned short Size; interface I { readonly attribute Size length; getter long get(unsigned long index); };",
}


def generate(root):
    root.mkdir(parents=True, exist_ok=True)
    for name, source in SEEDS.items():
        (root / name).write_bytes(source.encode("utf-8"))


if __name__ == "__main__":
    generate(Path(__file__).resolve().parent / "seeds" / "webidl")
