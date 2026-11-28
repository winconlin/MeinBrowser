#!/usr/bin/env python3
"""Generate small first-party seeds; no downloads or third-party font data."""

from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).resolve().parent / "seeds"


def write(target, name, data):
    directory = ROOT / target
    directory.mkdir(parents=True, exist_ok=True)
    (directory / name).write_bytes(data.encode() if isinstance(data, str) else data)


def font():
    # Small synthetic sfnt, following the project's own OpenType unit fixtures.
    head = bytearray(54)
    struct.pack_into(">H", head, 18, 1000)
    struct.pack_into(">H", head, 50, 1)  # long loca offsets
    hhea = bytearray(36)
    struct.pack_into(">hh", hhea, 4, 800, -200)
    struct.pack_into(">H", hhea, 34, 1)
    cmap = struct.pack(">6H", 0, 1, 3, 1, 0, 12)
    # Printable ASCII maps to glyph 1 through the glyph ID array right after
    # idRangeOffset, then the final segment for U+FFFF.
    cmap += struct.pack(">16H", 4, 32 + 2 * 95, 0, 4, 4, 1, 0, 0x7E, 0xFFFF, 0, 0x20, 0xFFFF, 0, 1, 4, 0)
    cmap += struct.pack(">95H", *[1] * 95)
    glyph = struct.pack(">5h2H", 1, 0, 0, 10, 10, 3, 0)
    glyph += bytes([0x31, 0x33, 0x35, 0x23, 10, 10, 10, 0])
    # A family name, so the page target's font library selects this face.
    family = "Fuzz".encode("utf-16-be")
    name = struct.pack(">9H", 0, 1, 18, 3, 1, 0x409, 1, len(family), 0) + family
    tables = {
        b"cmap": cmap, b"glyf": glyph + glyph, b"head": head, b"hhea": hhea,
        b"hmtx": struct.pack(">Hhh", 500, 0, 0), b"loca": struct.pack(">III", 0, len(glyph), 2 * len(glyph)),
        b"maxp": struct.pack(">IH", 0x00005000, 2), b"name": name,
    }
    header = struct.pack(">I4H", 0x00010000, len(tables), 0, 0, 0)
    directory = bytearray()
    body = bytearray()
    for tag, data in sorted(tables.items()):
        offset = 12 + 16 * len(tables) + len(body)
        # This reader deliberately does not require sfnt checksums.
        directory += tag + struct.pack(">III", 0, offset, len(data))
        body += data
        body += b"\0" * (-len(body) % 4)
    return header + directory + body


def runtime_seeds():
    """Operation programs for the runtime models, not JavaScript source."""
    def heap(name, commands):
        write("js-runtime", name, bytes([0]) + bytes(value for command in commands for value in command))

    def objects(name, commands):
        write("js-runtime", name, bytes([1]) + bytes(value for command in commands for value in command))

    def arrays(name, commands):
        write("js-runtime", name, bytes([2]) + bytes(value for command in commands for value in command))

    def conversions(name, commands):
        write("js-runtime", name, bytes([3]) + bytes(value for command in commands for value in command))

    def text_keys(name, commands):
        write("js-runtime", name, bytes([4]) + bytes(value for command in commands for value in command))

    heap("heap-cycle", [
        (1, 0, 2, 0), (1, 1, 1, 0), (2, 0, 0, 1), (2, 1, 0, 0),
        (4, 2, 0, 0), (5, 0, 0, 0), (5, 1, 0, 0), (6, 0, 0, 0),
        (5, 2, 0, 0), (6, 0, 0, 0),
    ])
    heap("heap-strings", [
        (1, 0, 4, 0), (0, 1, 7, 65), (0, 2, 0xD8, 0),
        (2, 0, 0, 1), (2, 0, 1, 2), (5, 1, 0, 0), (5, 2, 0, 0),
        (6, 0, 0, 0), (3, 0, 0, 255), (6, 0, 0, 0),
        (5, 0, 0, 0), (6, 0, 0, 0),
    ])
    heap("heap-nested-scopes", [
        (7, 0, 16, 32), (4, 1, 0, 0), (6, 0, 0, 0), (5, 0, 0, 0),
        (0, 0, 0, 0), (4, 3, 1, 0), (6, 0, 0, 0),
        (5, 1, 0, 0), (5, 3, 0, 0), (6, 0, 0, 0),
    ])
    heap("heap-boundaries", [
        (0, 0, 0, 0), (1, 1, 0, 0), (2, 1, 0, 0), (3, 1, 4, 128),
        (1, 2, 4, 0), (2, 2, 4, 0), (2, 2, 3, 15), (6, 0, 0, 0),
        (4, 0, 15, 0), (5, 1, 0, 0), (6, 0, 0, 0),
    ])

    # Full data descriptors: low three bits are the opcode, bits 3..5 W/E/C.
    objects("object-descriptors", [
        (56, 0, 7, 42), (24, 0, 8, 1), (24, 0, 8, 2),
        (16, 0, 8, 2), (1, 0, 8, 3), (2, 0, 8, 0),
        (56, 0, 8, 2), (0, 0, 9, 0), (5, 0, 0, 0),
        (4, 0, 0, 0), (56, 0, 10, 9), (1, 0, 7, 43),
        (2, 0, 7, 0), (56, 0, 7, 44),
    ])
    objects("object-key-order", [
        (56, 0, key, key) for key in reversed(range(16))
    ] + [
        (2, 0, 8, 0), (56, 0, 8, 33),
        (2, 0, 15, 0), (56, 0, 15, 34),
        (5, 0, 0, 0), (7, 0, 0, 0),
    ])
    objects("object-prototypes", [
        (48, 0, 7, 12), (3, 1, 0, 0), (1, 1, 7, 13),
        (56, 0, 8, 22), (1, 1, 8, 23), (3, 2, 1, 0),
        (3, 0, 2, 0), (4, 1, 0, 0), (3, 1, 0, 0),
        (3, 1, 3, 0), (5, 0, 0, 0), (7, 2, 8, 0),
    ])
    objects("object-utf16-symbols", [
        (62, 0, 13, 2), (62, 0, 14, 3), (62, 0, 15, 1),
        (3, 3, 0, 0), (5, 0, 0, 0), (1, 3, 14, 255),
        (2, 0, 15, 0), (62, 0, 15, 255), (5, 0, 0, 0),
    ])
    arrays("array-sparse-keys", [
        (56, 0, 6, 12), (56, 0, 7, 13), (56, 0, 8, 14),
        (56, 0, 9, 15), (2, 0, 6, 0), (6, 0, 0, 0),
        (8, 1, 10, 9), (56, 1, 6, 42), (3, 1, 0, 8),
        (1, 1, 7, 43), (6, 0, 0, 0),
    ])
    arrays("array-partial-shrink", [
        (56, 0, 0, 1), (24, 0, 2, 2), (56, 0, 3, 3),
        (56, 0, 4, 4), (19, 0, 0, 1), (1, 0, 1, 5),
        (1, 0, 3, 6), (2, 0, 2, 0), (6, 0, 0, 0),
    ])
    arrays("array-error-priority", [(3, 0, 0, 2), (4, 0, 0, 0)] + [
        command for payload in [11, 12, 13, 14, 15, 20, 21, 22, 23, 25, 28]
        for command in [(56, 0, 10, payload), (1, 0, 10, payload)]
    ] + [(11, 0, 0, 2), (2, 0, 10, 0), (6, 0, 0, 0)])
    arrays("array-length-inputs", [(3, 0, 0, payload) for payload in range(29)])
    arrays("array-nonextensible", [
        (56, 0, 0, 1), (5, 0, 0, 0), (3, 0, 0, 7),
        (1, 0, 0, 2), (1, 0, 1, 3), (1, 0, 9, 4),
        (4, 0, 0, 0), (3, 0, 0, 7), (6, 0, 0, 0),
    ])
    conversions("conversion-numbers", [(1, index, 0, 0) for index in range(25)] + [
        (kind, 255, 0, 255) for kind in range(7)
    ])
    conversions("conversion-exact-strings", [
        command for style in range(6)
        for command in [(7, 1, 128, style), (7, 0, 0, style)]
    ] + [(8, 255, 255, radix) for radix in range(3)] + [
        command for exponent in range(7)
        for command in [(11, 255, 127, exponent), (11, 1, 128, exponent)]
    ])
    conversions("conversion-errors", [(9, 0, index, 0) for index in range(18)] + [
        (9, 1, 0, 0), (9, 2, 0, 0), (6, 0, 0, 0), (6, 255, 255, 255),
    ])
    conversions("conversion-whitespace", [(10, index, 0, 0) for index in range(10)] + [
        (199, 0, 0, 2), (200, 255, 255, 2), (201, 1, 255, 255),
        (202, 3, 0, 0), (203, 1, 128, 6),
    ])
    conversions("conversion-ieee", [
        (0, index * 37 % 256, index * 131 % 256, 255 - index)
        for index in range(64)
    ])
    text_keys("text-key-numbers", [(3, index, 0, 0) for index in range(32)])
    text_keys("text-key-indexes", [(6, index, 0, 0) for index in range(21)])
    text_keys("text-key-primitives-symbols", [
        (0, 0, 0, 0), (1, 0, 0, 0), (2, 0, 0, 0), (2, 1, 0, 0),
        (8, 0, 0, 0), (8, 1, 0, 0), (8, 2, 255, 255), (8, 3, 0, 0),
        (3, 1, 0, 0), (6, 2, 0, 0),
    ])
    text_keys("text-key-dyadics", [
        (5, a, b, exponent)
        for exponent in range(9)
        for a, b in [(1, 0), (255, 255), (255, 127), (0, 128)]
    ])
    text_keys("text-key-utf16", [
        (7, 0xD8, 0, 1), (7, 0xDC, 0, 1), (7, 0, 0, 0), (7, 0, 0, 1),
        (7, 0xFE, 0xFF, 2), (7, 0xFF, 0xFF, 2), (7, 0xD8, 0, 16),
    ])
    text_keys("text-key-noncanonical", [
        (9, a, b, style)
        for style in range(8)
        for a, b in [(0, 0), (1, 0), (255, 255), (0, 16)]
    ])
    text_keys("text-key-max-commands", [
        (index, index * 37 % 256, index * 131 % 256, 255 - index)
        for index in range(64)
    ])


def execution_seeds():
    # Raw source also runs under a strict host quota; templates have oracles.
    write("js-execute", "source-loops", "for(;;); while(true){} do {} while(true);")
    write("js-execute", "source-properties", "var a=[1,,3]; let o={a:1,['b']:2};"
          "o.c=a[2]; delete o.a; 'b' in o; 0 instanceof o;")
    write("js-execute", "source-optional", "var o=null; o?.a?.[0] ?? 42;")
    write("js-execute", "source-unsupported", "function f(){} class A{} try { f(); } catch(e) {}")
    write("js-execute", "source-tagged-templates", "function t(s,...v){return s;} t`a${1}b\\u{`;"
          " var o={m(s){return this;}}; o.m`x`; t`a`===t`a`; (function(){return t`y`;})();")
    write("js-execute", "source-annex-b-labels-and-call-targets",
          "a: b: function f(){return g;} { l: function g(){} } f();"
          " try { f() = 1; } catch (e) {} try { for (f() of [1]) ; } catch (e) {}"
          " (function(){ eval('{ function h(){return h;} }'); return h(); })();")
    write("js-execute", "source-syntax-error", "let = ;")
    write("js-execute", "source-widths", "127;128;32768;70000;")
    write("js-execute", "source-constants", "".join(f"{value}.25;" for value in range(300)))
    write("js-execute", "source-utf16le-surrogate", struct.pack("<4H", 0x27, 0xD800, 0x27, 0x3B))
    write("js-execute", "source-functions", "function f(x){return x+1;} f(2);"
          "var g=function inner(v){if(v>0)return inner(v-1);return 0;};g(2);")
    write("js-execute", "source-lazy-loops", "function forever(){for(;;);}"
          "function recur(){return recur();}")
    write("js-execute", "source-function-unsupported", "function f(){return arguments;}"
          "function outer(x){return function(){return x;};}"
          "function defaults(a=1){return a;}"
          "function pending(){try{return 1;}catch(e){return 2;}}")
    write("js-execute", "source-function-calls", "var o={m(x,y){return x+y;},"
          "get v(){return 1;},set v(x){}};o.m(1,2);o?.m?.(3);o.v+=1;")
    for family in range(64):
        write("js-execute", f"template-{family:02}", bytes([family, 127, 128, 8]))
    write("js-execute", "loop-zero", bytes([1, 250, 3, 0]))
    write("js-execute", "loop-break", bytes([2, 8, 3, 8]))
    write("js-execute", "loop-continue", bytes([2, 1, 8, 8]))
    write("js-execute", "unicode-pair", bytes([10, 0xD8, 0, 1]))
    write("js-execute", "unicode-low", bytes([12, 0xDC, 0, 0]))
    write("js-execute", "unicode-high", bytes([12, 0xD8, 0, 0]))
    write("js-execute", "function-recursion-zero", bytes([25, 123, 45, 0]))
    write("js-execute", "function-recursion-eight", bytes([25, 128, 255, 8]))
    write("js-execute", "function-this-absent", bytes([21, 0, 0, 0]))
    write("js-execute", "function-optional-absent", bytes([22, 5, 0, 0]))
    write("js-execute", "function-optional-present", bytes([22, 5, 1, 0]))
    write("js-execute", "function-arguments", bytes([24, 128, 127, 8]))
    write("js-execute", "function-utf16", bytes([26, 0xD8, 0, 1]))
    write("js-execute", "function-callback", bytes([27, 255, 1, 8]))
    write("js-execute", "closure-loop-zero", bytes([37, 250, 7, 0]))
    write("js-execute", "closure-loop-let", bytes([37, 250, 7, 8]))
    write("js-execute", "closure-loop-var", bytes([38, 250, 7, 8]))
    write("js-execute", "closure-loop-const", bytes([39, 250, 7, 8]))
    write("js-execute", "closure-utf16", bytes([62, 0xD8, 0, 0]))
    write("js-execute", "arguments-mapping", bytes([46, 5, 9, 3]))
    write("js-execute", "arguments-deletion", bytes([48, 5, 9, 3]))
    write("js-execute", "default-environment", bytes([52, 5, 9, 3]))
    write("js-execute", "default-tdz", bytes([53, 5, 9, 3]))
    write("js-execute", "source-closures-parameters", "function make(p=1){"
          "return (q,...rest)=>{p+=q;return arguments[0]+p+rest.length;};}"
          "var f=make(2); f(3,4); f(5);")
    write("js-execute", "source-eval-direct", "var g=1; function f(a){ eval('var v=a+g');"
          "{ let b=2; eval('b'); } return v; } f(3); eval('function h(){} {function k(){}}');")
    write("js-execute", "source-eval-conflict", "function f(){ let x; { eval('var x'); } }"
          "try { f(); } catch (e) {} (0,eval)('var y=1'); delete y;")
    write("js-execute", "source-eval-strict", "'use strict'; var s='var x=1; x';"
          "eval(s); eval?.(s); new Function('a','return a')(1);")
    write("js-execute", "source-eval-parameters", "function f(a=eval('var z=2'), b=z)"
          "{ eval('var w'); return typeof w + b; } f();")


def main():
    execution_seeds()
    runtime_seeds()
    write("webidl", "interface", "[Exposed=Window] interface Example { readonly attribute DOMString name; undefined run(optional long x=0); };")
    write("webidl", "types", "dictionary Options { required DOMString name; sequence<long> values; }; typedef (DOMString or long) Choice; callback Handler = undefined (Choice value);")
    write("webidl", "numbers", "0 08 0129 -0Xff 0x 08.5 -.5 .5E-2 1e+ 1... ....5 -Infinity NaN")
    write("webidl", "comments-strings", '/* a /* b */ tail // CR\rstill\u2028here\n"raw\\"next" "multi\nline"')
    write("webidl", "unicode", "\u000c\u00a0\ufeffé🦀 _interface __x -name a-b")
    write("webidl", "unfinished-comments", "/*x" * 1024)
    write("webidl", "invalid-utf8", bytes([0xff, 0xc0, 0xaf, 0, 0xe2, 0x28, 0xa1]))
    write("webidl", "parser-definitions", 'interface mixin M { undefined f(); }; interface X { constructor(); }; X includes M; namespace N { readonly attribute long n; }; enum E {"a","b"};')
    write("webidl", "parser-extended-attributes", '[Exposed=(Window,Worker), Named=(A(B,C))] dictionary D { required DOMString x; sequence<long> values = []; };')
    write("webidl", "parser-deep-types", "typedef " + "sequence<" * 200 + "long" + ">" * 200 + " Deep;")
    write("webidl", "parser-many-members", "interface X {" + "attribute long n;" * 200 + "};")
    write("webidl", "parser-invalid", 'partial interface X { constructor(); }; typedef sequence<> Bad;')
    write("webidl", "index-forward-partials", 'partial interface X { attribute Options options; }; X includes M; partial interface mixin M { undefined run(X value); }; interface X : Base {}; interface Base {}; interface mixin M {}; dictionary Options {};')
    write("webidl", "index-inheritance-cycle", 'interface A : B {}; interface B : A {};')
    write("webidl", "index-typedef-cycle", 'typedef sequence<B> A; typedef FrozenArray<A> B;')
    write("webidl", "index-escaped-names", 'interface _interface {}; partial interface _interface { attribute _interface value; }; interface mixin _mixin {}; _interface includes _mixin;')
    write("webidl", "index-duplicate-definition", 'interface Example {}; dictionary _Example {};')
    write("webidl", "index-unknown-type", 'interface Example { undefined run(Missing value); };')
    write("webidl", "members-valid", 'interface X { constructor(); undefined run(optional long count = 0, DOMString... rest); }; dictionary D { required long value; }; enum E {"on","off"};')
    write("webidl", "members-dictionary-duplicate", 'dictionary A { long value; }; dictionary B : A { long _value; };')
    write("webidl", "members-mixin-collision", 'interface X { attribute long value; }; interface mixin M { undefined value(); }; X includes M;')
    write("webidl", "members-argument-duplicate", 'callback F = undefined (long value, DOMString _value);')
    write("webidl", "members-enum-duplicate", 'enum E {"on","on"};')
    write("webidl", "members-partial-overload", 'interface X { undefined run(); }; partial interface X { undefined run(long value); };')
    write("webidl", "members-variadic-not-last", 'interface X { undefined run(long... values, long other); };')
    write("webidl", "members-reserved-name", 'interface X { static attribute long prototype; };')
    write("webidl", "types-annotated-generics", 'interface Item {}; typedef [Tag] FrozenArray<sequence<[Element] Item?>> Values; interface X { readonly attribute Values values; };')
    write("webidl", "types-attribute-alias", 'typedef sequence<long> Values; interface X { attribute Values value; };')
    write("webidl", "types-nullable-alias", 'typedef long? Maybe; typedef Maybe? Invalid;')
    write("webidl", "types-union-nullable", 'typedef (long? or DOMString?) Invalid;')
    write("webidl", "types-constant-alias", 'typedef DOMString Text; interface X { const Text invalid = 1; };')
    write("webidl", "types-nullable-dictionary", 'dictionary D {}; callback F = undefined (D? value);')
    write("webidl", "types-promise-attribute", 'typedef Promise<long> P; interface X { attribute P value; };')
    write("webidl", "types-union-any-alias", 'typedef any A; typedef (A or DOMString) Invalid;')



    write("html", "document", "<!doctype html><table><tr><td>x</table><template><p>y</template>")
    write("html", "text-modes", "<textarea>&amp;</textarea><script>'<&'</script><svg><title>x</title></svg>")
    write("css", "stylesheet", "@layer a { div:nth-child(2n+1) { color: red; width: 10px } }")
    write("css", "selector", ":is(div, span) > a[href^='https:']:not(.hidden)")
    write("url", "absolute", "https://user:pass@example.test:443/a/../b?q=%FF#part")
    write("url", "relative", "../%2e%2e/c?x=a+b&y=%E2%82%AC")
    write("xml", "document", "<?xml version='1.0'?><a xmlns='urn:test'><b x='&amp;'/></a>")
    write("xml", "entity", "<!DOCTYPE a [<!ENTITY x 'text'>]><a>&x;</a>")
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(b"\0\xff\0\0\xff")) + chunk(b"IEND", b"")
    write("png", "red-pixel", png)
    write("png", "truncated", png[:20])
    # Self-authored 1x1 GIF: palette black/white, LZW Clear/black/End.
    gif = b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff"
    gif += b",\0\0\0\0\x01\0\x01\0\0\x02\x02\x44\x01\0;"
    write("gif", "black-pixel", gif)
    write("gif", "truncated", gif[:20])
    write("deflate", "zlib", zlib.compress(b"abc" * 20))
    compressor = zlib.compressobj(wbits=-15)
    write("deflate", "raw", compressor.compress(b"abc" * 20) + compressor.flush())
    write("font", "synthetic-sfnt", font())
    write("font", "truncated-collection", b"ttcf\0\x01\0\0\0\0\0\x01")
    write("json", "document", '{"a":[null,true,false,-1.5e2,"\\uD834\\uDD1E"]}')
    write("json", "truncated", '{"a":[1,')
    for selector in range(40):
        write("encoding", f"encoding-{selector:02}", bytes([selector]) + b"a\xe2\x82\xac\xff\0\x1b$B")
    write("unicode", "combining", "e\u0301 A\u030a\r\n\U0001f600")
    write("unicode", "idna", "xn--bcher-kva.example")
    write("js", "script", '"use strict";\n'
          "class A extends Object { #x = 1; static #y; static { this.z = 2; } get x() { return this.#x; }"
          " set x(v) { this.#x = v; } static m() { return #y in A; } }\n"
          "async function* g(a, [b, c] = [], {d, ...e} = {}) { for await (const x of a) yield* x;"
          " await using r = null; }\n"
          "label: for (let i = 0, j; i < 10; i++) { if (i % 2) continue label; else break label; }\n"
          "const f = async (x = 1, ...y) => ({ x, y, [`k${x}`]: /a[b-c]+/giu.test?.(`t${y}`) ?? 0n, n: 0x1_0 + .5e-3 });\n"
          'try { throw new Error("x"); } catch ({ message }) {} finally { switch (f) { case 1: default: } }\n')
    write("js", "module", 'import def, * as ns from "./a.js" with { type: "json" };\n'
          'import { a as b, "c d" as e } from "./b.js";\n'
          'export { b as other, e as "f g" };\n'
          'export * as all from "./c.js";\n'
          "export default class { static async *[Symbol.iterator]() {} }\n"
          'export const h = await import("./d.js", { with: { type: "json" } });\n'
          "using u = null;\n"
          "await using v = null;\n"
          "export let i = import.meta.url;\n")
    write("js", "annex-b", "<!-- comment\n"
          "var x = 010 + 08; with (Math) { if (x) function f() {} else ; }\n"
          "l: function g() {}\n"
          "--> comment\n"
          "a = { __proto__: null, __proto__() {} };\n"
          "for (var k = 0 in {}) ;\n")
    write("js", "early-error", '"use strict"; function f(a, a) {}')
    write("js", "deep", "(" * 300 + "a" + ")" * 300)
    write("js", "utf16le-surrogate", struct.pack("<8H", *b"x = '", 0xD800, *b"';"))
    write("js", "regexp", r"(?<a>x)|(?<a>[\p{L}--\q{ab|c}])\k<a>{2,3}?(?i-m:\u{1F600}.)(?<=\b[^\d])\1")

    def regexp(name, flags, pattern, subject):
        # Flags as bits in the order dgimsuvy, then pattern, NUL, subject (src/regexp.rs).
        bits = sum(1 << "dgimsuvy".index(flag) for flag in flags)
        write("regexp", name, bytes([bits]) + pattern.encode() + b"\0" + subject.encode())
    regexp("groups", "dg", r"(?<x>a)|(b)\k<x>", "xaby")
    regexp("unicode-case", "iu", r"\u{1F600}|[^\p{Lu}]", "A\U0001f600b")
    regexp("class-sets", "v", r"[\p{L}--[a-z]]+|\q{ab|c}", "abc\u00c4\u00d6")
    regexp("lookaround", "y", r"(?<=a)b\1(c)?(?!d)", "abc")
    regexp("quantifiers", "s", r"(a|ab)*?c{2,3}.$", "ababccc\n")
    regexp("multiline", "gm", r"^\w+\b|\B$", "one\ntwo\r\nthree")
    regexp("modifiers", "", r"(?i:a)(?-i:B)", "AB")
    regexp("invalid", "", r"(?<a>x)(?<a>y)", "xy")
    regexp("backtracking", "", r"(a*)*b", "a" * 30)

    def bigint(name, control, a, b):
        # Signs in bits 0 and 1, shift in bits 2 to 7, the length of a as two bytes, low first,
        # then a and b (src/bigint.rs).
        write("bigint", name, bytes([control]) + struct.pack("<H", len(a)) + a + b)
    bigint("small", 1 | 5 << 2, b"\x12\x34", b"\x05")
    bigint("zero-divisor", 2 | 3 << 2, b"\xff" * 9, b"")
    bigint("long-division", 1 << 2, b"\x80" + b"\x00" * 16, b"\xff" * 9)
    bigint("signs", 3 | 63 << 2, b"\xff" * 17, b"\x80" * 9)
    bigint("near-number", 1, b"\x20\x00\x00\x00\x00\x00\x01", b"\x01")

    def date(name, control, parameter, time, clock, text):
        # Zone, raw time and raw clock in the control byte, the zone parameter as two bytes,
        # time and clock as eight bytes each, low first, then the text (src/date.rs).
        time = struct.pack("<d", time) if control & 4 else struct.pack("<q", time)
        clock = struct.pack("<d", clock) if control & 8 else struct.pack("<q", clock)
        write("date", name, bytes([control]) + struct.pack("<H", parameter) + time + clock + text)
    now = 1577977445123
    date("iso", 0, 0, 0, now, b"2020-01-02T15:04:05.123Z")
    date("local-text", 1, 117, now, now, b"Thu Jan 02 2020 20:34:05 GMT+0530 (India)")
    date("legacy", 2, 2 * 1439, -1, now, b"1/2/2020, 3:04:05 PM")
    date("edges", 1, 0, -8640000000000000, now, b"+275760-09-13T00:00:00.000Z")
    date("identifier", 3, 0, 253402300799999, now, b"-23:59:59.999999999")
    date("raw", 4 | 8, 0, -0.0, float("nan"), b"Sat, 13 Sep 275760 00:00:00 GMT")

    def arraybuffer(name, length, extra, resizable, steps):
        # Length, extra maximum and resizability, then eleven bytes per step: the step (bits 3 and
        # 4 choose the kind of Number), the element type (bit 7 for little-endian), an offset and
        # eight value bytes, low first (src/arraybuffer.rs).
        data = bytes([length, extra, resizable])
        for op, kind, offset, value in steps:
            data += bytes([op, kind, offset]) + value
        write("arraybuffer", name, data)
    pi = struct.pack("<d", 3.141592653589793)
    arraybuffer("every-type", 16, 0, 0, [(0, kind, 3, struct.pack("<q", -5 << 4)) for kind in range(11)]
                + [(1, kind | 128, 2, bytes(8)) for kind in range(11)])
    arraybuffer("floats", 8, 0, 0, [(1 << 3, 8 | 128, 0, pi), (3 << 3, 8, 2, bytes([5]) + bytes(7)),
                                    (3 << 3, 9, 4, bytes([10]) + bytes(7)), (1 << 3, 10, 0, pi)])
    arraybuffer("resize", 4, 20, 1, [(5, 0, 2, b"\x41" * 8), (2, 0, 22, bytes(8)), (6, 0, 3, b"\x41" * 8),
                                     (2, 0, 1, bytes(8)), (4, 1, 22, b"\x41" * 8)])
    arraybuffer("transfer", 8, 8, 1, [(3, 129, 9, b"\x40" * 8), (3, 1, 30, b"\x40" * 8),
                                      (3, 0, 0, b"\x41" * 8), (0, 0, 0, bytes(8)), (7, 128, 7, b"\x41" * 8)])
    arraybuffer("bigints", 8, 0, 0, [(0, 6, 0, struct.pack("<q", -1)), (0, 7 | 128, 0, struct.pack("<q", 1 << 62)),
                                     (0, 6 + 11 * 10, 0, struct.pack("<q", -(1 << 63)))])
    typed = 0x20  # bit 5 of the step: the steps with typed arrays
    minus = struct.pack("<q", -5 << 4)
    arraybuffer("typed-arrays", 16, 8, 1, [
        step for kind in (0, 5, 6, 8, 10) for step in [
            (typed, kind | 128, 8, bytes([3]) * 8), (typed | 1, kind, 1, minus), (typed | 2, kind, 12, bytes([25]) * 8),
            (typed | 3, kind, 11, bytes([9]) * 8), (typed | 6, 1, 0, bytes(8)), (typed | 7, 9, 1, bytes([0x1b]) * 8),
        ]])
    arraybuffer("typed-resize", 16, 8, 1, [
        (typed, 133, 4, bytes([3]) * 8), (typed | 1, 1, 0, struct.pack("<q", 300 << 4)), (2, 0, 2, bytes(8)),
        (typed | 2, 1, 0, bytes(8)), (typed | 4, 1, 0, bytes(8)), (2, 0, 20, bytes(8)), (typed | 4, 0, 30, bytes(8)),
        (typed | 5, 1, 9, bytes([15]) * 8), (3, 0, 0, b"\x41" * 8), (typed, 2, 0, bytes([3]) * 8)])
    def weak(name, steps):
        # Four bytes per step: the kind (modulo 12, 11 ends a round), a slot, a second slot or a
        # WeakRef, and switches (src/weak.rs).
        write("weak", name, b"".join(bytes(step) for step in steps))
    weak("keys-and-refs", [(0, 0, 0, 0), (0, 1, 0, 0), (3, 0, 1, 0), (4, 1, 0, 8), (7, 0, 0, 0), (7, 1, 1, 0),
                           (9, 1, 2, 16 | 8), (11, 0, 0, 0), (2, 0, 0, 0), (2, 1, 0, 0), (11, 0, 0, 0),
                           (8, 2, 0, 0), (8, 3, 1, 0)])
    weak("ephemeron-chain", [(0, 0, 0, 0), (0, 1, 0, 0), (1, 2, 0, 0), (4, 0, 1, 8), (4, 1, 2, 9), (7, 1, 0, 0),
                             (7, 2, 1, 0), (9, 2, 0, 1), (2, 1, 0, 0), (2, 2, 0, 0), (11, 0, 0, 0), (8, 3, 0, 0),
                             (8, 4, 1, 0), (2, 0, 0, 0), (11, 0, 0, 0), (8, 5, 0, 0), (2, 3, 0, 0), (2, 4, 0, 0),
                             (2, 5, 0, 0)])
    weak("registries", [step for slot in range(8) for step in [(slot % 2, slot, 0, 0), (9, slot, slot * 8 + 7, 16 | slot)]]
         + [(10, 1, 0, 0), (10, 3, 0, 1), (2, 0, 0, 0), (2, 2, 0, 0), (2, 5, 0, 0), (11, 0, 0, 0),
            (0, 0, 0, 0), (9, 0, 1, 8), (2, 1, 0, 0), (11, 0, 0, 0), (2, 0, 0, 0)])
    weak("sets-and-errors", [(4, 0, 0, 0), (7, 0, 0, 0), (9, 0, 0, 0), (10, 0, 0, 0), (0, 0, 0, 0), (9, 0, 0, 8),
                             (6, 0, 0, 0), (6, 0, 0, 0), (1, 1, 0, 0), (6, 1, 0, 0), (6, 0, 0, 1), (5, 0, 0, 0),
                             (4, 1, 0, 9), (5, 1, 0, 1), (7, 1, 3, 0), (2, 1, 0, 0), (11, 0, 0, 0), (8, 2, 3, 0)])
    def atomics(name, header, steps):
        # Lengths, extra maximum and growth, then twelve bytes per step: the operation (modulo 13),
        # the element type with switches, an index, switches for the value and eight value bytes,
        # low first (src/atomics.rs).
        data = bytes(header)
        for op, kind, position, flags, value in steps:
            data += bytes([op, kind, position, flags]) + struct.pack("<q", value)
        write("atomics", name, data)
    atomics("read-modify-write", (3, 2, 1), [(op, 4, 1, 0, 5 + op) for op in range(9)]
            + [(0, 6, 1, 1, -5), (0, 6, 1, 0, -5), (0, 0x80, 0, 0, 1), (0, 4, 11, 0, 1), (7, 4, 0x82, 0, 0)])
    atomics("every-type", (4, 0, 0), [(index % 9, index // 6, 3, (0, 2, 4, 6, 0x11, 0xF0)[index % 6],
                                       -0x123456789ABCDEF1) for index in range(48)])
    atomics("grow-and-slice", (1, 4, 1), [(9, 0, 20, 0, 0), (8, 1, 19, 0, 7), (9, 0, 12, 0, 0), (9, 0x40, 30, 0, 0),
                                          (12, 0, 4, 0, 30), (12, 0, 0x2f, 8, 0), (12, 0x40, 4, 8, 0),
                                          (9, 0, 40, 0, 0), (0, 6, 4, 1, 1 << 62)])
    atomics("wait-and-notify", (2, 0, 0), [(10, 4 | 0x10, 2, 8, 0), (10, 6 | 0x20, 1, 8, 0), (10, 4, 2, 8, 0),
                                           (10, 4 | 0x30, 2, 8, 0), (10, 4 | 0x10, 3, 0, 1), (10, 0x44, 0, 8, 0),
                                           (11, 4, 2, 0x10, 0), (11, 6, 1, 8, 0), (11, 0x44, 2, 8, 0),
                                           (11, 2, 0, 8, 0), (10, 4 | 0x10, 0x87, 8, 0), (11, 4, 0x85, 0x40, 0)])
    def sha256(name, control, message):
        # The first byte splits the message for update(): bits 0-1 the kind, bits 2-7 a fixed
        # length (src/sha256.rs).
        write("sha256", name, bytes([control]) + message)
    sha256("empty", 0, b"")
    sha256("abc", 3, b"abc")
    sha256("padding-edge", 1 | 54 << 2, bytes(range(56)))
    sha256("two-blocks", 2, b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq" * 3)
    sha256("blocks", 1 | 63 << 2, bytes(index * 37 % 256 for index in range(1000)))
    def code_cache(name, source, changes=b""):
        # Bit 0 of the first byte: 0 compares fresh, stored and loaded runs; 1 changes the stored
        # entry after a zero byte, three bytes per change (src/code_cache.rs).
        if changes:
            write("code-cache", name, b"\x01" + source.encode() + b"\x00" + changes)
        else:
            write("code-cache", name, b"\x00" + source.encode())
    code_cache("functions", "function f(a, b = 2, ...c) { return a + b + c.length; } f(1)")
    code_cache("closures", "function outer() { let n = 0; return () => ++n; } var g = outer(); g(); g()")
    code_cache("classes", "class A { #x = 1; static s = 2; get x() { return this.#x; } static { A.t = 3; } }"
               " class B extends A { constructor() { super(); } } class C extends B {} new C().x + String(C)")
    code_cache("generators", "function* g() { yield 1; yield* [2, 3]; } async function f() { await 1; }"
               " async function* h() { yield 1; } [...g()].length")
    code_cache("eval-and-with", "var x = 1; function f() { var y = 2; return eval('x + y'); }"
               " with ({ z: 3 }) { f() + z }")
    code_cache("literals", "[1.5, 123n ** 3n, 'text', /a(b)?c/gu.exec('abc'), `t${1}`].length")
    code_cache("tail-calls", "'use strict'; function t(n) { return n ? t(n - 1) : 0; } t(100)")
    code_cache("tagged-templates", "function t(s, ...v) { return s.raw.join() + v.length; }"
               " var o = { m(s) { return this === o; } }; function f() { return t`x`; }"
               " [t`a${1}b\\u{`, o.m`y`, f() === f(), String.raw`\\n${2}`].join()")
    code_cache("annex-b-labels-and-call-targets", "l: function f() { return 1; }"
               " var r = []; { m: function g() { return 2; } r.push(g()); }"
               " try { f()++; } catch (e) { r.push(e.name); } r.push(f(), typeof g); r.join()")
    code_cache("changed-constants", "var g = function (a) { return [a, typeof a]; }; g(1)",
               bytes([20, 0, 0, 40, 0, 0xFF, 60, 0, 1]))
    code_cache("changed-code", "function f(a) { return a + 1; } f(1)", bytes([5, 0, 0x7F, 9, 0, 0x80]))
    def pre_check(name, control, source):
        # Bit 0 module, bit 1 UTF-16LE, bits 2-3 smallest body, bits 4-6 deepest body, bit 7
        # keeps `(function` whole (src/pre_check.rs); 0x70 pre-checks every body that can be.
        data = source.encode("utf-16-le", "surrogatepass") if control & 2 else source.encode()
        write("js-pre-check", name, bytes([control]) + data)
    pre_check("closures", 0x70, "function f(a, b = 2) { var c = a + b; return function g() { return c; }; }"
              " var h = (x) => { return () => this.y + x; };")
    pre_check("classes", 0x70, "class A extends Object { #p = 1; constructor() { super(); }"
              " m() { return () => super.m + this.#p; } static { var s = () => { return this; }; } }")
    pre_check("eval-and-with", 0x70, "function f(a) { return () => { return eval('a') + arguments[0]; }; }"
              " function g(o) { with (o) { return function () { return x; }; } }")
    pre_check("generators", 0x70, "function* g() { yield function* () { yield 1; }; }"
              " async function* h() { for await (const x of y) { yield () => x; } }")
    pre_check("early-error", 0x70, "function f() { 'use strict'; with (a) {} }")
    pre_check("spread-after-body", 0x70, "function f() { [...(a), ]; } [...b] = c;")
    pre_check("module", 0x71, "import { a } from 'x'; export default function () { return () => a + import.meta.url; }"
              " await 1; export async function h() { await 2; }")
    pre_check("thresholds", 0x24, "function f() { return 1; } function g() { return [[[[[[[[1]]]]]]]]; }"
              " function h() { var a = 1, b = 2, c = 3; return function () { return a + b + c; }; }")
    pre_check("utf16le", 0x72, "function f() { return '\ud800'; }")
    pre_check("immediate", 0xF0, "(function () { var a = 1; return function () { return a; }; })();"
              " !function () { return this; }();")
    part = b"\x1f\x1f"  # PART_SEPARATOR in src/lib.rs
    write("page", "document", "<!doctype html><style>@media (max-width: 400px) { p { margin: 0 } }"
          " p { border: 2px dashed red; padding: 4px; font-style: italic }"
          " div { overflow: scroll; height: 20px }</style><p>Text <b>bold</b></p><div>x<br>y</div>")
    write("page", "resources", b"<!doctype html><link rel=stylesheet href=1.css><p>Text</p><img src=2.png>"
          + part + b'@import "3.css"; img { width: 10px; height: 10px }' + part + png
          + part + b"p { background: rgb(0, 128, 0) }")
    write("page", "xhtml", '<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml">'
          "<head><style>p { color: blue }</style></head><body><p>x</p></body></html>")
    write("page", "charset", b"<meta charset=windows-1252><link rel=stylesheet href=1><p>\xe9</p>"
          + part + b'p::after { content: "\xfc" }')


if __name__ == "__main__":
    main()
