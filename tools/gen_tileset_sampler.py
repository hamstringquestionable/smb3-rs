#!/usr/bin/env python3
"""Generate a single test ROM where W1 early tiles host one representative level
per tileset, AND a chosen palette region is rainbow-painted. Boot once, walk
across W1's first ~9 tiles, see every tileset under the same probe.

Combines:
  - tools/gen_palette_rainbow.py (rainbow paint of a region)
  - the per-tileset rep map (one early level per tileset, taken from rom_map.json)
  - patches/smb3practice_SE.ips (open movement + warp whistles + level select)

Usage:
  python3 tools/gen_tileset_sampler.py --start 0x37400 --end 0x37600 --bands 8
  python3 tools/gen_tileset_sampler.py --copy-existing test_roms/palette_rainbow_37400_pool_slice3_w6.nes

Output: test_roms/sampler_<rainbow-name>.nes (start in W1, walk tiles 1..9)
"""

import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ROM_PATH = ROOT / "roms/Super Mario Bros. 3 (USA) (Rev 1).nes"
PRACTICE_IPS = ROOT / "patches/smb3practice_SE.ips"
OUT_DIR = ROOT / "test_roms"
OUT_DIR.mkdir(exist_ok=True)

# W1 pointer table layout (from /test-level skill / src/randomize/rom_data.rs):
#   base    + count bytes              = byrowtype (low nibble = tileset)
#   base+21 + count bytes              = scrcol
#   base+42 + count*2 bytes (LE words) = ObjSets   (CPU pointer to object/enemy data)
#   base+84 + count*2 bytes (LE words) = LevelLayouts (CPU pointer to layout data)
W1_BASE = 0x19438
W1_COUNT = 21

START_WORLD_OFFSET = 0x30CC3

# One representative level per tileset, pulled from the vanilla W1-W8 pointer
# tables (so we know the (obj, lay) pair is shipped-game valid and will load
# without crashes when dropped into any W1 slot).
# Format: (display_name, tileset_id_low_nibble, obj_cpu_ptr, lay_cpu_ptr)
REPS = [
    ("Plains",        1,  0xC527, 0xBB82),  # TS1 from W1 entry 1 (1-1)
    ("Hilly",         3,  0xC72B, 0xB3EB),  # TS3 from W1 entry 2
    ("Ice/Sky",       4,  0xCBE5, 0xB2BF),  # TS4 from W1 entry 6
    ("Cloud/Giant",   5,  0xD07C, 0xBC23),  # TS5 from W7 entry 12
    ("Pipe/Water",    6,  0xCE25, 0xB30A),  # TS6 from W3 entry 18 (water)
    ("Desert",        9,  0xD14D, 0xB1F6),  # TS9 from W2 entry 2
    ("Airship",      10,  0xD96F, 0xB8D3),  # TS10 from W8 entry 6
    ("GiantLand",    11,  0xD0EA, 0xBF5E),  # TS11 from W4 entry 1
    ("Ice",          12,  0xC640, 0xBF7E),  # TS12 from W6 entry 2
    ("Dungeon",       2,  0xD32B, 0xA95D),  # TS2 from W1 entry 12 (fortress)
    ("Underground",  14,  0xC92B, 0xAA41),  # TS14 from W1 entry 19
]


def parse_ips(path: Path):
    data = path.read_bytes()
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


RAINBOW = [
    (0x16, "red"), (0x27, "orange"), (0x28, "yellow"), (0x1A, "green"),
    (0x2C, "cyan"), (0x21, "blue"), (0x14, "purple"), (0x24, "magenta"),
]


def patch_w1_reps(rom: bytearray) -> list[tuple[int, str, int]]:
    """Overwrite W1 entries 0..len(REPS)-1 with the representative levels.
    Returns list of (tile_idx_in_w1, name, tileset) for the report."""
    base = W1_BASE
    count = W1_COUNT
    byrowtype = base
    objsets = base + count + count
    layouts = base + count + count + count * 2

    placed = []
    for idx, (name, tileset, obj_cpu, lay_cpu) in enumerate(REPS):
        if idx >= count:
            break
        # Preserve high nibble of byrowtype (row type info), set low nibble = tileset
        old = rom[byrowtype + idx]
        rom[byrowtype + idx] = (old & 0xF0) | (tileset & 0x0F)
        rom[objsets + idx * 2 + 0] = obj_cpu & 0xFF
        rom[objsets + idx * 2 + 1] = (obj_cpu >> 8) & 0xFF
        rom[layouts + idx * 2 + 0] = lay_cpu & 0xFF
        rom[layouts + idx * 2 + 1] = (lay_cpu >> 8) & 0xFF
        placed.append((idx, name, tileset))
    return placed


def paint_rainbow(rom: bytearray, start: int, end: int, bands: int) -> list:
    """Paint `bands` color bands into rom[start:end]. Skip 0x00 and 0x0F."""
    span = end - start
    band_size = span // bands
    out = []
    for i in range(bands):
        lo = start + i * band_size
        hi = start + (i + 1) * band_size if i < bands - 1 else end
        color, label = RAINBOW[i]
        painted = 0
        for off in range(lo, hi):
            if rom[off] not in (0x00, 0x0F):
                rom[off] = color
                painted += 1
        out.append((i, lo, hi, color, label, painted))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--start", type=lambda s: int(s, 0), required=False)
    ap.add_argument("--end",   type=lambda s: int(s, 0), required=False)
    ap.add_argument("--bands", type=int, default=8)
    ap.add_argument(
        "--copy-existing", type=Path, default=None,
        help="Skip painting, just take an already-painted ROM (e.g. one from gen_palette_rainbow.py) "
             "and patch the tileset reps onto W1.",
    )
    ap.add_argument("--name", default=None)
    args = ap.parse_args()

    if args.copy_existing is None and (args.start is None or args.end is None):
        ap.error("must provide either --copy-existing OR (--start AND --end)")

    if args.copy_existing:
        rom = bytearray(args.copy_existing.read_bytes())
        name = args.name or args.copy_existing.stem.replace("palette_rainbow_", "").replace("_w6", "")
        print(f"Reusing painted ROM: {args.copy_existing.name}")
    else:
        rom = bytearray(ROM_PATH.read_bytes())
        # Apply full practice IPS first (skip records that overlap our paint region)
        if PRACTICE_IPS.exists():
            for off, payload in parse_ips(PRACTICE_IPS):
                if not (off < args.end and off + len(payload) > args.start):
                    rom[off : off + len(payload)] = payload
        # Paint rainbow
        bands_info = paint_rainbow(rom, args.start, args.end, args.bands)
        print(f"Painted rainbow: {args.start:#08x}-{args.end:#08x} ({args.bands} bands)")
        for i, lo, hi, color, label, painted in bands_info:
            print(f"  band {i}: {lo:#08x}-{hi:#08x}  color {color:#04x} ({label:<7s})  {painted} painted")
        name = args.name or f"{args.start:06x}_{args.end:06x}"

    # Apply practice IPS for the copy-existing case too (it already has it baked in,
    # but only if the source ROM was generated with the practice IPS — which our
    # gen_palette_rainbow.py output is). So skip re-applying.

    # Patch W1 with tileset reps
    placed = patch_w1_reps(rom)

    # Set starting world to W1
    rom[START_WORLD_OFFSET] = 0  # W1

    out = OUT_DIR / f"sampler_{name}_w1.nes"
    out.write_bytes(rom)

    print()
    print(f"Wrote {out}")
    print()
    print("W1 early tile assignments (walk to each tile and enter):")
    for idx, name, tileset in placed:
        print(f"  tile {idx + 1}: {name:<15s}  tileset {tileset}")
    print()
    print("Note: tile #1 = first numbered map tile in W1 (the '1' on the map).")
    print("With open-movement applied, walk through every tile and enter each level.")


if __name__ == "__main__":
    main()
