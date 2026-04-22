#!/usr/bin/env python3
"""Extract Recolored's actual palette byte choices at each mapped palette region.

For every region we've empirically mapped (0x36BE4 slots 0-7, 0x37000+ slices,
character palettes, etc.), this tool:
  1. Reads vanilla ROM bytes for the range.
  2. Applies the Recolored IPS and reads the new bytes.
  3. Groups the new bytes by their NES luminance row (0-3).
  4. Reports the unique colors Recolored picked, broken down by role/row.

The output is a grounded starting point for palette pools — replacing my
hand-picked lists with what a visually-tuned recolor actually used.
"""

from pathlib import Path
from collections import defaultdict

ROOT = Path(__file__).resolve().parent.parent
ROM_PATH = ROOT / "Super Mario Bros. 3 (USA) (Rev 1).nes"
IPS_PATH = ROOT / "Super Mario Bros. 3 Recolored v1.0.ips"

# Regions with confirmed purpose from emulator probes.
# (label, start, end, notes)
REGIONS = [
    # Character palettes (known)
    ("char_mario_luigi_all",      0x10539, 0x10555, "Mario/Luigi power-up palettes"),
    # Themed slot table 0x36BE4 (450 B, 8 slots of ~56 B)
    ("slot0_W6_sky_overworld",    0x36BE4, 0x36C1C, "W6 sky overworld map + HUD"),
    ("slot1_W7_pipe_overworld",   0x36C1C, 0x36C54, "W7 pipe overworld map"),
    ("slot2_overworld_text",      0x36C54, 0x36C8C, "HELP text + world labels + hammer bro"),
    ("slot3_plains_bg_hud",       0x36C8C, 0x36CC4, "Plains 1-1 BG + HUD ($3F00 universal)"),
    ("slot4_giant_w4",            0x36CC4, 0x36CFC, "Giant tileset (W4)"),
    ("slot5_plains_enemies",      0x36CFC, 0x36D34, "Plains enemies + W7-5 sub-area BG"),
    ("slot6_fortress_hud",        0x36D34, 0x36D6C, "W4-F1/W8 fortress HUD"),
    ("slot7_fortress_bg",         0x36D6C, 0x36DA6, "Fortress BG + W7-5 sub-area enemies"),
    # Other known palettes
    ("lava_rotodisc",             0x36DAA, 0x36DAE, "Lava / Rotodisc"),
    ("bowser_donut",              0x36DFE, 0x36E02, "Bowser / Donut lift"),
    # Master pool slices
    ("slice1_water",              0x37000, 0x37200, "Water tileset"),
    ("slice2_desert_fort_airship", 0x37200, 0x37400, "Desert + fortress + airship"),
    ("slice3_giant",              0x37400, 0x37600, "Giant + water accents"),
    ("slice4_skyland_plains",     0x37600, 0x377E0, "Sky-Land + plains variants (safe subrange)"),
    ("slice4_tail",               0x37808, 0x37846, "Slice 4 tail"),

    # Narrower sub-bands used by the Rust randomizer's actual write ranges.
    ("slice4_band3_plains_exact", 0x376D8, 0x37720, "Slice 4 band 3 — plains BG variant (exact write range)"),
    ("slice4_band0_sky_enemies",  0x37600, 0x3763C, "Slice 4 band 0 — Sky-Land enemy palette"),
]


def emit_rust_const(label: str, colors: list[int]) -> str:
    """Format a Rust const &[u8] line from a sorted color list."""
    hexes = ", ".join(f"0x{c:02X}" for c in colors if c != 0xFF)
    return f"pub const POOL_{label.upper()}: &[u8] = &[{hexes}];"


def parse_ips(path: Path) -> list[tuple[int, bytes]]:
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


def luminance_row(color: int) -> int:
    return (color & 0x30) >> 4


def main():
    vanilla = bytes(ROM_PATH.read_bytes())
    records = parse_ips(IPS_PATH)
    recolored = bytearray(vanilla)
    for off, payload in records:
        recolored[off : off + len(payload)] = payload

    print("Recolored palette choices, grouped by region and luminance row.")
    print("Luminance rows: 0 = darkest (0x00-0x0C), 1, 2, 3 = brightest (0x30-0x3C)")
    print("Structural bytes (0x00, 0x0F) are omitted — they don't count as colors.")
    print()

    # Per-region: collect unique new colors (excluding structural 0x00/0x0F).
    per_region = {}
    for label, start, end, notes in REGIONS:
        v = vanilla[start:end]
        r = bytes(recolored[start:end])
        by_row = defaultdict(set)
        unique_all = set()
        changed_count = 0
        for i in range(len(r)):
            new = r[i]
            old = v[i]
            if new == 0x00 or new == 0x0F:
                continue
            if old != new:
                changed_count += 1
            by_row[luminance_row(new)].add(new)
            unique_all.add(new)
        per_region[label] = {
            "range": (start, end),
            "notes": notes,
            "by_row": {k: sorted(by_row[k]) for k in sorted(by_row)},
            "unique_all": sorted(unique_all),
            "changed_count": changed_count,
        }

    for label, info in per_region.items():
        start, end = info["range"]
        print(f"=== {label}  ({start:#08x}-{end:#08x}, {end-start} B)")
        print(f"    {info['notes']}")
        print(f"    {info['changed_count']} bytes changed by Recolored")
        for row in range(4):
            colors = info["by_row"].get(row, [])
            hexlist = ", ".join(f"0x{c:02X}" for c in colors) or "(none)"
            print(f"    row {row}: {hexlist}")
        all_hex = ", ".join(f"0x{c:02X}" for c in info["unique_all"]) or "(none)"
        print(f"    unique colors (all rows): [{all_hex}]")
        print()

    print("=" * 70)
    print("RUST CONST SNIPPETS (paste into src/randomize/palette_pools.rs)")
    print("=" * 70)
    print()
    for label, info in per_region.items():
        print(f"// {info['notes']} ({info['changed_count']} bytes changed)")
        print(emit_rust_const(label, info["unique_all"]))
        print()

    # Cross-region color frequency — what colors does Recolored reuse most?
    print("=" * 70)
    print("GLOBAL COLOR USAGE across all mapped regions")
    print("=" * 70)
    freq = defaultdict(int)
    for info in per_region.values():
        for c in info["unique_all"]:
            freq[c] += 1
    # Group by row
    for row in range(4):
        row_entries = [(c, freq[c]) for c in freq if luminance_row(c) == row]
        row_entries.sort(key=lambda x: (-x[1], x[0]))
        print(f"\nrow {row} ({16 + row*16:#04x}-{16 + row*16+12:#04x}):")
        for c, count in row_entries:
            bar = "█" * count
            print(f"  0x{c:02X} × {count:2d} {bar}")


if __name__ == "__main__":
    main()
