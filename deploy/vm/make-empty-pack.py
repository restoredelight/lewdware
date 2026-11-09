#!/usr/bin/env python3
"""Create a media-free .lwpack for dialog and session lifecycle UI tests.

Uses the current runtime schema by default; older CI packages can use an
explicit v0 source checkout. Requires Python 3.11+ (sqlite3.serialize).
This fixture cannot exercise image, video, audio, or wallpaper rendering.
"""

import argparse
from pathlib import Path
import sqlite3
import struct
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--legacy-v0-source", type=Path,
                        help="Generate a v0 pack using an older checkout's schemas (for older CI packages)")
    args = parser.parse_args()
    root = args.legacy_v0_source or Path(__file__).resolve().parents[2]
    legacy = args.legacy_v0_source is not None
    if legacy and "pub const VERSION: u8 = 0;" not in (root / "shared/src/pack.rs").read_text():
        parser.error("legacy source must use pack format version 0")

    with sqlite3.connect(":memory:") as db:
        for name in ("0001_init_schema.sql", "behaviour_schema.sql"):
            db.executescript((root / "shared/src/migrations" / name).read_text())
        if legacy:
            db.executescript((root / "shared/src/migrations/behaviour_video_volume.sql").read_text())
        db.execute("CREATE TABLE migrations (migration_index INTEGER NOT NULL)")
        db.execute("INSERT INTO migrations VALUES (?)", (2 if legacy else 1,))
        if not legacy:
            db.execute(
                "INSERT INTO pack_metadata "
                "(singleton, name, creator, description, version) VALUES (1, ?, ?, ?, ?)",
                ("VM smoke test", "Lewdware", "Empty synthetic pack for dialog and lifecycle testing", "1"),
            )
        db.commit()
        index = db.serialize()

    # v0 stores CBOR metadata separately. This is the fixed map
    # {"name": "VM smoke test"}; all other metadata fields are optional.
    metadata = b"\xa1\x64name\x6dVM smoke test" if legacy else b""
    if legacy:
        header = struct.pack("<6sBBQQQQ16s8x", b"LWPACK", 0, 0, 64,
                             len(index), 64 + len(index), len(metadata), uuid.uuid4().bytes)
    else:
        header = struct.pack(
            "<6sBBQQ16s24x", b"LWPACK", 1, 0, 64, len(index), uuid.uuid4().bytes
        )
    # Avoid accidentally replacing an existing pack.
    with args.output.open("xb") as pack:
        pack.write(header)
        pack.write(index)
        pack.write(metadata)
    print(args.output)


if __name__ == "__main__":
    main()
