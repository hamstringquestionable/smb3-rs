#!/usr/bin/env python3
"""Regenerate `src/randomize/palette_variants.rs` from Recolored IPS + vanilla ROM.

*** THE RUST FILE HAS DIVERGED FROM GENERATED OUTPUT — DO NOT REGEN CASUALLY ***
palette_variants.rs now carries content this tool cannot reproduce:
  - hand-curated third-party variants (e.g. the "tuscan" entries)
  - ROTATE_ONLY_QUARTETS (kept-vanilla chromatic quartets for hue rotation)
  - the hand-verified SLICE4_POST group at 0x3784C (fails the palette_like
    filter because of an adjacent non-palette byte)
Running this tool overwrites the file and LOSES all of that. It refuses to run
without --force. For incremental region work use extract_palette_variants.py
(prints to stdout) and hand-edit the Rust file.

Excluded regions:
  - 0x377E0-0x37807 — level-layout pointer-table CRASH TRAP.
  - 0x33046-0x335xx — Recolored restructured this stream (insertions); unsafe
    for in-place quartet swaps.

Usage:
    nix-shell -p python3 --run "python3 tools/gen_palette_variants.py --force"
"""

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ROM = ROOT / "roms/Super Mario Bros. 3 (USA) (Rev 1).nes"
IPS = ROOT / "patches/Super Mario Bros. 3 Recolored v1.0.ips"
OUT = ROOT / "src/randomize/palette_variants.rs"

# (rust_const_name, label, start, end, description)
REGIONS = [
    ("SLOT0_MAP_VARIANTS",     "slot 0",    0x36BE4, 0x36C1C,
        "W6 sky overworld map + map HUD."),
    ("SLOT1_MAP_VARIANTS",     "slot 1",    0x36C1C, 0x36C54,
        "W7 (pipe) overworld map."),
    ("SLOT2_VARIANTS",         "slot 2",    0x36C54, 0x36C8C,
        "Hammer Bro sprites + HELP/world-label text overlay."),
    ("PLAINS_SLOT3_VARIANTS",  "slot 3",    0x36C8C, 0x36CC4,
        "Plains BG + HUD ($3F00 universal)."),
    ("SLOT4_VARIANTS",         "slot 4",    0x36CC4, 0x36CFC,
        "Giant tileset (W4)."),
    ("SLOT5_VARIANTS",         "slot 5",    0x36CFC, 0x36D34,
        "Plains enemies AND W7-5 sub-area BG (shared)."),
    ("SLOT6_VARIANTS",         "slot 6",    0x36D34, 0x36D6C,
        "Fortress HUD / related."),
    ("SLOT7_VARIANTS",         "slot 7",    0x36D6C, 0x36DA6,
        "Fortress BG AND W7-5 sub-area enemies (shared)."),
    ("SLOT_TAIL_VARIANTS",     "slot tail", 0x36DA8, 0x36E20,
        "Trailing themed-slot data past slot 7 (starts at 0x36DA8, the first offset past slot 7 on the table's 4-byte grid)."),
    ("POOL_VARIANTS",          "pool",      0x36E20, 0x37000,
        "Palette pool incl. the confirmed water-sprite slot at 0x36F00 (walking from 0x36E20 aligns it; the old 0x36EE2 sub-start does not)."),
    ("SLICE1_WATER_VARIANTS",  "slice 1",   0x37000, 0x37200,
        "Water tileset per-level variants."),
    ("SLICE2_VARIANTS",        "slice 2",   0x37200, 0x37400,
        "Desert + fortress + airship variants."),
    ("SLICE3_GIANT_VARIANTS",  "slice 3",   0x37400, 0x37600,
        "Giant tileset + water accents."),
    ("SLICE4_HEAD_VARIANTS",   "slice 4 head (pre-pointer-table)", 0x37600, 0x377E0,
        "Sky-Land + plains variants. MUST stop before 0x377E0 (level-layout pointer table — painting it crashes the game)."),
    ("SLICE4_TAIL_VARIANTS",   "slice 4 tail", 0x37808, 0x37846,
        "Slice 4 tail (after pointer-table crash trap)."),
    ("SLICE4_POST_VARIANTS",   "slice 4 post", 0x37844, 0x37850,
        "Palette constants just past the documented pool end. NOTE: regen drops the hand-verified 0x3784C group (adjacent 0xAD byte fails palette_like)."),
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
    if "--force" not in sys.argv:
        sys.exit(
            "palette_variants.rs has hand-curated content this tool cannot "
            "reproduce (see docstring). Use tools/extract_palette_variants.py "
            "for incremental work, or pass --force to overwrite anyway."
        )
    vanilla = ROM.read_bytes()
    recolored = bytearray(vanilla)
    for off, payload in parse_ips(IPS):
        recolored[off : off + len(payload)] = payload

    out = []
    p = out.append
    p("//! Curated palette-group variants for themed palette randomization.")
    p("//!")
    p("//! Each entry is a position in the ROM (file offset) where a 4-byte palette")
    p("//! group lives, paired with two or more known-good variants (vanilla + sources")
    p("//! like \"Super Mario Bros. 3 Recolored v1.0\"). At randomization time the")
    p("//! randomizer picks one variant per position, so every combination emitted")
    p("//! is built from aesthetically pre-validated 4-byte groups — no pool-mixing,")
    p("//! no independent-byte picks, no clash risk.")
    p("//!")
    p("//! This sidesteps the failure mode where flat color-pool randomization")
    p("//! produces combinations the original palette artists never intended.")
    p("//!")
    p("//! Bootstrap: `tools/gen_palette_variants.py` regenerates this file from the")
    p("//! Recolored IPS. Hand-curated alternates from other palette hacks can be")
    p("//! appended to each entry's `variants` list — but regeneration will overwrite")
    p("//! them, so start hand-editing once the Recolored seeds feel right.")
    p("//!")
    p("//! Hard constraint: NEVER include the pointer-table range 0x377E0-0x37807")
    p("//! in any variant group — painting those bytes corrupts the level-layout")
    p("//! CPU pointers and crashes the game on world entry.")
    p("")
    p("/// A palette-group variant set at a specific file offset.")
    p("pub struct VariantGroup {")
    p("    pub offset: usize,")
    p("    /// List of known-good 4-byte variants. At least one variant (vanilla)")
    p("    /// must always be present. Additional variants widen the randomization")
    p("    /// space without adding clash risk.")
    p("    pub variants: &'static [[u8; 4]],")
    p("}")
    p("")

    for const_name, label, start, end, desc in REGIONS:
        changed = []
        offset = start
        while offset + 4 <= end:
            v = vanilla[offset : offset + 4]
            r = bytes(recolored[offset : offset + 4])
            if v != r:
                changed.append((offset, v, r))
            offset += 4
        p("// " + "-" * 74)
        p(f"// {label.capitalize()} ({start:#07x}-{end:#07x}) — {desc}")
        p(f"// {len(changed)} quartets changed by Recolored.")
        p("// " + "-" * 74)
        p("")
        p(f"pub const {const_name}: &[VariantGroup] = &[")
        for off, v, r in changed:
            v_hex = ", ".join(f"0x{b:02X}" for b in v)
            r_hex = ", ".join(f"0x{b:02X}" for b in r)
            p(f"    VariantGroup {{ offset: 0x{off:05X}, variants: &[")
            p(f"        [{v_hex}],  // vanilla")
            p(f"        [{r_hex}],  // recolored")
            p(f"    ]}},")
        p("];")
        p("")

    OUT.write_text("\n".join(out))
    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()
