#!/usr/bin/env python3
"""Generate original synthetic WebIDL literal/default conformance seeds."""

from pathlib import Path

SEEDS = {
    "value-u64-overflow": "interface I { const unsigned long long n = 18446744073709551616; };",
    "value-octal-overflow": "interface I { const octet n = 0400; };",
    "value-bigint-exact": "interface I { const bigint n = 0xffffffffffffffffffffffffffffffff; };",
    "value-float-overflow": "interface I { const float n = 3.4028236e38; };",
    "value-enum-default": 'enum E {"on"}; dictionary D { E value = "off"; };',
    "value-bytestring-scalar": 'dictionary D { ByteString value = "😀"; };',
    "value-sequence-default": "dictionary D { (sequence<long> or DOMString) value = []; };",
    "value-dictionary-optional": "dictionary D {}; interface I { undefined f(D options); };",
    "value-dictionary-ancestor": "dictionary Base { required long x; }; dictionary Child : Base {}; interface I { undefined f(Child options); };",
    "value-undefined-union": "typedef (undefined or long) U; interface I { undefined f(U value); };",
}


def generate(root):
    root.mkdir(parents=True, exist_ok=True)
    for name, source in SEEDS.items():
        (root / name).write_bytes(source.encode("utf-8"))


if __name__ == "__main__":
    generate(Path(__file__).resolve().parent / "seeds" / "webidl")
