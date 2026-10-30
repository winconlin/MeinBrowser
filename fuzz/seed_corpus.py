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
    # Printable ASCII maps to glyph 1, then the final segment for U+FFFF.
    cmap += struct.pack(">16H", 4, 32, 0, 4, 4, 1, 0, 0x7E, 0xFFFF, 0, 0x20, 0xFFFF, 0xFFE1, 1, 0, 0)
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


def main():
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
