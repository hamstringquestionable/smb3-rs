#!/usr/bin/env python3
"""FX slot diagnostic tool for SMB3 randomizer.

Cross-checks the 17 FortressFX slots against the actual overworld map tiles
and pointer table entries. Useful for verifying that fortress redistribution,
lock shuffle, and related overworld mutations leave the FX system consistent.

Usage:
    python3 tools/fx_check.py <rom_file>
    python3 tools/fx_check.py "Super Mario Bros. 3 (USA) (Rev 1).nes"

Output per world:
  - Fortress entries (tileset 2 from pointer table) with grid positions and map tiles
  - Airship entries for reference
  - FX slot assignments from FortressFX_W1-W8 table
  - For each active FX slot: decoded target position, actual map tile, expected
    lock/gap tile, replacement tile, VRAM address, pattern type
  - OK/BAD/OOB status for each slot

Exit code: 0 if all slots OK, 1 if any issues found.

Limitations:
  - Fortress detection uses tileset==2 check, which misses two vanilla fortresses
    that use non-standard tilesets (W2[13]=ts9, W6[27]=ts12). These are still
    detected by the vanilla FORTRESS_ENTRIES set but won't be found if they move
    to different entry indices after redistribution.
  - Airship classification uses hardcoded vanilla AIRSHIP_ENTRIES set.
"""

import sys

# -- Map tile grids (from map_walker.rs MAP_TILE_GRIDS) ----------------------
# Each tuple: (file_offset, total_columns, screen_count)
# Grid is always 9 rows. Each screen is 16 columns wide.
MAP_TILE_GRIDS = [
    (0x185BA, 16, 1),   # W1: 1 screen
    (0x1864B, 32, 2),   # W2: 2 screens
    (0x1876C, 48, 3),   # W3: 3 screens
    (0x1891D, 32, 2),   # W4: 2 screens
    (0x18A3E, 32, 2),   # W5: 2 screens
    (0x18B5F, 48, 3),   # W6: 3 screens
    (0x18D10, 32, 2),   # W7: 2 screens
    (0x18E31, 64, 4),   # W8: 4 screens
]

# -- Pointer tables (from map_walker.rs WORLDS) ------------------------------
# Each tuple: (rowtype_offset, entry_count)
# Per-world sub-tables are contiguous at rowtype_offset:
#   ByRowType[N], ByScrCol[N], ObjSets[N words], LevelLayouts[N words]
WORLDS = [
    (0x19438, 21),   # W1
    (0x194BA, 47),   # W2
    (0x195D8, 52),   # W3
    (0x19714, 34),   # W4
    (0x197E4, 42),   # W5
    (0x198E4, 57),   # W6
    (0x19A3E, 46),   # W7
    (0x19B56, 41),   # W8
]

# -- FX table offsets (17 slots, from overworld.rs / map_walker.rs) ----------
# Slots 0-12 are for W1-W7 fortresses, slots 13-16 are for W8.
FX_VADDR_H       = 0x147CD   # 17 bytes: VRAM address high byte
FX_VADDR_L       = 0x147DE   # 17 bytes: VRAM address low byte
FX_MAP_COMP_IDX  = 0x147EF   # 17 x 2 bytes: (col, bit) for Map_Completions persistence
FX_PATTERNS      = 0x14811   # 17 x 4 bytes: CHR pattern data for the FX animation
FX_MAP_LOC_ROW   = 0x14855   # 17 bytes: encoded grid row ((row+2)<<4)
FX_MAP_LOC       = 0x14866   # 17 bytes: encoded (col<<4)|screen
FX_MAP_TILE_REPL = 0x14877   # 17 bytes: tile to restore when lock/gap is cleared
FX_WORLD_TABLE   = 0x14888   # 8 x 4 bytes: FortressFX_W1-W8 slot assignments

# -- Known special tiles -----------------------------------------------------
TILE_LOCK       = 0x54
TILE_BRIDGE_GAP = 0x56
TILE_WATER_GAP  = 0x9D
TILE_SKY_GAP    = 0xE4
TILE_FORTRESS   = 0x67

GAP_TILES = {TILE_LOCK, TILE_BRIDGE_GAP, TILE_WATER_GAP, TILE_SKY_GAP}

PATTERN_NAMES = {
    (0xFE, 0xC0, 0xFE, 0xC0): "Lock",
    (0xFE, 0xFE, 0xE1, 0xE1): "Bridge/Sky",
    (0xD4, 0xD6, 0xD5, 0xD7): "Water",
    (0xFF, 0xFF, 0xFF, 0xFF): "W8Lock",
}

# -- Known vanilla entries ---------------------------------------------------
FORTRESS_ENTRIES = set([
    (0, 11),
    (1, 13),
    (2, 13), (2, 34),
    (3, 9), (3, 16),
    (4, 12), (4, 31),
    (5, 9), (5, 27), (5, 48),
    (6, 5), (6, 40),
    (7, 7), (7, 10), (7, 26), (7, 36),
])

AIRSHIP_ENTRIES = set([
    (0, 17), (1, 36), (2, 49), (3, 6), (4, 35), (5, 53), (6, 43),
])

BOWSER_ENTRY = (7, 40)


def map_tile_offset(world_idx, row, col):
    """Return the ROM file offset for a map tile at (row, col) in the given world."""
    base, cols, screens = MAP_TILE_GRIDS[world_idx]
    screen = col // 16
    col_in_screen = col % 16
    return base + screen * 144 + row * 16 + col_in_screen


def entry_grid_position(rom, world_idx, entry_idx):
    """Decode an entry's ByRowType/ByScrCol bytes into (grid_row, grid_col)."""
    rowtype_off, entry_count = WORLDS[world_idx]
    row_byte = rom[rowtype_off + entry_idx]
    scrcol_byte = rom[rowtype_off + entry_count + entry_idx]
    row_nibble = (row_byte >> 4) & 0x0F
    screen = (scrcol_byte >> 4) & 0x0F
    column = scrcol_byte & 0x0F
    grid_row = (row_nibble - 2) & 0xFF  # upper nibble encodes row+2
    grid_col = screen * 16 + column
    return grid_row, grid_col


def entry_tileset(rom, world_idx, entry_idx):
    """Read the tileset ID (lower nibble of ByRowType) for a pointer table entry."""
    rowtype_off, entry_count = WORLDS[world_idx]
    return rom[rowtype_off + entry_idx] & 0x0F


def entry_obj_ptr(rom, world_idx, entry_idx):
    """Read the ObjSets pointer (16-bit LE) for a pointer table entry."""
    rowtype_off, entry_count = WORLDS[world_idx]
    obj_off = rowtype_off + entry_count * 2
    lo = rom[obj_off + entry_idx * 2]
    hi = rom[obj_off + entry_idx * 2 + 1]
    return (hi << 8) | lo


def entry_lay_ptr(rom, world_idx, entry_idx):
    """Read the LevelLayouts pointer (16-bit LE) for a pointer table entry."""
    rowtype_off, entry_count = WORLDS[world_idx]
    lay_off = rowtype_off + entry_count * 4
    lo = rom[lay_off + entry_idx * 2]
    hi = rom[lay_off + entry_idx * 2 + 1]
    return (hi << 8) | lo


def decode_fx_slot(rom, slot):
    """Decode a single FX slot into a dict of human-readable fields.

    Returns dict with: vaddr, grid_row, grid_col, screen, col_in_screen,
    replace_tile, comp_col, comp_bit, patterns (4-tuple), pat_name.
    """
    vaddr = (rom[FX_VADDR_H + slot] << 8) | rom[FX_VADDR_L + slot]
    loc_row_byte = rom[FX_MAP_LOC_ROW + slot]
    loc_byte = rom[FX_MAP_LOC + slot]
    replace_tile = rom[FX_MAP_TILE_REPL + slot]
    comp_col = rom[FX_MAP_COMP_IDX + slot * 2]
    comp_bit = rom[FX_MAP_COMP_IDX + slot * 2 + 1]
    pats = tuple(rom[FX_PATTERNS + slot * 4 + i] for i in range(4))

    # Decode position from loc_byte and loc_row_byte
    screen = loc_byte & 0x0F
    col_in_screen = (loc_byte >> 4) & 0x0F
    grid_row = ((loc_row_byte >> 4) & 0x0F) - 2
    grid_col = screen * 16 + col_in_screen

    pat_name = PATTERN_NAMES.get(pats, f"Unknown{pats}")

    return {
        "vaddr": vaddr,
        "grid_row": grid_row,
        "grid_col": grid_col,
        "screen": screen,
        "col_in_screen": col_in_screen,
        "replace_tile": replace_tile,
        "comp_col": comp_col,
        "comp_bit": comp_bit,
        "patterns": pats,
        "pat_name": pat_name,
    }


def classify_entry(rom, world_idx, entry_idx):
    """Classify a pointer table entry as fortress/airship/bowser/toad/level/other.

    Returns (kind, tileset, obj_ptr, lay_ptr, grid_row, grid_col, map_tile).
    map_tile is None if the position is out of grid bounds.
    """
    ts = entry_tileset(rom, world_idx, entry_idx)
    obj = entry_obj_ptr(rom, world_idx, entry_idx)
    lay = entry_lay_ptr(rom, world_idx, entry_idx)
    row, col = entry_grid_position(rom, world_idx, entry_idx)
    _, cols, _ = MAP_TILE_GRIDS[world_idx]
    if col < cols and row < 9:
        tile_off = map_tile_offset(world_idx, row, col)
        tile = rom[tile_off]
    else:
        tile = None

    if (world_idx, entry_idx) == BOWSER_ENTRY:
        kind = "bowser"
    elif (world_idx, entry_idx) in AIRSHIP_ENTRIES:
        kind = "airship"
    elif ts == 2 and obj >= 0xC000 and lay != 0x0000:
        kind = "fortress"
    elif obj == 0x0700:
        kind = "toad"
    elif obj >= 0xC000 and lay != 0x0000:
        kind = "level"
    else:
        kind = "other"

    return kind, ts, obj, lay, row, col, tile


def check_rom(rom_path):
    """Main diagnostic: load a ROM and cross-check all FX slots. Returns issue count."""
    with open(rom_path, "rb") as f:
        rom = f.read()

    print(f"ROM: {rom_path} ({len(rom)} bytes)")
    print("=" * 72)

    total_issues = 0

    for wi in range(8):
        base, cols, screens = MAP_TILE_GRIDS[wi]
        _, entry_count = WORLDS[wi]
        print(f"\n--- World {wi+1} (grid: 9x{cols}, {screens} screen(s), {entry_count} entries) ---")

        # 1. Find all fortress entries (tileset 2, not airship, not bowser)
        forts = []
        airships = []
        for i in range(entry_count):
            kind, ts, obj, lay, row, col, tile = classify_entry(rom, wi, i)
            if kind == "fortress":
                vanilla = "V" if (wi, i) in FORTRESS_ENTRIES else " "
                forts.append({"idx": i, "row": row, "col": col, "tile": tile,
                              "obj": obj, "lay": lay, "vanilla": vanilla})
            elif kind == "airship":
                airships.append({"idx": i, "row": row, "col": col, "tile": tile})

        print(f"  Fortresses: {len(forts)}  Airships: {len(airships)}")
        for f in forts:
            tile_s = f"${f['tile']:02X}" if f['tile'] is not None else "OOB"
            print(f"    [{f['vanilla']}] entry {f['idx']:2d}  pos=({f['row']},{f['col']:2d})  "
                  f"tile={tile_s}  obj=${f['obj']:04X}  lay=${f['lay']:04X}")
        for a in airships:
            tile_s = f"${a['tile']:02X}" if a['tile'] is not None else "OOB"
            print(f"    [A] entry {a['idx']:2d}  pos=({a['row']},{a['col']:2d})  tile={tile_s}")

        # 2. FX slot assignments
        fx_base = FX_WORLD_TABLE + wi * 4
        raw_slots = [rom[fx_base + j] for j in range(4)]

        # Determine active slots: non-zero, plus slot 0 for W1
        real_slots = []
        for j, s in enumerate(raw_slots):
            if s != 0:
                real_slots.append((j, s))
            elif wi == 0 and j == 0:
                real_slots.append((j, s))  # W1 first slot is 0

        print(f"  FX slots: {[f'{s}' for _, s in real_slots]}  (raw: {[f'0x{s:02X}' for s in raw_slots]})")

        # 3. Decode and cross-check each active FX slot
        for j, slot_id in real_slots:
            fx = decode_fx_slot(rom, slot_id)
            row, col = fx["grid_row"], fx["grid_col"]

            # Check bounds
            oob = col >= cols or row < 0 or row >= 9
            if oob:
                actual_tile = None
                tile_hex = "OOB"
            else:
                tile_off = map_tile_offset(wi, row, col)
                actual_tile = rom[tile_off]
                tile_hex = f"${actual_tile:02X}"

            # Is the target tile a lock/gap?
            is_gap = not oob and actual_tile in GAP_TILES

            if oob:
                status = "OOB!"
                total_issues += 1
            elif is_gap:
                status = "OK"
            else:
                status = f"BAD (expected lock/gap, got ${actual_tile:02X})"
                total_issues += 1

            print(f"    slot {slot_id:2d} [{j}]  pos=({row},{col:2d})  "
                  f"actual={tile_hex:>4s}  replace=${fx['replace_tile']:02X}  "
                  f"type={fx['pat_name']:<12s}  vram=${fx['vaddr']:04X}  [{status}]")

    print(f"\n{'=' * 72}")
    if total_issues == 0:
        print("All FX slots OK — every slot points to a lock/gap tile.")
    else:
        print(f"ISSUES: {total_issues} FX slot(s) point to wrong tile or OOB position.")
    return total_issues


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <rom_file>")
        sys.exit(1)
    issues = check_rom(sys.argv[1])
    sys.exit(1 if issues > 0 else 0)
