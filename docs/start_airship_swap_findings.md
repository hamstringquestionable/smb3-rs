# Start ↔ Airship swap — POC findings

Status: **working POC** at `tools/poc_start_airship_swap.py`. Tested in emulator on the W1-W7 cross-screen and same-screen cases. Not yet ported to Rust.

This document records the engine internals we had to learn to make the swap work end-to-end, so future sessions don't re-derive them.

## Goal

For each world, swap the start tile (`0xE5`) with the airship tile (`0xC9`) on the overworld map. Mario should spawn at the new start position (= vanilla airship coords). Walking onto the new airship position (= vanilla start coords) and pressing A should load the airship level.

## Generation

```sh
nix-shell -p python3 --run 'python3 tools/poc_start_airship_swap.py'
# Produces SMB3R_POC.nes from vanilla + smb3practice_SE.ips + the swap.
```

The script applies `smb3practice_SE.ips` first (it bundles the same airship pointer-table redirects as `src/randomize/autoscroll.rs`, so airships load static autoscroll-free interiors), then layers the swap on top.

## Engine internals discovered

### Horizontal mirroring (not vertical)

MMC3 mirroring register at `$A000` is set to `$01` at file `0x3C599` (`PRG030_857E` world-enter routine). That's **horizontal** mirroring — NT0 and NT1 share VRAM. Only 16 cols of world data are visible at a time. To show a different "screen" of a multi-screen world, the engine **redraws the nametable** with new content; it doesn't scroll between two pre-populated nametables.

### `Map_Prev_XOff` / `Map_Prev_XHi` live at $0722 / $0724

NOT at `$7980` / `$7984` (which the project's prior beta notes claimed). `$7980`+ are dead per `prg011.asm:188-192` zero-stores. The real flow is:

```
PRG030_8634:
  LDA Map_Prev_XOff,Y   ; $0722,Y  → Horz_Scroll    (ZP $FD)
  LDA Map_Prev_XHi,Y    ; $0724,Y  → Horz_Scroll_Hi (ZP $12)
```

### `Scroll_Dirty_Update` sweeps 32 cols based on `Map_Prev_XHi`

On world entry, `PRG030_857E` calls `Scroll_Update_Ranges` (which reads `Map_Prev_XHi` to compute `Scroll_ColumnL`) then `Scroll_Dirty_Update` to load 32 columns. So `$0724` is the nametable LOAD selector — set it to Mario's screen index and the engine loads cols `(screen * 16)..(screen * 16 + 15)`.

### Map_Init patch sites (PRG011 file `0x16247`)

- `0x16257` (8 bytes): `LDA #$20 / STA $797A,X / STA $7982,X`. Replace with `JSR X-helper + 5 NOP`. The helper writes Mario's per-world X-low pixel from a free-space table.
- `0x16269` (3 bytes): `STA $7978,X` (Map_Entered_XHi). Replace with `JSR XHi-helper`. The helper also writes `$0722,X` (= 0) and `$0724,X` (= screen index) from free-space tables.

### Auto-pan in `Map_DoPlayer_Edge_Scroll`

Routine at PRG010 file `~0x150F4`. Fires when Mario's on-screen sprite X is `< 33`. Has a World_Num check at `0x15102` that **explicitly skips W5 and W8**. The left-edge CMP immediate is at `0x1512B`.

`Map_Player_SkidBack` ($073E,X) is an early-exit but does NOT auto-clear per frame — it only clears on Mario state transitions (death respawn, etc.). Not safe as a one-frame gate.

**Trade-off accepted:** W2 has a small auto-pan-left animation on entry because Mario spawns at sprite X = 32 < 33 and W2 isn't in the W5/W8 skip list. Disabling left-edge pan globally broke W6 left-walk navigation; finding a one-frame inhibit that auto-clears would need either a custom RAM counter (no clean WRAM slot) or modifying the auto-pan routine. For the POC, the small W2 entry-scroll is acceptable.

### Vanilla "Start" pointer-table entry

Every world has a real pointer-table entry sitting at its start tile coords with dummy `obj_ptr` / `lay_ptr`. The engine identifies start tiles by the tile byte (`0xE5`), not by the entry, so vanilla never follows those dummy pointers.

After tile-byte swap, if you leave this Start entry at the old start coords, **two entries share a grid position** (the moved airship + the orphaned Start). The sorted lookup at that position finds the Start entry's garbage `obj_ptr` first and loads junk. **The Start entry must be relocated alongside the airship entry.** This was the root cause of "airship tile doesn't enter the airship level" symptoms.

### Airship is `Map_Object` slot 1

Per southbird's disasm: "NOTE: Assumes Index 1 is the Airship!" Tables in PRG011 at `0x16020` (Y master), `0x16030` (XHi), `0x16040` (XLo). `rom_data.rs::write_map_sprite_position()` already supports this.

The airship is normally entered via sprite collision (Mario walks into the moving airship). The tile-press-A path works too, but requires the pointer-table entry to be at Mario's position.

## What the POC does (in order)

1. Apply `smb3practice_SE.ips`.
2. For each W1-W7:
   1. Swap tile bytes at `(start_row, col) ↔ (airship_row, col)` AND at the row above each (airship sprite is 2 tiles tall).
   2. Rewrite airship pointer-table entry's `rowtype/scrcol` to point at vanilla start coords (preserve low nibble = tileset code).
   3. Rewrite the vanilla Start entry's `rowtype/scrcol` to point at vanilla airship coords.
   4. Move Map_Object slot 1 to vanilla start coords.
   5. Re-sort the world's pointer table by `(screen, row_nib, col)` and rebuild its InitIndex sub-table.
3. Write `Map_Y_Starts` per world (overwrites vanilla 8-byte table at `0x3C39A`).
4. Write four 8-byte tables to PRG031 free space: `Map_X_Starts`, `Map_XHi_Starts`, `Map_ScrL_Starts`, `Map_ScrH_Starts`.
5. Write the X-helper (10 bytes) and XHi-helper (19 bytes) routines to PRG031 free space.
6. Patch Map_Init's two inline sites to JSR the helpers.

## Open items for Rust port

- New module `src/randomize/start_airship_swap.rs` running between `autoscroll` and `NodeCatalog::build()`.
- Free-space allocations in `rom_data.rs::FREE_SPACE_ALLOCATIONS` for the two helpers + four tables.
- Whether/how the swap interacts with `node_catalog::classify_entry`'s tile-based Start detection — the catalog reads positions from ROM at catalog time, so if the swap runs before catalog, the pipeline should see the swapped state naturally. Verify.
- Whether `overworld_build::fixed_positions_for_world` correctly pins the moved airship at the new position. `entry.grid_pos` is set from the (post-swap) rowtype/scrcol, so this should just work — verify.
- Flag-key bit allocation when promoting from beta.
- W8 (Bowser's castle) currently untouched. Different mechanics if we want to include it (no slot-1 airship sprite).
- Diagnostic tests: per-world assertion that each grid position has exactly one entry; airship entry's `obj_ptr` matches the practice/autoscroll redirect value.
