# Start ↔ Airship swap — engine internals

Reference notes for the `src/randomize/start_airship_swap.rs` module. Captures the SMB3 engine details that took the longest to derive, so future work in this area doesn't have to re-derive them.

## Goal

For each world, swap the start tile (`0xE5`) with the airship tile (`0xC9`) on the overworld map. Mario spawns at the new start position (= vanilla airship coords). Walking onto the new airship position (= vanilla start coords) and pressing A loads the airship level.

## Engine internals

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

- `0x16257` (8 bytes): `LDA #$20 / STA $797A,X / STA $7982,X`. Replaced with `JSR X-helper + 5 NOP`. The helper writes Mario's per-world X-low pixel from a free-space table.
- `0x1627E` (3 bytes): `STA $0724,X`. Replaced with `JSR XHi-helper`. The helper writes `$7978,X` (Map_Entered_XHi), `$0722,X`, and `$0724,X` from free-space tables. Running at the tail of the per-player loop body ensures the helper's writes win over any earlier inline zero-stores at the same targets.

### Auto-pan in `Map_DoPlayer_Edge_Scroll`

Routine at PRG010 file `~0x150F4`. Fires when Mario's on-screen sprite X is `< 33`. Has a World_Num check at `0x15102` that **explicitly skips W5 and W8**. The left-edge CMP immediate is at `0x1512B`.

`Map_Player_SkidBack` ($073E,X) is an early-exit but does NOT auto-clear per frame — it only clears on Mario state transitions (death respawn, etc.). Not safe as a one-frame gate.

**Trade-off accepted:** W2 has a small auto-pan-left animation on entry because Mario spawns at sprite X = 32 < 33 and W2 isn't in the W5/W8 skip list. Disabling left-edge pan globally broke W6 left-walk navigation; finding a one-frame inhibit that auto-clears would need either a custom RAM counter (no clean WRAM slot) or modifying the auto-pan routine. The small W2 entry-scroll is acceptable.

### Vanilla "Start" pointer-table entry

Every world has a real pointer-table entry sitting at its start tile coords with dummy `obj_ptr` / `lay_ptr`. The engine identifies start tiles by the tile byte (`0xE5`), not by the entry, so vanilla never follows those dummy pointers.

After tile-byte swap, if you leave this Start entry at the old start coords, **two entries share a grid position** (the moved airship + the orphaned Start). The sorted lookup at that position finds the Start entry's garbage `obj_ptr` first and loads junk. **The Start entry must be relocated alongside the airship entry.** This was the root cause of "airship tile doesn't enter the airship level" symptoms.

### Airship is `Map_Object` slot 1

Per southbird's disasm: "NOTE: Assumes Index 1 is the Airship!" Tables in PRG011 at `0x16020` (Y master), `0x16030` (XHi), `0x16040` (XLo). `rom_data.rs::write_map_sprite_position()` already supports this.

The airship is normally entered via sprite collision (Mario walks into the moving airship). The tile-press-A path works too, but requires the pointer-table entry to be at Mario's position.

### Castle-top sprite tile travels with the airship

The vanilla tile directly above the airship (`0xC8`, the castle's top half) is part of the visible airship sprite — it's a two-tile-tall composite. Without also swapping the tile above, the castle-top stays anchored above the vanilla airship location and dangles as a stray graphic.

For W4/W5/W7, the tile that ends up above the **new** start position (the tile that vanilla had above the start, dropped into the new airship-coords-for-start location) is a water square — fine above the original start but jarring elsewhere. The module overrides those three worlds with a generic land/sky blank (see `above_start_override` in `src/randomize/start_airship_swap.rs`).

### W3 reachability — the canoe-dock trap

In vanilla, the W3 start at `(8, 2)` sits on the mainland with a direct walking path to the canoe dock at `(6, 20)`. Whenever the player wants to visit an island, they walk to the dock first, board the boat, and ride. The boat is effectively "always at the dock the player is at" — every island arrival came via the dock.

When SAS swaps W3, the start moves to the vanilla airship position `(6, 41)` — an eastern dead-end region whose only escape is through a specific entry pipe. The pipe shuffle re-randomizes pipe positions, so in ~6% of W3-swap seeds, no pipe ends up connecting the start region to the rest of the mainland, including the canoe dock. The player can pipe to an island, but the boat is still at `(6, 20)` and they can never reach it — the airship at the vanilla-start position becomes physically unreachable.

The builder used `walk_map` for its connectivity checks (lock safety, pipe placement). `walk_map` historically treated canoe edges as free bidirectional teleports, so the builder believed every island was always reachable from the mainland and vice versa — including from a stranded start. This let unwinnable layouts pass the builder's checks.

**Fix landed in `src/randomize/map_walker.rs`:** `walk_map` now does a two-pass BFS. The first pass uses walking + pipes only; if any canoe mainland dock is in the resulting reachable set, the second pass enables canoe edges (bidirectional). If the dock isn't walk-reachable, no canoe edges are added at all. The bidirectional model in the second pass is still correct because once the player can reach the dock they can shuttle the boat between mainland and any island as needed.

After this change the SAS 1000-seed sweep drops from 63 unreachable W3 cases to 0. The W5 carve-out and other SAS mechanics are unaffected — they don't interact with canoes.

### Game Over → Continue (twirl-to-start) — the live-scroll trap

Dying with lives left is self-correcting and page-agnostic: the per-frame map
loop `PRG030_8775` continuously syncs the live scroll `Horz_Scroll`/`Horz_Scroll_Hi`
(`$FD`/`$12`) into `Map_Prev_XOff`/`XHi` (`$0722`/`$0724`) and `World_Map_*` into
`Map_Entered_*`, and every level entry snapshots those into the skid-back
backups. `MO_SkidToPrev` restores from the backups, so the respawn always tracks
live state. SAS needs to do nothing extra here.

**Game Over is different.** The vanilla continue animation assumes the start is at
page-0 hard-left:

- `GameOver_Timeout` (PRG010, state 3) picks `GameOver_TwirlFromAfar` (state 5)
  vs `GameOver_TwirlToStart` (state 4) by testing `Horz_Scroll`/`Horz_Scroll_Hi`.
  Any nonzero (i.e. the camera is scrolled off page-0-left) → TwirlFromAfar.
- `GameOver_TwirlFromAfar` flies Mario left, **scrolling `Horz_Scroll` down to 0**,
  then zeroes `Map_Prev_XOff`/`XHi`/`Map_Entered_XHi`.
- `AlignToStartY` (6) / `ReturnToStartX` (7) then operate entirely within page 0
  (`World_Map_X` 240 → `$20`) before landing at `PRG011_A698` (state 8).

So at the twirl landing the live scroll is **page 0**, no matter where the swapped
start actually is. The SAS finalize helper is hooked into that landing
(`STA Map_Prev_XHi2,X` at `$A6AA`) and stamps the per-player position
(`World_Map_X/XHi`) and scroll backup (`$0722`/`$0724`) to the real start. But on
continue, `PRG030_92B6` copies the **live** `Horz_Scroll`/`Horz_Scroll_Hi` into
`Map_Prev_XOff`/`XHi`, and the world re-enter at `PRG030_8634` reloads
`Horz_Scroll` *from* `Map_Prev` and does a full nametable redraw. A stale page-0
live scroll therefore wins: the map redraws on page 0 while Mario is placed on the
real (≥1) start page → **off-map softlock whenever the Game Over happened on a
different overworld page than the start tile.** (If the Game Over happened on the
start's own page, live scroll already matched, which is why it went unnoticed at
first.)

**Fix:** the finalize helper also stamps the live scroll ZP `Horz_Scroll` (`$FD`)
= 0 and `Horz_Scroll_Hi` (`$12`) = start screen index (global, not per-player).
The subsequent `92B6`/`8634` re-enter then carries the start page through and
redraws the nametable there. Values are identical to the Map_Init seeds, so
unswapped / page-0 worlds get no-op stores.

## Verification tooling

The W3 reachability bug was caught by `test_required_progression` in `src/randomize/overworld_build.rs`, a Dijkstra-based must-clear analyzer. Useful flow when changing anything SAS-related:

```sh
nix-shell -p gcc --run 'export PATH="$HOME/.cargo/bin:$PATH" && \
  PROG_SEEDS=1000 SAS=1 cargo test --release \
  test_required_progression -- --ignored --nocapture'
```

A `WARNING: N unreachable-target case(s)` line in the output is a builder bug — the analyzer reports per-world counts and example seeds. To reproduce a specific case visually:

```sh
DUMP_SEED=<seed> DUMP_WORLD=<world> SAS=1 STANDALONE=1 GRID=1 cargo test \
  --release test_dump_required_progression -- --ignored --nocapture
```

`STANDALONE=1` matches the distribution analyzer's standalone-build path (so seeds line up). Omit it to use the full pipeline (matches a real CLI playthrough; emits `progression_seed{N}.nes`).
