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


def execution_seeds():
    # Raw source also runs under a strict host quota; templates have oracles.
    write("js-execute", "source-loops", "for(;;); while(true){} do {} while(true);")
    write("js-execute", "source-properties", "var a=[1,,3]; let o={a:1,['b']:2};"
          "o.c=a[2]; delete o.a; 'b' in o; 0 instanceof o;")
    write("js-execute", "source-optional", "var o=null; o?.a?.[0] ?? 42;")
    write("js-execute", "source-unsupported", "function f(){} class A{} try { f(); } catch(e) {}")
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


def main():
    execution_seeds()
    runtime_seeds()
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
