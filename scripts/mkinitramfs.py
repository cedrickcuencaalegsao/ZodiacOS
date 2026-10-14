#!/usr/bin/env python3
"""Packs a directory into a ZodiacOS initramfs image: one 512-byte header sector
(magic "ZINI" + archive length) followed by a cpio "newc" archive.

usage: mkinitramfs.py <source_dir> <output_file>
"""
import os
import struct
import sys


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    src, out = sys.argv[1], sys.argv[2]
    if not os.path.isdir(src):
        sys.exit(f"mkinitramfs: '{src}' is not a directory (create it and add files)")

    archive = bytearray()
    counter = [0]

    def add(name, mode, data=b""):
        counter[0] += 1
        fields = (counter[0], mode, 0, 0, 1, 0, len(data), 0, 0, 0, 0, len(name) + 1, 0)
        header = "070701" + "".join("%08X" % v for v in fields)
        entry = header.encode() + name.encode() + b"\0"
        entry += b"\0" * (-len(entry) % 4)
        entry += data
        entry += b"\0" * (-len(data) % 4)
        archive.extend(entry)

    for root, dirs, files in os.walk(src):
        dirs[:] = sorted(d for d in dirs if not d.startswith("."))
        rel = os.path.relpath(root, src)
        if rel != ".":
            add(rel.replace(os.sep, "/"), 0o040755)
        for name in sorted(f for f in files if not f.startswith(".")):
            path = os.path.join(root, name)
            with open(path, "rb") as f:
                data = f.read()
            full = name if rel == "." else rel.replace(os.sep, "/") + "/" + name
            add(full, 0o100000 | (os.stat(path).st_mode & 0o777), data)
    add("TRAILER!!!", 0)

    blob = bytearray(b"ZINI" + struct.pack("<I", len(archive)))
    blob.extend(b"\0" * (512 - len(blob)))
    blob.extend(archive)
    blob.extend(b"\0" * (-len(blob) % 512))
    with open(out, "wb") as f:
        f.write(blob)
    print(f"mkinitramfs: {counter[0] - 1} entries, {len(archive)} bytes -> {out}")


if __name__ == "__main__":
    main()