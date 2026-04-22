#!/usr/bin/env python3
"""Append a hue-family-tinted variant to every VariantGroup in palette_variants.rs.

Takes a "pool" of 4 NES indices that name hue FAMILIES (e.g., gold/indigo/teal/coral
for the Tuscan/Dusk/Mint/Tomato Adobe palette) and, for each VariantGroup's vanilla
quartet, produces a new variant by column-remapping each hue byte to the nearest
family while preserving structural bytes (0x00, 0x0F, 0xFF, grayscale) and the
byte's original luminance row.

Result: every quartet keeps its dark/light structure but its hue vocabulary is
restricted to the chosen 4 families — giving the family's overall aesthetic feel
without breaking HUD contrast or the quartet's internal design.

New variants identical to an existing variant (vanilla OR previous alt) are skipped.

Usage:
    nix-shell -p python3 --run \\
        "python3 tools/add_variant_family.py --name tuscan --pool 0x27,0x12,0x2C,0x26"
"""

import argparse
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "src/randomize/palette_variants.rs"

STRUCTURAL = {0x00, 0x0F, 0xFF}


def family_from_pool(pool):
    """Build {orig_col: pool_col} map from a 4-index pool.

    Each pool index's low nibble is its hue column. Every NES column 1-C is
    mapped to the closest pool column by hue distance on the NES color wheel
    (columns 1-3 cool-blue, 4-6 warm-red, 7-9 warm-yellow, A-C cool-green).
    """
    pool_cols = [p & 0x0F for p in pool]
    # Which column best represents each "hue neighborhood"?
    # col 1-3: blue/violet → nearest pool col in {pool_cols} closest to 2
    # col 4-6: magenta/red → closest to 5
    # col 7-9: orange/gold/olive → closest to 8
    # col A-C: green/teal → closest to B
    neighborhoods = {
        0x1: 0x2, 0x2: 0x2, 0x3: 0x2,
        0x4: 0x5, 0x5: 0x5, 0x6: 0x5,
        0x7: 0x8, 0x8: 0x8, 0x9: 0x8,
        0xA: 0xB, 0xB: 0xB, 0xC: 0xB,
    }

    def closest(target_col):
        return min(pool_cols, key=lambda c: min(abs(c - target_col), 0xD - abs(c - target_col)))

    return {col: closest(hood) for col, hood in neighborhoods.items()}


def substitute_byte(byte, col_map):
    if byte in STRUCTURAL:
        return byte
    col = byte & 0x0F
    row = (byte & 0x30) >> 4
    if col == 0:  # grayscale column (0x00/0x10/0x20/0x30)
        return byte
    if col > 0xC:  # 0x0D/0x0E/0x0F handled by STRUCTURAL or should stay
        return byte
    new_col = col_map.get(col, col)
    return (row << 4) | new_col


def substitute_quartet(vanilla, col_map):
    return [substitute_byte(b, col_map) for b in vanilla]


def parse_variants(block):
    """Parse `[0x.., 0x.., 0x.., 0x..]` entries from a variants `&[...]` block."""
    out = []
    for m in re.finditer(r"\[\s*(0x[0-9A-Fa-f]{2}(?:\s*,\s*0x[0-9A-Fa-f]{2}){3})\s*\]", block):
        out.append([int(x, 16) for x in m.group(1).split(",")])
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--name", required=True, help="Comment label for new variant (e.g., 'tuscan')")
    ap.add_argument("--pool", required=True, help="Comma-separated NES indices (hex), e.g. 0x27,0x12,0x2C,0x26")
    args = ap.parse_args()

    pool = [int(x, 16) for x in args.pool.split(",")]
    col_map = family_from_pool(pool)

    src = SRC.read_text()
    # Walk every VariantGroup entry; rewrite its variants list to include the
    # new tinted variant (if distinct from existing variants).
    group_re = re.compile(
        r"(VariantGroup\s*\{\s*offset:\s*0x([0-9A-Fa-f]+)\s*,\s*variants:\s*&\[)(.+?)(\]\s*\},)",
        re.DOTALL,
    )

    added = 0
    skipped_dup = 0

    def rewrite(m):
        nonlocal added, skipped_dup
        head, offset_hex, body, tail = m.group(1), m.group(2), m.group(3), m.group(4)
        existing = parse_variants(body)
        if not existing:
            return m.group(0)
        vanilla = existing[0]
        new_variant = substitute_quartet(vanilla, col_map)
        # Skip if identical to any existing variant
        if any(new_variant == e for e in existing):
            skipped_dup += 1
            return m.group(0)
        # Append before closing bracket. Preserve trailing comma + indentation.
        new_line = (
            "        ["
            + ", ".join(f"0x{b:02X}" for b in new_variant)
            + f"],  // {args.name}\n    "
        )
        # body ends with "]},\n    " or similar — insert before tail
        # We want: insert a new `[...]` line before the last whitespace of body
        new_body = body.rstrip() + "\n" + new_line
        added += 1
        return head + new_body + tail

    new_src = group_re.sub(rewrite, src)
    SRC.write_text(new_src)
    print(f"added '{args.name}' variant to {added} VariantGroups (skipped {skipped_dup} where tint matched existing)")
    print(f"pool={[f'0x{p:02X}' for p in pool]}")
    print(f"col_map={ {f'0x{k:X}':f'0x{v:X}' for k,v in col_map.items()} }")


if __name__ == "__main__":
    main()
