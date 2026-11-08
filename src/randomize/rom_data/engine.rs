//! Engine symbols: the RAM addresses and vanilla routine entry points that
//! hand-assembled patches name.
//!
//! These are facts about the game, not about the randomizer — none of them can
//! ever change. They live here because six modules had independently declared
//! `World_Num = $0727`, and `tools/offset_dups.py` cannot see a duplicated *RAM*
//! address the way it sees a duplicated ROM offset. One home means the next
//! module reads a symbol instead of inventing it, and means the reasoning that
//! established an address is written down once.
//!
//! Zero-page symbols are `u8` because that is the width of the operand a patch
//! encodes them into; everything else is `u16`.

/// `Player_Current` — 0 Mario, 1 Luigi. Every `Map_Entered_*` and every live
/// map-position variable is a two-byte array indexed by it, so reading one back
/// means holding it in `X` first.
///
/// Not read off a label: the disassembly declares `Map_Prev_XOff` and
/// `Map_Prev_XHi` as two bytes each at `$0722` and `$0724`, then
/// `Player_Current` and `World_Num` as one byte each. `World_Num` is `$0727`
/// and is verified by the patches that already write it, which puts
/// `Player_Current` at `$0726`.
pub(crate) const PLAYER_CURRENT: u16 = 0x0726;

/// `World_Num`, 0-based: World 8 is 7.
pub(crate) const WORLD_NUM: u16 = 0x0727;

/// `PAGE_A000` — the MMC3 page latched into `$A000` by the next
/// [`PRGROM_CHANGE_A000`]. Store the page here rather than writing the mapper
/// directly, so anything that re-latches from the variable agrees with you.
pub(crate) const PAGE_A000: u16 = 0x0720;

/// `PRGROM_Change_A000` (`$FFC2`, the always-mapped PRG031): map page
/// [`PAGE_A000`] into `$A000`. Clobbers `A` only.
///
/// Code running in `$A000` can swap itself out by **tail-calling** this —
/// `LDA #page / STA PAGE_A000 / JMP PRGROM_Change_A000`. The `JMP` is fetched
/// before the swap, the routine runs from PRG031, and its `RTS` returns to
/// whoever called the swapped-out code.
pub(crate) const PRGROM_CHANGE_A000: u16 = 0xFFC2;

/// `StatusBar_Fill_World` (PRG026 `$B0C4`, file `0x350D4`): queue the status
/// bar's "WORLD n" digit into `Graphics_Buffer`. Callable only with PRG026 at
/// `$A000`. Clobbers `A`, `X` and `Y`.
///
/// It picks the VRAM row from `Level_7Vertical` (`$03EF`) and `Level_Tileset`,
/// and both read as "the map" there: every return from a level passes
/// `PRG030_8FCA`, whose `Clear_RAM_thru_ZeroPage` zeroes `$0000-$06FF`.
///
/// Vanilla calls it only where a bar is built, plus one frame of the item-box
/// flip — never per frame — so a change to the digit stays off screen until
/// something calls it again.
pub(crate) const STATUS_BAR_FILL_WORLD: u16 = 0xB0C4;

/// The highest zero-page byte a patch may park state in: `Temp_Var3` (`$02`).
///
/// The NMI pushes and pulls exactly `Temp_Var1`, `Temp_Var2` and `Temp_Var3`
/// around every frame and leaves the rest of the page to whoever was using it.
/// Anything a long-running routine holds in `$03` or above is destroyed
/// mid-loop, on hardware, invisibly — no emulated-CPU test can see it, because
/// there is no NMI there. `asm::Routine::zero_page` is the structural
/// enforcement; this is the number it is given, so it is test-only like the
/// checker that consumes it.
#[cfg(test)]
pub(crate) const NMI_SAFE_MAX: u8 = 0x02;

/// `Map_Completions` (`$7D00`) — 128 bytes, Mario's 64 columns then the
/// permanent mirror at `+$40`. The array `PRG030_84A0` opens by wiping and
/// `Map_Reload_with_Completions` replays onto the drawn map.
pub(crate) const MAP_COMPLETIONS: u16 = 0x7D00;

/// `World_Map_Tile` (zero page `$E5`) — the tile the player is standing on, as
/// `Map_GetTile` left it.
pub(crate) const WORLD_MAP_TILE: u8 = 0xE5;

/// The player's live map position, zero page, two bytes each (Mario/Luigi).
///
/// **`World_Map_Y & $F0` is `Map_Entered_Y`** — the same `(grid_row + 2) << 4`
/// the pipe destination tables and the arrival rows use, so one encoding serves
/// both. `GameOver_AlignToStartY` is the proof: it stores `Map_Y_Starts,Y`
/// straight into `World_Map_Y` with no adjustment.
///
/// Reading that off `Map_GetTile` alone gets it wrong by exactly one row. The
/// routine does `SUB #16 / AND #$F0`, which looks like `(row + 1) << 4` — but
/// it only adds `$100` to the screen base while the grid actually starts at
/// `+$110`, and that missing `$10` is a whole row. The first cut of the telepad
/// key made that mistake and every pad quietly entered its spade game instead
/// of teleporting.
///
/// Column is `World_Map_X >> 4` and screen is `World_Map_XHi`. All of these are
/// pixel coordinates, so the low nibbles are sub-tile offsets and a key must
/// mask them off rather than compare raw.
pub(crate) const WORLD_MAP_Y: u8 = 0x75;
/// See [`WORLD_MAP_Y`].
pub(crate) const WORLD_MAP_XHI: u8 = 0x77;
/// See [`WORLD_MAP_Y`].
pub(crate) const WORLD_MAP_X: u8 = 0x79;

/// CPU address of `PRG030_84A0`, "initialize the world map". Always mapped.
///
/// Reached from exactly two places in vanilla — the airship-cleared path
/// (`INC World_Num`) and the warp zone (`World_Num = Map_Warp_PrevWorld`) — and
/// it **never returns**: it falls through into `WorldMap_Loop`, which is why
/// every caller resets the stack first.
pub(crate) const WORLD_MAP_INIT_CPU: u16 = 0x84A0;

/// `Map_Reload_with_Completions` (PRG012, CPU `$A45D`) — the routine that
/// rebuilds `Tile_Mem` from the ROM grid and replays the completion bitfield.
pub(crate) const MAP_RELOAD_CPU: u16 = 0xA45D;

/// `Map_CompleteBit` (PRG011, CPU `$BA2D`) — `$80 $40 $20 $10 $08 $04 $02 $01`,
/// the engine's own row-bit lookup. `Map_MarkLevelComplete` indexes it with the
/// completion row; `map_objects` borrows it as a *world*-bit lookup, and
/// `lock_keys` uses it for its original purpose.
///
/// Verified against the ROM by `the_engine_still_has_the_bit_table_we_borrow`
/// rather than trusted from the disassembly's label. PRG011 is mapped at
/// `$A000` for the whole map, so both banks that name it can reach it.
pub(crate) const MAP_COMPLETE_BIT_CPU: u16 = 0xBA2D;
