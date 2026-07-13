#!/usr/bin/env python3
"""Required-progression analysis for arbitrary (randomized) SMB3 ROMs.

ROM-driven port of the Rust analyzer in
src/randomize/overworld_build/progression.rs (test_required_progression),
usable on ROMs we didn't build — e.g. the Fred (fcoughlin) reference ROMs
in /fred. Per world it reports:

  - how many enterable levels / fortresses exist on the map
  - minimum fortress/level clears to reach the airship (W8: Bowser castle)
    without a hammer and with one hammer

Engine model (vanilla semantics, which Fred ROMs keep):
  - level gate: a node tile blocks through-movement until completed iff
    tile >= threshold[palette page] (thresholds 03/67/BF/E9, universal —
    see docs/smb3_rom_reference.md "World-Map Level Gating")
  - fortress FX: beating the k-th fortress in a world (any fortress) fires
    the k-th FX slot of that world's FX list, replacing one map tile
    (lock -> path, water gap -> bridge, ...)
  - hammer: breaks one rock ($51/$52). Fred ROMs (and our
    hammer_breaks_locks flag) extend this to locks ($54/$56/$E4);
    auto-detected from the Inv_UseItem_Hammer byte signature.
  - W3 canoe: stateful boat — a canoe edge is usable only when the boat
    sits at the player's dock, and riding moves the boat with the player.

Usage:
  python3 tools/required_progression.py ROM [ROM ...]
  python3 tools/required_progression.py fred/*.nes

ACCURACY LIMITATION — read before comparing our ROMs to others:
  This tool classifies nodes from the FINAL ROM's map tiles. Two of our
  default-on features disguise forced levels as non-levels on the map, so
  this tool UNDER-COUNTS them on our own output:
    - hand traps  (tile 0xE6): a forced level shown as a hazard hand.
    - troll pipes (tile 0xBC, not in the pipe tables): a level shown as a
      pipe; this tool wrongly treats it as a streak-breaking transit.
  The Rust analyzer (progression.rs / test_required_progression) measures
  the pre-writer BuiltWorld, where these are still Level slots, so it is
  the source of truth for OUR ROMs. This Python tool is accurate for ROMs
  WITHOUT those disguises (vanilla, Fred/fcoughlin), which is what it's for
  (measuring external ROMs the Rust builder never produced).
"""

import heapq
import os
import sys
from collections import Counter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rom_map as rm

# Per-palette-page level-gate thresholds (file 0x18410, universal).
THRESHOLDS = [0x03, 0x67, 0xBF, 0xE9]

TILE_FORT = 0x67
TILE_AIRSHIP = 0xC9
TILE_BOWSER = 0xCC
ROCK_TILES = {0x51, 0x52}
LOCK_TILES = {0x54, 0x56, 0xE4}
# Enterable level-like tiles that do NOT gate movement (special-entry):
# spiral castles ($5F/$DF), dark-land unnumbered level ($E6).
SPECIAL_LEVEL_TILES = {0x5F, 0xDF, 0xE6}

# Blocking path tile -> its opened/broken form (Map_Removable_Tiles /
# Map_RemoveTo_Tiles at 0x18447/0x1844F).
REMOVE_TO = {0x51: 0x45, 0x52: 0x46, 0x54: 0x46, 0x67: 0x60,
             0xEB: 0xE3, 0xE4: 0xDA, 0x56: 0x45, 0x9D: 0xB3}

# W3 canoe docks (boat starts at the first mainland dock).
CANOE_EDGES_BY_WORLD = {
    2: [((6, 20), (5, 24)), ((6, 20), (0, 32))],
}

# Map-object IDs that force level entry when the player lands on them
# (Map_Object_Stationary): piranha plant, W8 navy/tank/air force.
AUTO_ENTER_OBJ_IDS = {0x07, 0x0D, 0x0E, 0x0F}

# Vanilla fortress enemy-data pointers and the file offset of each
# fortress's Boom-Boom Y byte (rom_data/tables.rs). The Y upper nibble is
# the fortress's 1-based FX ordinal: beating the fortress fires FX slot
# FX_WORLD_TABLE[world*4 + ordinal-1]. Level shuffles move pointer-table
# entries, not the enemy data, so the obj_ptr identifies the fortress in
# any shuffled ROM (ours and Fred's alike).
FORTRESS_OBJ_PTRS = [
    0xD32B, 0xD222, 0xD393, 0xD362, 0xD508, 0xD528, 0xD3D0, 0xD2B4,
    0xD4B0, 0xCAAB, 0xD470, 0xD4E4, 0xD41B, 0xD8CC, 0xD867, 0xD551,
    0xD91C,
]
BOOMBOOM_Y_OFFSETS = [
    0x0D35F, 0x0D262, 0x0D3D3, 0x0D3A1, 0x0D536, 0x0D55F, 0x0D40F,
    0x0D2C7, 0x0D4E1, 0x0CAE1, 0x0D4B0, 0x0D4FA, 0x0D47E, 0x0DA32,
    0x0DA37, 0x0D597, 0x0DA2D,
]
BOOMBOOM_Y_BY_OBJ = dict(zip(FORTRESS_OBJ_PTRS, BOOMBOOM_Y_OFFSETS))


def entry_obj_ptr(rom, world_idx, entry_idx):
    w = rm.WORLDS[world_idx]
    n = w["entry_count"]
    objsets = w["rowtype_offset"] + 2 * n
    return rom[objsets + entry_idx * 2] | (rom[objsets + entry_idx * 2 + 1] << 8)


def fort_ordinals(rom, world_idx):
    """{grid_pos: fx_ordinal} for every fortress-entry in the world.
    Ordinal k >= 1 means beating this fort fires the world's k-th FX slot.
    Covers both 0x67 fortress tiles and the W8 armies (navy/tank/air
    force), which are fortress entries too."""
    out = {}
    for i in range(rm.WORLDS[world_idx]["entry_count"]):
        obj = entry_obj_ptr(rom, world_idx, i)
        y_off = BOOMBOOM_Y_BY_OBJ.get(obj)
        if y_off is not None:
            out[rm.entry_grid_position(rom, world_idx, i)] = rom[y_off] >> 4
    return out

# Map-object master pointer tables (PRG011).
MAP_OBJ_YS, MAP_OBJ_XHIS, MAP_OBJ_XLOS, MAP_OBJ_IDS = (
    0x16020, 0x16030, 0x16040, 0x16050)


def forced_object_positions(rom, world_idx):
    """Grid positions of stationary auto-enter map objects (plants/armies)."""
    def ptr(master):
        off = master + world_idx * 2
        return 0x16010 + (rom[off] | (rom[off + 1] << 8)) - 0xA000
    ys, xhis, xlos, ids = (ptr(MAP_OBJ_YS), ptr(MAP_OBJ_XHIS),
                           ptr(MAP_OBJ_XLOS), ptr(MAP_OBJ_IDS))
    out = set()
    for s in range(9):
        if rom[ids + s] in AUTO_ENTER_OBJ_IDS:
            row = (rom[ys + s] >> 4) - 2
            col = rom[xhis + s] * 16 + (rom[xlos + s] >> 4)
            if row >= 0:
                out.add((row, col))
    return out


def node_kinds(rom, world_idx):
    """{grid_pos: entry_type} via rom_map's classifier — 'level', 'fortress',
    'pipe', 'hammer_bro', 'toad_house', 'bonus_game', etc. Used to decide
    which route nodes are streak-breaking activities."""
    _, lookup = rm.build_entry_lookup(rom, world_idx)
    return {pos: e["type"] for pos, e in lookup.items()}


def spiral_transit_pairs(rom, world_idx):
    """Spiral-castle transit teleports (e.g. W5 tower). Riding one means
    clearing the tower level, so these edges cost 1. Returns
    {(a, b), (b, a), ...} plus a map pos -> spiral tile pos to tally."""
    pairs = set()
    tally = {}
    for wi, from_idx, to_idx in rm.SPECIAL_TRANSITIONS:
        if wi != world_idx:
            continue
        a = rm.entry_grid_position(rom, world_idx, from_idx)
        b = rm.entry_grid_position(rom, world_idx, to_idx)
        pairs.update([(a, b), (b, a)])
        tally[(a, b)] = a
        tally[(b, a)] = a
    return pairs, tally


def gated(tile):
    return tile >= THRESHOLDS[tile >> 6]


def detect_hammer_breaks_locks(rom):
    """Vanilla Inv_UseItem_Hammer checks `tile - $51 < 2` (rocks only).
    Fred ROMs replace the compare with a JSR to a routine that also
    accepts $54/$56/$E4. Any deviation from the vanilla bytes is treated
    as 'hammers break locks too'."""
    return bytes(rom[0x346D5:0x346D8]) != b"\xE9\x51\xC9"


def entry_positions(rom, world_idx):
    w = rm.WORLDS[world_idx]
    return {rm.entry_grid_position(rom, world_idx, i)
            for i in range(w["entry_count"])}


def fx_requirements(rom, world_idx, ordinals):
    """{fx_target_pos: (fx_index, replace_tile)} for this world's FX slots.

    Only FX indices some fortress actually fires (its ordinal) are
    included — trailing world-table bytes are zero-filled and would
    otherwise alias FX slot 0."""
    slots = rm.read_fx_slots(rom)
    base = rm.FX_WORLD_TABLE + world_idx * 4
    fired = {k - 1 for k in ordinals if k >= 1}
    out = {}
    for j in range(4):
        if j not in fired:
            continue
        s = slots[rom[base + j]]
        out[(s["grid_row"], s["grid_col"])] = (j, s["replace_tile"])
    return out


def build_edges(grid, fx_req):
    """Edge list over the fully-open grid.

    Returns dict pos -> [(dest, path_pos, orig_path_tile)].  path_pos is
    None for teleport edges added later (pipes/canoes are handled by the
    caller).  Edges through blocked tiles (rocks/locks/FX targets) are
    included; the Dijkstra decides whether they are usable in a state.
    """
    rows, cols = len(grid), len(grid[0])

    def open_tile(r, c):
        t = grid[r][c]
        if (r, c) in fx_req:
            return fx_req[(r, c)][1]
        return REMOVE_TO.get(t, t)

    edges = {}
    for r in range(rows):
        for c in range(cols):
            t = grid[r][c]
            if t in rm.BACKGROUND_TILES:
                continue
            for dr, dc, valid in ((0, 1, rm.VALID_HORZ), (0, -1, rm.VALID_HORZ),
                                  (1, 0, rm.VALID_VERT), (-1, 0, rm.VALID_VERT)):
                pr, pc = r + dr, c + dc
                nr, nc = r + 2 * dr, c + 2 * dc
                if not (0 <= nr < rows and 0 <= nc < cols):
                    continue
                if open_tile(pr, pc) not in valid:
                    continue
                if grid[nr][nc] in rm.BACKGROUND_TILES:
                    continue
                edges.setdefault((r, c), []).append(
                    ((nr, nc), (pr, pc), grid[pr][pc]))
    return edges


def analyze_world(rom, world_idx, hammer_locks, hammer_target=None):
    """Min-clears Dijkstra. Returns dict or None if target unreachable."""
    grid = rm.read_tile_grid(rom, world_idx)
    rows, cols = len(grid), len(grid[0])
    start = rm.find_start(grid)
    target_tile = TILE_BOWSER if world_idx == 7 else TILE_AIRSHIP
    target = next(((r, c) for r in range(rows) for c in range(cols)
                   if grid[r][c] == target_tile), None)
    if start is None or target is None:
        return None

    ford = fort_ordinals(rom, world_idx)  # pos -> FX ordinal (0 = no FX)
    forts = sorted(ford)
    fort_bit = {pos: 1 << i for i, pos in enumerate(forts)}
    # For each FX index, the fort bits that fire it when beaten.
    fx_firers = {}
    for pos, k in ford.items():
        if k >= 1:
            fx_firers.setdefault(k - 1, 0)
            fx_firers[k - 1] |= fort_bit[pos]
    fx_req = fx_requirements(rom, world_idx, ford.values())
    entries = entry_positions(rom, world_idx)
    edges = build_edges(grid, fx_req)
    forced = forced_object_positions(rom, world_idx)
    spiral_pairs, spiral_tally = spiral_transit_pairs(rom, world_idx)
    kinds = node_kinds(rom, world_idx)

    pipe_lookup = {}
    for a, b in rm.read_pipe_pairs(rom)[world_idx]:
        pipe_lookup.setdefault(a, []).append(b)
        pipe_lookup.setdefault(b, []).append(a)

    canoe_edges = CANOE_EDGES_BY_WORLD.get(world_idx, [])
    boat0 = canoe_edges[0][0] if canoe_edges else None

    def usable(path_pos, orig, mask):
        if path_pos == hammer_target:
            return True
        if orig in ROCK_TILES:
            return False  # only the hammer opens rocks
        if path_pos in fx_req:
            j = fx_req[path_pos][0]
            return bool(mask & fx_firers.get(j, 0))
        return True  # plain path tile

    def node_cost(pos, mask):
        """(cost, new_mask, passable). Charges 1 for gated/enterable nodes."""
        t = grid[pos[0]][pos[1]]
        if pos == target:
            return 1, mask, True
        if pos in ford:
            bit = fort_bit[pos]
            if mask & bit:
                return 0, mask, True
            return 1, mask | bit, True
        if pos in forced:
            return 1, mask, True  # stationary plant/army: landing = playing
        if gated(t):
            if pos in entries:
                return 1, mask, True
            return 0, mask, False  # gated scenery with no entry: wall
        return 0, mask, True

    dist = {}
    prev = {}
    initial = (start, 0, boat0)
    dist[initial] = 0
    heap = [(0, initial)]
    goal = None
    while heap:
        cost, state = heapq.heappop(heap)
        if cost > dist.get(state, 1 << 30):
            continue
        pos, mask, boat = state
        if pos == target:
            goal = state
            break

        def relax(dest, boat_after, extra=0):
            c, new_mask, ok = node_cost(dest, mask)
            if not ok:
                return
            key = (dest, new_mask, boat_after)
            if cost + c + extra < dist.get(key, 1 << 30):
                dist[key] = cost + c + extra
                prev[key] = state
                heapq.heappush(heap, (cost + c + extra, key))

        for dest, path_pos, orig in edges.get(pos, ()):
            if usable(path_pos, orig, mask):
                relax(dest, boat)
        for dest in pipe_lookup.get(pos, ()):
            # Spiral-castle transits are levels: riding costs a clear.
            relax(dest, boat, extra=1 if (pos, dest) in spiral_pairs else 0)
        if boat == pos:
            for a, b in canoe_edges:
                if pos in (a, b):
                    dest = b if pos == a else a
                    relax(dest, dest)

    if goal is None:
        return None

    # Reconstruct; tally distinct fort/level positions (target excluded).
    chain = [goal]
    while chain[-1] in prev:
        chain.append(prev[chain[-1]])
    chain.reverse()
    fseen, lseen = set(), set()
    # Streak = longest run of back-to-back forced *level* plays with no other
    # activity between them. A fortress, pipe (ride or node), hammer-bro, or
    # crossed lock resets the run; toad-house/spade/walk pass through. Mirrors
    # the Rust analyzer in progression.rs.
    streak = max_streak = goal_stack = 0
    for i, (pos, _, _) in enumerate(chain):
        reset = added_level = False
        if i > 0:
            prev = chain[i - 1][0]
            hop = (prev, pos)
            if hop in spiral_tally:                 # riding a spiral tower level
                lseen.add(spiral_tally[hop])
                added_level = True
            elif pos in pipe_lookup.get(prev, ()):  # pipe ride
                reset = True
            else:                                   # walk hop: crossed a lock?
                for dest, path_pos, _ in edges.get(prev, ()):
                    if dest == pos and path_pos in fx_req:
                        reset = True
                        break
        if pos == target:
            if reset:
                streak = 0
            goal_stack = streak
            continue
        if pos != start:
            if pos in ford:
                fseen.add(pos)
                reset = True                        # fortress boss
            elif pos in forced or (gated(grid[pos[0]][pos[1]]) and pos in entries):
                lseen.add(pos)
                added_level = True
            elif kinds.get(pos) == "pipe":
                reset = True                        # pipe ride (deterministic)
            # hammer_bro / toad_house / bonus_game / walk tiles: transparent.
            # A hammer-bro *node* isn't a fight — the wandering sprite (dynamic,
            # unmodeled) triggers battles, so a spriteless HB node is empty path.
        if reset:
            streak = 0
        if added_level:
            streak += 1
            max_streak = max(max_streak, streak)
    return {"forts": len(fseen), "levels": len(lseen),
            "streak": max_streak, "goal_stack": goal_stack,
            "path": [p for p, _, _ in chain]}


def hammer_candidates(rom, world_idx, hammer_locks):
    grid = rm.read_tile_grid(rom, world_idx)
    breakable = ROCK_TILES | (LOCK_TILES if hammer_locks else set())
    return [(r, c) for r in range(len(grid)) for c in range(len(grid[0]))
            if grid[r][c] in breakable]


def count_world(rom, world_idx):
    """Level/fort census + map-topology metrics, with everything open
    (all FX fired, all rocks/locks broken, canoe free)."""
    grid = rm.read_tile_grid(rom, world_idx)
    ford = fort_ordinals(rom, world_idx)
    fx_req = fx_requirements(rom, world_idx, ford.values())
    # Open every removable path tile, then plain BFS for reachability.
    open_grid = [row[:] for row in grid]
    for r in range(len(grid)):
        for c in range(len(grid[0])):
            if (r, c) in fx_req:
                open_grid[r][c] = fx_req[(r, c)][1]
            else:
                open_grid[r][c] = REMOVE_TO.get(grid[r][c], grid[r][c])
    pipes = rm.read_pipe_pairs(rom)[world_idx]
    nodes, edges, _, _ = rm.walk_map(open_grid, pipes)
    entries = entry_positions(rom, world_idx)
    forced = forced_object_positions(rom, world_idx)
    levels = sum(1 for (r, c) in nodes
                 if (r, c) not in ford
                 and (((r, c) in forced)
                      or ((r, c) in entries
                          and (gated(grid[r][c]) or grid[r][c] in SPECIAL_LEVEL_TILES)
                          and grid[r][c] not in (TILE_AIRSHIP, TILE_BOWSER))))
    forts_reach = sum(1 for f in ford if f in nodes)

    # Topology: undirected edge count, fork nodes (degree >= 3), and
    # independent cycles (E - N + components) — 0 for a pure tree/line,
    # each +1 is one genuinely alternative route.
    und = set()
    for a, dests in edges.items():
        for dest, _, _ in dests:
            und.add(frozenset((a, dest)))
    degree = Counter()
    for e in und:
        for p in e:
            degree[p] += 1
    forks = sum(1 for _, d in degree.items() if d >= 3)
    cycles = len(und) - len(nodes) + 1  # walk_map graph is connected

    # Physical level-adjacency: level nodes joined by a single walk edge
    # (not a pipe/canoe teleport). Route-independent "levels side by side".
    ford = fort_ordinals(rom, world_idx)
    level_pos = {(r, c) for (r, c) in nodes
                 if (r, c) not in ford
                 and (((r, c) in forced)
                      or ((r, c) in entries
                          and (gated(grid[r][c]) or grid[r][c] in SPECIAL_LEVEL_TILES)
                          and grid[r][c] not in (TILE_AIRSHIP, TILE_BOWSER)))}
    adj = set()
    for a, dests in edges.items():
        if a not in level_pos:
            continue
        for dest, path_pos, _ in dests:
            if path_pos is not None and dest in level_pos:
                adj.add(frozenset((a, dest)))
    return {"levels": levels, "forts": forts_reach,
            "forks": forks, "cycles": cycles, "adj": len(adj)}


def analyze_rom(path):
    rom = open(path, "rb").read()
    hammer_locks = detect_hammer_breaks_locks(rom)
    results = []
    for wi in range(8):
        census = count_world(rom, wi)
        no_h = analyze_world(rom, wi, hammer_locks)
        best_h = no_h
        for cand in hammer_candidates(rom, wi, hammer_locks):
            r = analyze_world(rom, wi, hammer_locks, hammer_target=cand)
            if r and (best_h is None
                      or r["forts"] + r["levels"] < best_h["forts"] + best_h["levels"]):
                best_h = r
        results.append({**census, "no_hammer": no_h, "hammer": best_h})
    return hammer_locks, results


def fmt_clears(x):
    if x is None:
        return "UNREACHABLE"
    return f"{x['forts']}F+{x['levels']}L={x['forts'] + x['levels']:2}"


def main():
    paths = sys.argv[1:]
    if not paths:
        print(__doc__)
        sys.exit(1)

    agg = [{"levels": [], "clears_nh": [], "clears_h": [], "req_ratio": [],
            "opt": [], "streak": [], "goalstk": [], "adj": []} for _ in range(8)]
    for path in paths:
        hammer_locks, results = analyze_rom(path)
        name = os.path.basename(path)
        print(f"\n=== {name} ===")
        print(f"  hammer breaks locks: {'yes' if hammer_locks else 'no (rocks only)'}")
        print(f"  {'':4} {'lvls':>4} {'forts':>5} | {'no hammer':>12} | {'1 hammer':>12} |"
              f" {'req%':>5} {'opt':>4} {'strk':>5} {'gstk':>5} {'adj':>4}")
        tot_l = tot_nh = tot_h = 0
        for wi, r in enumerate(results):
            nh, h = r["no_hammer"], r["hammer"]
            # Linearity: share of the world's levels that are mandatory
            # (no-hammer). 100% = pure corridor, low = lots of choice.
            ratio = (nh["levels"] / r["levels"] * 100
                     if nh and r["levels"] else float("nan"))
            opt = r["levels"] - nh["levels"] if nh else 0
            s_nh = nh["streak"] if nh else 0
            gstk = nh["goal_stack"] if nh else 0
            print(f"  W{wi + 1}  {r['levels']:>4} {r['forts']:>5} |"
                  f" {fmt_clears(nh):>12} | {fmt_clears(h):>12} |"
                  f" {ratio:>4.0f}% {opt:>4} {s_nh:>5} {gstk:>5} {r['adj']:>4}")
            tot_l += r["levels"]
            if nh:
                tot_nh += nh["forts"] + nh["levels"]
                agg[wi]["clears_nh"].append(nh["forts"] + nh["levels"])
                agg[wi]["streak"].append(s_nh)
                agg[wi]["goalstk"].append(gstk)
                agg[wi]["opt"].append(opt)
                if r["levels"]:
                    agg[wi]["req_ratio"].append(nh["levels"] / r["levels"] * 100)
            if h:
                tot_h += h["forts"] + h["levels"]
                agg[wi]["clears_h"].append(h["forts"] + h["levels"])
            agg[wi]["levels"].append(r["levels"])
            agg[wi]["adj"].append(r["adj"])
        print(f"  total levels on maps: {tot_l}   "
              f"min clears to finish: {tot_nh} (no hammer) / {tot_h} (hammers)")

    if len(paths) > 1:
        def mmm(v, f=".1f"):
            if not v:
                return "-"
            return f"{min(v):.0f}/{sum(v) / len(v):{f}}/{max(v):.0f}"
        print(f"\n=== Aggregate over {len(paths)} ROMs (min/mean/max) ===")
        print(f"  {'':4} {'levels':>12} | {'clears nh':>12} | {'clears 1h':>12} |"
              f" {'req%':>12} | {'streak':>12} | {'goalstk':>12} | {'adj-pairs':>12}")
        all_streak, all_gs = [], []
        for wi in range(8):
            a = agg[wi]
            all_streak += a["streak"]
            all_gs += a["goalstk"]
            print(f"  W{wi + 1}  {mmm(a['levels']):>12} | {mmm(a['clears_nh']):>12} |"
                  f" {mmm(a['clears_h']):>12} | {mmm(a['req_ratio'], '.0f'):>12} |"
                  f" {mmm(a['streak']):>12} | {mmm(a['goalstk']):>12} | {mmm(a['adj']):>12}")
        if all_streak:
            ge2 = sum(1 for s in all_streak if s >= 2) / len(all_streak) * 100
            ge3 = sum(1 for s in all_streak if s >= 3) / len(all_streak) * 100
            gs2 = sum(1 for g in all_gs if g >= 2) / len(all_gs) * 100
            mean_s = sum(all_streak) / len(all_streak)
            print(f"  overall: mean streak {mean_s:.2f}  |  "
                  f"≥2 in {ge2:.0f}% of worlds, ≥3 in {ge3:.0f}%  |  "
                  f"goal-stack≥2 in {gs2:.0f}%")


if __name__ == "__main__":
    main()
