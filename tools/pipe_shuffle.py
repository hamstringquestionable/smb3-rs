#!/usr/bin/env python3
"""
SMB3 Pipe Shuffle Prototype — progressive pipe placement using the map walker.

Prototype for W2 first (1 pipe pair), then expand to other worlds.

Algorithm:
1. Collect all "swappable" node positions (action levels, toad houses, pipes)
2. Remove pipe tiles from map (swap pipe entries with non-pipe entries)
3. Walk from start to find reachable area
4. Find unreachable node positions
5. Place pipe pair connecting reachable <-> unreachable
6. Re-walk and repeat until fully connected

Usage:
    python3 tools/pipe_shuffle.py [rom_path] --world 2
"""

import os
import random
import sys
from collections import deque

# Import walker functions
sys.path.insert(0, os.path.dirname(__file__))
from map_walker import (
    BACKGROUND_TILES,
    DEST_TO_WORLD,
    DIRECTIONS,
    FORTRESS_ENTRIES,
    MAP_TILE_GRIDS,
    PIPE_MAP_X,
    PIPE_MAP_XHI,
    PIPE_MAP_Y,
    ROWS,
    VALID_HORZ,
    VALID_VERT,
    find_chokepoints,
    find_start,
    read_pipe_pairs,
    read_tile_grid,
    render_walk,
    walk_map,
)

ROM_SIZE = 393232

# Pointer table info per world
WORLDS = [
    {"rowtype_offset": 0x19438, "entry_count": 21},
    {"rowtype_offset": 0x194BA, "entry_count": 47},
    {"rowtype_offset": 0x195D8, "entry_count": 52},
    {"rowtype_offset": 0x19714, "entry_count": 34},
    {"rowtype_offset": 0x197E4, "entry_count": 42},
    {"rowtype_offset": 0x198E4, "entry_count": 57},
    {"rowtype_offset": 0x19A3E, "entry_count": 46},
    {"rowtype_offset": 0x19B56, "entry_count": 41},
]

# Known airship entries (world_idx, entry_idx)
AIRSHIP_ENTRIES = {
    (0, 17), (1, 36), (2, 49), (3, 6), (4, 35), (5, 53), (6, 43),
}

# Bowser's castle: W8 entry 40 at (5,60) tile 0xCC
BOWSER_ENTRY = (7, 40)

# Map transition entries
MAP_TRANSITIONS = {(4, 5)}

# Tile IDs for node types
TILE_PIPE = 0xBC
TILE_FORTRESS = 0x67
TILE_TOAD = 0x50

# Level panel tiles
LEVEL_PANEL_TILES = set(range(0x03, 0x0D))  # 0x03-0x0C


# ---------------------------------------------------------------------------
# ROM reading helpers
# ---------------------------------------------------------------------------

def read_word(rom, offset):
    return rom[offset] | (rom[offset + 1] << 8)


def read_all_entries(rom, world_idx):
    """Read all pointer table entries for a world.

    Returns list of dicts with: index, tileset, row_nib, screen, col,
    grid_row, grid_col, obj_ptr, lay_ptr, tile (map tile at position).
    """
    world = WORLDS[world_idx]
    n = world["entry_count"]
    rt = world["rowtype_offset"]
    sc = rt + n
    obj = sc + n
    lay = obj + n * 2

    grid = read_tile_grid(rom, world_idx)
    entries = []

    for i in range(n):
        rowtype = rom[rt + i]
        scrcol = rom[sc + i]
        row_nib = (rowtype >> 4) & 0x0F
        tileset = rowtype & 0x0F
        screen = (scrcol >> 4) & 0x0F
        col = scrcol & 0x0F
        obj_ptr = read_word(rom, obj + i * 2)
        lay_ptr = read_word(rom, lay + i * 2)

        grid_row = row_nib - 2
        grid_col = screen * 16 + col

        tile = None
        if 0 <= grid_row < ROWS and 0 <= grid_col < len(grid[0]):
            tile = grid[grid_row][grid_col]

        entries.append({
            "index": i,
            "tileset": tileset,
            "row_nib": row_nib,
            "screen": screen,
            "col": col,
            "grid_row": grid_row,
            "grid_col": grid_col,
            "obj_ptr": obj_ptr,
            "lay_ptr": lay_ptr,
            "tile": tile,
        })

    return entries


def classify_entry(world_idx, entry):
    """Classify a pointer table entry."""
    i = entry["index"]
    obj = entry["obj_ptr"]
    lay = entry["lay_ptr"]

    if (world_idx, i) in FORTRESS_ENTRIES:
        return "fortress"
    if (world_idx, i) in AIRSHIP_ENTRIES:
        return "airship"
    if (world_idx, i) in MAP_TRANSITIONS:
        return "transition"
    if obj == 0x0700:
        return "toad"
    if obj == 0x0001 and lay == 0x0000:
        return "bonus"
    if entry["tileset"] == 14:
        return "pipe"
    if obj >= 0xC000 and lay != 0x0000:
        return "level"
    return "other"


def get_pipe_pairs_for_world(rom, world_idx):
    """Get paired pipe entries (not solos) for a world.

    Returns list of (entry_a, entry_b) where both share obj_ptr.
    """
    entries = read_all_entries(rom, world_idx)
    pipe_entries = [e for e in entries if classify_entry(world_idx, e) == "pipe"]

    # Group by obj_ptr
    by_obj = {}
    for e in pipe_entries:
        by_obj.setdefault(e["obj_ptr"], []).append(e)

    pairs = []
    for obj_ptr, group in sorted(by_obj.items()):
        if len(group) == 2:
            pairs.append((group[0], group[1]))
        # Skip solos (warp zone etc.)

    return pairs


def get_swappable_positions(rom, world_idx):
    """Get all node positions that can participate in pipe swaps.

    Returns list of (grid_row, grid_col, entry_index, entry_type, tile).
    Excludes: airships, transitions, bonuses, hammerbros, "other".
    """
    entries = read_all_entries(rom, world_idx)

    # Detect hammer bros (duplicate obj+lay pairs)
    pair_counts = {}
    for e in entries:
        if e["obj_ptr"] >= 0xC000 and e["lay_ptr"] != 0x0000:
            key = (e["obj_ptr"], e["lay_ptr"])
            pair_counts[key] = pair_counts.get(key, 0) + 1
    hammer_pairs = {k for k, v in pair_counts.items() if v > 1}

    # Find start position to exclude it
    grid = read_tile_grid(rom, world_idx)
    start_pos = find_start(grid)

    positions = []
    for e in entries:
        etype = classify_entry(world_idx, e)
        if etype in ("airship", "transition", "bonus", "other"):
            continue
        if (e["obj_ptr"], e["lay_ptr"]) in hammer_pairs:
            continue
        if e["grid_row"] < 0 or e["grid_row"] >= ROWS:
            continue
        # Never place a pipe on the START tile
        if start_pos and (e["grid_row"], e["grid_col"]) == start_pos:
            continue

        positions.append({
            "grid_row": e["grid_row"],
            "grid_col": e["grid_col"],
            "entry_index": e["index"],
            "entry_type": etype,
            "tile": e["tile"],
            "entry": e,
        })

    return positions


def get_dest_indices_for_world(world_idx):
    """Get pipe destination table indices for a given world."""
    return [d for d, w in DEST_TO_WORLD.items() if w == world_idx]


def get_must_reach_positions(rom, world_idx):
    """Get positions that MUST be reachable: airships and Bowser's castle.

    Returns set of (grid_row, grid_col).
    """
    entries = read_all_entries(rom, world_idx)
    must_reach = set()

    for e in entries:
        key = (world_idx, e["index"])
        if key in AIRSHIP_ENTRIES or key == BOWSER_ENTRY:
            if e["grid_row"] >= 0 and e["grid_row"] < ROWS:
                must_reach.add((e["grid_row"], e["grid_col"]))

    return must_reach


def find_unreachable_components(grid, reachable, all_node_positions):
    """Find connected components among unreachable nodes using the walker.

    Walk from each unvisited unreachable node (no pipes) to find which
    unreachable nodes can reach each other via paths alone.

    Returns list of sets, each set being a connected component of node positions.
    """
    unreachable = all_node_positions - reachable
    if not unreachable:
        return []

    visited = set()
    components = []

    for start_node in unreachable:
        if start_node in visited:
            continue
        # BFS from this node using only grid paths (no pipes)
        component_nodes, _, _ = walk_map(grid, [], start_pos=start_node)
        component = set(component_nodes) & all_node_positions
        visited |= component
        components.append(component)

    return components


# ---------------------------------------------------------------------------
# Grid manipulation
# ---------------------------------------------------------------------------

def grid_set_tile(grid, row, col, tile):
    """Set a tile in the grid (mutable)."""
    grid[row][col] = tile


def make_mutable_grid(rom, world_idx):
    """Read grid as mutable (list of lists)."""
    return read_tile_grid(rom, world_idx)


# ---------------------------------------------------------------------------
# Progressive pipe placement
# ---------------------------------------------------------------------------

def place_pipes_progressive(rom, world_idx, seed=42):
    """Progressively place pipe pairs using the walker.

    1. Start with pipes removed (swapped to nearby positions)
    2. Walk to find reachable set
    3. Place pipes to connect reachable to unreachable
    4. Repeat until connected

    Returns the modified grid and pipe pair positions.
    """
    rng = random.Random(seed)
    grid = make_mutable_grid(rom, world_idx)
    pipe_pairs_data = get_pipe_pairs_for_world(rom, world_idx)
    positions = get_swappable_positions(rom, world_idx)
    dest_indices = get_dest_indices_for_world(world_idx)

    if not pipe_pairs_data:
        print("  No pipe pairs to shuffle")
        return grid, []

    print("  Pipe pairs to place: %d" % len(pipe_pairs_data))
    print("  Swappable positions: %d" % len(positions))
    print("  Dest table indices: %s" % dest_indices)

    # Step 0: Open all fortress-gated gaps so the walker sees full connectivity.
    # This simulates "all fortresses beaten" for pipe placement purposes.
    # - Locks ($54) become vertical path ($46)
    # - Bridges ($56) become horizontal path ($45)
    # - Water gaps ($9D) become bridge ($B3) if they sit between path/node tiles
    # - Sky gaps ($E4) become horizontal path ($45) — rare, W5 only
    cols = len(grid[0])
    for r in range(ROWS):
        for c in range(cols):
            t = grid[r][c]
            if t == 0x54:  # lock -> vert path
                grid_set_tile(grid, r, c, 0x46)
            elif t == 0x56:  # bridge -> horz path
                grid_set_tile(grid, r, c, 0x45)
            elif t == 0xE4:  # sky gap -> horz path
                grid_set_tile(grid, r, c, 0x45)
            elif t == 0x9D:  # water gap -> bridge, but only if it's on a path
                # Check if this water tile connects nodes/paths horizontally or vertically
                is_gap = False
                # Horizontal: node/path on left AND right (2 tiles away)
                if c >= 1 and c + 1 < cols:
                    left = grid[r][c - 1]
                    right = grid[r][c + 1]
                    if (left not in BACKGROUND_TILES and left != 0x9D and
                            right not in BACKGROUND_TILES and right != 0x9D):
                        is_gap = True
                # Vertical: node/path above AND below (2 tiles away)
                if r >= 1 and r + 1 < ROWS:
                    above = grid[r - 1][c]
                    below = grid[r + 1][c]
                    if (above not in BACKGROUND_TILES and above != 0x9D and
                            below not in BACKGROUND_TILES and below != 0x9D):
                        is_gap = True
                if is_gap:
                    grid_set_tile(grid, r, c, 0xB3)  # bridge tile (horz walkable)

    # Step 1: Remove all pipe tiles from grid, replace with path tile
    pipe_positions_orig = set()
    for pa, pb in pipe_pairs_data:
        for p in (pa, pb):
            r, c = p["grid_row"], p["grid_col"]
            pipe_positions_orig.add((r, c))
            # Replace pipe with a generic node tile that connects to adjacent paths
            replacement = infer_node_tile(grid, r, c)
            grid_set_tile(grid, r, c, replacement)

    # Collect non-pipe node positions and pipe entry positions
    non_pipe_positions = [p for p in positions if p["entry_type"] != "pipe"]
    pipe_entry_positions = [p for p in positions if p["entry_type"] == "pipe"]

    # All candidate positions = all swappable positions (now with pipes removed from grid)
    all_node_positions = [(p["grid_row"], p["grid_col"]) for p in positions]

    # Get must-reach positions (airships, Bowser) for this world
    must_reach = get_must_reach_positions(rom, world_idx)
    if must_reach:
        print("  Must-reach positions: %s" % must_reach)

    # Step 2: Walk with no pipes to find initial reachable set
    nodes, edges, path_tiles = walk_map(grid, [])
    reachable = set(nodes)

    print("  Initial reachable (no pipes): %d nodes" % len(reachable))

    # Step 3: Progressively place pipe pairs
    placed_pairs = []
    remaining_pairs = list(range(len(pipe_pairs_data)))
    rng.shuffle(remaining_pairs)

    # Find all node positions
    all_nodes_on_map = set()
    for p in positions:
        all_nodes_on_map.add((p["grid_row"], p["grid_col"]))

    used_positions = set()

    for pair_idx in remaining_pairs:
        available_nodes = all_nodes_on_map - used_positions
        unreachable_nodes = available_nodes - reachable
        reachable_available = available_nodes & reachable

        if not unreachable_nodes:
            print("  All nodes reachable! Placing remaining pipe %d at random positions" % pair_idx)
            candidates = list(reachable_available)
            if len(candidates) >= 2:
                rng.shuffle(candidates)
                a_pos = candidates[0]
                b_pos = candidates[1]
                placed_pairs.append((a_pos, b_pos))
                used_positions.add(a_pos)
                used_positions.add(b_pos)
                grid_set_tile(grid, a_pos[0], a_pos[1], TILE_PIPE)
                grid_set_tile(grid, b_pos[0], b_pos[1], TILE_PIPE)
            else:
                print("  WARNING: Not enough available positions for pipe pair!")
            continue

        # Prioritize unreachable components containing must-reach positions
        unreachable_must_reach = must_reach - reachable
        if unreachable_must_reach:
            # Find which unreachable nodes are in the same component as a must-reach
            components = find_unreachable_components(grid, reachable, all_nodes_on_map)
            priority_nodes = set()
            for comp in components:
                if comp & unreachable_must_reach:
                    priority_nodes |= (comp & unreachable_nodes)
            if priority_nodes:
                unreachable_candidates = list(priority_nodes)
                print("  [priority] Targeting must-reach component (%d candidates)" % len(unreachable_candidates))
            else:
                unreachable_candidates = list(unreachable_nodes)
        else:
            unreachable_candidates = list(unreachable_nodes)

        reachable_candidates = list(reachable_available)

        if not reachable_candidates:
            print("  WARNING: No reachable candidates for pipe placement!")
            break

        rng.shuffle(reachable_candidates)
        rng.shuffle(unreachable_candidates)

        a_pos = reachable_candidates[0]
        b_pos = unreachable_candidates[0]

        print("  Placing pipe pair: (%d,%d) <-> (%d,%d)" % (
            a_pos[0], a_pos[1], b_pos[0], b_pos[1]))

        placed_pairs.append((a_pos, b_pos))
        used_positions.add(a_pos)
        used_positions.add(b_pos)
        grid_set_tile(grid, a_pos[0], a_pos[1], TILE_PIPE)
        grid_set_tile(grid, b_pos[0], b_pos[1], TILE_PIPE)

        # Re-walk with new pipe
        nodes, edges, path_tiles = walk_map(grid, placed_pairs)
        reachable = set(nodes)
        print("    Now reachable: %d nodes" % len(reachable))

    # Final check: are all must-reach positions reachable?
    unreachable_must = must_reach - reachable
    if unreachable_must:
        print("  ERROR: Must-reach positions still unreachable: %s" % unreachable_must)
    else:
        print("  OK: All must-reach positions are reachable")

    return grid, placed_pairs


def infer_node_tile(grid, row, col):
    """Infer a reasonable node tile for a position based on adjacent path tiles.

    If there's a horizontal path adjacent, use a level panel tile.
    Otherwise use a generic panel tile.
    """
    cols = len(grid[0])

    # Check if horizontal or vertical paths are adjacent
    has_horz = False
    has_vert = False
    for dr, dc, valid_set, name in DIRECTIONS:
        ar, ac = row + dr, col + dc
        if 0 <= ar < ROWS and 0 <= ac < cols:
            if grid[ar][ac] in VALID_HORZ:
                has_horz = True
            if grid[ar][ac] in VALID_VERT:
                has_vert = True

    # Use a generic level panel tile — 0x47 is a common junction tile
    # that appears throughout the game at path intersections
    return 0x47


# ---------------------------------------------------------------------------
# ROM patching — update pointer tables and destination tables
# ---------------------------------------------------------------------------

PIPE_MAP_SCRL_XHI = 0x046F2


def grid_pos_to_rowtype_scrcol(grid_row, grid_col, tileset):
    """Convert grid position + tileset to ByRowType and ByScrCol bytes."""
    row_nib = grid_row + 2
    screen = grid_col // 16
    col = grid_col % 16
    rowtype = (row_nib << 4) | (tileset & 0x0F)
    scrcol = (screen << 4) | (col & 0x0F)
    return rowtype, scrcol


def grid_pos_to_dest_nibbles(grid_row, grid_col):
    """Convert grid position to pipe dest table nibble values.

    Returns (xhi_nib, x_nib, y_nib) — single nibble each.
    """
    row_nib = grid_row + 2
    screen = grid_col // 16
    col = grid_col % 16
    return screen, col, row_nib


def match_pairs_to_dests(rom, world_idx, pipe_pairs_data):
    """Match pipe pair entries to dest table indices.

    Returns list of (dest_idx, entry_a, entry_b) where entry_a is the "A" (upper
    nibble) endpoint and entry_b is "B" (lower nibble).
    """
    dests = sorted([d for d, w in DEST_TO_WORLD.items() if w == world_idx])
    matches = []

    for d in dests:
        xhi = rom[PIPE_MAP_XHI + d]
        x = rom[PIPE_MAP_X + d]
        y = rom[PIPE_MAP_Y + d]

        a_pos = ((y >> 4) - 2, (xhi >> 4) * 16 + (x >> 4))
        b_pos = ((y & 0xF) - 2, (xhi & 0xF) * 16 + (x & 0xF))

        for ea, eb in pipe_pairs_data:
            ea_pos = (ea["grid_row"], ea["grid_col"])
            eb_pos = (eb["grid_row"], eb["grid_col"])
            if ea_pos == a_pos and eb_pos == b_pos:
                matches.append((d, ea, eb))
                break
            elif ea_pos == b_pos and eb_pos == a_pos:
                matches.append((d, eb, ea))
                break

    return matches


def swap_entry_positions(rom, world_idx, idx_a, idx_b):
    """Swap the map positions of two pointer table entries.

    Swaps ByRowType (preserving each entry's tileset) and ByScrCol,
    plus the tile grid tiles at their positions.
    """
    world = WORLDS[world_idx]
    n = world["entry_count"]
    rt = world["rowtype_offset"]
    sc = rt + n
    grid_offset = MAP_TILE_GRIDS[world_idx]["file_offset"]

    # Read current values
    a_rowtype = rom[rt + idx_a]
    a_scrcol = rom[sc + idx_a]
    b_rowtype = rom[rt + idx_b]
    b_scrcol = rom[sc + idx_b]

    # Extract row and tileset separately — tileset stays with the entry,
    # row/screen/col get swapped
    a_row_nib = (a_rowtype >> 4) & 0x0F
    a_tileset = a_rowtype & 0x0F
    b_row_nib = (b_rowtype >> 4) & 0x0F
    b_tileset = b_rowtype & 0x0F

    # Swap: A gets B's position (but keeps A's tileset), B gets A's position
    rom[rt + idx_a] = (b_row_nib << 4) | a_tileset
    rom[sc + idx_a] = b_scrcol
    rom[rt + idx_b] = (a_row_nib << 4) | b_tileset
    rom[sc + idx_b] = a_scrcol

    # Swap tiles in the grid
    a_grid_row = a_row_nib - 2
    a_screen = (a_scrcol >> 4) & 0x0F
    a_col = a_scrcol & 0x0F
    a_grid_col = a_screen * 16 + a_col

    b_grid_row = b_row_nib - 2
    b_screen = (b_scrcol >> 4) & 0x0F
    b_col = b_scrcol & 0x0F
    b_grid_col = b_screen * 16 + b_col

    # Grid is stored per-screen: screen * 144 + row * 16 + col_in_screen
    a_rom_off = grid_offset + a_screen * 144 + a_grid_row * 16 + a_col
    b_rom_off = grid_offset + b_screen * 144 + b_grid_row * 16 + b_col

    rom[a_rom_off], rom[b_rom_off] = rom[b_rom_off], rom[a_rom_off]


def apply_pipe_shuffle_to_rom(rom, world_idx, pipe_pairs_data, placed_pairs):
    """Patch the ROM with new pipe positions using full entry swaps.

    For each pipe endpoint that moves to a new position:
    1. Find the entry currently at the target position
    2. Swap the two entries' positions (ByRowType/ByScrCol + tile grid)
    3. Update the pipe destination tables for the new pipe locations

    This ensures displaced entries (levels, toad houses, etc.) move to
    the pipe's old position rather than being overwritten.
    """
    world = WORLDS[world_idx]
    n = world["entry_count"]
    rt = world["rowtype_offset"]
    sc = rt + n

    # Match original pairs to dest indices
    dest_matches = match_pairs_to_dests(rom, world_idx, pipe_pairs_data)

    if len(dest_matches) != len(placed_pairs):
        print("  WARNING: dest match count (%d) != placed pairs (%d)" % (
            len(dest_matches), len(placed_pairs)))
        return

    # Build a live position -> entry index lookup (updated after each swap)
    pos_to_entry = {}
    for i in range(n):
        rowtype = rom[rt + i]
        scrcol = rom[sc + i]
        row_nib = (rowtype >> 4) & 0x0F
        screen = (scrcol >> 4) & 0x0F
        col = scrcol & 0x0F
        grid_row = row_nib - 2
        grid_col = screen * 16 + col
        pos_to_entry[(grid_row, grid_col)] = i

    for (dest_idx, orig_a, orig_b), (new_a_pos, new_b_pos) in zip(dest_matches, placed_pairs):
        pipe_a_idx = orig_a["index"]
        pipe_b_idx = orig_b["index"]

        # Find current position of pipe A entry (may have moved from earlier swaps)
        cur_a_rt = rom[rt + pipe_a_idx]
        cur_a_sc = rom[sc + pipe_a_idx]
        cur_a_row = ((cur_a_rt >> 4) & 0x0F) - 2
        cur_a_col = ((cur_a_sc >> 4) & 0x0F) * 16 + (cur_a_sc & 0x0F)
        cur_a_pos = (cur_a_row, cur_a_col)

        # Swap pipe A to new_a_pos if not already there
        if cur_a_pos != new_a_pos:
            target_idx = pos_to_entry.get(new_a_pos)
            if target_idx is not None:
                swap_entry_positions(rom, world_idx, pipe_a_idx, target_idx)
                # Update lookup
                pos_to_entry[new_a_pos] = pipe_a_idx
                pos_to_entry[cur_a_pos] = target_idx
                print("  Swap: pipe A entry[%d] (%d,%d) <-> entry[%d] (%d,%d)" % (
                    pipe_a_idx, cur_a_pos[0], cur_a_pos[1],
                    target_idx, new_a_pos[0], new_a_pos[1]))
            else:
                print("  WARNING: No entry at target position (%d,%d) for pipe A" % new_a_pos)

        # Find current position of pipe B entry
        cur_b_rt = rom[rt + pipe_b_idx]
        cur_b_sc = rom[sc + pipe_b_idx]
        cur_b_row = ((cur_b_rt >> 4) & 0x0F) - 2
        cur_b_col = ((cur_b_sc >> 4) & 0x0F) * 16 + (cur_b_sc & 0x0F)
        cur_b_pos = (cur_b_row, cur_b_col)

        # Swap pipe B to new_b_pos if not already there
        if cur_b_pos != new_b_pos:
            target_idx = pos_to_entry.get(new_b_pos)
            if target_idx is not None:
                swap_entry_positions(rom, world_idx, pipe_b_idx, target_idx)
                pos_to_entry[new_b_pos] = pipe_b_idx
                pos_to_entry[cur_b_pos] = target_idx
                print("  Swap: pipe B entry[%d] (%d,%d) <-> entry[%d] (%d,%d)" % (
                    pipe_b_idx, cur_b_pos[0], cur_b_pos[1],
                    target_idx, new_b_pos[0], new_b_pos[1]))
            else:
                print("  WARNING: No entry at target position (%d,%d) for pipe B" % new_b_pos)

        # Update pipe destination table with final positions
        a_xhi, a_x, a_y = grid_pos_to_dest_nibbles(new_a_pos[0], new_a_pos[1])
        b_xhi, b_x, b_y = grid_pos_to_dest_nibbles(new_b_pos[0], new_b_pos[1])

        rom[PIPE_MAP_XHI + dest_idx] = (a_xhi << 4) | b_xhi
        rom[PIPE_MAP_X + dest_idx] = (a_x << 4) | b_x
        rom[PIPE_MAP_Y + dest_idx] = (a_y << 4) | b_y
        rom[PIPE_MAP_SCRL_XHI + dest_idx] = (a_xhi << 4) | b_xhi

        print("  Patched dest %d: A->(%d,%d) B->(%d,%d)" % (
            dest_idx, new_a_pos[0], new_a_pos[1], new_b_pos[0], new_b_pos[1]))

    # Re-sort the entire pointer table by (screen, row, col).
    # The game's lookup scans entries per-screen, matching row first then col,
    # so entries MUST be sorted by screen, then row_nib, then scrcol.
    resort_pointer_table(rom, world_idx)


def resort_pointer_table(rom, world_idx):
    """Re-sort all pointer table entries for a world by (screen, row, col).

    The game looks up entries by scanning from InitIndex[screen], matching
    row first then column. Entries must be sorted: grouped by screen, then
    by row_nib within each screen, then by column.

    This also rewrites the InitIndex table with correct per-screen offsets.
    """
    world = WORLDS[world_idx]
    n = world["entry_count"]
    rt = world["rowtype_offset"]
    sc = rt + n
    obj = sc + n
    lay = obj + n * 2

    # InitIndex is stored just before ByRowType in the ROM
    # Master table at 0x193DA has 9 word pointers into the sub-tables
    INIT_INDEX_MASTER = 0x193DA
    init_ptr_lo = rom[INIT_INDEX_MASTER + world_idx * 2]
    init_ptr_hi = rom[INIT_INDEX_MASTER + world_idx * 2 + 1]
    init_cpu = (init_ptr_hi << 8) | init_ptr_lo
    init_file = 0x18010 + (init_cpu - 0x8000)

    # Number of screens for this world
    info = MAP_TILE_GRIDS[world_idx]
    num_screens = info["screens"]

    # Read all entries
    entries = []
    for i in range(n):
        rowtype = rom[rt + i]
        scrcol = rom[sc + i]
        obj_lo = rom[obj + i * 2]
        obj_hi = rom[obj + i * 2 + 1]
        lay_lo = rom[lay + i * 2]
        lay_hi = rom[lay + i * 2 + 1]

        row_nib = (rowtype >> 4) & 0x0F
        screen = (scrcol >> 4) & 0x0F
        col = scrcol & 0x0F

        entries.append({
            "rowtype": rowtype,
            "scrcol": scrcol,
            "obj_lo": obj_lo, "obj_hi": obj_hi,
            "lay_lo": lay_lo, "lay_hi": lay_hi,
            "screen": screen,
            "row_nib": row_nib,
            "col": col,
        })

    # Sort by (screen, row_nib, col)
    entries.sort(key=lambda e: (e["screen"], e["row_nib"], e["col"]))

    # Write back sorted entries
    for i, e in enumerate(entries):
        rom[rt + i] = e["rowtype"]
        rom[sc + i] = e["scrcol"]
        rom[obj + i * 2] = e["obj_lo"]
        rom[obj + i * 2 + 1] = e["obj_hi"]
        rom[lay + i * 2] = e["lay_lo"]
        rom[lay + i * 2 + 1] = e["lay_hi"]

    # Rebuild InitIndex: one byte per screen, value = offset of first entry on that screen
    for s in range(num_screens):
        offset = 0
        for i, e in enumerate(entries):
            if e["screen"] == s:
                offset = i
                break
        rom[init_file + s] = offset

    print("  Re-sorted %d entries by (screen, row, col)" % n)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    rom_path = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes"
    output_path = None
    world_filter = None
    seed = 42

    args = sys.argv[1:]
    i = 0
    while i < len(args):
        if args[i] == "--world" and i + 1 < len(args):
            world_filter = int(args[i + 1]) - 1
            i += 2
        elif args[i] == "--seed" and i + 1 < len(args):
            seed = int(args[i + 1])
            i += 2
        elif args[i] == "--output" and i + 1 < len(args):
            output_path = args[i + 1]
            i += 2
        elif not args[i].startswith("-"):
            rom_path = args[i]
            i += 1
        else:
            i += 1

    if not os.path.exists(rom_path):
        print("ROM not found:", rom_path)
        sys.exit(1)

    rom = bytearray(open(rom_path, "rb").read())
    if len(rom) != ROM_SIZE:
        print("Unexpected ROM size:", len(rom))
        sys.exit(1)

    worlds = [world_filter] if world_filter is not None else list(range(8))

    for wi in worlds:
        info = MAP_TILE_GRIDS[wi]
        print("=== %s ===" % info["name"])

        pipe_pairs_data = get_pipe_pairs_for_world(rom, wi)
        grid, placed_pairs = place_pipes_progressive(rom, wi, seed=seed)

        # Show result
        print("\n  Final pipe pairs: %d" % len(placed_pairs))
        for a, b in placed_pairs:
            print("    (%d,%d) <-> (%d,%d)" % (a[0], a[1], b[0], b[1]))

        # Walk final state and render
        nodes, edges, path_tiles = walk_map(grid, placed_pairs)
        chokepoints = find_chokepoints(nodes, edges)

        print("  Final reachable: %d nodes" % len(nodes))
        print("  Chokepoints: %d" % len(chokepoints))
        print()
        print(render_walk(grid, nodes, edges, path_tiles, chokepoints, placed_pairs))
        print()

        # Apply ROM patches if outputting
        if output_path and placed_pairs:
            apply_pipe_shuffle_to_rom(rom, wi, pipe_pairs_data, placed_pairs)

    if output_path:
        with open(output_path, "wb") as f:
            f.write(rom)
        print("Patched ROM written to: %s" % output_path)


if __name__ == "__main__":
    main()
