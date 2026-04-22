#!/usr/bin/env python3
"""Extract quartet-level VariantGroup entries from Recolored.

For every mapped palette region, walk 4-byte-aligned quartets from region
start, compare vanilla vs. Recolored bytes, and print a ready-to-paste
`VariantGroup { offset, variants: &[[vanilla], [recolored]]}` entry for
every quartet that Recolored changed.

Used to seed `src/randomize/palette_variants.rs` with the Recolored variant
at every known-changed position. Hand-added alternates (curated, from other
palette hacks) are appended to each entry's `variants` list after the fact.

Regions excluded from the output:
  - Slot 0 / Slot 1 (overworld maps) — stretch goal, out of scope for tileset
    variant-swap randomizer.
  - 0x377E0-0x37807 inside slice 4 — level-layout pointer table (CRASH TRAP).
    The slice4 region stops at 0x377E0; slice4_tail picks up at 0x37808.
"""

from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ROM = ROOT / "Super Mario Bros. 3 (USA) (Rev 1).nes"
IPS = ROOT / "Super Mario Bros. 3 Recolored v1.0.ips"

REGIONS = [
    ("slot2_overworld_text",       0x36C54, 0x36C8C),
    ("slot3_plains_bg_hud",        0x36C8C, 0x36CC4),
    ("slot4_giant_w4",             0x36CC4, 0x36CFC),
    ("slot5_plains_enemies",       0x36CFC, 0x36D34),
    ("slot6_fortress_hud",         0x36D34, 0x36D6C),
    ("slot7_fortress_bg",          0x36D6C, 0x36DA6),
    ("slice1_water",               0x37000, 0x37200),
    ("slice2_desert_fort_airship", 0x37200, 0x37400),
    ("slice3_giant",               0x37400, 0x37600),
    ("slice4_skyland_plains",      0x37600, 0x377E0),
    ("slice4_tail",                0x37808, 0x37846),
]


def parse_ips(p):
    d = p.read_bytes()
    assert d[:5] == b"PATCH"
    pos = 5
    out = []
    while d[pos : pos + 3] != b"EOF":
        off = (d[pos] << 16) | (d[pos + 1] << 8) | d[pos + 2]
        pos += 3
        sz = (d[pos] << 8) | d[pos + 1]
        pos += 2
        if sz == 0:
            n = (d[pos] << 8) | d[pos + 1]
            v = d[pos + 2]
            pos += 3
            out.append((off, bytes([v]) * n))
        else:
            out.append((off, d[pos : pos + sz]))
            pos += sz
    return out


def main():
    vanilla = ROM.read_bytes()
    recolored = bytearray(vanilla)
    for off, payload in parse_ips(IPS):
        recolored[off : off + len(payload)] = payload

    for label, start, end in REGIONS:
        changed = []
        offset = start
        while offset + 4 <= end:
            v = vanilla[offset : offset + 4]
            r = bytes(recolored[offset : offset + 4])
            if v != r:
                changed.append((offset, v, r))
            offset += 4
        print(f"// === {label}  ({start:#08x}-{end:#08x}, {len(changed)} quartets changed) ===")
        for off, v, r in changed:
            v_hex = ", ".join(f"0x{b:02X}" for b in v)
            r_hex = ", ".join(f"0x{b:02X}" for b in r)
            print(f"    VariantGroup {{ offset: 0x{off:05X}, variants: &[")
            print(f"        [{v_hex}],  // vanilla")
            print(f"        [{r_hex}],  // recolored")
            print(f"    ]}},")
        print()


if __name__ == "__main__":
    main()
