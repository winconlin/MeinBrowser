#!/usr/bin/env python3
"""Generate original synthetic WebIDL collection conformance seeds."""

from pathlib import Path

SEEDS = {
    "collection-conflicting-kinds": "interface I { iterable<DOMString,long>; maplike<DOMString,long>; };",
    "collection-mixin-name": "interface I { setlike<long>; }; interface mixin M { readonly attribute long add; }; I includes M;",
    "collection-static-operation": "interface I { maplike<DOMString,long>; static undefined get(); };",
    "collection-async-required": "interface I { async_iterable<long>(long count); };",
    "collection-async-variadic": "interface I { async_iterable<long>(DOMString... options); };",
    "collection-value-getter": "interface I { iterable<long>; };",
    "collection-value-alias": "typedef [Tag] long Value; interface I { readonly attribute unsigned long length; getter Value item(unsigned long index); iterable<long>; };",
    "collection-union-order": "interface I { readonly attribute unsigned long length; getter (long or DOMString) item(unsigned long index); iterable<(DOMString or long)>; };",
    "collection-map-indexed": "interface I { readonly attribute unsigned long length; getter long item(unsigned long index); maplike<DOMString,long>; };",
    "collection-nullable-union": "interface I { readonly attribute unsigned long length; getter (long or DOMString)? item(unsigned long index); iterable<(DOMString or long?)>; };",
}


def generate(root):
    root.mkdir(parents=True, exist_ok=True)
    for name, source in SEEDS.items():
        (root / name).write_bytes(source.encode("utf-8"))


if __name__ == "__main__":
    generate(Path(__file__).resolve().parent / "seeds" / "webidl")
