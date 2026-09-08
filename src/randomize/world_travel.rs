//! World-maze fast travel: the warp whistle becomes a hop between worlds the
//! player has already been to.
//!
//! Two routines, and they are deliberately independent of everything else the
//! maze installs:
//!
//! | routine | bank | fires |
//! |---|---|---|
//! | [`MARK_VISITED`] | PRG010 | every map frame, from `MO_NormalMoveEnter` |
//! | [`WHISTLE_TRAVEL`] | PRG011 | once, at the whistle's world switch |
//!
//! **A world counts as visited once the player stands on its start tile.**
//!
//! **The dependency here inverted on 2026-09-07 and the old wording said the
//! opposite.** It used to read that invariant 3 — every world's start region
//! escapable — is what made a whistle destination safe to travel to. That rule
//! is retired (see `maze::fill::assign_keys`): the constructive fill gives every
//! gate a key already reachable from the global start, which is strictly
//! stronger, and the rule additionally rejected the mode's own formula by
//! refusing to count foreign fortresses.
//!
//! So it is now the other way round: **the whistle is what makes a trapped
//! start region survivable**, and that makes this module a safety property
//! rather than a convenience. Game over, airship arrival and whistle travel all
//! deposit the player on a start tile that may have no walk-out; what saves
//! them is that the whistle is never consumed, survives a game over, and always
//! has the spine's first world to return to.
//!
//! Two consequences worth keeping in view. A world only ever pad-hopped into
//! the middle of is still correctly not whistle-able — the marker fires on the
//! start tile, not on arrival. And if the mode is ever shipped without a
//! whistle, game over has to return the player to the spine's first world
//! instead of the one they died in; the note at `remove_whistles` in
//! `randomizer::randomize_inner` carries the mechanism.
//!
//! # Why a position compare and not `Map_GetTile`
//!
//! `Map_GetTile` (PRG010, CPU `$D1FE`) derives its row one short: it does
//! `SBC #16 / AND #$F0` but only adds `$100` to the screen base while the grid
//! actually starts at `+$110`. That cost a playtest already — see
//! [`super::world_persist`]'s note on the pad key, where every pad quietly
//! entered its spade game instead of teleporting.
//!
//! The compare below needs no row arithmetic at all. It asks two questions
//! about where the player is standing, against numbers the *engine* holds:
//!
//! * `World_Map_X` and `World_Map_XHi` against [`START_XKEY`], an eight-byte
//!   table this module emits per seed from the map the writer actually laid
//!   down;
//! * `World_Map_Y` against `Map_Y_Starts[World_Num]`, the engine's own
//!   per-world start row, read from the ROM rather than transcribed.
//!
//! # Why the column comes from a table
//!
//! The obvious version compares against `#$20` — `Map_Init` really does say
//! "Set starting X position (forced to $20!)" and hardcodes screen 0. That is
//! true of vanilla and **false of a randomized ROM**:
//! [`super::start_airship_swap`] replaces `Map_Init`'s scroll store with a
//! helper that re-stamps `Map_Entered_X`/`Map_Entered_XHi` from its own
//! per-world tables, so a swapped world starts wherever its airship was.
//!
//! A `#$20` compare would silently never fire for those worlds: they would
//! drop out of the whistle's cycle, with no crash and nothing to reproduce.
//! So the column is a table, emitted here, and it is emitted **unconditionally**
//! — `start_airship_swap`'s own `FS_SAS_*` tables only exist when that option
//! is on, and depending on them would trade one conditional bug for another.
//!
//! **Ordering requirement: [`apply`] must run after the overworld writer.** It
//! reads each world's START tile out of the ROM's own grid, which is where the
//! writer puts the (possibly swapped) start. Running it earlier would emit
//! vanilla positions onto a randomized map. [`apply`] cross-checks every row
//! it derives against `Map_Y_Starts` and panics rather than emitting a table
//! that disagrees with the engine, so the ordering cannot be got wrong quietly.
//!
//! # Reusing the start row as the marker byte
//!
//! `MARK_VISITED` stores the value it just compared — `Map_Y_Starts[world]` —
//! rather than an `LDA #$01`, saving two bytes. Every possible value is
//! `(grid_row + 2) << 4` for a grid row of 0..8, i.e. `$20`..`$A0`, so it can
//! never be zero and `BNE` remains a correct "have I been here" test.
//! `map_y_starts_can_never_be_zero` pins that against the ROM.
//!
//! # Zero page
//!
//! Neither routine uses any. The key is composed in `A` with an `ORA` against
//! the sub-tile guard, so nothing has to be parked anywhere the NMI might
//! clobber — see `completion_bits`' `no_routine_parks_state_in_unprotected
//! _zero_page`, which exists because three playtests died of exactly that.

use crate::rom::Rom;

use super::maze_state::{VISITED_TABLE, VISITED_TABLE_LEN};
#[cfg(test)]
use super::rom_data::NMI_SAFE_MAX;
use super::rom_data::{
    FS_MAZE_TRAVEL, FS_MAZE_VISITED, Grid, MAP_Y_STARTS_OFF, PLAYER_CURRENT, WORLD_MAP_INIT_CPU,
    WORLD_MAP_X, WORLD_MAP_XHI, WORLD_MAP_Y, WORLD_NUM, find_start, prg010_file_to_cpu,
    prg011_file_to_cpu, prg030_file_to_cpu,
};

// --- Addresses ----------------------------------------------------------

const MARK_VISITED_CPU: u16 = prg010_file_to_cpu(FS_MAZE_VISITED);
const WHISTLE_TRAVEL_CPU: u16 = prg011_file_to_cpu(FS_MAZE_TRAVEL);

/// `Map_Y_Starts` — the per-world start row, `$838A`. Read at runtime rather
/// than baked in, because `start_airship_swap` rewrites the table.
const MAP_Y_STARTS_CPU: u16 = prg030_file_to_cpu(MAP_Y_STARTS_OFF);

/// `Map_WarpWind_FX` (`$8B`) — the whistle's own state machine. Zeroed on the
/// way out, or the wind effect keeps advancing over a map that has already
/// been rebuilt.
const MAP_WARPWIND_FX: u8 = 0x8B;

/// `Map_NoLoseTurn` and `Map_WasInPipeway` — the two flags the marker's hook
/// site clears, replayed verbatim.
const MAP_NO_LOSE_TURN: u16 = 0x796E;
const MAP_WAS_IN_PIPEWAY: u16 = 0x7973;

/// `ARRIVAL_FLAG` — `world_persist`'s "a portal aimed you somewhere" byte.
///
/// Cleared before jumping to `$84A0`. A stale flag left by a telepad makes
/// `RESTORE_ARRIVAL` write the previous pad's coordinates over the ten
/// position variables `Map_Init` just set, parking the player off the map with
/// nowhere to walk — found on hardware, and documented in `world_persist`.
///
/// Named here rather than imported because that module is the POC's; the byte
/// itself is decided by `world_persist` and this is a mirror of one constant,
/// pinned by `arrival_flag_matches_world_persist`.
const ARRIVAL_FLAG: u16 = 0x7ABB;

// --- Part 1: the visited marker -----------------------------------------

/// `MO_NormalMoveEnter` (PRG010, CPU `$CDCA` = file 0x14DDA).
///
/// The map's "normal" operation handler — paths, canoe, bridges, entering
/// levels — so it runs on every map frame, moving or not. Its first eight
/// bytes are three whole instructions:
///
/// ```text
/// A9 00       LDA #$00
/// 8D 6E 79    STA Map_NoLoseTurn
/// 8D 73 79    STA Map_WasInPipeway
/// ```
///
/// Nothing branches into them, and PRG010 is the bank the routine lives in, so
/// the hook is a plain `JSR` with the tail NOP-padded. All eight are displaced
/// and all three replayed: the three bytes a five-byte hook would save are not
/// worth giving the routine an "A must be zero on exit" contract.
const MARK_HOOK_OFFSET: usize = 0x14DDA;
const MARK_HOOK_LEN: usize = 8;

/// Vanilla bytes at [`MARK_HOOK_OFFSET`].
#[cfg(test)]
#[rustfmt::skip]
const MARK_HOOK_VANILLA: [u8; MARK_HOOK_LEN] = [
    0xA9, 0x00,             // LDA #$00
    0x8D, 0x6E, 0x79,       // STA Map_NoLoseTurn
    0x8D, 0x73, 0x79,       // STA Map_WasInPipeway
];

/// Offset of [`START_XKEY`] inside [`MARK_VISITED`], and its CPU address.
///
/// The routine reads its own table absolutely, so it is **origin-locked**:
/// relocating `FS_MAZE_VISITED` without updating this breaks it silently, and
/// `.origin(MARK_VISITED_CPU)` in the checks is what catches that.
const MARK_TABLE_OFF: usize = 40;
const START_XKEY_CPU: u16 = MARK_VISITED_CPU + MARK_TABLE_OFF as u16;

/// Mark this world visited when the player is standing on its start tile.
///
/// 64 reserved, 48 used (40 code + an 8-byte table).
///
/// Two compares and a store, then the three displaced instructions. The
/// marking path *falls through* into the replay rather than jumping around it,
/// which is what makes all three failure branches share one target.
///
/// **The column test is exact, in both halves.** `AND #$0F / BNE` rejects any
/// sub-tile offset up front; with the low nibble known to be zero,
/// `World_Map_X | World_Map_XHi` is an injective key — the column lives in the
/// high nibble, the screen index is 0..3 — so one byte per world is enough and
/// a single `CMP` settles both. Composing the key with `ORA` *without* that
/// guard would alias: a player two pixels past the column on screen 1 would
/// match a start at the same column on screen 3.
///
/// The row is compared unmasked for the same reason. The net effect is that
/// "standing on the start tile" is true only when the player is actually
/// standing on it, not while sliding through it — and there is always such a
/// frame, because `Map_Init` places them there exactly and map moves land on
/// exact multiples of 16.
#[rustfmt::skip]
const MARK_VISITED: [u8; MARK_TABLE_OFF + 8] = [
    0xAE, PLAYER_CURRENT as u8, (PLAYER_CURRENT >> 8) as u8, //  0: LDX Player_Current
    0xB5, WORLD_MAP_X,                                       //  3: LDA World_Map_X,X
    0x29, 0x0F,                                              //  5: AND #$0F   ; sub-tile offset?
    0xD0, 0x16,                                              //  7: BNE +22 -> replay
    0xAC, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,           //  9: LDY World_Num
    0xB5, WORLD_MAP_X,                                       // 12: LDA World_Map_X,X
    0x15, WORLD_MAP_XHI,                                     // 14: ORA World_Map_XHi,X
    0xD9, START_XKEY_CPU as u8,
          (START_XKEY_CPU >> 8) as u8,                       // 16: CMP START_XKEY,Y
    0xD0, 0x0A,                                              // 19: BNE +10 -> replay
    0xB9, MAP_Y_STARTS_CPU as u8,
          (MAP_Y_STARTS_CPU >> 8) as u8,                     // 21: LDA Map_Y_Starts,Y
    0xD5, WORLD_MAP_Y,                                       // 24: CMP World_Map_Y,X
    0xD0, 0x03,                                              // 26: BNE +3 -> replay

    // A still holds Map_Y_Starts[World_Num], which is $20..$A0 and so never
    // zero: the marker byte is free.
    0x99, VISITED_TABLE as u8, (VISITED_TABLE >> 8) as u8,   // 28: STA VISITED,Y

    // ----- replay (31): the three displaced instructions -----
    0xA9, 0x00,                                              // 31: LDA #$00
    0x8D, MAP_NO_LOSE_TURN as u8,
          (MAP_NO_LOSE_TURN >> 8) as u8,                     // 33: STA Map_NoLoseTurn
    0x8D, MAP_WAS_IN_PIPEWAY as u8,
          (MAP_WAS_IN_PIPEWAY >> 8) as u8,                   // 36: STA Map_WasInPipeway
    0x60,                                                    // 39: RTS

    // ----- START_XKEY (40): one key per world, filled by `apply`. $FF is a
    // ----- key the live position can never produce (screen index is 0..3).
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
];

/// The packed column-and-screen key for one world's start tile.
///
/// `World_Map_X` is `(col % 16) << 4` and `World_Map_XHi` is `col / 16` — the
/// engine's own encoding, the same split `pipe_helpers::grid_pos_to_dest
/// _nibbles` produces for every other map coordinate in the codebase. With the
/// sub-tile nibble guaranteed zero the two `OR` together without collision.
fn start_xkey(col: usize) -> u8 {
    (((col % 16) as u8) << 4) | (col / 16) as u8
}

/// Derive [`START_XKEY`] and the row it implies, from the map in the ROM.
///
/// Returns the eight key bytes. Panics if a world has no START tile, or if the
/// row it sits on disagrees with `Map_Y_Starts` — which is what "you called
/// this before the overworld writer" looks like.
fn build_start_keys(rom: &Rom, grids: &[Grid]) -> [u8; 8] {
    let mut keys = [0u8; 8];
    for (world, key) in keys.iter_mut().enumerate() {
        let grid = &grids[world];
        let (row, col) = find_start(grid)
            .unwrap_or_else(|| panic!("W{} has no START tile on its map", world + 1));
        *key = start_xkey(col);

        // The engine's own answer for the same tile. If these disagree the
        // grid and `Map_Y_Starts` describe different maps, and a table built
        // from either would be wrong.
        let engine = rom.read_byte(MAP_Y_STARTS_OFF + world);
        let derived = (row as u8) * 0x10 + 0x20;
        assert_eq!(
            derived,
            engine,
            "W{}: START is at grid row {row}, which is World_Map_Y ${derived:02X}, but \
             Map_Y_Starts says ${engine:02X}. The map and the engine's start table \
             disagree — world_travel::apply must run after the overworld writer.",
            world + 1
        );
    }
    keys
}

// --- Part 2: the whistle cycler -----------------------------------------

/// `WWFX_WarpDoWind`'s world switch (PRG011, CPU `$A3C1` = file 0x163D1).
///
/// **The one point where vanilla has decided the whistle is taking you
/// somewhere.** Everything before it — the inventory flip, the tune, the white
/// flash, the wind sweeping the player off-screen — is reused verbatim; all
/// this replaces is the four instructions that name World 9 as the
/// destination:
///
/// ```text
/// AD 27 07    LDA World_Num
/// 8D F4 03    STA Map_Warp_PrevWorld
/// A9 08       LDA #$08
/// 8D 27 07    STA World_Num
/// ```
///
/// `Map_Warp_PrevWorld` is dropped along with them: it exists only for the
/// warp island's return trip, which nothing here reaches.
const TRAVEL_HOOK_OFFSET: usize = 0x163D1;
const TRAVEL_HOOK_LEN: usize = 11;

/// Vanilla bytes at [`TRAVEL_HOOK_OFFSET`].
#[cfg(test)]
#[rustfmt::skip]
const TRAVEL_HOOK_VANILLA: [u8; TRAVEL_HOOK_LEN] = [
    0xAD, 0x27, 0x07,       // LDA World_Num
    0x8D, 0xF4, 0x03,       // STA Map_Warp_PrevWorld
    0xA9, 0x08,             // LDA #$08
    0x8D, 0x27, 0x07,       // STA World_Num
];

/// Go to the next visited world, wrapping; stay put if there is nowhere to go.
///
/// 128 reserved, 34 used.
///
/// The scan walks `World_Num + 1` through `World_Num + 8` mod 8, so its last
/// probe is the world the player is standing in. That is what makes the
/// "nowhere to go" case free: after eight fruitless steps `Y` has wrapped back
/// to `World_Num` on its own, and the exit path stores it unchanged. One
/// visited world and eight visited worlds take the same code.
///
/// The exit sequence is the canonical world change, and every line of it is
/// load-bearing:
///
/// * `STY World_Num` **is** the transition trigger. There is no flag any more;
///   `completion_bits`' pack and expand hooks both test `World_Num !=
///   LIVE_WORLD`, so setting it before `$84A0` is the whole signal.
/// * `ARRIVAL_FLAG = 0` — see the constant.
/// * `Map_WarpWind_FX = 0` — otherwise the wind state machine advances into
///   `WWFX_WarpIslandInit` over the map that was just rebuilt.
/// * `LDX #$FF / TXS` — `$84A0` never returns, and this fires several frames
///   deep inside `Map_DoOperation`, so without the stack reset every trip
///   leaks a stack frame.
///
/// **It goes through `$84A0`, not `PRG030_857E`.** The partial re-init the
/// vanilla warp zone uses skips `Map_Init`, skips the `Map_Entered_*` restore,
/// and skips the `$84CD` hook where the outgoing world's progress is packed —
/// taking it would silently discard the world being left.
///
/// **Consequence accepted for the first cut: there is no palette fade-out.**
/// `Palette_FadeOut` lives in PRG026, which is not mapped from PRG011, so the
/// transition cuts to black. The airship path fades because it reaches `$84A0`
/// from code that could afford the bank switch; this cannot, and a fade is not
/// worth a trampoline.
///
/// Landing on the destination's start tile costs nothing: `Map_Init`
/// unconditionally stamps `Map_Y_Starts[World_Num]` into all ten position
/// variables (and `start_airship_swap`'s helper the column), and with
/// `ARRIVAL_FLAG` clear the arrival restore is a no-op.
#[rustfmt::skip]
const WHISTLE_TRAVEL: [u8; 34] = [
    0xAC, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,           //  0: LDY World_Num
    0xA2, VISITED_TABLE_LEN as u8,                           //  3: LDX #8      ; tries

    // ----- scan (5) -----
    0xC8,                                                    //  5: INY
    0x98,                                                    //  6: TYA
    0x29, (VISITED_TABLE_LEN - 1) as u8,                     //  7: AND #$07    ; wrap
    0xA8,                                                    //  9: TAY
    0xB9, VISITED_TABLE as u8, (VISITED_TABLE >> 8) as u8,   // 10: LDA VISITED,Y
    0xD0, 0x03,                                              // 13: BNE +3 -> go
    0xCA,                                                    // 15: DEX
    0xD0, 0xF3,                                              // 16: BNE -13 -> scan

    // ----- go (18): Y is the destination, or World_Num if nothing was found
    // ----- (eight wraps land back on it) -----
    0x8C, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,           // 18: STY World_Num
    0xA9, 0x00,                                              // 21: LDA #$00
    0x8D, ARRIVAL_FLAG as u8, (ARRIVAL_FLAG >> 8) as u8,     // 23: STA ARRIVAL_FLAG
    0x85, MAP_WARPWIND_FX,                                   // 26: STA Map_WarpWind_FX
    0xA2, 0xFF,                                              // 28: LDX #$FF
    0x9A,                                                    // 30: TXS
    0x4C, WORLD_MAP_INIT_CPU as u8,
          (WORLD_MAP_INIT_CPU >> 8) as u8,                   // 31: JMP $84A0   ; never returns
];

// --- Writer -------------------------------------------------------------

/// `JSR Inv_UseItem_ShiftOver` inside `Inv_UseItem_WarpWhistle` — the three
/// bytes that DELETE the whistle from the inventory once it is blown. CPU
/// `$A7A9` in PRG026, which is mapped at `$A000` while the inventory is open.
///
/// Vanilla wants this: a whistle is a one-shot warp. **The maze does not.**
/// Here the whistle is fast travel between worlds already visited, and the
/// mode's whole promise is that using it again takes you on to the next one —
/// which one whistle and one use cannot deliver. `Inv_UseItem_ShiftOver` does
/// nothing but back the remaining items over the used one (`prg026.asm:987`),
/// so skipping it leaves the whistle in the inventory and changes nothing else;
/// `Inventory_ForceFlip` two instructions later still closes the panel.
///
/// This is safe to give away because a maze whistle **can never reach anywhere
/// new**: the cycler only visits worlds whose `VISITED` byte is already set, so
/// an unlimited whistle is unlimited *backtracking*, not a sequence break. That
/// is the same argument `remove_whistles` rests on, and it is why that option
/// keeps its intent under this mode while changing mechanism.
pub(crate) const WHISTLE_CONSUME_OFFSET: usize = 0x347B9;

/// What is there in vanilla, asserted before it is replaced.
#[cfg(test)]
const WHISTLE_CONSUME_VANILLA: [u8; 3] = [0x20, 0x1B, 0xA6];

/// Install both halves of whistle fast travel.
///
/// **Must run after the overworld writer** — see the module doc; the start-key
/// table is derived from the map in the ROM, and [`build_start_keys`] panics
/// rather than emit one that disagrees with `Map_Y_Starts`.
///
/// Order between the two routines does not matter — they share no bytes and no
/// hook site — but both are required: the cycler with no marker can only ever
/// stay put, and the marker with no cycler writes a table nothing reads.
///
/// **Depends on `completion_bits` being installed**, because the whole
/// transition rests on its `World_Num != LIVE_WORLD` hooks packing the
/// outgoing world. Not asserted here: this module writes ROM, it cannot see
/// what else the pipeline chose.
pub(crate) fn apply(rom: &mut Rom, grids: &[Grid]) {
    let keys = build_start_keys(rom, grids);

    rom.push_tag("world_travel");

    let mut marker = MARK_VISITED;
    marker[MARK_TABLE_OFF..].copy_from_slice(&keys);
    rom.write_range(FS_MAZE_VISITED, &marker);
    let mut hook = [0xEA_u8; MARK_HOOK_LEN];
    hook[0] = 0x20; // JSR
    hook[1] = MARK_VISITED_CPU as u8;
    hook[2] = (MARK_VISITED_CPU >> 8) as u8;
    rom.write_range(MARK_HOOK_OFFSET, &hook);

    // Keep the whistle. See `WHISTLE_CONSUME_OFFSET`: the mode asks for
    // repeated use, and vanilla deletes the item on the first one.
    rom.write_range(WHISTLE_CONSUME_OFFSET, &[0xEA, 0xEA, 0xEA]);

    rom.write_range(FS_MAZE_TRAVEL, &WHISTLE_TRAVEL);
    // `JMP`, not `JSR`: the routine never comes back, so a return address
    // would be pure litter — and the stack is reset a few bytes later anyway.
    let mut hook = [0xEA_u8; TRAVEL_HOOK_LEN];
    hook[0] = 0x4C; // JMP
    hook[1] = WHISTLE_TRAVEL_CPU as u8;
    hook[2] = (WHISTLE_TRAVEL_CPU >> 8) as u8;
    rom.write_range(TRAVEL_HOOK_OFFSET, &hook);

    rom.pop_tag();
}

#[cfg(test)]
mod asm_checks {
    use mos6502::cpu::CPU;
    use mos6502::instruction::Ricoh2a03;
    use mos6502::memory::{Bus, Memory};
    use mos6502::registers::StackPointer;

    use super::*;
    use crate::randomize::rom_data::asm;

    fn load_vanilla() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    /// A fully randomized ROM — the overworld writer has run, so the START
    /// tiles on the map are the ones the engine will spawn the player on.
    /// `swap` turns `start_airship_swap` on, which is the arm a `#$20` column
    /// compare gets wrong.
    fn built(seed: u64, swap: bool) -> Option<Rom> {
        let mut rom = load_vanilla()?;
        let opts = crate::randomizer::Options {
            swap_start_airship: swap,
            ..crate::randomizer::Options::default()
        };
        crate::randomizer::randomize(&mut rom, seed, &opts);
        Some(rom)
    }

    // --- Structural ------------------------------------------------------

    #[test]
    fn mark_visited_is_well_formed() {
        asm::check(&MARK_VISITED)
            .allocation(FS_MAZE_VISITED)
            .origin(MARK_VISITED_CPU)
            .data_from(MARK_TABLE_OFF)
            // The three live position bytes are engine variables it reads and
            // never writes; it parks nothing of its own in the page. Naming
            // them keeps the bound at the NMI-safe three rather than raising
            // it to `$79` and permitting everything below.
            .zero_page(NMI_SAFE_MAX, &[WORLD_MAP_Y, WORLD_MAP_XHI, WORLD_MAP_X])
            .assert_ok();
    }

    #[test]
    fn whistle_travel_is_well_formed() {
        asm::check(&WHISTLE_TRAVEL)
            .allocation(FS_MAZE_TRAVEL)
            .origin(WHISTLE_TRAVEL_CPU)
            // `Map_WarpWind_FX` is the engine's own whistle state, cleared on
            // the way out; nothing else in the page is touched.
            .zero_page(NMI_SAFE_MAX, &[MAP_WARPWIND_FX])
            .assert_ok();
    }

    /// The marker's hook displaces three whole instructions and the routine
    /// replays all three, so a frame on any other tile is byte-for-byte what
    /// vanilla did.
    #[test]
    fn mark_hook_displaces_whole_instructions() {
        let Some(rom) = load_vanilla() else { return };
        assert_eq!(
            rom.read_range(MARK_HOOK_OFFSET, MARK_HOOK_LEN),
            MARK_HOOK_VANILLA,
            "MO_NormalMoveEnter has moved, or something else already hooked it"
        );
        assert_eq!(
            MARK_VISITED[31..39],
            MARK_HOOK_VANILLA,
            "the replay must be exactly what the hook overwrote"
        );

        let mut jsr = [0xEA_u8; MARK_HOOK_LEN];
        jsr[0] = 0x20;
        jsr[1] = MARK_VISITED_CPU as u8;
        jsr[2] = (MARK_VISITED_CPU >> 8) as u8;
        asm::check(&MARK_VISITED)
            .allocation(FS_MAZE_VISITED)
            .origin(MARK_VISITED_CPU)
            .data_from(MARK_TABLE_OFF)
            .hook(&MARK_HOOK_VANILLA, 0, &jsr)
            .assert_ok();
    }

    /// The cycler's hook displaces four whole instructions and replays none of
    /// them: it *replaces* the decision they encoded.
    #[test]
    fn travel_hook_displaces_whole_instructions() {
        let Some(rom) = load_vanilla() else { return };
        assert_eq!(
            rom.read_range(TRAVEL_HOOK_OFFSET, TRAVEL_HOOK_LEN),
            TRAVEL_HOOK_VANILLA,
            "WWFX_WarpDoWind's world switch has moved"
        );

        let mut jmp = [0xEA_u8; TRAVEL_HOOK_LEN];
        jmp[0] = 0x4C;
        jmp[1] = WHISTLE_TRAVEL_CPU as u8;
        jmp[2] = (WHISTLE_TRAVEL_CPU >> 8) as u8;
        asm::check(&WHISTLE_TRAVEL)
            .allocation(FS_MAZE_TRAVEL)
            .origin(WHISTLE_TRAVEL_CPU)
            .hook(&TRAVEL_HOOK_VANILLA, 0, &jmp)
            .assert_ok();
    }

    /// Both hooks name their routine, and `apply` writes both.
    #[test]
    fn apply_installs_both_hooks() {
        let Some(rom) = load_vanilla() else { return };
        let mut patched = rom.clone();
        let grids = crate::randomize::rom_data::read_all_tile_grids(&patched);
        apply(&mut patched, &grids);

        assert_eq!(
            patched.read_range(MARK_HOOK_OFFSET, 3),
            [0x20, MARK_VISITED_CPU as u8, (MARK_VISITED_CPU >> 8) as u8],
        );
        assert_eq!(
            patched.read_range(TRAVEL_HOOK_OFFSET, 3),
            [0x4C, WHISTLE_TRAVEL_CPU as u8, (WHISTLE_TRAVEL_CPU >> 8) as u8],
        );
        assert_eq!(
            patched.read_range(FS_MAZE_VISITED, MARK_TABLE_OFF),
            &MARK_VISITED[..MARK_TABLE_OFF]
        );
        assert_eq!(patched.read_range(FS_MAZE_TRAVEL, WHISTLE_TRAVEL.len()), WHISTLE_TRAVEL);
    }

    /// The marker's two table reads name the right addresses. `Map_Y_Starts`
    /// points *outside* the routine so `.origin` cannot judge it, and
    /// `START_XKEY` is what makes the routine origin-locked.
    #[test]
    fn the_marker_reads_the_right_tables() {
        assert_eq!(MARK_VISITED[16], 0xD9, "offset 16 is not a CMP abs,Y");
        assert_eq!(
            u16::from_le_bytes([MARK_VISITED[17], MARK_VISITED[18]]),
            START_XKEY_CPU,
            "the column compare must read this routine's own key table"
        );
        assert_eq!(
            START_XKEY_CPU,
            MARK_VISITED_CPU + MARK_TABLE_OFF as u16,
            "the key table must sit at the routine's declared table offset"
        );

        assert_eq!(MARK_VISITED[21], 0xB9, "offset 21 is not an LDA abs,Y");
        assert_eq!(
            u16::from_le_bytes([MARK_VISITED[22], MARK_VISITED[23]]),
            MAP_Y_STARTS_CPU,
            "the row compare must read Map_Y_Starts"
        );
        assert_eq!(MAP_Y_STARTS_CPU, 0x838A, "Map_Y_Starts is not where PRG030 puts it");
    }

    /// `ARRIVAL_FLAG` is `world_persist`'s byte, mirrored here. If that module
    /// moves it, a whistle trip starts writing over whatever took its place.
    #[test]
    fn arrival_flag_matches_world_persist() {
        // The flag sits in completion_bits' half of the $7A73 run, immediately
        // before LIVE_WORLD at $7ABC.
        assert_eq!(ARRIVAL_FLAG, 0x7ABB);
        const { assert!(ARRIVAL_FLAG < VISITED_TABLE) };
    }

    /// **The marker byte is the start row itself, so the start row must never
    /// be zero.** A zero would be indistinguishable from "never been here" and
    /// the world would drop out of the whistle's cycle.
    ///
    /// Rows are `(grid_row + 2) << 4` for grid rows 0..8, so the range is
    /// `$20..$A0` by construction; this pins the ROM against that.
    #[test]
    fn map_y_starts_can_never_be_zero() {
        let Some(rom) = load_vanilla() else { return };
        for world in 0..8 {
            let y = rom.read_byte(MAP_Y_STARTS_OFF + world);
            assert!(
                (0x20..=0xA0).contains(&y) && y & 0x0F == 0,
                "W{}: Map_Y_Starts is ${y:02X}, outside the $20..$A0 row encoding",
                world + 1
            );
        }
        assert_eq!(VANILLA_Y_STARTS, rom.read_range(MAP_Y_STARTS_OFF, 8), "the fixture is stale");
    }

    /// The visited table the cycler scans is the one `maze_state` allocated,
    /// and the scan's wrap mask matches its length.
    #[test]
    fn the_cycler_scans_the_allocated_table() {
        assert_eq!(WHISTLE_TRAVEL[10], 0xB9, "offset 10 is not an LDA abs,Y");
        assert_eq!(u16::from_le_bytes([WHISTLE_TRAVEL[11], WHISTLE_TRAVEL[12]]), VISITED_TABLE);
        assert_eq!(WHISTLE_TRAVEL[4], VISITED_TABLE_LEN as u8, "the try count is not the length");
        assert_eq!(WHISTLE_TRAVEL[8], (VISITED_TABLE_LEN - 1) as u8, "the wrap mask is wrong");
        assert!(VISITED_TABLE_LEN.is_power_of_two(), "AND-wrapping needs a power-of-two length");
        // The marker writes into the same table.
        assert_eq!(u16::from_le_bytes([MARK_VISITED[29], MARK_VISITED[30]]), VISITED_TABLE);
    }

    // --- The start-key table --------------------------------------------

    /// The emitted table is what the map says, world by world — on a vanilla
    /// ROM, and on a randomized one with `start_airship_swap` on, which is the
    /// arm the old `#$20` compare got wrong.
    ///
    /// The oracle is `find_start` over the ROM's own grid, which is a
    /// different code path from `start_xkey`'s arithmetic, and the swapped arm
    /// additionally checks the table against `start_airship_swap`'s own
    /// `FS_SAS_*` position tables — the bytes `Map_Init` will really stamp.
    #[test]
    fn the_start_key_table_matches_the_map() {
        use crate::randomize::rom_data::{FS_SAS_X_TABLE, FS_SAS_XHI_TABLE};

        let Some(vanilla) = load_vanilla() else { return };

        let mut moved_worlds = 0;
        for (label, swap, rom) in [
            ("vanilla", false, vanilla.clone()),
            ("built", false, built(0x5EED, false).unwrap()),
            ("built + start/airship swap", true, built(0x5EED, true).unwrap()),
        ] {
            let mut patched = rom.clone();
            let grids = crate::randomize::rom_data::read_all_tile_grids(&patched);
            apply(&mut patched, &grids);
            let table = patched.read_range(FS_MAZE_VISITED + MARK_TABLE_OFF, 8).to_vec();

            for (world, &key) in table.iter().enumerate() {
                let grid = crate::randomize::rom_data::read_tile_grid(&rom, world);
                let (_, col) = find_start(&grid).unwrap();
                assert_eq!(
                    key,
                    start_xkey(col),
                    "{label} W{}: START is at column {col}, key ${:02X}, table says ${:02X}",
                    world + 1,
                    start_xkey(col),
                    key,
                );
                if key != 0x20 {
                    moved_worlds += 1;
                }
                if swap {
                    // The engine's own answer, from the tables Map_Init reads.
                    let x = rom.read_byte(FS_SAS_X_TABLE + world);
                    let xhi = rom.read_byte(FS_SAS_XHI_TABLE + world);
                    assert_eq!(
                        key,
                        x | xhi,
                        "{label} W{}: Map_Init will place the player at X ${x:02X} / XHi \
                         ${xhi:02X}, key ${:02X}, but the table says ${:02X}",
                        world + 1,
                        x | xhi,
                        key,
                    );
                }
            }
        }
        assert!(
            moved_worlds > 0,
            "no world in any arm started away from column 2 screen 0, so this test never \
             exercised the case a #$20 compare gets wrong — pick another seed"
        );
    }

    /// `apply` refuses to emit a table that disagrees with the engine, which
    /// is what calling it before the overworld writer looks like.
    #[test]
    fn a_map_that_disagrees_with_map_y_starts_is_refused() {
        let Some(rom) = load_vanilla() else { return };
        let mut broken = rom.clone();
        // Move W1's start row in the engine's table only.
        broken.write_byte(MAP_Y_STARTS_OFF, 0x70);
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let mut b = broken;
            let grids = crate::randomize::rom_data::read_all_tile_grids(&b);
            apply(&mut b, &grids);
        }));
        assert!(err.is_err(), "a stale Map_Y_Starts must not produce a silently wrong table");
    }

    // --- Execution -------------------------------------------------------
    //
    // `asm::check` proves the bytes decode; it cannot see that they compute
    // the right thing. Four planted mutations survived every structural test
    // on `PAD_ENTER` before it was executed, so both routines here are run.
    // Neither calls out to the engine before its final `RTS`/`JMP`, which is
    // what makes that possible.

    /// Vanilla `Map_Y_Starts`. Checked against the ROM by
    /// `map_y_starts_can_never_be_zero`, so the fixture cannot go stale
    /// silently.
    const VANILLA_Y_STARTS: [u8; 8] = [0x40, 0xA0, 0xA0, 0x40, 0x80, 0x60, 0x30, 0x50];

    /// Vanilla start columns — every world starts at column 2 of screen 0,
    /// which is what `Map_Init` hardcodes. The swapped case is covered by
    /// `a_start_away_from_column_2_still_marks`.
    const VANILLA_START_COLS: [usize; 8] = [2; 8];

    /// Where the fixtures park a return address; reaching it means the routine
    /// executed an `RTS`.
    const SENTINEL: u16 = 0x0F00;

    fn run(cpu: &mut CPU<Memory, Ricoh2a03>, entry: u16, stop: u16) -> bool {
        let ret = SENTINEL.wrapping_sub(1);
        cpu.memory.set_byte(0x01FF, (ret >> 8) as u8);
        cpu.memory.set_byte(0x01FE, ret as u8);
        cpu.registers.stack_pointer = StackPointer(0xFD);
        cpu.registers.program_counter = entry;
        for _ in 0..10_000 {
            match cpu.registers.program_counter {
                pc if pc == stop => return true,
                SENTINEL => return false,
                _ => {
                    cpu.single_step();
                }
            }
        }
        panic!("routine ran away");
    }

    /// Run the marker with `code`, a start-column table, and the player parked
    /// at the raw pixel bytes `(xhi, x, y)`; returns the visited table.
    fn mark_with(
        code: &[u8],
        cols: &[usize; 8],
        world: u8,
        player: u8,
        xhi: u8,
        x: u8,
        y: u8,
    ) -> [u8; 8] {
        let mut prog = code.to_vec();
        for (wi, col) in cols.iter().enumerate() {
            prog[MARK_TABLE_OFF + wi] = start_xkey(*col);
        }

        let mut mem = Memory::new();
        mem.set_bytes(MARK_VISITED_CPU, &prog);
        mem.set_bytes(MAP_Y_STARTS_CPU, &VANILLA_Y_STARTS);
        mem.set_byte(PLAYER_CURRENT, player);
        mem.set_byte(WORLD_NUM, world);
        mem.set_byte(WORLD_MAP_XHI as u16 + player as u16, xhi);
        mem.set_byte(WORLD_MAP_X as u16 + player as u16, x);
        mem.set_byte(WORLD_MAP_Y as u16 + player as u16, y);
        for i in 0..VISITED_TABLE_LEN {
            mem.set_byte(VISITED_TABLE + i as u16, 0);
        }
        // Poison both replayed flags, so "was replayed" is visible.
        mem.set_byte(MAP_NO_LOSE_TURN, 0xAA);
        mem.set_byte(MAP_WAS_IN_PIPEWAY, 0xAA);

        let mut cpu = CPU::new(mem, Ricoh2a03);
        assert!(!run(&mut cpu, MARK_VISITED_CPU, WORLD_MAP_INIT_CPU), "the marker must RTS");
        // The displaced instructions run on every path, marked or not.
        assert_eq!(cpu.memory.get_byte(MAP_NO_LOSE_TURN), 0, "Map_NoLoseTurn was not replayed");
        assert_eq!(cpu.memory.get_byte(MAP_WAS_IN_PIPEWAY), 0, "Map_WasInPipeway was not replayed");

        let mut out = [0u8; 8];
        for (i, b) in out.iter_mut().enumerate() {
            *b = cpu.memory.get_byte(VISITED_TABLE + i as u16);
        }
        out
    }

    /// The vanilla-layout shorthand: every world starts at column 2, screen 0.
    fn mark(code: &[u8], world: u8, player: u8, xhi: u8, x: u8, y: u8) -> [u8; 8] {
        mark_with(code, &VANILLA_START_COLS, world, player, xhi, x, y)
    }

    /// Standing on the start tile marks exactly this world, for both players
    /// and all eight worlds.
    #[test]
    fn standing_on_the_start_tile_marks_the_world() {
        for world in 0..8u8 {
            for player in 0..2u8 {
                let got = mark(
                    &MARK_VISITED,
                    world,
                    player,
                    0x00,
                    0x20,
                    VANILLA_Y_STARTS[world as usize],
                );
                assert_ne!(got[world as usize], 0, "W{} P{player} was not marked", world + 1);
                for (i, &b) in got.iter().enumerate() {
                    if i != world as usize {
                        assert_eq!(b, 0, "W{} P{player} also marked world {}", world + 1, i + 1);
                    }
                }
            }
        }
    }

    /// **The bug this table exists for.** A world whose start is not column 2
    /// of screen 0 — what `start_airship_swap` produces — still marks, and the
    /// vanilla position in that same world does not.
    ///
    /// Column 22 is screen 1, column 6: key `$61`. A `CMP #$20` against
    /// `World_Map_X` alone would never fire for it, and a compare that ignored
    /// the screen would fire at column 6 of screen 0 too.
    #[test]
    fn a_start_away_from_column_2_still_marks() {
        let mut cols = VANILLA_START_COLS;
        cols[4] = 22; // W5's start moved to screen 1, column 6
        cols[7] = 35; // W8's to screen 2, column 3
        let w5_y = VANILLA_Y_STARTS[4];
        let w8_y = VANILLA_Y_STARTS[7];

        assert_ne!(
            mark_with(&MARK_VISITED, &cols, 4, 0, 0x01, 0x60, w5_y)[4],
            0,
            "W5's moved start did not mark"
        );
        assert_ne!(
            mark_with(&MARK_VISITED, &cols, 7, 0, 0x02, 0x30, w8_y)[7],
            0,
            "W8's moved start did not mark"
        );

        // ...and the places a sloppier compare would have fired.
        for (what, world, xhi, x, y) in [
            ("W5 at the vanilla column 2 screen 0", 4u8, 0x00, 0x20, w5_y),
            ("W5 at column 6 of the wrong screen", 4, 0x00, 0x60, w5_y),
            ("W5 at column 6 of screen 2", 4, 0x02, 0x60, w5_y),
            ("W8 at column 3 of screen 0", 7, 0x00, 0x30, w8_y),
            ("W8 at column 3 of screen 3", 7, 0x03, 0x30, w8_y),
        ] {
            assert_eq!(
                mark_with(&MARK_VISITED, &cols, world, 0, xhi, x, y),
                [0u8; 8],
                "{what} marked the world"
            );
        }
    }

    /// **The aliasing the sub-tile guard exists for.**
    ///
    /// The key is `World_Map_X | World_Map_XHi`, which is injective only while
    /// the column's low nibble is zero. Give W3 a start at column 2 of screen 3
    /// (key `$23`) and a player two pixels past column 2 on **screen 1** would
    /// compose `$22 | $01 = $23` and match a tile three screens away. The
    /// `AND #$0F / BNE` up front is what makes that impossible, and this is the
    /// case that proves it does.
    #[test]
    fn a_sub_tile_offset_cannot_alias_onto_another_screen() {
        let mut cols = VANILLA_START_COLS;
        cols[2] = 50; // screen 3, column 2 -> key $23
        let y = VANILLA_Y_STARTS[2];

        assert_eq!(
            mark_with(&MARK_VISITED, &cols, 2, 0, 0x01, 0x22, y),
            [0u8; 8],
            "two pixels past column 2 on screen 1 aliased onto the start on screen 3"
        );
        // The real tile still marks.
        assert_ne!(mark_with(&MARK_VISITED, &cols, 2, 0, 0x03, 0x20, y)[2], 0);
    }

    /// The marker byte is the start row, and the start row is never zero — so
    /// `BNE` is a sound "have I been here" test.
    #[test]
    fn the_marker_byte_is_the_nonzero_start_row() {
        for world in 0..8u8 {
            let got = mark(&MARK_VISITED, world, 0, 0x00, 0x20, VANILLA_Y_STARTS[world as usize]);
            assert_eq!(got[world as usize], VANILLA_Y_STARTS[world as usize]);
        }
    }

    /// Everything that is not the start tile leaves the table alone —
    /// including both mid-move cases. The column guard (`AND #$0F`) and the
    /// unmasked row compare between them mean "standing on the start tile" is
    /// false while the player is sliding through it on either axis.
    #[test]
    fn nowhere_else_marks_the_world() {
        let w = 2u8; // W3, start row $A0
        let start_y = VANILLA_Y_STARTS[w as usize];
        let cases: [(&str, u8, u8, u8); 8] = [
            ("one column right", 0x00, 0x30, start_y),
            ("one column left", 0x00, 0x10, start_y),
            ("one row down", 0x00, 0x20, start_y + 0x10),
            ("one row up", 0x00, 0x20, start_y - 0x10),
            ("the wrong screen", 0x01, 0x20, start_y),
            ("screen 3", 0x03, 0x20, start_y),
            ("mid horizontal move (column low nibble set)", 0x00, 0x28, start_y),
            ("mid vertical move (row low nibble set)", 0x00, 0x20, start_y + 0x08),
        ];
        for (what, xhi, x, y) in cases {
            assert_eq!(mark(&MARK_VISITED, w, 0, xhi, x, y), [0u8; 8], "{what} marked the world");
        }
    }

    /// **Mutation test for the marker.** Each planted fault is one a structural
    /// check passes and a player would eventually hit.
    #[test]
    fn the_marker_tests_catch_planted_faults() {
        let start_y = VANILLA_Y_STARTS[2];
        let ok = |code: &[u8; MARK_TABLE_OFF + 8]| {
            asm::check(code).origin(MARK_VISITED_CPU).data_from(MARK_TABLE_OFF).assert_ok();
        };

        // 1. The column compare's branch is 3 bytes short, so a mismatch falls
        //    into the row compare instead of the replay.
        let mut wrong_branch = MARK_VISITED;
        wrong_branch[20] = 0x07; // BNE at 19: +10 -> +7, landing on `LDA Map_Y_Starts,Y`
        ok(&wrong_branch);
        assert_ne!(
            mark(&wrong_branch, 2, 0, 0x00, 0x30, start_y),
            [0u8; 8],
            "a wrong branch displacement must break `nowhere_else_marks_the_world`"
        );

        // 2. The key table is read one byte high, so every world checks the
        //    next world's start column.
        let mut wrong_key_index = MARK_VISITED;
        wrong_key_index[17] = wrong_key_index[17].wrapping_add(1);
        ok(&wrong_key_index);
        let mut cols = VANILLA_START_COLS;
        cols[3] = 22; // give W4 a different start, so W3's row-3 read differs
        assert_eq!(
            mark_with(&wrong_key_index, &cols, 2, 0, 0x00, 0x20, start_y),
            [0u8; 8],
            "an off-by-one START_XKEY index must break the start-tile test"
        );
        // ...and the same fault marks W3 from W4's start tile, which is worse.
        assert_ne!(
            mark_with(&wrong_key_index, &cols, 2, 0, 0x01, 0x60, start_y)[2],
            0,
            "the off-by-one must be visible as a false positive too"
        );

        // 3. The store indexes on X (the player) instead of Y (the world).
        let mut wrong_index = MARK_VISITED;
        wrong_index[28] = 0x9D; // STA abs,Y -> STA abs,X
        ok(&wrong_index);
        let got = mark(&wrong_index, 2, 0, 0x00, 0x20, start_y);
        assert_eq!(
            got[2], 0,
            "a wrong table index must break `standing_on_the_start_tile_marks_the_world`"
        );
        assert_ne!(got[0], 0, "...by marking world 1 (X = Player_Current = 0) instead");

        // 4. The row table is read at World_Num + 1.
        let mut wrong_table = MARK_VISITED;
        wrong_table[22] = wrong_table[22].wrapping_add(1);
        ok(&wrong_table);
        assert_eq!(
            mark(&wrong_table, 2, 0, 0x00, 0x20, start_y),
            [0u8; 8],
            "an off-by-one Map_Y_Starts read must break the start-tile test"
        );

        // 5. The sub-tile guard is defeated (`AND #$00` can never be
        //    non-zero), which re-opens the cross-screen aliasing the guard
        //    exists to close.
        let mut no_guard = MARK_VISITED;
        no_guard[6] = 0x00;
        ok(&no_guard);
        let mut cols3 = VANILLA_START_COLS;
        cols3[2] = 50; // screen 3, column 2 -> key $23
        assert_ne!(
            mark_with(&no_guard, &cols3, 2, 0, 0x01, 0x22, start_y),
            [0u8; 8],
            "defeating the sub-tile guard must break \
             `a_sub_tile_offset_cannot_alias_onto_another_screen`"
        );
    }

    /// A CPU with `code` at the cycler's origin, `world` current and `visited`
    /// stamped into the table.
    fn travel_cpu(code: &[u8], world: u8, visited: &[u8]) -> CPU<Memory, Ricoh2a03> {
        let mut mem = Memory::new();
        mem.set_bytes(WHISTLE_TRAVEL_CPU, code);
        mem.set_byte(WORLD_NUM, world);
        for (i, &v) in visited.iter().enumerate() {
            mem.set_byte(VISITED_TABLE + i as u16, v);
        }
        // Poison what the exit sequence must clear.
        mem.set_byte(ARRIVAL_FLAG, 0xAA);
        mem.set_byte(MAP_WARPWIND_FX as u16, 0x02);
        CPU::new(mem, Ricoh2a03)
    }

    /// Run the cycler; returns the world it left in `World_Num`.
    fn travel(code: &[u8], world: u8, visited: &[u8]) -> u8 {
        let mut cpu = travel_cpu(code, world, visited);
        assert!(
            run(&mut cpu, WHISTLE_TRAVEL_CPU, WORLD_MAP_INIT_CPU),
            "the cycler must reach $84A0 on every path"
        );
        assert_eq!(cpu.memory.get_byte(ARRIVAL_FLAG), 0, "ARRIVAL_FLAG was not cleared");
        assert_eq!(cpu.memory.get_byte(MAP_WARPWIND_FX as u16), 0, "Map_WarpWind_FX not cleared");
        assert_eq!(
            cpu.registers.stack_pointer,
            StackPointer(0xFF),
            "the stack was not reset before the jump that never returns"
        );
        cpu.memory.get_byte(WORLD_NUM)
    }

    /// The visited-world table as a byte array. Visited worlds carry their
    /// start row, exactly as the marker writes it.
    fn table(worlds: &[u8]) -> [u8; 8] {
        let mut t = [0u8; 8];
        for &w in worlds {
            t[w as usize] = VANILLA_Y_STARTS[w as usize];
        }
        t
    }

    /// The cycle: next visited world, skipping the unvisited, wrapping at the
    /// end.
    #[test]
    fn the_whistle_cycles_through_visited_worlds() {
        let t = table(&[0, 2, 5]);
        assert_eq!(travel(&WHISTLE_TRAVEL, 0, &t), 2, "W1 -> W3");
        assert_eq!(travel(&WHISTLE_TRAVEL, 2, &t), 5, "W3 -> W6");
        assert_eq!(travel(&WHISTLE_TRAVEL, 5, &t), 0, "W6 wraps to W1");
        // From a world that is not itself visited, the scan still works.
        assert_eq!(travel(&WHISTLE_TRAVEL, 3, &t), 5, "W4 (unvisited) -> W6");
        assert_eq!(travel(&WHISTLE_TRAVEL, 6, &t), 0, "W7 (unvisited) wraps to W1");

        let all = table(&[0, 1, 2, 3, 4, 5, 6, 7]);
        for w in 0..8u8 {
            assert_eq!(travel(&WHISTLE_TRAVEL, w, &all), (w + 1) % 8, "W{} -> next", w + 1);
        }
    }

    /// One visited world, or none at all, is a no-op: the player is put back
    /// on their own world's start tile rather than sent anywhere.
    #[test]
    fn one_visited_world_is_a_no_op() {
        for w in 0..8u8 {
            assert_eq!(travel(&WHISTLE_TRAVEL, w, &table(&[w])), w, "W{} alone", w + 1);
            assert_eq!(travel(&WHISTLE_TRAVEL, w, &[0u8; 8]), w, "W{}, nothing visited", w + 1);
        }
        // Visited somewhere else entirely, and standing in an unvisited world:
        // that one destination is where the whistle goes.
        assert_eq!(travel(&WHISTLE_TRAVEL, 4, &table(&[1])), 1);
    }

    /// **Mutation test for the cycler.**
    #[test]
    fn the_cycler_tests_catch_planted_faults() {
        let t = table(&[0, 2, 5]);

        // 1. The loop-back branch is one byte short, so the scan restarts at
        //    `TYA` and never advances Y past the first probe.
        let mut wrong_branch = WHISTLE_TRAVEL;
        wrong_branch[17] = 0xF4; // BNE -13 -> -12, landing on `TYA` instead of `INY`
        asm::check(&wrong_branch).origin(WHISTLE_TRAVEL_CPU).assert_ok();
        assert_ne!(
            travel(&wrong_branch, 0, &t),
            2,
            "a wrong loop-back displacement must break the cycle test"
        );

        // 2. The scan reads one byte past the table (WAND_COUNT's byte, in the
        //    real map) — an off-by-one in the table address.
        let mut wrong_index = WHISTLE_TRAVEL;
        wrong_index[11] = wrong_index[11].wrapping_add(1);
        asm::check(&wrong_index).origin(WHISTLE_TRAVEL_CPU).assert_ok();
        assert_ne!(
            travel(&wrong_index, 0, &t),
            2,
            "an off-by-one table address must break the cycle test"
        );

        // 3. The wrap mask is $0F rather than $07, so the scan walks off the
        //    end of the table.
        let mut wrong_mask = WHISTLE_TRAVEL;
        wrong_mask[8] = 0x0F;
        asm::check(&wrong_mask).origin(WHISTLE_TRAVEL_CPU).assert_ok();
        assert_ne!(travel(&wrong_mask, 5, &t), 0, "a wrong wrap mask must break the wraparound");
    }
}

#[cfg(test)]
mod whistle_reuse {
    use super::*;

    fn vanilla() -> Option<Rom> {
        let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&bytes).ok()
    }

    /// The site really is the `JSR Inv_UseItem_ShiftOver` the disassembly
    /// names, read out of the ROM rather than transcribed — and it is one
    /// whole instruction, so NOPping it displaces nothing else.
    #[test]
    fn the_consume_site_is_the_shift_over_call() {
        let Some(rom) = vanilla() else { return };
        assert_eq!(
            rom.read_range(WHISTLE_CONSUME_OFFSET, 3),
            WHISTLE_CONSUME_VANILLA,
            "the whistle's consume call is not where this module thinks it is"
        );
        // $A61B is Inv_UseItem_ShiftOver: PRG026 at $A000, file 0x34010.
        let target = u16::from_le_bytes([WHISTLE_CONSUME_VANILLA[1], WHISTLE_CONSUME_VANILLA[2]]);
        assert_eq!(target, 0xA61B, "the JSR does not go to Inv_UseItem_ShiftOver");
    }

    /// A maze whistle survives being blown, and nothing else in the handler
    /// moves. The mode asks for repeated use; vanilla deletes the item on the
    /// first one, so this is the difference between fast travel and a
    /// single-shot warp.
    #[test]
    fn the_maze_whistle_is_not_consumed() {
        let Some(rom) = vanilla() else { return };
        let mut patched = rom.clone();
        let grids = crate::randomize::rom_data::read_all_tile_grids(&patched);
        apply(&mut patched, &grids);
        assert_eq!(
            patched.read_range(WHISTLE_CONSUME_OFFSET, 3),
            [0xEA, 0xEA, 0xEA],
            "the whistle is still consumed on use"
        );
        // The instructions either side are untouched: STX Map_WarpWind_FX
        // before, LDA #MUS2A_WARPWHISTLE after.
        assert_eq!(patched.read_range(WHISTLE_CONSUME_OFFSET - 2, 2), [0x86, 0x8B]);
        assert_eq!(patched.read_range(WHISTLE_CONSUME_OFFSET + 3, 2), [0xA9, 0x0B]);
    }
}
