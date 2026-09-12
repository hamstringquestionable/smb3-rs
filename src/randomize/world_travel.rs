//! World-maze fast travel: the warp whistle becomes a hop between worlds the
//! player has already been to.
//!
//! Three routines, and they are deliberately independent of everything else
//! the maze installs:
//!
//! | routine | bank | fires |
//! |---|---|---|
//! | [`MARK_VISITED`] | PRG010 | every map frame, from `MO_NormalMoveEnter` |
//! | [`WHISTLE_TRAVEL`] | PRG011 | once, at the whistle's world switch |
//! | [`GAMEOVER_RETURN`] | PRG010 | once, at the game-over finalize |
//!
//! **A world counts as visited once the player stands on its start tile.**
//!
//! Plus two pokes that are not routines at all, and are about the mode's pace
//! rather than about the whistle: the warp wind sweeps four times faster
//! ([`WIND_DELTA_OFFSET`]) and the motionless "WORLD n" card on the far side
//! is halved ([`INTRO_DWELL_OFFSET`]). Vanilla priced both for a once-a-game
//! event; the maze pays them on every hop.
//!
//! **The cycle runs in play order, not internal world order.** Which internal
//! world is "WORLD 3" is `world_order`'s decision, so a cycler that stepped
//! the internal index would visit the worlds in a sequence the player has no
//! way to predict. [`build_next_world`] turns the spine into a successor
//! table instead.
//!
//! **The dependency here inverted on 2026-09-07 and the old wording said the
//! opposite.** It used to read that invariant 3 — every world's start region
//! escapable — is what made a whistle destination safe to travel to. That rule
//! is retired (see `maze::fill::assign_keys`): the constructive fill gives every
//! gate a key already reachable from the global start, which is strictly
//! stronger, and the rule additionally rejected the mode's own formula by
//! refusing to count foreign fortresses.
//!
//! So it is now the other way round: **something has to make a trapped start
//! region survivable.** Game over, airship arrival and whistle travel all
//! deposit the player on a start tile that may have no walk-out.
//!
//! That used to be the whistle's job alone, and this module was a safety
//! property rather than a convenience on the strength of it. It is
//! [`GAMEOVER_RETURN`]'s job now: a game over sends the player to the world a
//! new game starts in, which is the one place the fill guarantees everything
//! is still reachable from. The whistle is back to being fast travel — welcome,
//! and no longer load-bearing.
//!
//! One consequence worth keeping in view: a world only ever pad-hopped into the
//! middle of is still correctly not whistle-able — the marker fires on the
//! start tile, not on arrival.
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
    FS_MAZE_GAMEOVER, FS_MAZE_TRAVEL, FS_MAZE_VISITED, Grid, MAP_Y_STARTS_OFF, PLAYER_CURRENT,
    WORLD_MAP_INIT_CPU, WORLD_MAP_X, WORLD_MAP_XHI, WORLD_MAP_Y, WORLD_NUM, find_start,
    prg010_file_to_cpu, prg011_file_to_cpu, prg030_file_to_cpu,
};

// --- Addresses ----------------------------------------------------------

const MARK_VISITED_CPU: u16 = prg010_file_to_cpu(FS_MAZE_VISITED);
const WHISTLE_TRAVEL_CPU: u16 = prg011_file_to_cpu(FS_MAZE_TRAVEL);
const GAMEOVER_RETURN_CPU: u16 = prg010_file_to_cpu(FS_MAZE_GAMEOVER);

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

/// Offset of [`NEXT_WORLD`] inside [`WHISTLE_TRAVEL`], and its CPU address.
///
/// Read absolutely, so the routine is **origin-locked** the same way the
/// marker is; `.origin(WHISTLE_TRAVEL_CPU)` in the checks is what catches a
/// relocation that forgets this.
const TRAVEL_TABLE_OFF: usize = 33;
const NEXT_WORLD_CPU: u16 = WHISTLE_TRAVEL_CPU + TRAVEL_TABLE_OFF as u16;

/// Go to the next visited world in **play order**, wrapping; stay put if there
/// is nowhere to go.
///
/// 128 reserved, 41 used (33 code + an 8-byte table).
///
/// # Why the successor comes from a table
///
/// The first cut walked `World_Num + 1 .. World_Num + 8` mod 8, i.e. the
/// *internal* world index. In a randomized ROM that is not an order the player
/// can see: `world_order` decides which internal world is displayed as "WORLD
/// 1", so an internal-index cycle visits the worlds in what looks like an
/// arbitrary sequence — and one that changes shape every seed. Playtesting
/// called it awkward, which it is: fast travel whose order you cannot predict
/// is fast travel you have to step through blind.
///
/// So the successor is a per-seed table, `internal -> next internal`, laid out
/// in the order the player numbers the worlds: the airship spine first, then
/// any off-spine world (`world_count < 7` leaves some, reachable only by
/// telepad) in internal order. Blowing the whistle repeatedly now walks WORLD
/// 1, 2, 3 ... and wraps, skipping the ones not yet visited.
///
/// It is also one byte *smaller* than the arithmetic it replaced: `LDA tbl,Y /
/// TAY` is 4 bytes where `INY / TYA / AND #$07 / TAY` was 5.
///
/// # The scan
///
/// [`NEXT_WORLD`] is a single eight-cycle by construction, so eight steps from
/// `World_Num` land back on `World_Num`: the last probe is the world the player
/// is standing in. That is what makes the "nowhere to go" case free — after
/// eight fruitless steps `Y` holds `World_Num` again and the exit path stores
/// it unchanged. One visited world and eight visited worlds take the same code.
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
const WHISTLE_TRAVEL: [u8; TRAVEL_TABLE_OFF + 8] = [
    0xAC, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,           //  0: LDY World_Num
    0xA2, VISITED_TABLE_LEN as u8,                           //  3: LDX #8      ; tries

    // ----- scan (5) -----
    0xB9, NEXT_WORLD_CPU as u8,
          (NEXT_WORLD_CPU >> 8) as u8,                       //  5: LDA NEXT_WORLD,Y
    0xA8,                                                    //  8: TAY
    0xB9, VISITED_TABLE as u8, (VISITED_TABLE >> 8) as u8,   //  9: LDA VISITED,Y
    0xD0, 0x03,                                              // 12: BNE +3 -> go
    0xCA,                                                    // 14: DEX
    0xD0, 0xF4,                                              // 15: BNE -12 -> scan

    // ----- go (17): Y is the destination, or World_Num if nothing was found
    // ----- (eight steps round the cycle land back on it) -----
    0x8C, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,           // 17: STY World_Num
    0xA9, 0x00,                                              // 20: LDA #$00
    0x8D, ARRIVAL_FLAG as u8, (ARRIVAL_FLAG >> 8) as u8,     // 22: STA ARRIVAL_FLAG
    0x85, MAP_WARPWIND_FX,                                   // 25: STA Map_WarpWind_FX
    0xA2, 0xFF,                                              // 27: LDX #$FF
    0x9A,                                                    // 29: TXS
    0x4C, WORLD_MAP_INIT_CPU as u8,
          (WORLD_MAP_INIT_CPU >> 8) as u8,                   // 30: JMP $84A0   ; never returns

    // ----- NEXT_WORLD (33): internal world -> the next one in play order,
    // ----- filled by `apply`. The placeholder is the internal-index cycle,
    // ----- which is a legal eight-cycle and exactly what this routine did
    // ----- before the table existed — so an unfilled table degrades to the
    // ----- old behaviour rather than walking off the end of VISITED.
    1, 2, 3, 4, 5, 6, 7, 0,
];

/// `internal world -> the next one in play order`, as a single eight-cycle.
///
/// `play_order` is `world_order::randomize`'s return: the airship spine, in the
/// order the player will number the worlds. Any world it leaves out — which is
/// what `world_count < 7` produces — is appended in internal order, because an
/// off-spine world has no display number to sort by but is still somewhere the
/// whistle has to be able to reach.
///
/// Duplicates and out-of-range entries are dropped rather than trusted: the
/// result has to be a permutation with exactly one cycle, or the scan's "eight
/// steps come home" property fails and it can settle on a world it already
/// rejected.
fn build_next_world(play_order: &[u8]) -> [u8; VISITED_TABLE_LEN] {
    let mut order: Vec<u8> = Vec::with_capacity(VISITED_TABLE_LEN);
    for world in play_order.iter().copied().chain(0..VISITED_TABLE_LEN as u8) {
        if (world as usize) < VISITED_TABLE_LEN && !order.contains(&world) {
            order.push(world);
        }
    }
    let mut next = [0u8; VISITED_TABLE_LEN];
    for (i, &world) in order.iter().enumerate() {
        next[world as usize] = order[(i + 1) % VISITED_TABLE_LEN];
    }
    next
}

// --- Part 3: game over returns to the starting world --------------------

/// `Map_GameOver_CursorY` (`$7DCB`) — the CONTINUE/END cursor, `$60` or `$68`.
///
/// **It is the only thing at the finalize that still remembers which the
/// player chose**, and it remembers it in a form that needs no mask: the
/// CONTINUE branch *zeroes* the byte on its way past (`$9300`), and the END
/// branch skips that store and leaves `$68` standing. So a plain `BNE` at the
/// finalize means "somebody gave up", which in two-player is a partner who is
/// still mid-run and must not be dragged anywhere.
const MAP_GAMEOVER_CURSOR_Y: u16 = 0x7DCB;

/// `JMP PRG030_879B` at the end of `PRG030_933E` (CPU `$9349` = file
/// 0x3D359) — three bytes, one whole instruction, and the single point every
/// concluded game over passes through on its way back to the map loop.
///
/// **The finalize, not the animation.** `GameOver_TwirlToStart` moves the
/// player home by a per-frame delta with a hardcoded column-2 skid test, so it
/// cannot be retargeted — `start_airship_swap` learned that the expensive way
/// and settled on stamping the answer at the finalize instead (see
/// [`super::start_airship_swap`]). This is the same lesson one step further
/// out: let the twirl play out in the world the player died in, then change
/// worlds once it is over.
///
/// **It has to be after `PRG030_9314`.** That loop is the game-over completion
/// penalty, and it ANDs the *live* array — so the wipe has to land while the
/// world being left is still the live one, before `$84A0` packs it away.
const GAMEOVER_HOOK_OFFSET: usize = 0x3D359;
#[cfg(test)]
const GAMEOVER_HOOK_LEN: usize = 3;

/// Vanilla bytes at [`GAMEOVER_HOOK_OFFSET`], and the address they name — the
/// player-switch at the top of the map loop, which the quit path still needs.
#[cfg(test)]
#[rustfmt::skip]
const GAMEOVER_HOOK_VANILLA: [u8; GAMEOVER_HOOK_LEN] = [
    0x4C, 0x9B, 0x87,       // JMP PRG030_879B
];
const GAMEOVER_RESUME_CPU: u16 = 0x879B;

/// Offset of the starting-world operand inside [`GAMEOVER_RETURN`].
const GAMEOVER_WORLD_OFF: usize = 6;

/// Continue after a game over, in the world a new game starts in.
///
/// 32 reserved, 16 used.
///
/// Vanilla returns the player to the start tile of the world they died in,
/// which is the right answer for a game that only ever moves forwards. The
/// maze is a graph: the world you died in can be one a telepad dropped you
/// into, with a start region whose only way out is a gate you have no key for.
/// Sending the player to the *global* start instead is the one destination
/// that is always live — `maze::fill::assign_keys` hands every gate a key
/// already reachable from there, and map reachability is monotone, so
/// everything that was ever reachable still is.
///
/// **That discharges the whistle's safety role.** The fill dropped the
/// start-region rule on the grounds that the whistle is never consumed and can
/// always take the player back to the spine's first world; the note there said
/// a mode shipped without a whistle would need game over to do this instead.
/// It does it now either way, and the whistle is back to being a convenience.
///
/// # Why a `JMP $84A0` is the whole mechanism
///
/// Changing `World_Num` alone would leave the previous world's map on screen
/// and its completions in `$7D00`: Continue does *not* re-init the map — it
/// rejoins the loop at `PRG030_879B` — so nothing would pack the outgoing
/// world or expand the incoming one. `$84A0` is the canonical world change and
/// carries all of it: `completion_bits`' `World_Num != LIVE_WORLD` hooks pack
/// and expand, `Map_Init` (and `start_airship_swap`'s helper inside it) stamps
/// the destination's start into all ten position variables and both camera
/// backups, `World_8_Dark` is recomputed from the new world, and the map is
/// redrawn from the restored bits.
///
/// Dying in the starting world takes the same path. The hooks compare equal
/// and do nothing, so the live array — penalty already applied — simply stays
/// live, and the player gets the "WORLD 1" card that every other game over
/// gets. Two bytes cheaper than a special case, and one path to playtest.
///
/// # No stack reset, unlike the other two
///
/// [`WHISTLE_TRAVEL`] and `world_persist`'s `PAD_ENTER` both `LDX #$FF / TXS`
/// before the jump because they fire frames deep inside `Map_DoOperation`.
/// This one does not need it: vanilla's own airship transition does
/// `INC World_Num / JMP $84A0` (`world_order::WORLD_INC_OFFSET`) from this
/// very frame — the level-exit chain the game-over path is a branch of — so
/// the stack is already at the depth `$84A0` expects.
///
/// `ARRIVAL_FLAG` needs no clearing for the same kind of reason: every pass
/// through `$84A0` ends in `RESTORE_ARRIVAL`, which consumes the flag, and a
/// game over is downstream of at least one.
#[rustfmt::skip]
const GAMEOVER_RETURN: [u8; 16] = [
    0xAD, MAP_GAMEOVER_CURSOR_Y as u8,
          (MAP_GAMEOVER_CURSOR_Y >> 8) as u8,               //  0: LDA Map_GameOver_CursorY
    0xD0, 0x08,                                             //  3: BNE +8 -> quit
    0xA9, 0x00,                                             //  5: LDA #starting world (`apply`)
    0x8D, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,          //  7: STA World_Num
    0x4C, WORLD_MAP_INIT_CPU as u8,
          (WORLD_MAP_INIT_CPU >> 8) as u8,                  // 10: JMP $84A0   ; never returns

    // ----- quit (13): the displaced instruction, for a two-player partner
    // ----- who is still playing -----
    0x4C, GAMEOVER_RESUME_CPU as u8,
          (GAMEOVER_RESUME_CPU >> 8) as u8,                 // 13: JMP PRG030_879B
];

// --- Part 4: the transition animations, faster --------------------------

/// `Map_WW_DeltaX` (PRG011, CPU `$A2F6` = file 0x16306) — how far the warp
/// wind moves each frame, one signed byte per direction of travel.
///
/// Vanilla sweeps the full 240 pixels at 2 px/frame: **120 frames, two whole
/// seconds**, and that is on top of the ~32-frame white flash before it. In
/// vanilla that is paid once a game, for a one-shot warp to the warp island.
/// The maze whistle is not that — it is fast travel, blown again and again,
/// and a world three steps down the cycle costs three of those animations.
///
/// At 8 px/frame the sweep is 30 frames, so a hop is about a second end to
/// end. The flash is left alone deliberately: `WarpWhistle_Flash` is shared
/// with the hand trap (`HT_Flash`), and shortening it there would be a visible
/// change to something nobody asked about.
///
/// **8 is chosen because it divides 16.** `WWFX_WarpDoWind` erases the
/// player's map sprite on an exact `CMP` of the wind's X against the player's
/// screen X, and map positions are always multiples of 16 — a delta that does
/// not divide 16 (6, say) would step straight past the player, who would then
/// stay drawn while the gust blew through them. 240 is a multiple of 8 too, so
/// the target-edge compare that ends the state still lands.
///
/// Only `WWFX_WarpDoWind` and `WWFX_WarpLanding` read this table, and both are
/// warp-whistle states; the landing state belongs to the warp island, which
/// the cycler makes unreachable. The hand trap shares the `Map_WWOrHT_*`
/// variables but not this table.
const WIND_DELTA_OFFSET: usize = 0x16306;

/// What vanilla has there: `+2` travelling right, `-2` travelling left.
#[cfg(test)]
const WIND_DELTA_VANILLA: [u8; 2] = [0x02, 0xFE];

/// Four times the speed, in the same encoding.
const WIND_DELTA_FAST: [u8; 2] = [0x08, 0xF8];

/// The `LDA #$80` operand inside `WorldIntro_BoxTimer_NoSym` (PRG010, CPU
/// `$C50F` = file 0x1451F) — how many frames the "WORLD n" card sits there
/// before the starry wipe drops the player onto the tile.
///
/// **This one is not the whistle's**, and it is sited here for the gate rather
/// than for the subject: it fires on every world entry — whistle, telepad and
/// beaten airship alike — and [`apply`] runs exactly when the maze is on, which
/// is the only condition it wants. A second module hook for a single byte would
/// buy nothing.
///
/// The entry sequence is three phases and only one of them is animation:
///
/// | phase | frames |
/// |---|---|
/// | the card, motionless (`Map_Intro_Tick`) | **128** |
/// | erase + the stars opening out (`Map_StarsOutRad += 4` to `$5F`) | ~24 |
/// | the stars closing onto the player | ~24 |
///
/// So nearly three quarters of the ~2.9 s is a still image. Vanilla enters a
/// world eight times a run; the maze does it constantly, and every telepad hop
/// pays this. Halving the card to 64 frames takes the sequence to about 1.6 s
/// and leaves both star sweeps — the part that is actually a transition —
/// untouched.
///
/// **Not below readable.** The card is where the player is told which world the
/// whistle actually landed them in, which in this mode is real information
/// rather than a formality. A second is comfortably above reading a digit you
/// are already looking for; `$30` or `$20` would still work and are the same
/// one-byte change, but they start to read as a flash rather than a card.
///
/// All three of the mode's entry paths reach this through `$84A0`, whose init
/// block zeroes `Map_Intro_Tick` and `World_EnterState` — so the card always
/// re-seeds itself here and this operand is the whole dial. The other two
/// `Map_Intro_Tick = $80` sites in the ROM are the warp island's landing
/// (unreachable once the cycler takes over `WWFX_WarpDoWind`) and a level-return
/// path; `Map_Intro_Tick` is a shared scratch counter, which is why this patches
/// the card's own self-init and not the variable's every writer.
const INTRO_DWELL_OFFSET: usize = 0x1451F;

/// What vanilla has there: 128 frames, about 2.1 seconds.
#[cfg(test)]
const INTRO_DWELL_VANILLA: u8 = 0x80;

/// Half of it.
const INTRO_DWELL_FAST: u8 = 0x40;

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
/// `play_order` is `world_order::randomize`'s return — the airship spine, in
/// the order the player numbers the worlds. It decides the cycle the whistle
/// walks; see [`build_next_world`].
///
/// **Depends on `completion_bits` being installed**, because the whole
/// transition rests on its `World_Num != LIVE_WORLD` hooks packing the
/// outgoing world. Not asserted here: this module writes ROM, it cannot see
/// what else the pipeline chose.
pub(crate) fn apply(rom: &mut Rom, grids: &[Grid], play_order: &[u8]) {
    let keys = build_start_keys(rom, grids);
    let next_world = build_next_world(play_order);

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

    let mut cycler = WHISTLE_TRAVEL;
    cycler[TRAVEL_TABLE_OFF..].copy_from_slice(&next_world);
    rom.write_range(FS_MAZE_TRAVEL, &cycler);
    // `JMP`, not `JSR`: the routine never comes back, so a return address
    // would be pure litter — and the stack is reset a few bytes later anyway.
    let mut hook = [0xEA_u8; TRAVEL_HOOK_LEN];
    hook[0] = 0x4C; // JMP
    hook[1] = WHISTLE_TRAVEL_CPU as u8;
    hook[2] = (WHISTLE_TRAVEL_CPU >> 8) as u8;
    rom.write_range(TRAVEL_HOOK_OFFSET, &hook);

    // Two seconds of wind per hop is vanilla's price for a once-a-game warp.
    // See `WIND_DELTA_OFFSET`.
    rom.write_range(WIND_DELTA_OFFSET, &WIND_DELTA_FAST);
    // ...and two more of the motionless "WORLD n" card on the far side, on
    // every entry rather than only the whistle's. See `INTRO_DWELL_OFFSET`.
    rom.write_byte(INTRO_DWELL_OFFSET, INTRO_DWELL_FAST);

    // Game over goes home rather than back to where it happened. The starting
    // world is `play_order`'s first entry, which is the byte `world_order`
    // baked into the title screen's own `LDA #$00` — so this and a new game
    // agree on where "WORLD 1" is by construction.
    let mut gameover = GAMEOVER_RETURN;
    gameover[GAMEOVER_WORLD_OFF] = play_order.first().copied().unwrap_or(0);
    rom.write_range(FS_MAZE_GAMEOVER, &gameover);
    rom.write_range(
        GAMEOVER_HOOK_OFFSET,
        &[0x4C, GAMEOVER_RETURN_CPU as u8, (GAMEOVER_RETURN_CPU >> 8) as u8],
    );

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
            .data_from(TRAVEL_TABLE_OFF)
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
            .data_from(TRAVEL_TABLE_OFF)
            .hook(&TRAVEL_HOOK_VANILLA, 0, &jmp)
            .assert_ok();
    }

    /// Both hooks name their routine, and `apply` writes both.
    #[test]
    fn apply_installs_both_hooks() {
        let Some(rom) = load_vanilla() else { return };
        let mut patched = rom.clone();
        let grids = crate::randomize::rom_data::read_all_tile_grids(&patched);
        apply(&mut patched, &grids, &IDENTITY_ORDER);

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
        assert_eq!(
            patched.read_range(WIND_DELTA_OFFSET, 2),
            WIND_DELTA_FAST,
            "the warp wind is still travelling at vanilla speed"
        );
    }

    #[test]
    fn gameover_return_is_well_formed() {
        asm::check(&GAMEOVER_RETURN)
            .allocation(FS_MAZE_GAMEOVER)
            .origin(GAMEOVER_RETURN_CPU)
            // It touches no zero page at all: the cursor, `World_Num` and both
            // jump targets are absolute.
            .zero_page(NMI_SAFE_MAX, &[])
            .assert_ok();
    }

    /// The game-over hook displaces one whole instruction, and the routine
    /// replays it on the path that must stay vanilla.
    #[test]
    fn gameover_hook_displaces_one_whole_instruction() {
        let Some(rom) = load_vanilla() else { return };
        assert_eq!(
            rom.read_range(GAMEOVER_HOOK_OFFSET, GAMEOVER_HOOK_LEN),
            GAMEOVER_HOOK_VANILLA,
            "PRG030_933E's return to the map loop has moved, or something else hooked it"
        );
        assert_eq!(
            GAMEOVER_RETURN[13..16],
            GAMEOVER_HOOK_VANILLA,
            "the quit path must be exactly what the hook overwrote"
        );

        let jmp = [0x4C, GAMEOVER_RETURN_CPU as u8, (GAMEOVER_RETURN_CPU >> 8) as u8];
        asm::check(&GAMEOVER_RETURN)
            .allocation(FS_MAZE_GAMEOVER)
            .origin(GAMEOVER_RETURN_CPU)
            .zero_page(NMI_SAFE_MAX, &[])
            .hook(&GAMEOVER_HOOK_VANILLA, 0, &jmp)
            .assert_ok();
    }

    /// The cursor the return reads really is the CONTINUE/END cursor, and the
    /// CONTINUE branch really does zero it — which is what lets a bare `BNE`
    /// stand in for `AND #$08`.
    ///
    /// Both facts live in `PRG030_92B6`, six bytes apart: `LDA $7DCB / AND
    /// #$08 / BNE +$65` decides, and the `STA $7DCB` inside the CONTINUE
    /// branch is the store the END branch jumps over.
    #[test]
    fn the_continue_branch_zeroes_the_cursor() {
        let Some(rom) = load_vanilla() else { return };
        // CPU $92BE: LDA Map_GameOver_CursorY / AND #$08 / BNE PRG030_932A
        assert_eq!(
            rom.read_range(0x3D2CE, 7),
            [
                0xAD,
                MAP_GAMEOVER_CURSOR_Y as u8,
                (MAP_GAMEOVER_CURSOR_Y >> 8) as u8,
                0x29,
                0x08,
                0xD0,
                0x65
            ],
            "the game-over finalize no longer branches on Map_GameOver_CursorY"
        );
        // CPU $92E8, inside the CONTINUE branch and short of that BNE's
        // target: LDA #$00, then three stores, the third of them the cursor.
        assert_eq!(
            rom.read_range(0x3D2F8, 11),
            [
                0xA9,
                0x00, // LDA #$00
                0x9D,
                0x3E,
                0x07, // STA Map_Player_SkidBack,X
                0x8D,
                0x28,
                0x07, // STA World_EnterState
                0x8D,
                MAP_GAMEOVER_CURSOR_Y as u8,
                (MAP_GAMEOVER_CURSOR_Y >> 8) as u8, // STA Map_GameOver_CursorY
            ],
            "CONTINUE no longer clears the cursor, so a bare BNE cannot read the choice"
        );
    }

    /// The destination is the world a new game starts in — the operand
    /// `world_order` bakes into the title screen's own `LDA #$00`. If those two
    /// ever disagree, "back to World 1" means a world the player has never
    /// numbered 1.
    #[test]
    fn the_destination_is_where_a_new_game_starts() {
        for seed in [1_u64, 7, 99] {
            let Some(mut rom) = load_vanilla() else { return };
            let opts = crate::randomizer::Options {
                world_maze: true,
                ..crate::randomizer::Options::default()
            };
            crate::randomizer::randomize(&mut rom, seed, &opts);
            assert_eq!(
                rom.read_byte(FS_MAZE_GAMEOVER + GAMEOVER_WORLD_OFF),
                rom.read_byte(crate::randomize::world_order::WORLD_INIT_OPERAND),
                "seed {seed}: game over returns to a different world than a new game starts in"
            );
            assert_eq!(
                rom.read_range(GAMEOVER_HOOK_OFFSET, GAMEOVER_HOOK_LEN),
                [0x4C, GAMEOVER_RETURN_CPU as u8, (GAMEOVER_RETURN_CPU >> 8) as u8],
                "seed {seed}: the game-over hook is not installed"
            );
        }
    }

    /// **Run it.** [`GAMEOVER_RETURN`] is one of the few routines here that
    /// can be executed whole: it calls nothing and reads three absolute bytes.
    /// `asm::check` proves the branch lands on an instruction boundary — it
    /// cannot tell a `BNE` from a `BEQ`, and swapping those two is the whole
    /// bug (every game over would strand the player, or none would move).
    #[test]
    fn the_return_moves_the_player_only_on_continue() {
        // $60 is CONTINUE and $68 is END, but the finalize zeroes the byte on
        // the CONTINUE branch, so the routine sees 0 or $68. Both raw cursor
        // values are here too: either one reaching this code means somebody
        // reached the finalize without going through the branch, and the safe
        // reading of that is "do not move the player".
        for (cursor, moves) in [(0x00_u8, true), (0x68, false), (0x60, false)] {
            for world in [0_u8, 3, 7] {
                let mut code = GAMEOVER_RETURN;
                code[GAMEOVER_WORLD_OFF] = world;
                let mut mem = Memory::new();
                mem.set_bytes(GAMEOVER_RETURN_CPU, &code);
                mem.set_byte(MAP_GAMEOVER_CURSOR_Y, cursor);
                // The world the player died in — poison, so "unchanged" shows.
                mem.set_byte(WORLD_NUM, 0xAA);
                let mut cpu = CPU::new(mem, Ricoh2a03);
                cpu.registers.stack_pointer = StackPointer(0xFD);
                cpu.registers.program_counter = GAMEOVER_RETURN_CPU;

                let mut landed = None;
                for _ in 0..100 {
                    match cpu.registers.program_counter {
                        WORLD_MAP_INIT_CPU | GAMEOVER_RESUME_CPU => {
                            landed = Some(cpu.registers.program_counter);
                            break;
                        }
                        _ => {
                            cpu.single_step();
                        }
                    }
                }
                let landed = landed.expect("GAMEOVER_RETURN ran away");

                if moves {
                    assert_eq!(landed, WORLD_MAP_INIT_CPU, "cursor {cursor:#04X} did not re-init");
                    assert_eq!(
                        cpu.memory.get_byte(WORLD_NUM),
                        world,
                        "cursor {cursor:#04X}: the destination world was not stored"
                    );
                } else {
                    assert_eq!(
                        landed, GAMEOVER_RESUME_CPU,
                        "cursor {cursor:#04X} moved a player who did not continue"
                    );
                    assert_eq!(
                        cpu.memory.get_byte(WORLD_NUM),
                        0xAA,
                        "cursor {cursor:#04X}: World_Num must be left alone"
                    );
                }
            }
        }
    }

    /// The internal-index play order, which reproduces the placeholder table
    /// baked into [`WHISTLE_TRAVEL`] — so the fixture arm above can compare
    /// the written bytes against the constant.
    const IDENTITY_ORDER: [u8; 8] = [0, 1, 2, 3, 4, 5, 6, 7];

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

    /// The wind-speed poke lands on `Map_WW_DeltaX` and nothing else — the
    /// bytes either side are `Map_WW_StartX` and `Map_WW_TargetX`, which the
    /// same states compare against and which must stay at the screen edges.
    ///
    /// The divisibility assert is the one that matters: `WWFX_WarpDoWind`
    /// erases the player's map sprite on an exact `CMP`, and map positions are
    /// multiples of 16.
    #[test]
    fn the_wind_delta_site_is_the_delta_table() {
        let Some(rom) = load_vanilla() else { return };
        assert_eq!(
            rom.read_range(WIND_DELTA_OFFSET, 2),
            WIND_DELTA_VANILLA,
            "Map_WW_DeltaX is not where this module thinks it is"
        );
        // Map_WW_StartX before it, Map_WW_TargetX after: 0/240 and 240/0.
        assert_eq!(rom.read_range(WIND_DELTA_OFFSET - 2, 2), [0x00, 0xF0]);
        assert_eq!(rom.read_range(WIND_DELTA_OFFSET + 2, 2), [0xF0, 0x00]);

        // The two directions are equal and opposite, and both step the wind
        // onto every multiple of 16 between the edges.
        assert_eq!(
            WIND_DELTA_FAST[0].wrapping_add(WIND_DELTA_FAST[1]),
            0,
            "the two directions must be equal and opposite"
        );
        assert_eq!(
            16 % WIND_DELTA_FAST[0],
            0,
            "a delta that does not divide 16 steps past the player without erasing them"
        );
        assert_eq!(240 % WIND_DELTA_FAST[0], 0, "the wind must land exactly on the target edge");
        assert!(
            WIND_DELTA_FAST[0] > WIND_DELTA_VANILLA[0],
            "this patch exists to make the sweep faster"
        );
    }

    /// The intro-card poke lands on the `LDA #$80` inside
    /// `WorldIntro_BoxTimer_NoSym`, and the instructions around it are the
    /// ones that make it the card's whole dial: the `BNE` that skips the
    /// re-seed when the tick is already running, and the `DEC` that spends it.
    #[test]
    fn the_intro_dwell_site_is_the_card_timer() {
        let Some(rom) = load_vanilla() else { return };
        // LDA #$80 / STA Map_Intro_Tick / DEC Map_Intro_Tick
        assert_eq!(
            rom.read_range(INTRO_DWELL_OFFSET - 1, 9),
            [0xA9, INTRO_DWELL_VANILLA, 0x8D, 0x11, 0x07, 0xCE, 0x11, 0x07, 0xD0],
            "WorldIntro_BoxTimer_NoSym is not where this module thinks it is"
        );
        // ...reached by `LDA Map_Intro_Tick / BNE` straight over the re-seed,
        // which is what makes a caller that pre-seeds the tick keep its own
        // value and this operand the card's own.
        assert_eq!(
            rom.read_range(INTRO_DWELL_OFFSET - 6, 5),
            [0xAD, 0x11, 0x07, 0xD0, 0x05],
            "the re-seed is not guarded by the tick test"
        );

        const { assert!(INTRO_DWELL_FAST > 0, "a zero dwell underflows: DEC wraps to 255 frames") };
        const { assert!(INTRO_DWELL_FAST < INTRO_DWELL_VANILLA, "this patch shortens the card") };

        let mut patched = rom.clone();
        let grids = crate::randomize::rom_data::read_all_tile_grids(&patched);
        apply(&mut patched, &grids, &IDENTITY_ORDER);
        assert_eq!(patched.read_byte(INTRO_DWELL_OFFSET), INTRO_DWELL_FAST);
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

    /// The cycler's two table reads name the right addresses: the visited
    /// table `maze_state` allocated, and its own successor table.
    #[test]
    fn the_cycler_scans_the_allocated_table() {
        assert_eq!(WHISTLE_TRAVEL[9], 0xB9, "offset 9 is not an LDA abs,Y");
        assert_eq!(u16::from_le_bytes([WHISTLE_TRAVEL[10], WHISTLE_TRAVEL[11]]), VISITED_TABLE);
        assert_eq!(WHISTLE_TRAVEL[4], VISITED_TABLE_LEN as u8, "the try count is not the length");
        // The marker writes into the same table.
        assert_eq!(u16::from_le_bytes([MARK_VISITED[29], MARK_VISITED[30]]), VISITED_TABLE);

        assert_eq!(WHISTLE_TRAVEL[5], 0xB9, "offset 5 is not an LDA abs,Y");
        assert_eq!(
            u16::from_le_bytes([WHISTLE_TRAVEL[6], WHISTLE_TRAVEL[7]]),
            NEXT_WORLD_CPU,
            "the successor read must name this routine's own table"
        );
        assert_eq!(
            NEXT_WORLD_CPU,
            WHISTLE_TRAVEL_CPU + TRAVEL_TABLE_OFF as u16,
            "the successor table must sit at the routine's declared table offset"
        );
    }

    /// **The successor table must be one eight-cycle**, whatever it is handed.
    /// The scan's "eight steps come home" exit — and with it the whole
    /// nowhere-to-go case — is false for a permutation with two cycles, and
    /// false in a way that shows up as the whistle settling on a world it has
    /// already passed rather than as a crash.
    #[test]
    fn the_successor_table_is_always_one_cycle() {
        let orders: &[&[u8]] = &[
            &[],
            &[0, 1, 2, 3, 4, 5, 6, 7],
            &[3, 0, 6, 1, 4, 2, 5, 7],
            &[5, 2, 7],             // a short spine: world_count < 7
            &[2, 2, 9, 0xFF, 4, 4], // duplicates and out-of-range entries
        ];
        for order in orders {
            let next = build_next_world(order);
            let mut seen = [false; 8];
            let mut w = 0u8;
            for _ in 0..8 {
                assert!(!seen[w as usize], "{order:?} produced a short cycle at W{}", w + 1);
                seen[w as usize] = true;
                w = next[w as usize];
            }
            assert_eq!(w, 0, "{order:?}: eight steps did not come home");
            assert!(seen.iter().all(|&v| v), "{order:?} left a world out of the cycle");
        }
    }

    /// The spine leads, and the worlds it leaves out follow in internal order.
    /// This is the whole point of the table: the player blows the whistle and
    /// walks WORLD 1, 2, 3 ... rather than the internal indices behind them.
    #[test]
    fn the_cycle_follows_play_order() {
        // A four-world spine over internal worlds 5, 2, 0, 7.
        let next = build_next_world(&[5, 2, 0, 7]);
        assert_eq!(next[5], 2);
        assert_eq!(next[2], 0);
        assert_eq!(next[0], 7);
        // ...then the off-spine worlds, ascending, then home.
        assert_eq!(next[7], 1);
        assert_eq!(next[1], 3);
        assert_eq!(next[3], 4);
        assert_eq!(next[4], 6);
        assert_eq!(next[6], 5);

        // And the routine really walks it: with every world visited, the
        // whistle steps the spine in order.
        let mut code = WHISTLE_TRAVEL;
        code[TRAVEL_TABLE_OFF..].copy_from_slice(&next);
        let all = table(&[0, 1, 2, 3, 4, 5, 6, 7]);
        for (from, to) in [(5u8, 2u8), (2, 0), (0, 7), (7, 1)] {
            assert_eq!(travel(&code, from, &all), to, "W{} -> W{}", from + 1, to + 1);
        }

        // With only the spine visited, the off-spine worlds are skipped over
        // rather than stepped through.
        let spine_only = table(&[5, 2, 0, 7]);
        assert_eq!(travel(&code, 7, &spine_only), 5, "the spine wraps past the off-spine worlds");
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
            apply(&mut patched, &grids, &IDENTITY_ORDER);
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
            apply(&mut b, &grids, &IDENTITY_ORDER);
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

        let ok = |code: &[u8; TRAVEL_TABLE_OFF + 8]| {
            asm::check(code).origin(WHISTLE_TRAVEL_CPU).data_from(TRAVEL_TABLE_OFF).assert_ok();
        };

        // 1. The loop-back branch is three bytes short, so the scan restarts
        //    at `TAY` and never advances Y past the first probe.
        let mut wrong_branch = WHISTLE_TRAVEL;
        wrong_branch[16] = 0xF7; // BNE -12 -> -9, landing on `TAY` instead of the successor read
        ok(&wrong_branch);
        assert_ne!(
            travel(&wrong_branch, 0, &t),
            2,
            "a wrong loop-back displacement must break the cycle test"
        );

        // 2. The scan reads one byte past the visited table (the wand table's
        //    first byte, in the real map) — an off-by-one in the address.
        let mut wrong_index = WHISTLE_TRAVEL;
        wrong_index[10] = wrong_index[10].wrapping_add(1);
        ok(&wrong_index);
        assert_ne!(
            travel(&wrong_index, 0, &t),
            2,
            "an off-by-one visited-table address must break the cycle test"
        );

        // 3. The successor table is read one byte high, so every world takes
        //    the *next* world's successor and the cycle collapses.
        let mut wrong_order = WHISTLE_TRAVEL;
        wrong_order[6] = wrong_order[6].wrapping_add(1);
        ok(&wrong_order);
        assert_ne!(
            travel(&wrong_order, 2, &t),
            5,
            "an off-by-one successor-table address must break the cycle test"
        );

        // 4. The destination is stored from X (which is the try counter) rather
        //    than Y, so the whistle lands somewhere unrelated to the scan.
        let mut wrong_store = WHISTLE_TRAVEL;
        wrong_store[17] = 0x8E; // STY abs -> STX abs
        ok(&wrong_store);
        assert_ne!(travel(&wrong_store, 0, &t), 2, "storing the wrong register must be visible");
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
        apply(&mut patched, &grids, &[0, 1, 2, 3, 4, 5, 6, 7]);
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
