#!/usr/bin/env python3
"""Generate synthetic WebIDL union-distinguishability seeds."""

from pathlib import Path

SEEDS = {
    "union-numeric-ambiguous": "typedef (long or double) Choice;",
    "union-string-enum": 'enum State {"on"}; typedef (State or DOMString) Choice;',
    "union-interface-ancestor": "interface Base {}; interface Child : Base {}; typedef (Base or Child) Choice;",
    "union-interface-siblings": "interface Base {}; interface Left : Base {}; interface Right : Base {}; typedef (Left or Right) Choice;",
    "union-distinct-buffers": "typedef (ArrayBuffer or DataView or Uint8Array) Choice;",
    "union-legacy-callback": "[LegacyTreatNonObjectAsNull] callback F = undefined (); dictionary D {}; typedef (F or D) Choice;",
    "union-sequence-category": "typedef (sequence<long> or FrozenArray<DOMString>) Choice;",
    "union-shared-alias": "typedef (long or DOMString) Inner; typedef (Inner or Inner) Choice;",
}


def generate(root):
    root.mkdir(parents=True, exist_ok=True)
    for name, source in SEEDS.items():
        (root / name).write_bytes(source.encode("utf-8"))


if __name__ == "__main__":
    generate(Path(__file__).resolve().parent / "seeds" / "webidl")
