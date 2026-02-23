#!/usr/bin/env python3
"""
SMB3 Map Walker — BFS-based map connectivity analysis with fortress progression.

Walks each world's overworld map to determine:
1. Which tiles are reachable from the start position
2. How fortress progression opens locks/bridges to expand reachable area
3. Which path tiles are chokepoints (articulation points)

Movement model (from Southbird disassembly, PRG010):
- Player moves from node to node, 2 tiles at a time
- Between nodes is a "path tile" checked against per-direction valid tile lists
- Horizontal valid: $45 $B2 $B3 $AC $B7 $B8 $DA $B9 $E6
- Vertical valid:   $46 $B1 $AA $AB $B0 $DB $BA
- Pipes create bidirectional teleport edges between two node positions

Usage:
    python3 tools/map_walker.py [rom_path]
    python3 tools/map_walker.py [rom_path] --world 1
    python3 tools/map_walker.py [rom_path] --world 6 --progression
"""

import os
import sys
from collections import deque

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

ROM_SIZE = 393232

MAP_TILE_GRIDS = [
    {"name": "World 1", "file_offset": 0x185BA, "columns": 16, "screens": 1},
    {"name": "World 2", "file_offset": 0x1864B, "columns": 32, "screens": 2},
    {"name": "World 3", "file_offset": 0x1876C, "columns": 48, "screens": 3},
    {"name": "World 4", "file_offset": 0x1891D, "columns": 32, "screens": 2},
    {"name": "World 5", "file_offset": 0x18A3E, "columns": 32, "screens": 2},
    {"name": "World 6", "file_offset": 0x18B5F, "columns": 48, "screens": 3},
    {"name": "World 7", "file_offset": 0x18D10, "columns": 32, "screens": 2},
    {"name": "World 8", "file_offset": 0x18E31, "columns": 64, "screens": 4},
]

ROWS = 9

# Per-direction valid path tiles (from Map_Object_Valid_Left/Right/Down/Up in PRG010)
VALID_HORZ = {0x45, 0xB2, 0xB3, 0xAC, 0xB7, 0xB8, 0xDA, 0xB9, 0xE6}
VALID_VERT = {0x46, 0xB1, 0xAA, 0xAB, 0xB0, 0xDB, 0xBA}

# Directions: (delta_row, delta_col, valid_set, name)
DIRECTIONS = [
    (0, +1, VALID_HORZ, "right"),
    (0, -1, VALID_HORZ, "left"),
    (+1, 0, VALID_VERT, "down"),
    (-1, 0, VALID_VERT, "up"),
]

# Start tile
TILE_START = 0xE5

# Tiles that are "background" / non-walkable (player can never stand on these)
BACKGROUND_TILES = {0xB4, 0xFF, 0x02}

# Pipe destination tables (PRG002)
PIPE_MAP_XHI = 0x046AA   # 24 bytes, packed nibbles: screen number
PIPE_MAP_X   = 0x046C2   # 24 bytes, packed nibbles: column
PIPE_MAP_Y   = 0x046DA   # 24 bytes, packed nibbles: row nibble

# Dest byte -> world index (0-based)
DEST_TO_WORLD = {
    0x01: 1,  # W2
    0x02: 5, 0x03: 5,  # W6
    0x04: 6, 0x05: 6, 0x06: 6, 0x07: 6,  # W7
    0x08: 6, 0x09: 6, 0x0A: 6, 0x0B: 6,  # W7
    0x0C: 7, 0x0D: 7, 0x0E: 7, 0x0F: 7, 0x10: 7, 0x11: 7,  # W8
    0x12: 2, 0x13: 2, 0x14: 2,  # W3
    0x15: 3, 0x16: 3,  # W4
    0x17: 4,  # W5
}

# FX table offsets (17 slots)
FX_VADDR_H         = 0x147CD
FX_VADDR_L         = 0x147DE
FX_MAP_LOC_ROW     = 0x14855
FX_MAP_LOC         = 0x14866
FX_MAP_TILE_REPLACE = 0x14877
FX_WORLD_TABLE     = 0x14888  # 32 bytes, 4 per world

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

# Known fortress entries (world_idx, entry_idx)
FORTRESS_ENTRIES = {
    (0, 11),
    (1, 13),
    (2, 13), (2, 34),
    (3, 9), (3, 16),
    (4, 12), (4, 31),
    (5, 9), (5, 27), (5, 48),
    (6, 5), (6, 40),
    (7, 7), (7, 10), (7, 26), (7, 36),
}

# Tile IDs for locks/bridges (gap tiles that block movement)
LOCK_BRIDGE_TILES = {0x54, 0x56, 0x9D, 0xE4}


# ---------------------------------------------------------------------------
# ROM helpers
# ---------------------------------------------------------------------------

def read_tile_grid(rom, world_idx):
    """Read a world's tile grid as a 2D list [row][col]."""
    info = MAP_TILE_GRIDS[world_idx]
    start = info["file_offset"]
    cols = info["columns"]

    grid = []
    for r in range(ROWS):
        row = []
        for c in range(cols):
            screen = c // 16
            col_in_screen = c % 16
            tile = rom[start + screen * 144 + r * 16 + col_in_screen]
            row.append(tile)
        grid.append(row)
    return grid


def find_start(grid):
    """Find the START tile ($E5) position in a grid."""
    for r in range(ROWS):
        for c in range(len(grid[0])):
            if grid[r][c] == TILE_START:
                return (r, c)
    return None


def read_pipe_pairs(rom):
    """Read all pipe pairs from the destination tables.

    Returns dict: world_idx -> list of ((row_a, col_a), (row_b, col_b))
    """
    pipes_by_world = {i: [] for i in range(8)}

    for dest in range(0x18):
        if dest not in DEST_TO_WORLD:
            continue
        world_idx = DEST_TO_WORLD[dest]

        xhi = rom[PIPE_MAP_XHI + dest]
        x   = rom[PIPE_MAP_X + dest]
        y   = rom[PIPE_MAP_Y + dest]

        a_scr = (xhi >> 4) & 0x0F
        b_scr = xhi & 0x0F
        a_col = (x >> 4) & 0x0F
        b_col = x & 0x0F
        a_row_nib = (y >> 4) & 0x0F
        b_row_nib = y & 0x0F

        a_grid_row = a_row_nib - 2
        b_grid_row = b_row_nib - 2
        a_grid_col = a_scr * 16 + a_col
        b_grid_col = b_scr * 16 + b_col

        pipes_by_world[world_idx].append(
            ((a_grid_row, a_grid_col), (b_grid_row, b_grid_col))
        )

    return pipes_by_world


def read_fx_slots(rom):
    """Read all 17 FX slots — position and replacement tile.

    Returns list of dicts with keys: grid_row, grid_col, screen, col_in_screen, replace_tile
    """
    slots = []
    for i in range(17):
        loc_row = rom[FX_MAP_LOC_ROW + i]
        loc = rom[FX_MAP_LOC + i]
        replace_tile = rom[FX_MAP_TILE_REPLACE + i]

        grid_row = (loc_row >> 4) - 2
        col_in_screen = (loc >> 4) & 0x0F
        screen = loc & 0x0F

        slots.append({
            "grid_row": grid_row,
            "grid_col": screen * 16 + col_in_screen,
            "screen": screen,
            "col_in_screen": col_in_screen,
            "replace_tile": replace_tile,
        })
    return slots


def read_world_fx_assignments(rom):
    """Read FortressFX_W1-W8: which FX slots each world uses.

    Returns dict: world_idx -> list of slot indices (excluding 0x00 padding and 0xFF unused).
    The list is ordered by fortress ordinal (1st fortress -> slots[0], etc.)
    """
    assignments = {}
    for wi in range(8):
        base = FX_WORLD_TABLE + wi * 4
        slots = []
        for i in range(4):
            val = rom[base + i]
            if val == 0xFF:
                continue
            # For worlds with fewer fortresses, trailing 0x00s are padding.
            # But slot 0x00 is a valid slot index (W1 uses it).
            # We use the fortress count to know how many are real.
            slots.append(val)
        assignments[wi] = slots
    return assignments


def read_fortress_positions(rom):
    """Read grid positions of all fortress entries.

    Returns dict: world_idx -> list of (grid_row, grid_col) in fortress ordinal order.
    """
    # Group fortress entries by world, preserving order
    by_world = {}
    for wi, ei in sorted(FORTRESS_ENTRIES):
        by_world.setdefault(wi, []).append(ei)

    positions = {}
    for wi, entries in by_world.items():
        world = WORLDS[wi]
        n = world["entry_count"]
        rt_off = world["rowtype_offset"]
        sc_off = rt_off + n

        pos_list = []
        for ei in entries:
            row_nib = (rom[rt_off + ei] >> 4) & 0x0F
            scrcol = rom[sc_off + ei]
            screen = (scrcol >> 4) & 0x0F
            col = scrcol & 0x0F
            grid_row = row_nib - 2
            grid_col = screen * 16 + col
            pos_list.append((grid_row, grid_col))
        positions[wi] = pos_list

    return positions


# ---------------------------------------------------------------------------
# BFS map walker
# ---------------------------------------------------------------------------

def walk_map(grid, pipe_pairs, start_pos=None):
    """BFS from the start tile, returning reachable nodes and the edges between them.

    Args:
        grid: 2D tile grid (may be modified with opened locks/bridges)
        pipe_pairs: list of ((row_a, col_a), (row_b, col_b)) pipe connections
        start_pos: optional (row, col) to start from instead of the START tile

    Returns:
        nodes: set of (row, col) positions the player can reach
        edges: dict mapping (row, col) -> list of ((row, col), path_tile_pos, path_tile_id)
        path_tiles: set of (row, col) that are path tiles used in connections
    """
    rows = len(grid)
    cols = len(grid[0])
    start = start_pos if start_pos is not None else find_start(grid)
    if start is None:
        return set(), {}, set()

    # Build pipe lookup: position -> list of destinations
    pipe_lookup = {}
    for a, b in pipe_pairs:
        pipe_lookup.setdefault(a, []).append(b)
        pipe_lookup.setdefault(b, []).append(a)

    nodes = set()
    edges = {}
    path_tiles = set()
    queue = deque([start])
    nodes.add(start)

    while queue:
        r, c = queue.popleft()
        if (r, c) not in edges:
            edges[(r, c)] = []

        # Orthogonal movement: node -> path tile -> node (2 tiles)
        for dr, dc, valid_set, _name in DIRECTIONS:
            pr, pc = r + dr, c + dc
            if pr < 0 or pr >= rows or pc < 0 or pc >= cols:
                continue
            path_tile = grid[pr][pc]
            if path_tile not in valid_set:
                continue

            nr, nc = r + 2 * dr, c + 2 * dc
            if nr < 0 or nr >= rows or nc < 0 or nc >= cols:
                continue
            dest_tile = grid[nr][nc]
            if dest_tile in BACKGROUND_TILES:
                continue

            path_tiles.add((pr, pc))
            edges[(r, c)].append(((nr, nc), (pr, pc), path_tile))

            if (nr, nc) not in nodes:
                nodes.add((nr, nc))
                queue.append((nr, nc))

        # Pipe edges: direct teleport to the other end
        if (r, c) in pipe_lookup:
            for dest in pipe_lookup[(r, c)]:
                if dest not in nodes:
                    nodes.add(dest)
                    queue.append(dest)
                edges[(r, c)].append((dest, None, "pipe"))

    return nodes, edges, path_tiles


# ---------------------------------------------------------------------------
# Fortress progression simulation
# ---------------------------------------------------------------------------

def simulate_progression(rom, world_idx, pipe_pairs):
    """Simulate fortress progression for a world.

    Iteratively:
    1. Walk the map from start
    2. Find reachable fortresses that haven't been beaten
    3. Beat the lowest-ordinal reachable fortress, opening its FX slot
    4. Replace the lock/bridge tile with the FX replacement tile
    5. Repeat until no more fortresses can be reached

    Returns list of steps, each a dict with:
        fort_idx: which fortress ordinal was beaten (or None for initial)
        fort_pos: (row, col) of fortress beaten
        fx_pos: (row, col) of lock/bridge opened
        fx_old_tile: the lock/bridge tile that was there
        fx_new_tile: the replacement tile
        nodes: set of reachable nodes after this step
        path_tiles: set of path tiles used
    """
    grid = read_tile_grid(rom, world_idx)
    fx_slots = read_fx_slots(rom)
    fx_assignments = read_world_fx_assignments(rom)
    fortress_positions = read_fortress_positions(rom)

    world_fx_slots = fx_assignments.get(world_idx, [])
    world_forts = fortress_positions.get(world_idx, [])

    # Track which fortresses have been beaten
    beaten = set()
    steps = []

    # Initial walk
    nodes, edges, path_tiles = walk_map(grid, pipe_pairs)
    steps.append({
        "fort_idx": None,
        "fort_pos": None,
        "fx_pos": None,
        "fx_old_tile": None,
        "fx_new_tile": None,
        "nodes": set(nodes),
        "path_tiles": set(path_tiles),
        "grid": [row[:] for row in grid],  # snapshot
    })

    while True:
        # Find reachable fortresses not yet beaten
        reachable_forts = []
        for i, pos in enumerate(world_forts):
            if i not in beaten and pos in nodes:
                reachable_forts.append(i)

        if not reachable_forts:
            break

        # Beat the lowest-ordinal reachable fortress
        fort_idx = reachable_forts[0]
        fort_pos = world_forts[fort_idx]
        beaten.add(fort_idx)

        # Open corresponding FX slot
        fx_pos = None
        fx_old = None
        fx_new = None
        if fort_idx < len(world_fx_slots):
            slot_idx = world_fx_slots[fort_idx]
            slot = fx_slots[slot_idx]
            fx_r, fx_c = slot["grid_row"], slot["grid_col"]
            fx_old = grid[fx_r][fx_c]
            fx_new = slot["replace_tile"]
            grid[fx_r][fx_c] = fx_new
            fx_pos = (fx_r, fx_c)

        # Re-walk with updated grid
        nodes, edges, path_tiles = walk_map(grid, pipe_pairs)
        steps.append({
            "fort_idx": fort_idx,
            "fort_pos": fort_pos,
            "fx_pos": fx_pos,
            "fx_old_tile": fx_old,
            "fx_new_tile": fx_new,
            "nodes": set(nodes),
            "path_tiles": set(path_tiles),
            "grid": [row[:] for row in grid],  # snapshot
        })

    return steps


# ---------------------------------------------------------------------------
# Articulation point detection (chokepoints)
# ---------------------------------------------------------------------------

def find_chokepoints(nodes, edges):
    """Find path tiles whose removal disconnects the node graph.

    Only considers orthogonal path tiles (not pipe edges).
    Returns set of (path_row, path_col) for each chokepoint.
    """
    if not nodes:
        return set()

    # Build adjacency: node -> list of (neighbor, path_pos_or_None)
    adj = {n: [] for n in nodes}
    for node, neighbors in edges.items():
        for dest, path_pos, path_tile in neighbors:
            adj[node].append((dest, path_pos))

    # Collect all unique path tile positions (exclude pipe edges)
    path_positions = set()
    for node, neighbors in edges.items():
        for dest, path_pos, path_tile in neighbors:
            if path_pos is not None:
                path_positions.add(path_pos)

    # A path tile is a chokepoint if removing it disconnects the node graph.
    chokepoints = set()
    start = next(iter(nodes))

    for path_pos in path_positions:
        visited = set()
        q = deque([start])
        visited.add(start)
        while q:
            n = q.popleft()
            for dest, pp in adj[n]:
                if pp == path_pos:
                    continue
                if dest not in visited:
                    visited.add(dest)
                    q.append(dest)

        if len(visited) < len(nodes):
            chokepoints.add(path_pos)

    return chokepoints


# ---------------------------------------------------------------------------
# Visualization
# ---------------------------------------------------------------------------

# ANSI color codes
RESET   = "\033[0m"
RED     = "\033[1;31m"
GREEN   = "\033[1;32m"
CYAN    = "\033[1;36m"
MAGENTA = "\033[1;35m"
YELLOW  = "\033[1;33m"
DIM     = "\033[2m"
WHITE   = "\033[1;37m"
BLUE    = "\033[1;34m"


def render_walk(grid, nodes, edges, path_tiles, chokepoints, pipe_pairs,
                fortress_positions=None, opened_positions=None):
    """Render the map with walk results overlaid, with ANSI color."""
    rows_count = len(grid)
    cols = len(grid[0])

    fort_set = set(fortress_positions) if fortress_positions else set()
    opened_set = set(opened_positions) if opened_positions else set()

    # Build set of pipe positions for display
    pipe_positions = set()
    for a, b in pipe_pairs:
        pipe_positions.add(a)
        pipe_positions.add(b)

    lines = []
    header = "     "
    for c in range(cols):
        header += "%X" % (c % 16)
    lines.append(header)
    lines.append("     " + "-" * cols)

    for r in range(rows_count):
        row_str = "r%d | " % r
        for c in range(cols):
            tile = grid[r][c]
            pos = (r, c)

            if pos in opened_set:
                row_str += BLUE + "O" + RESET  # opened lock/bridge
            elif pos in fort_set and pos in nodes:
                row_str += WHITE + "F" + RESET  # reachable fortress
            elif pos in fort_set:
                row_str += YELLOW + "F" + RESET  # unreachable fortress
            elif pos in chokepoints:
                row_str += RED + "X" + RESET
            elif pos in pipe_positions and pos in nodes:
                row_str += MAGENTA + "P" + RESET
            elif pos in nodes:
                row_str += GREEN + "*" + RESET
            elif pos in path_tiles:
                ch = "-" if tile in VALID_HORZ else "|"
                row_str += CYAN + ch + RESET
            elif tile in BACKGROUND_TILES or tile == 0x02:
                row_str += DIM + "." + RESET
            else:
                row_str += YELLOW + "~" + RESET
        lines.append(row_str)

    lines.append("")
    lines.append("  Legend: %s* node%s  %sX choke%s  %s- | path%s  %sP pipe%s  "
                 "%sF fort%s  %sO opened%s  %s~ blocked%s  %s. void%s" % (
        GREEN, RESET, RED, RESET, CYAN, RESET, MAGENTA, RESET,
        WHITE, RESET, BLUE, RESET, YELLOW, RESET, DIM, RESET))

    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    rom_path = "Super Mario Bros. 3 (USA) (Rev 1).nes"
    world_filter = None
    show_progression = False

    args = sys.argv[1:]
    i = 0
    while i < len(args):
        if args[i] == "--world" and i + 1 < len(args):
            world_filter = int(args[i + 1]) - 1
            i += 2
        elif args[i] == "--progression":
            show_progression = True
            i += 1
        elif not args[i].startswith("-"):
            rom_path = args[i]
            i += 1
        else:
            i += 1

    if not os.path.exists(rom_path):
        print("ROM not found:", rom_path)
        sys.exit(1)

    rom = open(rom_path, "rb").read()
    if len(rom) != ROM_SIZE:
        print("Unexpected ROM size:", len(rom))
        sys.exit(1)

    pipes_by_world = read_pipe_pairs(rom)
    fortress_pos = read_fortress_positions(rom)
    worlds = range(8) if world_filter is None else [world_filter]

    for wi in worlds:
        info = MAP_TILE_GRIDS[wi]
        pipe_pairs = pipes_by_world[wi]
        world_forts = fortress_pos.get(wi, [])

        print("=== %s ===" % info["name"])

        if show_progression:
            steps = simulate_progression(rom, wi, pipe_pairs)
            opened_so_far = set()

            for step_num, step in enumerate(steps):
                if step["fort_idx"] is None:
                    print("  Step %d: Initial state" % step_num)
                else:
                    fr, fc = step["fort_pos"]
                    print("  Step %d: Beat fortress #%d at (%d, %d)" % (
                        step_num, step["fort_idx"] + 1, fr, fc), end="")
                    if step["fx_pos"]:
                        fxr, fxc = step["fx_pos"]
                        print(" -> opened (%d, %d) [0x%02X -> 0x%02X]" % (
                            fxr, fxc, step["fx_old_tile"], step["fx_new_tile"]))
                        opened_so_far.add(step["fx_pos"])
                    else:
                        print(" (no FX slot)")

                print("  Reachable nodes: %d" % len(step["nodes"]))

                # Walk on snapshot grid for edges/chokepoints
                nodes, edges, path_tiles = walk_map(step["grid"], pipe_pairs)
                chokepoints = find_chokepoints(nodes, edges)

                print("  Chokepoints: %d" % len(chokepoints))
                print()
                print(render_walk(step["grid"], step["nodes"], edges,
                                  step["path_tiles"], chokepoints, pipe_pairs,
                                  fortress_positions=world_forts,
                                  opened_positions=opened_so_far))
                print()
        else:
            grid = read_tile_grid(rom, wi)
            start = find_start(grid)

            if start is None:
                print("  No START tile found!")
                print()
                continue

            print("  Start: row %d, col %d" % start)
            print("  Pipe pairs: %d" % len(pipe_pairs))
            print("  Fortresses: %d" % len(world_forts))

            nodes, edges, path_tiles = walk_map(grid, pipe_pairs)
            chokepoints = find_chokepoints(nodes, edges)

            print("  Reachable nodes: %d" % len(nodes))
            print("  Path tiles used: %d" % len(path_tiles))
            print("  Chokepoints: %d" % len(chokepoints))

            if chokepoints:
                print("  Chokepoint positions:")
                for r, c in sorted(chokepoints):
                    tile = grid[r][c]
                    print("    row %d, col %d (tile 0x%02X)" % (r, c, tile))

            print()
            print(render_walk(grid, nodes, edges, path_tiles, chokepoints,
                              pipe_pairs, fortress_positions=world_forts))
            print()


if __name__ == "__main__":
    main()
