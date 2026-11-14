# Level terrain and the hazard scans

Dev-time tooling only. Nothing here reaches the CLI, the WASM build or a
generated ROM: the loader and every scan are `#[cfg(test)]`, and the scans are
ignored tests that skip when the vanilla ROM is not at
`roms/Super Mario Bros. 3 (USA) (Rev 1).nes`.

## The terrain loader

`rom_data::level_index::terrain::load(&rom, &area)` returns an area's tile grid
(`Tile_Mem`, 15 screens of 27 rows by 16 columns) and the tileset's solidity
thresholds (`Tile_AttrTable`).

It does not decode the layout itself. It runs the game's own loader,
`LevelLoad_ByTileset` (PRG030 `$9A1D`), then `FillTileAttrTable`, on the
`mos6502` crate's CPU (a dev-dependency), with the four PRG windows SMB3 maps
while a level loads: `$8000` = PRG030, `$A000` = the tileset's bank,
`$C000` = PRG014, `$E000` = PRG031. So whatever the engine builds, the grid has,
including the generators nobody has hand-decoded.

Solidity of a tile, as the corridor census reads it: with `q = tile >> 6`, the
tile is fully solid if `tile >= attr[q + 4]`, solid on top only if
`tile >= attr[q]`, and open otherwise.

**Checked:** all 14 tilesets match Mesen byte for byte, except the random
power-up clouds `LoadLevel_RandomPUpClouds` scatters by RNG (the loader's RNG is
zero, so it stacks them all at `$6000`). `terrain_render` with
`TERRAIN_COMPARE=<dump>` does that comparison against a 6488-byte dump
(`Tile_Mem` + `Tile_AttrTable`) from a real emulator.

**Things the loader has to work around:**

- **The `mos6502` crate sets carry wrongly after `SBC`.** `call` recomputes it
  after every `SBC` from the operands. `sbc_probe` shows the bug;
  `NO_SBC_FIX=1` turns the correction off.
- **Plains generator 45 executes a `BRK`** (a vanilla `BEQ`-for-`BNE` bug). On
  hardware the IRQ handler sees `Reset_Latch = $5A` and returns, so the loader
  sets that and `Raster_Effect = $80` before running.
- **Some areas touch addresses the model doesn't back.** `terrain_sweep` loads
  every area once and lists them: 233 clean, 5 that read just below SRAM
  (`$5FEF`/`$5FFF`) or write the APU (`$4010`). They still load all the way
  through. They are listed so a new one stands out, not because they are wrong.

**What it can't see:**

- **Objects.** Moving lifts, donut blocks, spinners and other platforms that are
  objects are not in the grid. A "gap" can be crossed by one.
- **Junctions.** It loads an area as a level entry does, not as a pipe or door
  arrival.
- **Vertical areas.** They load, but the scans below skip them.

## The instruments

Run each one with
`cargo test --release --lib <name> -- --ignored --nocapture`.

| Name | Where | What it answers |
|---|---|---|
| `terrain_render` | `level_index/terrain.rs` | Draws a level's areas as text: `#` solid, `-` solid on top, `o` other, `.` the clear tile. `TERRAIN_LEVEL=4-1` picks the level (default 1-1). |
| `terrain_sweep` | `level_index/terrain.rs` | Loads every area once and lists any that touch unbacked addresses. |
| `hazard_census` | `enemies/hazard_census.rs` | Low-ceiling corridors: slots where headroom is too low to jump a hazard, scored against the existing `Exclude(HAZARDS)` rules. `HAZARD_CUT` sets the headroom cut-off (default 3). |
| `platform_census` | `enemies/platform_census.rs` | Gap, narrow landing, gap: slots on a landing Mario may be forced onto, with the widths of each part. |
| `rotodisc_reach` | `enemies/rotodisc_reach.rs` | Slots where a Rotodisc would swing over the spot Mario appears at (level start or a junction arrival). Needs no terrain. `ROTO_AREA=4F2` prints every arrival and slot for one level. |

Each scan prints a shortlist for a person to judge, not a verdict. The
protections that came out of them (2026-11) are in
`enemies/protections.rs`, and each row's `why` says which kind it is:

- **Low ceilings:** 8B/1 and 5-2.
- **Rotodiscs over arrivals:** 4-6 (both areas), 1-2, 2-1, 3-3, 5F1, 4F2's
  hidden door, 8B/1.
- **Narrow landings:** 4-2, 7F2, 8B/0.

**Looked at and left alone:**

- 8F1 (low ceiling).
- 5-7's Pile Driver and 2-Pyr's Buzzy Beetle (Rotodisc near misses).
- 6F1's vanilla Rotodisc.
- 1F, 6-10 and 6-3 (narrow landings with a way round: a jump, a spinner, a
  moving platform).
- 1-6, β5 and β7 were listed but not reviewed.

`platform_census` misses 4-1's giant green troopa (`0x0CEC9`), which has a rule;
its landing doesn't match the scan's shape.

## Using it for a new scan

Load an area with `terrain::load`, walk the slots with `read_objects`, and
report. `hazard_census` exposes `Solid`, `solidity`, `swappable` and
`other_cover` (what already keeps hazards out of a slot) for sibling scans.
`rotodisc_reach` has the arrival-point decoding (`start_arrival`,
`junction_arrival`), documented in `smb3_rom_reference.md` under "Junction
Spawn Positions".

To look at one sub-area in game, `testrom --place 5-7/1` puts area 1 of 5-7 on
a map tile of its own.
