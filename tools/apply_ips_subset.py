#!/usr/bin/env python3
"""Apply a subset of an IPS patch's records (filtered by file offset range) to a ROM.

Usage:
    python3 tools/apply_ips_subset.py <patch.ips> <target.nes> <start_hex> <end_hex>

Records are kept if their starting offset falls in [start, end) of the ROM file
(i.e. counted from the beginning of the .nes file, not the iNES header).

Example — apply only PRG010-011 records (file offsets 0x14010..0x18010) of the
practice patch to a freshly-generated test ROM:
    python3 tools/apply_ips_subset.py patches/smb3practice_SE.ips test_level.nes 0x14010 0x18010
"""

import sys


def parse_ips(path):
    with open(path, "rb") as f:
        data = f.read()
    if data[:5] != b"PATCH":
        sys.exit(f"ERROR: bad IPS header in {path!r}")
    pos = 5
    records = []
    while pos < len(data):
        if data[pos:pos + 3] == b"EOF":
            break
        offset = (data[pos] << 16) | (data[pos + 1] << 8) | data[pos + 2]
        pos += 3
        size = (data[pos] << 8) | data[pos + 1]
        pos += 2
        if size == 0:
            rle_len = (data[pos] << 8) | data[pos + 1]
            value = data[pos + 2]
            pos += 3
            records.append((offset, bytes([value] * rle_len)))
        else:
            records.append((offset, data[pos:pos + size]))
            pos += size
    return records


def main():
    if len(sys.argv) != 5:
        sys.exit(__doc__)
    patch_path, rom_path, start_hex, end_hex = sys.argv[1:]
    start = int(start_hex, 16)
    end = int(end_hex, 16)

    records = parse_ips(patch_path)
    with open(rom_path, "rb") as f:
        rom = bytearray(f.read())

    applied = 0
    skipped = 0
    bytes_written = 0
    for offset, payload in records:
        if start <= offset < end:
            if offset + len(payload) > len(rom):
                sys.exit(f"ERROR: record at 0x{offset:06X} (+{len(payload)}) exceeds ROM size {len(rom)}")
            rom[offset:offset + len(payload)] = payload
            applied += 1
            bytes_written += len(payload)
        else:
            skipped += 1

    with open(rom_path, "wb") as f:
        f.write(rom)

    print(f"Applied {applied} records ({bytes_written} bytes) to {rom_path}; skipped {skipped} out-of-range records.")


if __name__ == "__main__":
    main()
