#!/usr/bin/env python3
"""Generate original synthetic WebIDL type-attribute conformance seeds."""
from pathlib import Path

SEEDS = {
    "attributes-integer-alias": "typedef [Tag] long Number; interface I { undefined clamp([Clamp] Number value); undefined enforce([EnforceRange] Number value); readonly attribute Number plain; };",
    "attributes-empty-nullable": "typedef [LegacyNullToEmptyString] DOMString? Text;",
    "attributes-buffer-flags": "typedef [AllowShared,AllowResizable] DataView? View;",
    "attributes-shared-buffer": "typedef [AllowShared] ArrayBuffer Buffer;",
    "attributes-optional-placement": "interface I { undefined run([Clamp] optional long value=0); };",
    "attributes-required-field": "dictionary D { [Clamp] long count; required [EnforceRange] long requiredCount; };",
    "attributes-readonly-alias": "typedef [Clamp] long Count; interface I { readonly attribute Count value; };",
    "attributes-alias-conflict": "typedef [Clamp] long Count; interface I { undefined run([EnforceRange] Count value); };",
    "attributes-form": "typedef [Clamp=Value] long Count;",
    "attributes-opaque": "typedef [Custom=(Clamp,EnforceRange)] long Count;",
    "attributes-nested-readonly": "interface I { readonly attribute FrozenArray<[Clamp] long> values; };",
    "attributes-clamp-boolean": "typedef [Clamp] boolean Value;",
}


def generate(root):
    root.mkdir(parents=True, exist_ok=True)
    for name, source in SEEDS.items():
        (root / name).write_bytes(source.encode("utf-8"))


if __name__ == "__main__":
    generate(Path(__file__).resolve().parent / "seeds" / "webidl")
