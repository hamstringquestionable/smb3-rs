# Overworld Shuffle — Logic & Rules

## 1. Goal

Shuffle the world map layouts so that each playthrough has a visually different overworld with level nodes in new positions, while preserving completability. This is distinct from existing **level shuffle** (which swaps what level data a map node points to) — overworld shuffle changes the map itself.

---

## 2. Scope of "Overworld Shuffle"

There are several possible interpretations, from simplest to most complex:

| Approach | Description | Complexity |
|----------|-------------|------------|
| **A. Swap entire world maps** | Reassign which tile grid is used for each world (e.g., play World 3's map layout during World 1) | Low |
| **B. Shuffle node positions within a world** | Keep the same map grid but move level/toad/fortress nodes to different path tiles | Medium |
| **C. Regenerate path networks** | Procedurally generate new path layouts and place nodes on them | Very High |
| **D. Shuffle map segments** | Cut world maps into connected path segments and reassemble them | High |

**Recommended first target: Approach B** — shuffle node positions within each world's existing path network. This preserves the visual identity and connectivity of each map while giving a fresh feel. Approach A is trivial but less interesting. Approaches C and D require solving graph generation problems and extensive map tile encoding knowledge.

---

## 3. World Map Data Structures

### 3.1 Map Tile Grids (PRG012: 0x185A8–0x19101)

**Pointer table:** 9 × 2-byte little-endian CPU pointers at file offset **0x185A8** (CPU $A598).
Each points to a world's tile grid data in PRG012.

| World | CPU Addr | File Offset | Columns | Rows | Data Size | Screens |
|-------|----------|-------------|---------|------|-----------|---------|
| W1 | $A5AA | 0x185BA | 16 | 9 | 144 + 1 | 1 |
| W2 | $A63B | 0x1864B | 32 | 9 | 288 + 1 | 2 |
| W3 | $A75C | 0x1876C | 48 | 9 | 432 + 1 | 3 |
| W4 | $A90D | 0x1891D | 32 | 9 | 288 + 1 | 2 |
| W5 | $AA2E | 0x18A3E | 32 | 9 | 288 + 1 | 2 |
| W6 | $AB4F | 0x18B5F | 48 | 9 | 432 + 1 | 3 |
| W7 | $AD00 | 0x18D10 | 32 | 9 | 288 + 1 | 2 |
| W8 | $AE21 | 0x18E31 | 64 | 9 | 576 + 1 | 4 |
| Warp | $B062 | 0x19072 | — | 9 | — | — |

**Storage format:** Row-major per screen (confirmed from `Map_Reload_with_Completions` in prg012.asm). Each screen is a 144-byte block of 9 rows × 16 columns, stored row-major (16 consecutive bytes per row). Multi-screen worlds have consecutive 144-byte blocks. A `0xFF` terminator byte follows each world's grid data. Total tile data spans **0x185BA–0x19101** (~2.9 KB).

**Addressing:** Tile at grid (row R, column C) is at file offset:
```
world_start + (C // 16) * 144 + R * 16 + (C % 16)
```

The game's `Map_Reload_with_Completions` loads each 144-byte screen block with a sequential `LDA [src],Y / STA [dst],Y` loop (Y = 0..143), then advances the destination pointer by $1B0 for the next screen (the gap accommodates unused vertical space in tile memory).

**Tile IDs:** ~36 unique tile IDs appear under pointer table entries (confirmed via 100% hit rate mapping). Key categories from tile classification:

| Tile ID(s) | Category | Notes |
|------------|----------|-------|
| 0x03–0x0C | Level panel tiles | Border-range tiles reused as level dots (entries land here) |
| 0x44 | Path tile | Horizontal path segment |
| 0x47, 0x48, 0x4A, 0x4B | Path tiles | Various directional path segments (most common under entries) |
| 0x50 | Toad house / special | Toad houses and special map nodes |
| 0x5F | Path tile | Rare path variant |
| 0x67 | Fortress tile | Mini-fortress entrance |
| 0x68, 0x69 | Pipe tiles | Map pipe connectors |
| 0xAE, 0xAF | Fortress parts | Alternate fortress tiles |
| 0xB4 | Background (void) | Empty space / water fill (NO entries land here) |
| 0xB5, 0xBB, 0xBC | Path/level tiles | Various level-associated tiles |
| 0xC9 | Airship dock | Airship landing tile |
| 0xCC | Bowser's castle | Final castle tile |
| 0xD9, 0xDC–0xDE | Dark Land tiles | W8-specific level tiles |
| 0xE0 | Special node | Alternate toad house / special |
| 0xE5, 0xE6 | Level tiles | Various level entries |
| 0xE8 | Bonus game tile | Spade panel / N-Spade |
| 0xEB | Fortress tile | Alternate fortress |
| 0xFF | Border / unused | Map edge |

**Path tile directional encoding (TODO):** Each path tile ID encodes which directions Mario can walk from it (up, down, left, right). The exact encoding must be extracted from the Southbird disassembly's map movement routines. This is critical for verifying connectivity after any shuffle.

### 3.2 Level Pointer Tables (PRG012: 0x19434–0x19C4C)

Five master pointer tables (9 × 2-byte pointers each) at:

| Table | File Offset | Description |
|-------|-------------|-------------|
| `Map_ByXHi_InitIndex` | 0x193DA | Per-screen starting index into sub-tables |
| `Map_ByRowType` | 0x193EC | Row position (upper nibble) + tileset (lower nibble) |
| `Map_ByScrCol` | 0x193FE | Screen (upper nibble) + column (lower nibble) |
| `Map_ObjSets` | 0x19410 | Enemy/object data pointer per entry |
| `Map_LevelLayouts` | 0x19422 | Level layout data pointer per entry |

Each master table entry is a CPU pointer to that world's sub-table.

**Per-world sub-tables (contiguous):**

For a world with N entries:
```
[ByXHi_InitIndex: ? bytes (1 per screen)]
[ByRowType: N bytes]
[ByScrCol: N bytes]
[ObjSets: N × 2 bytes]
[LevelLayouts: N × 2 bytes]
```

**Entry counts per world:**

| World | Entries | ByRowType Offset |
|-------|---------|-----------------|
| W1 | 21 | 0x19438 |
| W2 | 47 | 0x194BA |
| W3 | 52 | 0x195D8 |
| W4 | 34 | 0x19714 |
| W5 | 42 | 0x197E4 |
| W6 | 57 | 0x198E4 |
| W7 | 46 | 0x19A3E |
| W8 | 41 | 0x19B56 |

**Map position matching (confirmed from disassembly):** When Mario stands on a map tile and presses A, `Map_PrepareLevel` searches the current world's pointer table. It compares `(ByRowType & 0xF0)` against `World_Map_Y` (the player's Y position byte), then `ByScrCol` against `(World_Map_XHi << 4) | (World_Map_X >> 4)`. The matched entry's `ObjSets` and `LevelLayouts` pointers determine what level loads.

**Coordinate mapping (confirmed — 100% hit rate across all 340 entries):**

The `ByRowType` upper nibble ("row_nibble") maps to tile grid rows via:

```
grid_row = row_nibble - 2
```

Derivation: Map tiles are loaded at `Tile_Mem_Addr + $110`, but `Map_GetTile` uses base `Tile_Mem_Addr + $100`. The tile offset is `((World_Map_Y - 16) & 0xF0) | column`. With `World_Map_Y = row_nibble × 16`:

```
tile_offset = ((row_nibble * 16 - 16) & 0xF0) | col
map_data_offset = tile_offset - $10 = (row_nibble - 2) * 16 + col
→ grid_row = row_nibble - 2
```

| row_nibble | World_Map_Y | grid_row |
|-----------|-------------|----------|
| 0x2 | 0x20 | 0 |
| 0x3 | 0x30 | 1 |
| 0x4 | 0x40 | 2 |
| 0x5 | 0x50 | 3 |
| 0x6 | 0x60 | 4 |
| 0x7 | 0x70 | 5 |
| 0x8 | 0x80 | 6 |
| 0x9 | 0x90 | 7 |
| 0xA | 0xA0 | 8 |

The vanilla game only uses even row_nibble values (2, 4, 6, 8, A) → even grid rows (0, 2, 4, 6, 8). Odd grid rows (1, 3, 5, 7) contain path/decoration but no enterable nodes.

The `ByScrCol` byte maps to tile grid columns via:
```
screen = ByScrCol >> 4
column = ByScrCol & 0x0F
grid_col = screen * 16 + column
```

**InitIndex:** The `Map_ByXHi_InitIndex` sub-table has 1 byte per screen, telling the game which entry index to start searching from for that screen. This is an optimization — must be updated if entries are reordered.

### 3.3 Map Scroll Limits (PRG010)

**`World_Map_Max_PanR`** at file offset **0x14F44** (8 bytes):

```
W1=0x10, W2=0x20, W3=0x30, W4=0x30, W5=0x00, W6=0x30, W7=0x20, W8=0x00
```

Units: 0x10 = 1 screen of scroll. Value = max rightward scroll distance.
- 0x00 → 1 screen (no scroll)
- 0x10 → 2 screens
- 0x20 → 3 screens
- 0x30 → 4 screens

**Note:** Some worlds' Max_PanR doesn't match their tile grid column count (e.g., W4 has Max_PanR=0x30 (4 screens) but only 32 columns (2 screens) of tile data; W8 has Max_PanR=0x00 (1 screen) but 64 columns (4 screens) of tile data). This needs investigation — W5 and W8 may use special scrolling modes (W5 is split ground/sky, W8 is a linear stage sequence).

### 3.4 Map Objects (PRG011: 0x16010–0x1800F)

Pointer tables indexed by `World_Num` (8 entries each):

| Table | Description |
|-------|-------------|
| `Map_List_Object_Ys` | Y coordinates of map objects |
| `Map_List_Object_XHis` | X high bytes (screen number) |
| `Map_List_Object_XLos` | X low bytes (position within screen) |
| `Map_List_Object_IDs` | Object type (Hammer Bro, Airship, HELP bubble, etc.) |
| `Map_List_Object_Items` | Item carried by object |

**Up to 9 objects per world.** Object types include roaming Hammer Bros (3–5 per world), the Airship, and the HELP bubble.

### 3.5 Fortress Lock & Bridge FX (PRG010: 0x147CD–0x148B7)

17 FX slots (0x00–0x10) controlling lock-open and bridge-build animations when fortresses are cleared. Each slot has:

- **VRAM address** (where to write replacement tiles in video RAM)
- **Map tile position** (row, screen, column — where to replace the map tile)
- **Replacement tile** ($45=bridge, $46=open path, $B3=water bridge, $DA=sky bridge)
- **Map_Completions bit** (which completion flag to set so the change persists)

These are **hard-wired to specific map tile positions**. If fortress positions move, ALL of these tables must be recomputed.

### 3.6 Player Starting Position

| Data | Location | Notes |
|------|----------|-------|
| `Map_Y_Starts` | PRG010 (offset TBD) | 8 bytes, one Y coord per world |
| X start | Fixed at 0x20 | Same for all worlds |

### 3.7 World BGM Table

**File offset 0x3C424** (PRG030), 9 bytes: music track per world (1–8 + warp zone).

### 3.8 Airship Travel Data

Per-world airship travel destination tables in PRG011. Each world has 3 sets of 6 Y/X coordinate pairs controlling where the airship flies after the player dies.

---

## 4. Entry Types on the Map

Each pointer table entry represents one "interactive tile" on the map. Entry types (determined by `ObjSets` and `LevelLayouts` pointer values):

| Type | Detection | Shuffleable? | Notes |
|------|-----------|-------------|-------|
| **Action level** | `obj ≥ $C000, lay ≠ $0000` | ✅ Yes | Regular playable level |
| **Fortress** | Action level + Boom-Boom in sub-areas | ❌ No | Tied to FX/lock system |
| **Airship** | Hardcoded entry indices | ❌ No | Autoscroll patch dependencies |
| **Bowser's Castle** | W8 index 40 | ❌ No | Game ending trigger |
| **Toad House** | `obj == $0700` | ❌ Separate | Item reward, not a level |
| **Bonus game** | `obj == $0001, lay == $0000` | ❌ No | N-Spade / card matching |
| **Hammer Bros** | Duplicate (obj, lay) pairs | ❌ No | Roaming; position = map object |
| **Hand trap** | Small obj values | ❌ No | W8 hand traps |
| **Pipe connector** | Short levels (< 3 screens) | ❌ No | Map-to-map pipe warp |
| **Map transition** | Hardcoded (W5 index 5) | ❌ No | Ground-to-sky transition |

---

## 5. Approach B: Shuffle Node Positions

### 5.1 Concept

Within each world's existing tile grid:
1. Identify all **path tiles** that host level/toad/fortress nodes
2. Identify all **empty path tiles** (path tiles with no node)
3. Shuffle which path positions get which nodes
4. Update the pointer tables' `ByRowType` / `ByScrCol` to reflect new positions
5. Update `InitIndex` per-screen tables
6. Update visual map tiles (swap node tile ↔ plain path tile)
7. Update fortress FX tables if fortress positions changed
8. Update map objects (Hammer Bros / Airship positions)

### 5.2 Constraints

#### Hard Constraints (must satisfy or game breaks)

1. **Every level node must be on a path tile.** The game matches Mario's position to pointer table entries. If a node is placed on a non-path tile, Mario can never stand on it.

2. **Path connectivity must be preserved.** The existing path network is not modified — only which nodes sit on which path tiles. Since we reuse existing path positions, connectivity is guaranteed as long as we don't modify path tiles.

3. **Fortress nodes must precede their locks/bridges on the path.** A fortress opens a lock/bridge when cleared. If the fortress is placed *after* the lock on the path, the player is stuck. The fortress FX system breaks the lock at a fixed map tile position — so fortress nodes must remain at positions where clearing them is meaningful for forward progress.

4. **The castle/airship node must be reachable.** The world's final boss (castle for W1–7, final area for W8) must be reachable after all fortresses are cleared.

5. **Pointer table entry counts are fixed.** Each world has a fixed number of entries (21, 47, 52, etc.). We cannot add or remove entries — only reassign their positions.

6. **`ByRowType` upper nibble + `ByScrCol` must be unique per world.** No two entries can have the same (row, screen, column) — that's how the game disambiguates map positions.

7. **`InitIndex` must be correct.** After reordering entries, the per-screen starting indices must be recomputed so the game's search finds entries on the correct screen.

8. **Starting position must be on a path tile adjacent to the first level.**

#### Soft Constraints (desirable for quality)

9. **Level progression should feel natural.** Early levels should be near the start, harder levels further along the path. (Or this can be fully random for max chaos.)

10. **Toad Houses should be off the main path (or at least not blocking it).** In the original game, Toad Houses are on side branches.

11. **Pipe connectors should connect sensible map positions.** (These are hard to move — probably best left in place.)

12. **W5 (Sky Land) and W8 (Dark Land) have special map structures** that may need special handling.

### 5.3 What Must Change

For each moved node, update:

| Data | What Changes |
|------|-------------|
| `ByRowType[i]` upper nibble | New row position on map |
| `ByScrCol[i]` | New screen + column position |
| `InitIndex` | Recompute per-screen starting indices |
| Map tile at old position | Change from node tile → plain path tile |
| Map tile at new position | Change from plain path tile → node tile |
| Fortress FX tables (if fortress moved) | VRAM addr, map location, map tile, Map_Completions |
| Map object positions (if Hammer Bros / Airship moved) | Y, XHi, XLo in PRG011 tables |
| `Map_Y_Starts` (if start node moved) | Starting Y position |

### 5.4 What Does NOT Change

- Path tile layout (the road network stays identical)
- `ObjSets` and `LevelLayouts` pointers (what level each node loads stays the same — level shuffle is orthogonal)
- `ByRowType` lower nibble (tileset — travels with the level data)
- World BGM
- Map scroll limits
- Tile grid dimensions
- Border/decoration tiles

---

## 6. Algorithm Sketch (Approach B)

```
for each world W:
    1. Parse the tile grid to build a PATH GRAPH
       - Each path tile → node in graph
       - Edges between adjacent path tiles (using directional connectivity)
    
    2. Classify all path positions:
       - OCCUPIED: has a pointer table entry (level, toad, fortress, etc.)
       - STRUCTURAL: fortress/castle/airship/pipe/special (cannot move)
       - MOVEABLE_NODE: regular action level or toad house
       - EMPTY: path tile with no entry (candidate target position)
    
    3. Collect all MOVEABLE_NODE entries and all EMPTY path positions
    
    4. Validate: |MOVEABLE_NODEs| ≤ |EMPTY positions| + |MOVEABLE positions|
       (there must be enough path slots for all nodes)
    
    5. Shuffle MOVEABLE_NODEs into available positions (EMPTY + MOVEABLE)
       Respect ordering constraints:
       - Fortress must be before its lock/bridge on the path
       - Final boss must be after all fortresses
    
    6. For each moved node:
       a. Update ByRowType upper nibble and ByScrCol
       b. Swap tile IDs in the tile grid (old pos → plain path, new pos → node tile)
    
    7. Recompute InitIndex for this world
    
    8. If any fortress moved:
       - Recompute FortressFX map position tables
       - Recompute FortressFX VRAM address tables
       - Recompute Map_Completions bit assignments
    
    9. Update map object positions (Hammer Bros, Airship)
```

---

## 7. Research TODO

Before implementation, the following must be resolved:

### 7.1 Path Tile Encoding (Critical)

- [ ] Extract the full path tile → directional connectivity mapping from the Southbird disassembly
- [ ] Document which tile IDs allow movement in which directions (up/down/left/right)
- [ ] Determine how the game resolves movement at intersections and dead-ends
- [ ] Identify which tile IDs represent "level node" vs "plain path" variants of the same connectivity pattern (e.g., a path-with-dot vs plain path going left-right)

### 7.2 Map Position Coordinate System — ✅ RESOLVED

- [x] **Row mapping:** `grid_row = row_nibble - 2` (derived from `Map_GetTile` tile memory layout: map loaded at `Tile_Mem_Addr + $110`, accessed from base `Tile_Mem_Addr + $100`, offset `((Y-16) & 0xF0) | col`)
- [x] **Column mapping:** `grid_col = screen * 16 + column` where `screen = ByScrCol >> 4`, `column = ByScrCol & 0x0F`
- [x] **Tile grid format:** Row-major per screen (144-byte blocks), NOT column-major
- [x] **Validation:** 340/340 entries (100%) map to non-background tiles across all 8 worlds

### 7.3 Enterable Tile Types

- [ ] Find `Map_EnterSpecialTiles` table in PRG010 — which tile IDs trigger level entry when Mario presses A
- [ ] Determine if level entry requires standing on a specific tile type OR just matching a pointer table entry
- [ ] Document the "bug" where the tile entry check loop reads past the table (noted in ROM reference)

### 7.4 Fortress FX Recomputation

- [ ] Document how VRAM addresses are computed from map row/screen/column
- [ ] Document how `Map_Completions` bit assignments work
- [ ] Determine if FX replacement pattern bytes ($FE/$C0/etc.) are position-dependent or fixed per type (lock vs bridge)

### 7.5 Special World Handling

- [ ] **W5 (Sky Land):** Understand the ground/sky split — are they two separate tile grids? How does the transition tile work? Max_PanR=0x00 but 32 columns of data suggests 2 maps packed together.
- [ ] **W8 (Dark Land):** Linear stage sequence with Max_PanR=0x00 but 64 columns. Understand how the game selects which "screen" to show. Hand traps need special handling.
- [ ] **Warp Zone (W9):** Tile grid exists but may not be shuffleable.

### 7.6 Map Object Positioning

- [ ] Find the exact file offsets of `Map_List_Object_*` pointer tables in PRG011
- [ ] Understand how Hammer Bros roaming positions relate to the path tile grid
- [ ] Understand how Airship dock positions work

### 7.7 Starting Position

- [ ] Find `Map_Y_Starts` file offset
- [ ] Understand if the starting position is a specific tile coordinate or just "walk Mario to this position from off-screen"

---

## 8. Risk Assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| Path tile encoding is more complex than expected | High | May need to parse the full map movement state machine from disassembly |
| Fortress FX recomputation is error-prone | High | Can keep fortresses in fixed positions initially |
| W5/W8 special structures break assumptions | Medium | Exclude W5 and W8 from shuffling initially |
| Not enough empty path tiles for all nodes | Medium | Validate before shuffling; skip worlds where it's impossible |
| PRG012 space constraints | Low | Tile grids are fixed size, pointer tables are fixed size — no space issues for in-place updates |
| Interaction with existing level shuffle | Low | Level shuffle and overworld shuffle are orthogonal — one changes what levels load, the other changes where nodes are on the map |
| Interaction with world order shuffle | Low | World order only changes which world follows which — map data is per-world and independent |

---

## 9. Implementation Steps

Each step is small, testable independently, and builds on the previous one. Steps are grouped into milestones but each step should be its own commit/PR-able unit.

---

### Milestone A: See the Map (Tooling — no ROM changes)

**Step A1: ASCII map renderer (`tools/map_viz.py`)** — ✅ DONE

Standalone Python script that reads the ROM and dumps each world's tile grid as a 9-row ASCII map. Supports `--world N`, `--raw` (hex tiles), `--summary` modes.

**Step A2: Overlay pointer table entries onto the map** — ✅ DONE

Extended `map_viz.py` to read ByRowType/ByScrCol entries and plot them on the ASCII map. **Coordinate mapping resolved:** `grid_row = row_nibble - 2`, `grid_col = screen * 16 + column`. Also discovered tile grid is row-major per screen (not column-major as initially documented). Achieved **100% hit rate** (340/340 entries land on non-background tiles).

**Step A3: Identify path tiles vs node tiles** — ✅ DONE

Tile classification built automatically from entry overlay. 36 unique tile IDs appear under pointer table entries. Key node tiles: 0x03–0x0C (level panels), 0x47/0x48/0x4A (path segments under hammer/level entries), 0x50 (toad/special), 0x67 (fortress), 0xE8 (bonus), 0xC9 (airship).

**Step A4: Identify empty path tiles (candidate swap targets)**

For each world, list all path tile positions that do NOT have a pointer table entry. These are the "empty slots" where nodes could be moved to.

- Output: Per-world list: `W1: 12 occupied path tiles, 8 empty path tiles, 20 total`
- Test: occupied + empty = total path tiles; occupied count matches pointer table entry count (minus non-level entries)

---

### Milestone B: Prove a Swap Works (Minimal ROM change)

**Step B1: Hardcoded 2-node swap in World 1 (Rust)**

Write a new `randomize::overworld` module with a single function that swaps exactly 2 known action level entries in W1. Pick two levels from Step A4's output that are both simple action levels on unambiguous path positions.

The swap changes:
- `ByRowType` upper nibble (row) for both entries
- `ByScrCol` for both entries
- The two map tile grid cells (node tile ↔ plain path tile)

Do NOT touch InitIndex yet — if both nodes are on screen 0, it doesn't matter.

- Test: Load the ROM in an emulator. Walk to where 1-1 used to be → it should load the other level. Walk to where the other level was → it should load 1-1.

**Step B2: Add InitIndex recomputation**

Write a function that recomputes a world's InitIndex sub-table from its ByRowType/ByScrCol data (scan entries, find the first entry on each screen). Call it after the swap from B1.

- Test: Swap two nodes that are on DIFFERENT screens. Verify the game still finds both levels correctly.

**Step B3: Add a unit test for the swap**

Write a Rust test that sets up a minimal ROM with known W1 pointer table data, performs the swap, and asserts:
- ByRowType/ByScrCol values moved correctly
- Tile grid bytes swapped correctly
- InitIndex is valid

---

### Milestone C: Shuffle One World (W1)

**Step C1: Collect shuffleable node positions**

In Rust, write `collect_map_positions(rom, world_idx)` that returns two lists:
- `occupied`: positions that have a moveable action level entry (with entry index)
- `available`: all path positions where a node could go (occupied + empty)

Reuse the tile classification from Step A3. Exclude fortresses, toad houses, airships, pipes, castles, bonus games.

- Test: Assert W1 has the expected number of occupied and available positions (cross-reference with A4 output).

**Step C2: Shuffle and reassign positions**

Write `shuffle_map_nodes(rom, rng, world_idx)` that:
1. Calls `collect_map_positions` to get occupied entries and available slots
2. Shuffles the available slot list
3. Assigns each occupied entry to a new slot
4. Updates ByRowType, ByScrCol, and tile grid for each moved entry
5. Recomputes InitIndex

- Test: Determinism test (same seed → same result). Also assert all entries still have unique (row, scrcol) keys.

**Step C3: Playtest W1 in emulator**

Generate a ROM with only W1 overworld shuffle enabled. Walk the entire W1 map and verify:
- All levels are reachable and load correctly
- No duplicate or missing nodes
- Fortress lock still works (fortress hasn't moved yet)
- Toad houses still work at their original positions

---

### Milestone D: All Standard Worlds (W1–W4, W6–W7)

**Step D1: Extend to W2, W3, W4, W6, W7**

Call `shuffle_map_nodes` for each world. W5 and W8 are excluded (special map structures).

- Test: Determinism test across all worlds. Unique position keys per world.

**Step D2: Playtest each world**

Generate a full ROM. Walk each world's map, verify all levels load, no softlocks.

**Step D3: Wire up as a randomizer option**

Add `overworld_shuffle: bool` to `Options`, add `--overworld-shuffle` CLI flag, wire into `randomizer.rs`. Add to web frontend if applicable.

- Test: `cargo test` passes, option defaults to false, enabling it produces a different ROM.

---

### Milestone E: Handle Fortresses

**Step E1: Research fortress FX recomputation**

Document how to recompute `FortressFX_VAddrH/L`, `FortressFX_MapLocation`, `FortressFX_MapLocationRow`, `FortressFX_MapTileReplace`, and `FortressFX_MapCompIdx` from a fortress's new (row, screen, column) position. This may require understanding the VRAM address formula.

**Step E2: Allow fortress nodes to move**

Remove fortresses from the exclusion list. After shuffling, recompute all FortressFX tables for moved fortresses. Validate that fortress → lock ordering is preserved on the path (fortress must be reachable before the lock it opens).

- Test: Clear a fortress at its new position → lock/bridge opens at the correct map tile.

---

### Milestone F: Special Worlds

**Step F1: W5 (Sky Land)**

Research the ground/sky split. Determine if the two halves can be shuffled independently. Implement if feasible.

**Step F2: W8 (Dark Land)**

Research the linear stage sequence. Determine what's shuffleable (hand traps, fortresses, tank stages). Implement if feasible.

---

## 10. File Offsets Summary

| Data | File Offset | Size | Bank |
|------|-------------|------|------|
| Map tile grid pointers | 0x185A8 | 18 bytes (9 × 2) | PRG012 |
| Map tile grids (all worlds) | 0x185BA–0x19071 | ~2,744 bytes | PRG012 |
| Map_ByXHi_InitIndex (master) | 0x193DA | 18 bytes (9 × 2) | PRG012 |
| Map_ByRowType (master) | 0x193EC | 18 bytes (9 × 2) | PRG012 |
| Map_ByScrCol (master) | 0x193FE | 18 bytes (9 × 2) | PRG012 |
| Map_ObjSets (master) | 0x19410 | 18 bytes (9 × 2) | PRG012 |
| Map_LevelLayouts (master) | 0x19422 | 18 bytes (9 × 2) | PRG012 |
| Per-world sub-tables | 0x19434–0x19C4C | ~2,072 bytes | PRG012 |
| World_Map_Max_PanR | 0x14F44 | 8 bytes | PRG010 |
| FortressFX tables | 0x147CD–0x148B7 | ~235 bytes | PRG010 |
| Map object tables | PRG011 (0x16010–0x1800F) | varies | PRG011 |
| World_BGM | 0x3C424 | 9 bytes | PRG030 |
| Map_Y_Starts | TBD (PRG010) | 8 bytes | PRG010 |
