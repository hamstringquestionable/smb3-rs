#!/usr/bin/env python3
"""Parse an IPS patch file and dump all patch records with offsets, sizes, and data."""

import os
import sys


def parse_ips(filepath):
    with open(filepath, "rb") as f:
        data = f.read()

    # Validate header
    if data[:5] != b"PATCH":
        print(f"ERROR: Invalid IPS header: {data[:5]!r}", file=sys.stderr)
        sys.exit(1)

    pos = 5
    records = []

    while pos < len(data):
        # Check for EOF marker
        if data[pos:pos + 3] == b"EOF":
            break

        if pos + 3 > len(data):
            print(f"ERROR: Unexpected end of patch at pos {pos}", file=sys.stderr)
            sys.exit(1)

        # Read 3-byte offset (big-endian)
        offset = (data[pos] << 16) | (data[pos + 1] << 8) | data[pos + 2]
        pos += 3

        if pos + 2 > len(data):
            print(f"ERROR: Unexpected end of patch reading size at pos {pos}", file=sys.stderr)
            sys.exit(1)

        # Read 2-byte size (big-endian)
        size = (data[pos] << 8) | data[pos + 1]
        pos += 2

        if size == 0:
            # RLE record
            if pos + 3 > len(data):
                print(f"ERROR: Unexpected end of patch reading RLE at pos {pos}", file=sys.stderr)
                sys.exit(1)
            rle_count = (data[pos] << 8) | data[pos + 1]
            rle_value = data[pos + 2]
            pos += 3
            records.append({
                "type": "RLE",
                "offset": offset,
                "count": rle_count,
                "value": rle_value,
                "data": bytes([rle_value] * rle_count),
            })
        else:
            # Raw record
            if pos + size > len(data):
                print(f"ERROR: Unexpected end of patch reading payload at pos {pos}", file=sys.stderr)
                sys.exit(1)
            payload = data[pos:pos + size]
            pos += size
            records.append({
                "type": "RAW",
                "offset": offset,
                "count": size,
                "data": payload,
            })

    return records


def format_hex_dump(data, bytes_per_line=16):
    lines = []
    for i in range(0, len(data), bytes_per_line):
        chunk = data[i:i + bytes_per_line]
        hex_part = " ".join(f"{b:02X}" for b in chunk)
        ascii_part = "".join(chr(b) if 32 <= b < 127 else "." for b in chunk)
        lines.append(f"    {i:04X}: {hex_part:<{bytes_per_line * 3 - 1}}  |{ascii_part}|")
    return "\n".join(lines)


def main():
    if len(sys.argv) < 2:
        # Default to the reference IPS in the project
        script_dir = os.path.dirname(os.path.abspath(__file__))
        project_dir = os.path.dirname(script_dir)
        filepath = os.path.join(project_dir, "Super_Mario_Bros_3_NoAutoscrolls(Except 5-9).ips")
        if not os.path.exists(filepath):
            print(f"Usage: {sys.argv[0]} <ips_file>", file=sys.stderr)
            print(f"Default file not found: {filepath}", file=sys.stderr)
            sys.exit(1)
    else:
        filepath = sys.argv[1]

    print(f"Parsing: {filepath}")
    print(f"File size: {os.path.getsize(filepath)} bytes")
    print()

    records = parse_ips(filepath)

    print(f"Total records: {len(records)}")
    print("=" * 80)

    for i, rec in enumerate(records):
        offset = rec["offset"]
        count = rec["count"]
        end = offset + count - 1

        if rec["type"] == "RLE":
            print(f"\nRecord #{i + 1}: RLE")
            print(f"  Offset: 0x{offset:06X} - 0x{end:06X} ({count} bytes)")
            print(f"  Value:  0x{rec['value']:02X}")
        else:
            print(f"\nRecord #{i + 1}: RAW")
            print(f"  Offset: 0x{offset:06X} - 0x{end:06X} ({count} bytes)")
            print(f"  Data:")
            print(format_hex_dump(rec["data"]))

    # Summary: show all affected byte offsets for easy comparison
    print()
    print("=" * 80)
    print("BYTE-LEVEL CHANGES (offset -> new value):")
    print("=" * 80)
    for rec in records:
        offset = rec["offset"]
        for j, b in enumerate(rec["data"]):
            print(f"  0x{offset + j:06X} = 0x{b:02X}")


if __name__ == "__main__":
    main()
