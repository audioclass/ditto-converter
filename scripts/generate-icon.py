#!/usr/bin/env python3
"""Generate the original Ditto Converter icon; no external artwork or fonts.

Geometry: two offset rounded loop rings, cream/red on graphite, drawn as pixels.
The generated asset and construction are MIT licensed with the project.
"""
from pathlib import Path
import struct
import zlib


def png(size=256):
    rows = bytearray()
    for y in range(size):
        rows.append(0)
        for x in range(size):
            color = (35, 35, 35, 255)
            for cx, cy, radius, tint in [(104, 128, 62, (236, 227, 205, 255)),
                                          (152, 128, 62, (204, 59, 51, 255))]:
                distance = (x - cx) ** 2 + (y - cy) ** 2
                if (radius - 13) ** 2 <= distance <= radius ** 2:
                    color = tint
            rows.extend(color)
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(bytes(rows), 9)) + chunk(b'IEND', b''))


if __name__ == '__main__':
    destination = Path(__file__).resolve().parents[1] / 'src-tauri/icons/icon.png'
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(png())
    print('Generated original MIT-licensed 256px RGBA icon')
