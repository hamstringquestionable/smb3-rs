//! Two players, two worlds: in the maze each player keeps the world they are
//! standing in, and a turn hand-over carries the map with it.
//!
//! # Why the maze needs this and vanilla does not
//!
//! Vanilla never returns you to a world and never lets the two players be
//! anywhere but the same one: `World_Num` is a single global byte, and worlds
//! only ever advance. The maze breaks both halves of that. A telepad moves the
//! player who stepped on it, so the moment one player hops, the other one's
//! world is gone — hand the turn over and they were standing in a world that no
//! longer exists, at coordinates belonging to a map they never walked.
//!
//! Everything *positional* is already per-player, which is what makes this
//! small: `World_Map_Y/XHi/X`, `Map_Entered_*` and `Map_Prev_XOff/XHi` are all
//! two-byte arrays indexed by `Player_Current`, and the map init restores both
//! players' from the backups. The only thing that is not per-player is the world
//! number itself, so that is the only thing this module adds —
//! [`PLAYER_WORLD`], two SRAM bytes.
//!
//! **It stays one shared maze, not two games.** Completions are packed per
//! *world* (both halves of `Map_Completions`, see
//! [`super::completion_bits`]), so a fortress one player clears is cleared for
//! the other when they arrive. The wand table, the visited table and the map
//! objects are all per-world too. What is per-player is only *where you are*.
//!
//! # The turn hand-over is the whole feature
//!
//! `PRG030_8775` ends a turn: it backs up the outgoing player's scroll and map
//! coordinates, picks the next living player at `PRG030_879B`, and jumps to
//! `PRG030_84D7` — which is the byte immediately after the `Map_Completions`
//! wipe inside `$84A0`. So vanilla *already* re-runs the back half of the map
//! init on every hand-over, and that half redraws the map from `World_Num`
//! and restores both players' positions. Point it at a different world and it
//! draws a different world.
//!
//! What it does *not* re-run is the front half: `Map_Init`, which rebuilds the
//! world's nine map-object slots from ROM, and the per-world flags `$84A0`
//! clears (`Map_Anchored`, `Map_Got13Warp`, `BigQBlock_GotIt`,
//! `Map_Airship_Dest`). Skipping those is right when the world has not changed
//! and wrong when it has — a hand-over into another world would keep the
//! previous world's Hammer Bros standing on the new map.
//!
//! So [`TURN_SWAP`] does not do the swap itself. It decides *which entry* the
//! hand-over takes: `$84D7` when the incoming player is in the world already on
//! screen, exactly as vanilla, and `$84A0` — the full init — when they are not.
//! Everything else then falls out of machinery that already exists and is
//! already tested: `$84A0` runs `Map_Init` for the new world, and the two
//! `completion_bits` hooks inside it pack the world being left and expand the
//! one being entered, because their trigger is `World_Num != LIVE_WORLD` and
//! nothing else.
//!
//! # Why it lives in PRG030
//!
//! PRG030 is always mapped, and 17 of its last 42 bytes is real rent — but the
//! hand-over cannot be reasoned about any other way. The map loop banks PRG026
//! into `$A000` at `PRG030_874F` on its way here, and `$84D7` opens by calling
//! `SetPages_ByTileset` *because* it is entered with arbitrary banks (the
//! death path at `PRG030_9130` jumps straight there out of a level). A
//! `JMP` into the map bank from this site would be a bet on the window; a
//! routine that only touches RAM and jumps back into PRG030 is not.
//!
//! It takes the *bottom* of that gap on purpose. `FS_STOMP_RISE`'s note flags
//! two `JSR $9FF4` byte sequences in PRG010/PRG012 data whose status is
//! unsettled; `$9FF4` is in the top of the gap and this reservation stops well
//! below it, so that question is left exactly as it was.
//!
//! # The other player's marker
//!
//! `Map_No_Pan` (PRG010) draws the inactive player's map marker whenever the
//! game is two-player, the world is not the Warp Zone, that player is alive and
//! their X is on screen. In separate worlds that is a marker standing wherever
//! their own map put them — a Luigi in the middle of World 5's ocean.
//!
//! [`MARKER_GATE`] hooks the `LDA Player_Lives,X` that feeds the alive test and
//! answers `$80` — reads as "deceased", so vanilla's own `BMI` skips the draw —
//! when that player is in a different world. Three bytes over three, and it
//! reuses the branch vanilla already had rather than adding one.
//!
//! # What keeps [`PLAYER_WORLD`] true
//!
//! Two writers, and between them every way a world can change:
//!
//! * `completion_bits::WIPE_REPLACEMENT`, on the arm it takes when the world
//!   really is changing. That is the single choke point for *every* world
//!   change the live player can make — a telepad, the whistle, an airship
//!   clear, the warp zone — because all of them reach `$84A0` with `World_Num`
//!   already set and `LIVE_WORLD` still naming the world being left. Stamping
//!   there rather than at each of those sites is what keeps this module from
//!   having a second definition of "a transition happened".
//! * `completion_bits::NEW_GAME_INIT`, which seeds both players with the
//!   starting world. It has to: `world_order` can start the game in any world,
//!   so zeroed bytes would tell the second player they began in World 1.
//!
//! Both of those live in `completion_bits` because they are edits to its
//! arrays, not calls into this module.
//!
//! # One-player mode is untouched
//!
//! `Player_Current` never leaves 0 in one-player mode — the hand-over loop only
//! advances while `Total_Players` is 2 — so `PLAYER_WORLD[0]` is the only entry
//! ever stamped, and it is stamped from `World_Num` itself. [`TURN_SWAP`]'s
//! compare is therefore always equal, its store always writes the byte that is
//! already there, and the hand-over always takes `$84D7`. [`MARKER_GATE`] sits
//! behind vanilla's own `Total_Players` test and is never reached at all.

use crate::rom::Rom;

use super::maze_state::PLAYER_WORLD;
use super::rom_data::{
    FS_MARKER_GATE, FS_TURN_SWAP, WORLD_NUM, prg010_file_to_cpu, prg030_file_to_cpu,
};

// --- Addresses ----------------------------------------------------------

/// Where [`TURN_SWAP`] runs. PRG030, always mapped. `$9FD6`.
const TURN_SWAP_CPU: u16 = prg030_file_to_cpu(FS_TURN_SWAP);

/// Where [`MARKER_GATE`] runs. PRG010 — it is only reached from PRG010's own
/// `Map_No_Pan`, so the bank is its caller's. `$D69C`.
const MARKER_GATE_CPU: u16 = prg010_file_to_cpu(FS_MARKER_GATE);

/// `PRG030_84A0`, "initialize the world map" — the full init, `Map_Init` and
/// the per-world flag clears included.
const MAP_INIT_CPU: u16 = 0x84A0;

/// `PRG030_84D7`, the second entry into that routine: everything after the
/// `Map_Completions` wipe. Vanilla's own hand-over target.
const TURN_REINIT_CPU: u16 = 0x84D7;

/// `Player_Lives` (`$0736`, Mario/Luigi) — what [`MARKER_GATE`] returns when it
/// lets the draw through.
const PLAYER_LIVES: u16 = 0x0736;

/// `JMP PRG030_84D7` at the end of `PRG030_87A9`, CPU `$87BA`: the three bytes
/// that decide where a hand-over lands, and the only thing this module hooks in
/// PRG030.
///
/// The instruction before it is `STA Map_PlayerLost2PVs` and the byte after it
/// is `PRG030_87BD`, a branch target — so three bytes is exactly what is
/// available, which is why [`TURN_SWAP`] is a routine and not a splice.
const HANDOVER_JMP_OFFSET: usize = 0x3C7CA;

/// `LDA Player_Lives,X` in `Map_No_Pan` (PRG010, CPU `$D140`), three bytes,
/// reached only by falling through the `TAX` before it.
const MARKER_LIVES_OFFSET: usize = 0x15150;

// --- The hand-over router -----------------------------------------------

/// Pick the map init entry the hand-over should take, and adopt the incoming
/// player's world on the way.
///
/// ```text
/// $9FD6  BD DA 7A   LDA PLAYER_WORLD,X   ; X = Player_Current, still live
/// $9FD9  CD 27 07   CMP World_Num
/// $9FDC  8D 27 07   STA World_Num        ; unconditional; a no-op when equal
/// $9FDF  D0 03      BNE change
/// $9FE1  4C D7 84   JMP PRG030_84D7      ; same world: vanilla's hand-over
/// $9FE4  4C A0 84   change: JMP PRG030_84A0
/// ```
///
/// **`X` is vanilla's, not ours.** `PRG030_87A9` opens `LDA Player_Current /
/// TAX` and nothing between there and the `JMP` this replaces touches `X`, so
/// the router arrives with the incoming player's index already in it and saves
/// the three bytes an `LDX Player_Current` would cost. That re-derivation is
/// unconditional — the loop above it can leave `X` at `Total_Players` when it
/// wraps to Mario, and `$87A9` reloads from `Player_Current` regardless — so
/// this is a guarantee and not an observation about one path.
///
/// **The store is unconditional because the compare has already happened.**
/// `CMP` sets the flags; `STA` does not touch them. Storing before the branch
/// costs nothing and saves the three bytes a second `STA` on the taken arm
/// would, and writing the byte that is already there is what one-player mode
/// does every single turn.
///
/// `$84A0` is a legitimate mid-map-loop entry — vanilla jumps there itself from
/// the airship transition and the warp zone — so the changed arm is not doing
/// anything to the engine that the engine does not do to itself.
#[rustfmt::skip]
const TURN_SWAP: [u8; 17] = [
    0xBD, PLAYER_WORLD as u8, (PLAYER_WORLD >> 8) as u8, //  0: LDA PLAYER_WORLD,X
    0xCD, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,      //  3: CMP World_Num
    0x8D, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,      //  6: STA World_Num
    0xD0, 0x03,                                         //  9: BNE +3 -> change
    0x4C, TURN_REINIT_CPU as u8,
          (TURN_REINIT_CPU >> 8) as u8,                 // 11: JMP PRG030_84D7
    0x4C, MAP_INIT_CPU as u8,
          (MAP_INIT_CPU >> 8) as u8,                    // 14: JMP PRG030_84A0  ; change
];

// --- The other player's marker ------------------------------------------

/// Answer `Map_No_Pan`'s "is the other player alive" question with "no" when
/// they are in a different world, so their marker is not drawn onto a map they
/// are not standing on.
///
/// ```text
/// $D69C  BD DA 7A   LDA PLAYER_WORLD,X   ; X = the other player
/// $D69F  CD 27 07   CMP World_Num
/// $D6A2  F0 03      BEQ here
/// $D6A4  A9 80      LDA #$80             ; negative: caller's BMI skips the draw
/// $D6A6  60         RTS
/// $D6A7  BD 36 07   here: LDA Player_Lives,X
/// $D6AA  60         RTS
/// ```
///
/// The hook is the `LDA Player_Lives,X` itself, which the gate replays on the
/// same-world path — so the *only* new behaviour is one extra way to reach a
/// `BMI` the caller already had. Vanilla's own reasons to skip the marker (one
/// player, the Warp Zone, a dead partner, off-screen) all still apply first or
/// unchanged.
///
/// `$80` rather than `$FF` for no reason beyond it being the cheapest negative;
/// the caller only ever tests the sign.
#[rustfmt::skip]
const MARKER_GATE: [u8; 15] = [
    0xBD, PLAYER_WORLD as u8, (PLAYER_WORLD >> 8) as u8, //  0: LDA PLAYER_WORLD,X
    0xCD, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,      //  3: CMP World_Num
    0xF0, 0x03,                                         //  6: BEQ +3 -> here
    0xA9, 0x80,                                         //  8: LDA #$80
    0x60,                                               // 10: RTS
    0xBD, PLAYER_LIVES as u8, (PLAYER_LIVES >> 8) as u8, // 11: LDA Player_Lives,X ; here
    0x60,                                               // 14: RTS
];

// --- Application --------------------------------------------------------

/// Install both routines and their hooks.
///
/// Maze-only, and unconditional within the maze: this is not a player-facing
/// choice but the behaviour two-player maze mode has to have, and it costs
/// one-player mode nothing (see the module docs).
///
/// Ordering: after `completion_bits::apply`, which owns the two `$84A0` hooks
/// that keep [`PLAYER_WORLD`] true. Nothing here touches those sites, so the
/// rule is about reading the source in one order rather than about bytes.
pub(crate) fn apply(rom: &mut Rom) {
    rom.write_range(FS_TURN_SWAP, &TURN_SWAP);
    rom.write_range(HANDOVER_JMP_OFFSET, &jmp(0x4C, TURN_SWAP_CPU));

    rom.write_range(FS_MARKER_GATE, &MARKER_GATE);
    rom.write_range(MARKER_LIVES_OFFSET, &jmp(0x20, MARKER_GATE_CPU));
}

/// A three-byte `JSR`/`JMP` to `target`.
const fn jmp(opcode: u8, target: u16) -> [u8; 3] {
    [opcode, target as u8, (target >> 8) as u8]
}

#[cfg(test)]
mod asm_checks {
    use super::*;
    use crate::randomize::rom_data::asm;

    /// Vanilla at [`HANDOVER_JMP_OFFSET`]: `JMP PRG030_84D7`.
    const HANDOVER_VANILLA: [u8; 3] = [0x4C, 0xD7, 0x84];
    /// Vanilla at [`MARKER_LIVES_OFFSET`]: `LDA Player_Lives,X`.
    const MARKER_VANILLA: [u8; 3] = [0xBD, 0x36, 0x07];

    #[test]
    fn the_handover_router_is_well_formed() {
        asm::check(&TURN_SWAP)
            .allocation(FS_TURN_SWAP)
            .origin(TURN_SWAP_CPU)
            .hook(&HANDOVER_VANILLA, 0, &jmp(0x4C, TURN_SWAP_CPU))
            .assert_ok();
    }

    #[test]
    fn the_marker_gate_is_well_formed() {
        asm::check(&MARKER_GATE)
            .allocation(FS_MARKER_GATE)
            .origin(MARKER_GATE_CPU)
            .hook(&MARKER_VANILLA, 0, &jmp(0x20, MARKER_GATE_CPU))
            .assert_ok();
    }

    /// The two hook sites hold what this module thinks they hold. A vanilla
    /// byte that moved would otherwise be spliced over silently — and both of
    /// these are three bytes wide with a branch target immediately after, so
    /// there is no slack to absorb a mistake.
    #[test]
    fn the_hook_sites_are_where_vanilla_put_them() {
        let Some(rom) = vanilla() else { return };
        assert_eq!(
            rom.data[HANDOVER_JMP_OFFSET..HANDOVER_JMP_OFFSET + 3],
            HANDOVER_VANILLA,
            "PRG030_87A9 no longer ends in JMP PRG030_84D7",
        );
        assert_eq!(
            rom.data[MARKER_LIVES_OFFSET..MARKER_LIVES_OFFSET + 3],
            MARKER_VANILLA,
            "Map_No_Pan's other-player alive test is not LDA Player_Lives,X",
        );
    }

    /// The gate hands back exactly what it displaced on the same-world path.
    ///
    /// Cheap to assert and the one way the hook could be wrong without being
    /// malformed: a gate that returned a *different* player's lives, or a
    /// constant, would hide or draw the marker on the path that is supposed to
    /// behave like vanilla.
    #[test]
    fn the_gate_replays_the_instruction_it_displaced() {
        assert_eq!(MARKER_GATE[11..14], MARKER_VANILLA, "the same-world path must replay the LDA");
    }

    /// Both routines index [`PLAYER_WORLD`] by a register vanilla loaded, so a
    /// table that grew past two entries would read a neighbouring allocation
    /// with no symptom until a second player existed.
    #[test]
    fn the_table_has_exactly_one_byte_per_player() {
        assert_eq!(
            crate::randomize::maze_state::PLAYER_WORLD_LEN,
            2,
            "PLAYER_WORLD is indexed by Player_Current, which is 0 or 1",
        );
    }

    fn vanilla() -> Option<Rom> {
        let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&bytes).ok()
    }
}
