# Overworld Builder Rewrite

Branch: `overworld-builder`

## Phase Overview

| Phase | Module | Status |
|-------|--------|--------|
| 1. Node Catalog | `node_catalog.rs` | DONE |
| 2. Clear/Pick-up | `overworld_pickup.rs` | DONE |
| 3. Build | `overworld_build.rs` | TODO |
| 4. Write | `overworld_writer.rs` (existing) | TODO — adapt to Phase 3 output |

Old builder still active via `#[path]` in mod.rs until Phase 3+4 replace it.

---

## Phase 1: Node Catalog (DONE)

`node_catalog.rs` — classifies all 340 pointer table entries into `CatalogEntry` structs with `NodeKind` enum (Level, Fortress, Pipe, Airship, Bowser, Start, ToadHouse, BonusGame, HammerBro, MapObject). 8 tests.

## Phase 2: Clear/Pick-up (DONE)

`overworld_pickup.rs` — picks up all `is_level_like()` entries (Level, Fortress, Pipe, Airship, Bowser) into a global pool of 135 entries. Produces `PickupResult` with cleared grids (theme-aware blank tiles) and pool indices. 7 tests.

## Phase 3: Build (TODO)

`overworld_build.rs` — takes `PickupResult` + `NodeCatalog` + RNG, produces slot assignments per world.

### Algorithm

#### Step 0: Fortress Redistribution
- W8 keeps its 4 fortresses
- 13 remaining fortresses distributed randomly across W1-W7
- Each world gets 1-3 fortresses
- This step only decides **counts** per world, not grid positions

#### Step 1: Pipe Placement
- Uses cleared grid from Phase 2 (pool entries blanked, FX gaps pre-opened)
- BFS from start to find reachable blank slots
- **Connectivity pipes**: If airship is NOT BFS-reachable, place pipe pairs one at a time (one endpoint in reachable area, one in unreachable) until airship is reachable
- **Remaining pipes**: Continue placing remaining pipe endpoints (vanilla count minus connectivity pipes used) to connect more unreachable islands, providing more slots for levels
- Vanilla pipe endpoint counts per world: W1=0, W2=4, W3=6, W4=4, W5=2, W6=4, W7=16, W8=12 (48 total = 24 pairs)
- Constraint: try not to skip too many sections or jump straight to the end (soft)

#### Step 2: BFS Sectioning
- BFS from start on the grid (with all pipes now placed)
- Order all reachable blank slots by BFS distance
- Divide into N sections where N = fortress count for this world
- Each section gets roughly equal number of node slots

#### Step 3: Populate Sections
- Each section gets 1 fortress at a random position within the section
- Remaining blank slots tagged as "level" (no specific pool entry assigned)
- Fixed positions (not in sections):
  - Start: vanilla position, always present
  - Airship: vanilla position, 1 per world (W1-W7)
  - Bowser: vanilla position (W8 only)
  - Toad houses: vanilla positions (not shuffled)
- Bonus game vanilla slots become available as level slots

#### Step 4: Lock Placement
- **Every fortress gets exactly one lock/bridge** in its world
- Process fortresses in BFS section order
- For each fortress, find lockable path tiles (`LOCKABLE_TILES`)
- **Hard rule**: the lock must NOT block access to its own fortress (the one that opens it)
- **Soft goals** (priority order):
  1. Block progression into the next section
  2. Block progression to the airship
  3. Block access to an optional/side area
- If no position achieves any soft goal, place on any valid lockable tile that doesn't violate the hard rule
- BFS validate after each lock placement
- Gap tile type determined by path orientation (lock=$54/vert, bridge=$56/horiz, water=$9D, sky=$E4)

### Output Structure

Phase 3 produces per-world slot assignments — each blank grid position tagged with a role:
- `Level` — writer pulls a level from the pool
- `Fortress` — writer pulls a fortress from the pool
- `Pipe` — writer pulls a pipe endpoint from the pool
- Lock positions — list of (path_tile_position, gap_tile, which_fortress_opens_it)

Phase 3 does NOT:
- Assign specific pool entries to slots (writer does that)
- Compute FX slot data (writer does that)
- Write to the ROM (writer does that)

### Fixed Elements (not assigned by Phase 3)
- Start tile: vanilla position
- Airship: vanilla position
- Bowser's castle: vanilla position
- Toad houses: vanilla positions
- Hammer bros: map objects, not grid tiles
- Map objects (W7 piranhas): stay as-is

---

## Phase 4: Write (TODO — adapt existing `overworld_writer.rs`)

Takes Phase 3 output + pool, assigns specific pool entries to slots, writes:
- Tile grid to ROM
- Level entry data (tileset, obj/lay pointers) per placement
- Pointer table position bytes (ByRowType upper nibble, ByScrCol)
- Boom-Boom Y-byte ordinals
- FX slot fields (VRAM, map location, replacement tile, completion bits, patterns)
- Per-world FX assignment table
- Pipe destination tables (MapXHi, MapX, MapY, MapScrlXHi)
- Pointer table re-sorting
- Map object sprite position sync

---

## Key Constants

- 340 total entries: 62 Level, 17 Fortress, 48 Pipe, 7 Airship, 1 Bowser, 8 Start, + ToadHouse/BonusGame/HammerBro/MapObject
- 135 pool entries (is_level_like): 62 + 17 + 48 + 7 + 1
- 24 pipe pairs (48 endpoints)
- 17 FX slots total
- Grid: 9 rows, 16 cols per screen (W1=1scr, W2=2, W3=3, W4=2, W5=2, W6=3, W7=2, W8=4)
