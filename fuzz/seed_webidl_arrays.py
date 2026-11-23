#!/usr/bin/env python3
"""Generate original synthetic WebIDL reference-array use-site seeds."""
from pathlib import Path

SEEDS = {
    "arrays-frozen-attribute": "interface I { static attribute FrozenArray<sequence<long>> values; };",
    "arrays-observable-attribute": "typedef long? Element; interface I { attribute ObservableArray<Element> values; };",
    "arrays-mixin-partial": "interface mixin M {}; partial interface mixin M { readonly attribute ObservableArray<long> values; }; interface I {}; I includes M;",
    "arrays-frozen-operation": "typedef FrozenArray<long> Values; interface I { Values run(); };",
    "arrays-observable-static": "typedef ObservableArray<long> Values; interface I { static attribute Values values; };",
    "arrays-frozen-dictionary": "dictionary D { FrozenArray<long> values; };",
    "arrays-observable-element": "typedef sequence<long> Item; interface I { attribute ObservableArray<Item> values; };",
    "arrays-nested-parameter": "typedef FrozenArray<long> Values; typedef Promise<Values> Nested;",
    "arrays-frozen-union": "interface I { attribute (FrozenArray<long> or DOMString) values; };",
    "arrays-observable-union-element": "dictionary D {}; interface I { attribute ObservableArray<(D or DOMString)> values; };",
    "arrays-callback-result": "callback F = FrozenArray<long> ();",
    "arrays-namespace": "namespace N { readonly attribute FrozenArray<long> values; };",
}


def generate(root):
    root.mkdir(parents=True, exist_ok=True)
    for name, source in SEEDS.items():
        (root / name).write_bytes(source.encode("utf-8"))


if __name__ == "__main__":
    generate(Path(__file__).resolve().parent / "seeds" / "webidl")
