#!/usr/bin/env python3
# pyright: basic
"""
SMB3 World Map Visualizer (Steps A1–A4)

Reads the ROM and renders each world's overworld map as ASCII art,
overlaying pointer table entries to visualize where levels, toad houses,
fortresses, and other nodes sit on the tile grid.

Usage:
    python3 tools/map_viz.py [rom_path]
    python3 tools/map_viz.py [rom_path] --world 1
    python3 tools/map_viz.py [rom_path] --raw          # show raw hex tile IDs
    python3 tools/map_viz.py [rom_path] --summary       # just show slot counts

Default ROM: "roms/Super Mario Bros. 3 (USA) (Rev 1).nes"
"""

import os
import sys
from collections import defaultdict

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

ROM_SIZE = 393232
PRG012_BASE = 0x18010  # PRG012 file offset (maps to CPU $A000)

# Map tile grid pointer table: 9 x 2-byte LE CPU pointers at file 0x185A8
MAP_TILE_GRID_PTR_TABLE = 0x185A8
MAP_TILE_GRID_ROWS = 9

# Per-world tile grid metadata (derived from pointer table analysis)
MAP_TILE_GRIDS = [
    {"name": "World 1 (Grass Land)",  "cpu_addr": 0xA5AA, "file_offset": 0x185BA, "columns": 16, "screens": 1},
    {"name": "World 2 (Desert Land)", "cpu_addr": 0xA63B, "file_offset": 0x1864B, "columns": 32, "screens": 2},
    {"name": "World 3 (Water Land)",  "cpu_addr": 0xA75C, "file_offset": 0x1876C, "columns": 48, "screens": 3},
    {"name": "World 4 (Giant Land)",  "cpu_addr": 0xA90D, "file_offset": 0x1891D, "columns": 32, "screens": 2},
    {"name": "World 5 (Sky Land)",    "cpu_addr": 0xAA2E, "file_offset": 0x18A3E, "columns": 32, "screens": 2},
    {"name": "World 6 (Ice Land)",    "cpu_addr": 0xAB4F, "file_offset": 0x18B5F, "columns": 48, "screens": 3},
    {"name": "World 7 (Pipe Land)",   "cpu_addr": 0xAD00, "file_offset": 0x18D10, "columns": 32, "screens": 2},
    {"name": "World 8 (Dark Land)",   "cpu_addr": 0xAE21, "file_offset": 0x18E31, "columns": 64, "screens": 4},
]

# Per-world pointer sub-table info
WORLDS = [
    {"name": "World 1", "rowtype_offset": 0x19438, "entry_count": 21},
    {"name": "World 2", "rowtype_offset": 0x194BA, "entry_count": 47},
    {"name": "World 3", "rowtype_offset": 0x195D8, "entry_count": 52},
    {"name": "World 4", "rowtype_offset": 0x19714, "entry_count": 34},
    {"name": "World 5", "rowtype_offset": 0x197E4, "entry_count": 42},
    {"name": "World 6", "rowtype_offset": 0x198E4, "entry_count": 57},
    {"name": "World 7", "rowtype_offset": 0x19A3E, "entry_count": 46},
    {"name": "World 8", "rowtype_offset": 0x19B56, "entry_count": 41},
]

# Known fortress entries (world_idx, entry_idx) — from levels.rs
FORTRESS_ENTRIES = {
    (0, 11), (1, 13),
    (2, 13), (2, 34),
    (3, 9), (3, 16),
    (4, 12), (4, 31),
    (5, 9), (5, 27), (5, 48),
    (6, 5), (6, 40),
    (7, 7), (7, 10), (7, 26), (7, 36),
}

# Known airship entries (world_idx, entry_idx) — from levels.rs
AIRSHIP_ENTRIES = {
    (0, 17), (1, 36), (2, 49), (3, 6), (4, 35), (5, 53), (6, 43),
}

# Bowser's castle
BOWSER_CASTLE = (7, 40)

# Map transition entries
MAP_TRANSITIONS = {(4, 5)}

# ---------------------------------------------------------------------------
# ROM reading helpers
# ---------------------------------------------------------------------------

def read_word(rom, offset):
    return rom[offset] | (rom[offset + 1] << 8)


def read_tile_grid(rom, world_idx):
    """Read a world's tile grid as a 2D list [row][col].

    ROM layout (confirmed from Map_Reload_with_Completions in prg012.asm):
    Data is stored as consecutive 144-byte screen blocks (16 cols × 9 rows,
    row-major within each block).  The loading code copies 144 bytes per
    screen with a simple sequential LDA/STA loop using the same Y index
    for source and destination, then advances the destination pointer by
    $1B0 for the next screen.

    Within each 144-byte block:
      byte_offset = row * 16 + col_within_screen

    For a world with S screens (S = cols / 16):
      tile(r, c) = rom[start + (c // 16) * 144 + r * 16 + (c % 16)]
    """
    grid_info = MAP_TILE_GRIDS[world_idx]
    start = grid_info["file_offset"]
    cols = grid_info["columns"]
    rows = MAP_TILE_GRID_ROWS

    grid = []
    for r in range(rows):
        row = []
        for c in range(cols):
            screen = c // 16
            col_in_screen = c % 16
            # Row-major per screen: 144 bytes/screen, 16 bytes/row
            tile = rom[start + screen * 144 + r * 16 + col_in_screen]
            row.append(tile)
        grid.append(row)
    return grid


def read_pointer_entries(rom, world_idx):
    """Read all pointer table entries for a world.

    Returns a list of dicts with keys:
        index, rowtype, scrcol, row_nibble, tileset, screen, column,
        obj_ptr, lay_ptr, entry_type
    """
    world = WORLDS[world_idx]
    n = world["entry_count"]
    rt_off = world["rowtype_offset"]
    sc_off = rt_off + n
    obj_off = sc_off + n
    lay_off = obj_off + n * 2

    entries = []
    for i in range(n):
        rowtype = rom[rt_off + i]
        scrcol = rom[sc_off + i]
        obj_ptr = read_word(rom, obj_off + i * 2)
        lay_ptr = read_word(rom, lay_off + i * 2)

        row_nibble = (rowtype >> 4) & 0x0F
        tileset = rowtype & 0x0F
        screen = (scrcol >> 4) & 0x0F
        column = scrcol & 0x0F

        # Classify entry type
        entry_type = classify_entry(world_idx, i, obj_ptr, lay_ptr)

        entries.append({
            "index": i,
            "rowtype": rowtype,
            "scrcol": scrcol,
            "row_nibble": row_nibble,
            "tileset": tileset,
            "screen": screen,
            "column": column,
            "obj_ptr": obj_ptr,
            "lay_ptr": lay_ptr,
            "entry_type": entry_type,
        })

    return entries


def classify_entry(world_idx, entry_idx, obj_ptr, lay_ptr):
    """Classify a pointer table entry by type."""
    key = (world_idx, entry_idx)

    if key == BOWSER_CASTLE:
        return "bowser"
    if key in FORTRESS_ENTRIES:
        return "fortress"
    if key in AIRSHIP_ENTRIES:
        return "airship"
    if key in MAP_TRANSITIONS:
        return "transition"
    if obj_ptr == 0x0700:
        return "toad"
    if obj_ptr == 0x0001 and lay_ptr == 0x0000:
        return "bonus"
    if obj_ptr >= 0xC000 and lay_ptr != 0x0000:
        # Check for duplicate detection would require a second pass;
        # we'll mark as "level" and let the caller detect hammer bros
        return "level"
    if obj_ptr < 0x1000:
        return "special"
    return "unknown"


# ---------------------------------------------------------------------------
# Coordinate mapping (confirmed from Southbird disassembly)
# ---------------------------------------------------------------------------

# The ByRowType upper nibble encodes the player's World_Map_Y position
# divided by 16.  The game's Map_GetTile function converts World_Map_Y
# to a tile-memory offset via:
#
#   offset = ((World_Map_Y - 16) & 0xF0) | column
#
# Map tiles are loaded at Tile_Mem_Addr + $110, but Map_GetTile uses
# base Tile_Mem_Addr + $100.  Setting the two equal:
#
#   row * 16 = ((row_nibble * 16 - 16) & 0xF0) - 0x10
#            = (row_nibble - 1) * 16 - 16
#   row      = row_nibble - 2
#
# Valid row_nibble range: 2–10 (0x2–0xA) → grid rows 0–8.
# Odd nibbles (3,5,7,9) map to odd grid rows (1,3,5,7) but the vanilla
# game only uses even nibbles (2,4,6,8,A).

def row_nibble_to_grid_row(row_nibble):
    """Convert a ByRowType upper-nibble value to a tile-grid row.

    Returns the grid row (0-8) or None if out of range.
    Formula: grid_row = row_nibble - 2  (derived from SMB3 disassembly).
    """
    grid_row = row_nibble - 2
    if 0 <= grid_row < MAP_TILE_GRID_ROWS:
        return grid_row
    return None


def scrcol_to_grid_col(screen, column, total_columns):
    """Convert (screen, column) to absolute grid column index.

    ByScrCol encodes (XHi << 4) | (World_Map_X >> 4).
    Each screen is 16 tile columns wide.
    """
    abs_col = screen * 16 + column
    if 0 <= abs_col < total_columns:
        return abs_col
    return None


def validate_row_mapping(rom):
    """Validate the row-2 mapping across all worlds and report stats.

    A 'hit' is when an entry's mapped grid position lands on a non-background
    tile (not 0xB4 or 0xFF).
    """
    BACKGROUND_TILES = {0xB4, 0xFF}
    hits = 0
    misses = 0
    oob = 0
    miss_details = []

    for world_idx in range(8):
        grid = read_tile_grid(rom, world_idx)
        entries = read_pointer_entries(rom, world_idx)
        cols = MAP_TILE_GRIDS[world_idx]["columns"]

        for e in entries:
            abs_col = scrcol_to_grid_col(e["screen"], e["column"], cols)
            grid_row = row_nibble_to_grid_row(e["row_nibble"])
            if abs_col is None or grid_row is None:
                oob += 1
                continue
            tile = grid[grid_row][abs_col]
            if tile not in BACKGROUND_TILES:
                hits += 1
            else:
                misses += 1
                miss_details.append((world_idx, e["index"], e["entry_type"],
                                     e["row_nibble"], grid_row, abs_col, tile))

    return hits, misses, oob, miss_details


# ---------------------------------------------------------------------------
# Tile classification
# ---------------------------------------------------------------------------

# We'll build the classification dynamically based on what we observe.
# These are seed categories based on initial ROM analysis.

# Tiles that appear under pointer table entries are "node" or "path" tiles.
# Everything else is decoration/border/background.

KNOWN_BACKGROUND = {0xB4, 0xFF}

# Border tiles (map frame)
KNOWN_BORDER = set(range(0x02, 0x0D))  # 0x02-0x0C

TILE_CATEGORY_NAMES = {
    "bg": ".",       # background/void
    "border": "B",   # map border
    "path": "#",     # walkable path without a node
    "node": "*",     # path tile with a pointer table entry on it
    "unknown": "?",  # unclassified
}


def classify_tiles(rom):
    """Build a tile_id -> category mapping based on ROM analysis.

    Phase 1: Mark tiles that appear under pointer table entries as 'node_tile'.
    Phase 2: Try to distinguish path tiles from decoration.
    """
    node_tiles = set()
    all_tiles = set()

    for world_idx in range(8):
        grid = read_tile_grid(rom, world_idx)
        entries = read_pointer_entries(rom, world_idx)
        cols = MAP_TILE_GRIDS[world_idx]["columns"]

        for r in range(MAP_TILE_GRID_ROWS):
            for c in range(cols):
                all_tiles.add(grid[r][c])

        for e in entries:
            abs_col = scrcol_to_grid_col(e["screen"], e["column"], cols)
            grid_row = row_nibble_to_grid_row(e["row_nibble"])
            if abs_col is not None and grid_row is not None:
                node_tiles.add(grid[grid_row][abs_col])

    return all_tiles, node_tiles


def build_tile_under_entries(rom):
    """For each world, find which tile ID each entry sits on.

    Returns dict: tile_id -> set of entry types that sit on it.
    """
    tile_entry_types = defaultdict(set)
    tile_counts = defaultdict(int)

    for world_idx in range(8):
        grid = read_tile_grid(rom, world_idx)
        entries = read_pointer_entries(rom, world_idx)
        cols = MAP_TILE_GRIDS[world_idx]["columns"]

        for e in entries:
            abs_col = scrcol_to_grid_col(e["screen"], e["column"], cols)
            grid_row = row_nibble_to_grid_row(e["row_nibble"])
            if abs_col is not None and grid_row is not None:
                tile = grid[grid_row][abs_col]
                tile_entry_types[tile].add(e["entry_type"])
                tile_counts[tile] += 1

    return tile_entry_types, tile_counts


# ---------------------------------------------------------------------------
# ASCII rendering
# ---------------------------------------------------------------------------

ENTRY_TYPE_CHARS = {
    "level":      "L",
    "fortress":   "F",
    "airship":    "A",
    "toad":       "T",
    "bonus":      "$",
    "bowser":     "W",  # final boss
    "special":    "!",
    "transition": "^",
    "unknown":    "?",
}


def render_world_ascii(rom, world_idx, raw_hex=False):
    """Render a world's map as ASCII art with entry overlays."""
    grid_info = MAP_TILE_GRIDS[world_idx]
    grid = read_tile_grid(rom, world_idx)
    entries = read_pointer_entries(rom, world_idx)
    cols = grid_info["columns"]

    # Build entry position map: (row, col) -> entry
    entry_map = {}
    unmapped_entries = []
    for e in entries:
        abs_col = scrcol_to_grid_col(e["screen"], e["column"], cols)
        grid_row = row_nibble_to_grid_row(e["row_nibble"])
        if abs_col is not None and grid_row is not None:
            entry_map[(grid_row, abs_col)] = e
        else:
            unmapped_entries.append(e)

    # Detect duplicate (obj, lay) pairs for hammer bros detection
    pair_counts = defaultdict(int)
    for e in entries:
        if e["entry_type"] == "level":
            pair_counts[(e["obj_ptr"], e["lay_ptr"])] += 1
    hammer_pairs = {k for k, v in pair_counts.items() if v > 1}

    lines = []
    lines.append(f"=== {grid_info['name']} ({cols} cols x {MAP_TILE_GRID_ROWS} rows, {grid_info['screens']} screen(s)) ===")
    lines.append("")

    # Column header (screen boundaries)
    header = "     "
    for c in range(cols):
        if c % 16 == 0:
            header += f"|scr{c // 16}"
            header += " " * max(0, 2 - len(str(c // 16)))
        else:
            header += "  " if raw_hex else " "
    lines.append(header)

    # Column numbers
    col_nums = "     "
    for c in range(cols):
        if raw_hex:
            col_nums += f"{c:02X}"
        else:
            col_nums += f"{c % 16:X}"
    lines.append(col_nums)

    # Separator
    sep_width = 5 + cols * (2 if raw_hex else 1)
    lines.append("     " + "-" * (cols * (2 if raw_hex else 1)))

    # Grid rows
    for r in range(MAP_TILE_GRID_ROWS):
        row_str = f"  {r}: "
        for c in range(cols):
            tile = grid[r][c]
            entry = entry_map.get((r, c))

            if raw_hex:
                if entry:
                    et = entry["entry_type"]
                    if et == "level" and (entry["obj_ptr"], entry["lay_ptr"]) in hammer_pairs:
                        et = "hammer"
                    row_str += f"\033[1;33m{tile:02X}\033[0m"  # yellow for entries
                else:
                    row_str += f"{tile:02X}"
            else:
                if entry:
                    et = entry["entry_type"]
                    if tile == 0xBC:
                        ch = "P"  # pipe (tile 0xBC renders as pipe in-game)
                    elif tile in (0x68, 0x69):
                        ch = "p"  # pyramid (tile renders as pyramid in-game)
                    elif et == "level" and (entry["obj_ptr"], entry["lay_ptr"]) in hammer_pairs:
                        ch = "H"  # hammer bros
                    else:
                        ch = ENTRY_TYPE_CHARS.get(et, "?")
                    row_str += ch
                elif tile in KNOWN_BACKGROUND:
                    row_str += "."
                elif tile in KNOWN_BORDER:
                    row_str += "b"
                elif 0x42 <= tile <= 0x4F:
                    row_str += "#"  # path-like tiles
                elif 0x50 <= tile <= 0x5F:
                    row_str += "o"  # path with features
                elif tile == 0x67:
                    row_str += "F"  # fortress (no entry)
                elif tile in (0x68, 0x69):
                    row_str += "p"  # pyramid
                elif 0x99 <= tile <= 0x9F:
                    row_str += "t"  # decoration/toad-like
                elif 0xA0 <= tile <= 0xAF:
                    row_str += "f"  # fortress structure
                elif tile == 0xB3:
                    row_str += "="  # bridge
                elif 0xC0 <= tile <= 0xCF:
                    row_str += "K"  # castle/king
                elif 0xD0 <= tile <= 0xDF:
                    row_str += "d"  # dark land
                elif 0xE0 <= tile <= 0xEF:
                    row_str += "a"  # airship/special
                else:
                    row_str += "~"  # everything else
        lines.append(row_str)

    lines.append("")

    # Entry table
    lines.append(f"  Pointer table entries ({len(entries)} total):")
    lines.append(f"  {'idx':>3s}  {'type':>10s}  row_nib  scr  col  grid(r,c)  tile  obj_ptr  lay_ptr  ts")
    lines.append(f"  {'---':>3s}  {'----':>10s}  -------  ---  ---  ---------  ----  -------  -------  --")

    for e in entries:
        abs_col = scrcol_to_grid_col(e["screen"], e["column"], cols)
        grid_row = row_nibble_to_grid_row(e["row_nibble"])
        et = e["entry_type"]
        if et == "level" and (e["obj_ptr"], e["lay_ptr"]) in hammer_pairs:
            et = "hammer"

        if abs_col is not None and grid_row is not None:
            tile = grid[grid_row][abs_col]
            grid_pos = f"({grid_row},{abs_col:2d})"
            tile_str = f"0x{tile:02X}"
        else:
            grid_pos = "  OOB  "
            tile_str = " -- "

        lines.append(
            f"  {e['index']:3d}  {et:>10s}  "
            f"  0x{e['row_nibble']:X}     {e['screen']:1d}  0x{e['column']:X}  "
            f"{grid_pos:>9s}  {tile_str}  "
            f"0x{e['obj_ptr']:04X}  0x{e['lay_ptr']:04X}  {e['tileset']:2d}"
        )

    if unmapped_entries:
        lines.append(f"\n  ⚠ {len(unmapped_entries)} entries could not be mapped to grid positions")

    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Summary / slot analysis (Step A4)
# ---------------------------------------------------------------------------

def summarize_slots(rom):
    """For each world, count occupied vs empty path-like tiles."""
    lines = []
    lines.append("=== Slot Summary ===")
    lines.append("")
    lines.append(f"{'World':<25s}  {'Entries':>7s}  {'Levels':>6s}  {'Fort':>4s}  {'Toad':>4s}  {'Other':>5s}  {'Hamm':>4s}")
    lines.append(f"{'-' * 25}  {'-' * 7}  {'-' * 6}  {'-' * 4}  {'-' * 4}  {'-' * 5}  {'-' * 4}")

    for world_idx in range(8):
        grid_info = MAP_TILE_GRIDS[world_idx]
        entries = read_pointer_entries(rom, world_idx)

        # Count by type
        type_counts = defaultdict(int)
        pair_counts = defaultdict(int)
        for e in entries:
            if e["entry_type"] == "level":
                pair_counts[(e["obj_ptr"], e["lay_ptr"])] += 1

        hammer_pairs = {k for k, v in pair_counts.items() if v > 1}
        for e in entries:
            et = e["entry_type"]
            if et == "level" and (e["obj_ptr"], e["lay_ptr"]) in hammer_pairs:
                et = "hammer"
            type_counts[et] += 1

        lines.append(
            f"{grid_info['name']:<25s}  {len(entries):7d}  "
            f"{type_counts.get('level', 0):6d}  "
            f"{type_counts.get('fortress', 0):4d}  "
            f"{type_counts.get('toad', 0):4d}  "
            f"{type_counts.get('special', 0) + type_counts.get('bonus', 0) + type_counts.get('airship', 0) + type_counts.get('bowser', 0) + type_counts.get('transition', 0):5d}  "
            f"{type_counts.get('hammer', 0):4d}"
        )

    lines.append("")

    # Tile classification report
    lines.append("=== Tile Classification ===")
    lines.append("")
    lines.append("Tiles found under pointer table entries (by entry type):")
    lines.append("")

    tile_entry_types, tile_counts = build_tile_under_entries(rom)
    for tile_id in sorted(tile_entry_types.keys()):
        types = ", ".join(sorted(tile_entry_types[tile_id]))
        lines.append(f"  0x{tile_id:02X}: {tile_counts[tile_id]:3d} entries ({types})")

    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Step A4: Empty path tile analysis
# ---------------------------------------------------------------------------

def analyze_slots(rom):
    """Step A4: For each world, identify occupied and empty node-capable positions.

    Uses the A3 tile classification (build_tile_under_entries) to determine which
    tile IDs can host a pointer table entry.  Then for each world, scans the grid
    for those tiles and reports which are occupied vs empty.
    """
    # Data-driven: any tile that appears under a pointer table entry is node-capable
    node_capable_tiles, _ = build_tile_under_entries(rom)
    node_capable_set = set(node_capable_tiles.keys())

    lines = []
    lines.append("=== Step A4: Empty Path Tile Analysis ===")
    lines.append("")
    lines.append(f"  Node-capable tiles (from A3): {len(node_capable_set)} unique tile IDs")
    lines.append(f"  {', '.join(f'0x{t:02X}' for t in sorted(node_capable_set))}")
    lines.append("")
    lines.append(f"{'World':<25s}  {'Capable':>7s}  {'Occupied':>8s}  {'Empty':>5s}  "
                 f"{'Levels':>6s}  {'Fort':>4s}  {'Toad':>4s}  {'Hamm':>4s}  {'Other':>5s}")
    lines.append(f"{'-'*25}  {'-'*7}  {'-'*8}  {'-'*5}  "
                 f"{'-'*6}  {'-'*4}  {'-'*4}  {'-'*4}  {'-'*5}")

    total_empty = 0
    total_occupied = 0

    for world_idx in range(8):
        grid_info = MAP_TILE_GRIDS[world_idx]
        grid = read_tile_grid(rom, world_idx)
        entries = read_pointer_entries(rom, world_idx)
        cols = grid_info["columns"]

        # Build set of occupied grid positions
        occupied_positions = set()
        entry_at = {}  # (row, col) -> entry type
        pair_counts = defaultdict(int)
        for e in entries:
            if e["entry_type"] == "level":
                pair_counts[(e["obj_ptr"], e["lay_ptr"])] += 1
        hammer_pairs = {k for k, v in pair_counts.items() if v > 1}

        for e in entries:
            abs_col = scrcol_to_grid_col(e["screen"], e["column"], cols)
            grid_row = row_nibble_to_grid_row(e["row_nibble"])
            if abs_col is not None and grid_row is not None:
                occupied_positions.add((grid_row, abs_col))
                et = e["entry_type"]
                if et == "level" and (e["obj_ptr"], e["lay_ptr"]) in hammer_pairs:
                    et = "hammer"
                entry_at[(grid_row, abs_col)] = et

        # Scan grid for node-capable positions
        capable_positions = set()
        empty_positions = []
        for r in range(MAP_TILE_GRID_ROWS):
            for c in range(cols):
                tile = grid[r][c]
                if tile in node_capable_set:
                    capable_positions.add((r, c))
                    if (r, c) not in occupied_positions:
                        empty_positions.append((r, c, tile))

        # Count occupied by type
        type_counts = defaultdict(int)
        for pos, et in entry_at.items():
            type_counts[et] += 1

        occupied_count = len(occupied_positions)

        lines.append(
            f"{grid_info['name']:<25s}  {len(capable_positions):7d}  {occupied_count:8d}  "
            f"{len(empty_positions):5d}  "
            f"{type_counts.get('level', 0):6d}  "
            f"{type_counts.get('fortress', 0):4d}  "
            f"{type_counts.get('toad', 0):4d}  "
            f"{type_counts.get('hammer', 0):4d}  "
            f"{type_counts.get('bonus', 0) + type_counts.get('airship', 0) + type_counts.get('bowser', 0) + type_counts.get('special', 0) + type_counts.get('transition', 0):5d}"
        )

        total_empty += len(empty_positions)
        total_occupied += occupied_count

    lines.append(f"{'-'*25}  {'-'*7}  {'-'*8}  {'-'*5}  {'-'*6}  {'-'*4}  {'-'*4}  {'-'*4}  {'-'*5}")
    lines.append(f"{'TOTAL':<25s}  {'':>7s}  {total_occupied:8d}  {total_empty:5d}")

    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Row mapping analysis
# ---------------------------------------------------------------------------

def print_mapping_validation(rom):
    """Print validation of the row-2 mapping."""
    hits, misses, oob, miss_details = validate_row_mapping(rom)
    total = hits + misses + oob

    print("=== Row Mapping Validation (grid_row = row_nibble - 2) ===")
    print()
    print(f"  Derived from SMB3 disassembly: Map_GetTile uses")
    print(f"  offset = ((World_Map_Y - 16) & 0xF0) | column")
    print(f"  with map tiles loaded at Tile_Mem_Addr + $110,")
    print(f"  base at Tile_Mem_Addr + $100 → row = row_nibble - 2")
    print()
    print(f"  Total entries:  {total}")
    print(f"  Hits (non-BG):  {hits}  ({hits/total:.1%})")
    print(f"  Misses (on BG): {misses}")
    print(f"  Out of bounds:  {oob}")
    print()

    if miss_details:
        print(f"  Entries landing on background tiles:")
        print(f"  {'World':<8s}  {'idx':>3s}  {'type':>10s}  row_nib  grid(r,c)  tile")
        print(f"  {'-----':<8s}  {'---':>3s}  {'----':>10s}  -------  ---------  ----")
        for world_idx, idx, etype, rn, gr, gc, tile in miss_details:
            print(f"  W{world_idx+1:<7d}  {idx:3d}  {etype:>10s}    0x{rn:X}    ({gr},{gc:2d})   0x{tile:02X}")
        print()


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    # Parse args
    rom_path = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes"
    world_filter = None
    raw_hex = False
    summary_only = False
    slots_only = False

    args = sys.argv[1:]
    i = 0
    while i < len(args):
        if args[i] == "--world" and i + 1 < len(args):
            world_filter = int(args[i + 1]) - 1  # 1-indexed input
            i += 2
        elif args[i] == "--raw":
            raw_hex = True
            i += 1
        elif args[i] == "--summary":
            summary_only = True
            i += 1
        elif args[i] == "--slots":
            slots_only = True
            i += 1
        elif args[i] == "--help" or args[i] == "-h":
            print(__doc__)
            sys.exit(0)
        elif not args[i].startswith("-"):
            rom_path = args[i]
            i += 1
        else:
            print(f"Unknown option: {args[i]}")
            sys.exit(1)

    # Load ROM
    if not os.path.exists(rom_path):
        print(f"ROM not found: {rom_path}")
        sys.exit(1)

    with open(rom_path, "rb") as f:
        rom = f.read()

    if len(rom) != ROM_SIZE:
        print(f"Unexpected ROM size: {len(rom)} (expected {ROM_SIZE})")
        sys.exit(1)

    # Step 1: Validate row mapping
    print_mapping_validation(rom)

    if slots_only:
        print(analyze_slots(rom))
        return

    if summary_only:
        print(summarize_slots(rom))
        return

    # Step 2: Render maps
    worlds_to_render = range(8)
    if world_filter is not None:
        if 0 <= world_filter < 8:
            worlds_to_render = [world_filter]
        else:
            print(f"Invalid world number (use 1-8)")
            sys.exit(1)

    for world_idx in worlds_to_render:
        print(render_world_ascii(rom, world_idx, raw_hex=raw_hex))
        print()

    # Step 3: Summary
    print(summarize_slots(rom))


if __name__ == "__main__":
    main()
