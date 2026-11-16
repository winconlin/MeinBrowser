#!/usr/bin/env python3
"""Generate synthetic WebIDL dictionary-inclusion seeds."""

from pathlib import Path

SEEDS = {
    "dictionary-direct": "dictionary D { D self; };",
    "dictionary-partial-alias": "partial dictionary A { Items next; }; typedef sequence<B> Items; dictionary A {}; dictionary B { A back; };",
    "dictionary-inherited-cycle": "dictionary Base { Child child; }; dictionary Child : Base {};",
    "dictionary-record-cycle": "dictionary A { record<DOMString,FrozenArray<B>> values; }; dictionary B { A back; };",
    "dictionary-promise-boundary": "dictionary D { Promise<D> deferred; async_sequence<D> stream; };",
    "dictionary-safe-ancestor": "dictionary Base { long n; }; dictionary Child : Base { Base ancestor; };",
    "dictionary-shared-dag": "dictionary Leaf { long n; }; dictionary A { Leaf left; Leaf right; }; dictionary B { A left; A right; };",
    "dictionary-nullable-sequence": "typedef sequence<D>? Items; dictionary D { Items values; };",
}


def generate(root):
    root.mkdir(parents=True, exist_ok=True)
    for name, source in SEEDS.items():
        (root / name).write_bytes(source.encode("utf-8"))


if __name__ == "__main__":
    generate(Path(__file__).resolve().parent / "seeds" / "webidl")
