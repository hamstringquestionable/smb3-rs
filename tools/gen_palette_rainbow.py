#!/usr/bin/env python3
"""Rainbow probe: paint a single ROM region in N color-coded bands so you can
identify which sub-range drives which graphic with ONE emulator boot.

Each band of the region gets a distinct, easily-distinguishable NES color.
Run the ROM and note which color appears where:

  Band 0 → red       (0x16)
  Band 1 → orange    (0x27)
  Band 2 → yellow    (0x28)
  Band 3 → green     (0x1A)
  Band 4 → cyan      (0x2C)
  Band 5 → blue      (0x21)
  Band 6 → purple    (0x14)
  Band 7 → magenta   (0x24)

Default region is 0x36EE2-0x37000 (the master pool slice that contains sky+HUD)
split into 8 bands. Override with --start / --end / --bands.

Usage:
  python3 tools/gen_palette_rainbow.py
  python3 tools/gen_palette_rainbow.py --start 0x36EE2 --end 0x37000 --bands 8
  python3 tools/gen_palette_rainbow.py --start 0x36BE4 --end 0x36DA6 --bands 6 --world 1
"""

import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ROM_PATH = ROOT / "roms/Super Mario Bros. 3 (USA) (Rev 1).nes"
PRACTICE_IPS = ROOT / "patches/smb3practice_SE.ips"
START_WORLD_OFFSET = 0x30CC3
OUT_DIR = ROOT / "test_roms"
OUT_DIR.mkdir(exist_ok=True)

# 8 visually distinct NES colors; ordered for easy "left → right" recall.
RAINBOW = [
    (0x16, "red"),
    (0x27, "orange"),
    (0x28, "yellow"),
    (0x1A, "green"),
    (0x2C, "cyan"),
    (0x21, "blue"),
    (0x14, "purple"),
    (0x24, "magenta"),
]


def parse_ips(path: Path):
    data = path.read_bytes()
    if data[:5] != b"PATCH":
        raise SystemExit(f"bad IPS: {path}")
    pos = 5
    out = []
    while data[pos : pos + 3] != b"EOF":
        off = (data[pos] << 16) | (data[pos + 1] << 8) | data[pos + 2]
        pos += 3
        sz = (data[pos] << 8) | data[pos + 1]
        pos += 2
        if sz == 0:
            n = (data[pos] << 8) | data[pos + 1]
            v = data[pos + 2]
            pos += 3
            out.append((off, bytes([v]) * n))
        else:
            out.append((off, data[pos : pos + sz]))
            pos += sz
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--start", type=lambda s: int(s, 0), default=0x36EE2)
    ap.add_argument("--end",   type=lambda s: int(s, 0), default=0x37000)
    ap.add_argument("--bands", type=int, default=8)
    ap.add_argument("--world", type=int, default=6, choices=range(1, 9))
    ap.add_argument("--name",  default=None,
                    help="output filename suffix (default: derived from start-end)")
    args = ap.parse_args()

    if args.bands > len(RAINBOW):
        raise SystemExit(f"max {len(RAINBOW)} bands (we only have that many distinct colors)")

    rom = bytearray(ROM_PATH.read_bytes())

    # Apply full practice IPS for traversal
    if PRACTICE_IPS.exists():
        for off, payload in parse_ips(PRACTICE_IPS):
            # Skip records that would land inside the rainbow region (would corrupt our paint)
            if not (off < args.end and off + len(payload) > args.start):
                rom[off : off + len(payload)] = payload

    # Set starting world
    rom[START_WORLD_OFFSET] = args.world - 1

    span = args.end - args.start
    band_size = span // args.bands
    print(f"Rainbow region: {args.start:#08x}-{args.end:#08x}  ({span} B)  starting W{args.world}")
    print(f"Bands: {args.bands} × ~{band_size} B each")
    print()
    for i in range(args.bands):
        lo = args.start + i * band_size
        hi = args.start + (i + 1) * band_size if i < args.bands - 1 else args.end
        color, label = RAINBOW[i]
        painted = 0
        for off in range(lo, hi):
            if rom[off] not in (0x00, 0x0F):
                rom[off] = color
                painted += 1
        print(f"  band {i}: {lo:#08x}-{hi:#08x}  color {color:#04x} ({label:<7s})  {painted} painted")

    name = args.name or f"{args.start:06x}_{args.end:06x}"
    out = OUT_DIR / f"palette_rainbow_{name}_w{args.world}.nes"
    out.write_bytes(rom)
    print()
    print(f"Wrote {out}")
    print()
    print("In emulator: walk through the game, note which areas show which color.")
    print("Each color → a contiguous slice of the region drove that graphic.")


if __name__ == "__main__":
    main()
