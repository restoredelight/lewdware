#!/usr/bin/env python3
"""Build a deterministic, synthetic wallpaper fixture using an exact source schema.

Usage: make-wallpaper-pack.py OUTPUT_DIRECTORY --source SOURCE_CHECKOUT
Supports runtime pack formats 0 and 1. No external Python dependencies.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sqlite3
import struct
import uuid
import zlib


def png(label, width, height, color):
    # Large block letters and distinct corners survive desktop cropping.
    glyph = {"A": [14, 17, 17, 31, 17, 17, 17],
             "B": [30, 17, 17, 30, 17, 17, 30]}[label]
    rows = []
    for y in range(height):
        row = bytearray([0])
        for x in range(width):
            rgb = color
            gx, gy = (x - width // 2 + 100) // 40, (y - height // 2 + 140) // 40
            if 0 <= gx < 5 and 0 <= gy < 7 and glyph[gy] & (1 << (4 - gx)):
                rgb = (255, 255, 255)
            if x < 80 and y < 80:
                rgb = (255, 0, 0)
            if x >= width - 80 and y < 80:
                rgb = (0, 255, 0)
            if x < 80 and y >= height - 80:
                rgb = (0, 0, 255)
            if x >= width - 80 and y >= height - 80:
                rgb = (255, 255, 0)
            row.extend(rgb)
        rows.append(row)
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(b''.join(rows), 9)) + chunk(b'IEND', b''))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('output', type=Path)
    p.add_argument('--source', type=Path, required=True)
    a = p.parse_args()
    legacy = 'pub const VERSION: u8 = 0;' in (a.source / 'shared/src/pack.rs').read_text()
    a.output.mkdir(parents=True, exist_ok=False)
    images = [('A', 960, 600, (20, 80, 140)), ('B', 600, 960, (140, 40, 100))]
    payload = bytearray()
    with sqlite3.connect(':memory:') as db:
        for schema in ['0001_init_schema.sql', 'behaviour_schema.sql']:
            db.executescript((a.source / 'shared/src/migrations' / schema).read_text())
        if legacy:
            db.executescript((a.source / 'shared/src/migrations/behaviour_video_volume.sql').read_text())
        db.execute('CREATE TABLE migrations (migration_index INTEGER NOT NULL)')
        db.execute('INSERT INTO migrations VALUES (?)', (2 if legacy else 1,))
        if not legacy:
            db.execute("INSERT INTO pack_metadata (singleton,name,version) VALUES (1,'Wallpaper fixture','1')")
        for label, w, h, color in images:
            data = png(label, w, h, color)
            name = label + '.png'
            (a.output / name).write_bytes(data)
            db.execute('INSERT INTO media (file_name,file_type,offset,length,width,height,transparent,hash) VALUES (?,\'image\',?,?,?,?,0,?)',
                       (name, 64 + len(payload), len(data), w, h, hashlib.sha256(data).digest()))
            payload.extend(data)
        db.commit()
        index = db.serialize()
    identity = uuid.UUID('b10936d7-e41e-401c-a852-ef27a840014d').bytes
    if legacy:
        metadata = b'\xa1\x64name\x71Wallpaper fixture'
        header = struct.pack('<6sBBQQQQ16s8x', b'LWPACK', 0, 0, 64 + len(payload), len(index), 64 + len(payload) + len(index), len(metadata), identity)
    else:
        metadata = b''
        header = struct.pack('<6sBBQQ16s24x', b'LWPACK', 1, 0, 64 + len(payload), len(index), identity)
    (a.output / 'wallpaper.lwpack').write_bytes(header + payload + index + metadata)
    manifest = {f.name: hashlib.sha256(f.read_bytes()).hexdigest() for f in sorted(a.output.iterdir())}
    (a.output / 'sha256.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(manifest, indent=2))


if __name__ == '__main__':
    main()
