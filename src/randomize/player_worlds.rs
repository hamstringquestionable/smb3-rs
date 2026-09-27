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
//! # Only the live player is ever repositioned
//!
//! That is the whole model, and everything below is its enforcement.
//!
//! `Map_Init`'s player loop (`PRG011_A23E`) walks *both* player slots and resets
//! each to `Map_Y_Starts[World_Num]` with X forced to `$20`, wiping
//! `Map_Entered_*`, `Map_Previous_*` and the `Map_Prev_XOff/XHi` scroll backups.
//! `$84A0` then copies `Map_Entered_*` into `World_Map_*`. **That loop is not a
//! bug — it is vanilla's answer to a world change**, where beating a world drags
//! both players forward and neither has a position in the new world worth
//! keeping.
//!
//! In the maze each player owns their world, so nothing either player does may
//! move the other. Three situations, distinguished by
//! [`HANDOVER`](super::maze_state::HANDOVER):
//!
//! | situation | who is repositioned | why |
//! |---|---|---|
//! | turn hand-over (`$01`) | nobody | the incoming player is *returning* to where they stood |
//! | ordinary world change (`$00`) | the live player | they really did arrive somewhere new; the partner did not move |
//! | new game (`$02`) | both | the only time both players genuinely start together |
//!
//! `$00` covers the airship, the castle, the whistle, the warp zone and the
//! game-over return, and it is the resting value — so a world change this module
//! has never heard of gets the right behaviour by default.
//!
//! **Game over needs no code of its own**, which falls out of the same rule.
//! `world_travel::GAMEOVER_RETURN` sends the player who ran out of lives to the
//! world a new game starts in — load-bearing, since that is the one world the
//! fill guarantees is escapable — and because only the live player is
//! repositioned, the survivor is untouched. When the turn passes to them, the
//! router sees their own world and the hand-over path keeps their position.
//!
//! # The pattern: vanilla's loops over `Total_Players`
//!
//! Enforcing that rule is not one edit, because vanilla asserts "both players
//! share a world" in more than one place, and each one is a loop over
//! `Total_Players` touching per-player map state. Three found so far, and a
//! playtest found two of them:
//!
//! 1. **`Map_Init`'s player loop** — position. [`MAP_INIT_GATE`] and
//!    [`LOOP_TAIL`].
//! 2. **The king's-wand-return cutscene** (`PRG030_9062`, after an airship or
//!    castle clear and nothing else) — *camera*. It zeroes both players'
//!    `Map_Prev_XOff/XHi`, which `PRG030_8634` restores `Horz_Scroll` from, so
//!    the partner came back correctly placed with the viewport on page 0.
//!    [`CAMERA_KEEP`].
//! 3. **`world_persist::RESTORE_ARRIVAL`** — not a loop but the same
//!    assumption, writing Mario's absolute slots for whoever took a telepad.
//!
//! Two siblings were checked and left alone: `PRG030_92B6` (the game-over
//! continue) writes only `Player_Current`'s camera and is already right, and
//! the 2P Vs Challenge *swaps* the two players' backups, which is what that
//! minigame means to do.
//!
//! # Two hooks on one loop, and why not one
//!
//! The loop counts `X` down from `Total_Players - 1`, so running it once for the
//! live player needs both ends: [`MAP_INIT_GATE`] over the `LDX Total_Players`
//! at its head, and [`LOOP_TAIL`] over the `DEX / BPL` at its foot — without the
//! second, Luigi's pass would be followed by Mario's and clobber him.
//!
//! Reimplementing the body instead would be about the same size and worse:
//! `start_airship_swap` splices its own `JSR` over the body's last store
//! (`MAP_INIT_SCROLL_SITE`, byte-adjacent to [`LOOP_TAIL`]'s hook) to re-stamp a
//! swapped world's start column, so a private copy would silently ignore swapped
//! starts. Reusing vanilla's body keeps that working: the helper stamps
//! whichever slot the loop is on, so one pass stamps one player and a skipped
//! loop stamps none.
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
//! PRG030 is always mapped, and 22 of its last 42 bytes is real rent — but the
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
//! * [`MAP_INIT_GATE`], on the ordinary-world-change path. `Map_Init` runs on
//!   every entry to `$84A0` and every world change reaches `$84A0` with
//!   `World_Num` already set, so one stamp there covers the telepad, the
//!   whistle, an airship clear, the warp zone and the game-over return — and
//!   stamps only the live player, which is the point.
//! * `completion_bits::NEW_GAME_INIT`, which seeds *both* entries with the
//!   starting world and raises `HANDOVER = $02`. It has to seed them:
//!   `world_order` can start the game in any world, so zeroed bytes would tell
//!   the second player they began in World 1.
//!
//! An earlier cut also stamped from `completion_bits::WIPE_REPLACEMENT`, which
//! reads as the tidier place to say "the world changed" and was redundant —
//! every path reaching that hook ran `Map_Init` twenty bytes earlier.
//!
//! # One-player mode is untouched
//!
//! `Player_Current` never leaves 0 in one-player mode — the hand-over loop only
//! advances while `Total_Players` is 2 — so `PLAYER_WORLD[0]` is the only entry
//! ever stamped, and it is stamped from `World_Num` itself. [`TURN_SWAP`]'s
//! compare is therefore always equal, its store always writes the byte that is
//! already there, and the hand-over always takes `$84D7`, so `HANDOVER` is never
//! raised. The gate's one-player pass is the single iteration vanilla ran
//! anyway (`Total_Players - 1` is 0), and [`MARKER_GATE`] sits behind vanilla's
//! own `Total_Players` test and is never reached at all.

use crate::rom::Rom;

use super::maze_state::{HANDOVER, PLAYER_WORLD};
use super::rom_data::{
    FS_CAMERA_KEEP, FS_LOOP_TAIL, FS_MAP_INIT_GATE, FS_MARKER_GATE, FS_TURN_SWAP, PLAYER_CURRENT,
    WORLD_NUM, prg_bank_file_to_cpu, prg010_file_to_cpu, prg030_file_to_cpu,
};

// --- Addresses ----------------------------------------------------------

/// Where [`TURN_SWAP`] runs. PRG030, always mapped. `$9FD6`.
const TURN_SWAP_CPU: u16 = prg030_file_to_cpu(FS_TURN_SWAP);

/// Where [`MARKER_GATE`] runs. PRG010 — it is only reached from PRG010's own
/// `Map_No_Pan`, so the bank is its caller's. `$D69C`.
const MARKER_GATE_CPU: u16 = prg010_file_to_cpu(FS_MARKER_GATE);

/// Where [`MAP_INIT_GATE`] runs, and where [`LOOP_TAIL`] does. PRG010 is safe
/// for both even though they are reached from `Map_Init` in PRG011, because
/// `Map_Init` has exactly one caller — `$84A0`'s `JSR` at `$84AD` — and
/// `$84A0`'s first act is to map PRG010 into `$C000` and PRG011 into `$A000`.
const MAP_INIT_GATE_CPU: u16 = prg010_file_to_cpu(FS_MAP_INIT_GATE);
const LOOP_TAIL_CPU: u16 = prg010_file_to_cpu(FS_LOOP_TAIL);

/// Offset of [`LOOP_TAIL`]'s shared exit, which [`MAP_INIT_GATE`] jumps
/// straight to on the hand-over path so the two paths share one flag clear.
const LOOP_TAIL_EXIT_OFF: u16 = 11;
const LOOP_TAIL_EXIT_CPU: u16 = LOOP_TAIL_CPU + LOOP_TAIL_EXIT_OFF;

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

/// `Total_Players` (`$072B`) — the loop bound [`MAP_INIT_GATE`] displaces and
/// replays on the paths that want vanilla's count.
const TOTAL_PLAYERS: u16 = 0x072B;

/// The per-player map camera: `Map_Prev_XOff` (`$0722`, low) and
/// `Map_Prev_XHi` (`$0724`, page). `PRG030_8634` restores `Horz_Scroll` and
/// `Horz_Scroll_Hi` from these on every map entry, so they *are* the viewport.
const MAP_PREV_XOFF: u16 = 0x0722;
const MAP_PREV_XHI: u16 = 0x0724;

/// Where [`CAMERA_KEEP`] runs. PRG027, which the cutscene has mapped at
/// `$A000` for its whole duration — see [`FS_CAMERA_KEEP`]'s note. `$BD5F`.
const CAMERA_KEEP_CPU: u16 = prg_bank_file_to_cpu(27, FS_CAMERA_KEEP);

/// The two camera stores inside the king's-wand-return cutscene's player loop
/// (`PRG030_9062`, CPU `$9068`), six bytes: `STA Map_Prev_XOff,X` and
/// `STA Map_Prev_XHi,X`. The loop's `BPL` targets `$9062`, so nothing branches
/// into the middle of the pair.
const WAND_CUTSCENE_CAMERA_OFFSET: usize = 0x3D078;

/// `LDX Total_Players` at the head of `Map_Init`'s player loop (PRG011, CPU
/// `$A23A`), three bytes. The `LDY World_Num` before it and the `DEX` after it
/// are whole instructions and the loop label `PRG011_A23E` is the byte after
/// that, so this is the last site where the loop can be diverted without
/// landing on a branch target.
const MAP_INIT_LOOP_OFFSET: usize = 0x1624A;

/// The loop's `DEX / BPL PRG011_A23E` (PRG011, CPU `$A271`), three bytes.
/// Nothing in the disassembly references `$A26x`-`$A27x`, so neither the `DEX`
/// nor the `STX Map_2PVsGame` after it is a branch target.
const MAP_INIT_TAIL_OFFSET: usize = 0x16281;

/// `PRG011_A23D`, vanilla's `DEX` — where the paths that keep `Total_Players`
/// rejoin.
const MAP_INIT_DEX_CPU: u16 = 0xA23D;
/// `PRG011_A23E`, the loop body's first byte.
const MAP_INIT_BODY_CPU: u16 = 0xA23E;
/// `STX Map_2PVsGame` at CPU `$A274`, the first instruction after the loop.
const MAP_INIT_AFTER_LOOP_CPU: u16 = 0xA274;

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
/// Raising `HANDOVER` is what tells [`MAP_INIT_GATE`] not to reposition
/// anybody. This is the only place that distinction exists: it is the one world
/// change that is a player *returning* to a world they were already standing in.
#[rustfmt::skip]
const TURN_SWAP: [u8; 22] = [
    0xBD, PLAYER_WORLD as u8, (PLAYER_WORLD >> 8) as u8, //  0: LDA PLAYER_WORLD,X
    0xCD, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,      //  3: CMP World_Num
    0x8D, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,      //  6: STA World_Num
    0xF0, 0x08,                                         //  9: BEQ +8 -> same

    0xA9, 0x01,                                         // 11: LDA #$01
    0x8D, HANDOVER as u8, (HANDOVER >> 8) as u8,        // 13: STA HANDOVER
    0x4C, MAP_INIT_CPU as u8,
          (MAP_INIT_CPU >> 8) as u8,                    // 16: JMP PRG030_84A0

    0x4C, TURN_REINIT_CPU as u8,
          (TURN_REINIT_CPU >> 8) as u8,                 // 19: JMP PRG030_84D7  ; same
];

// --- `Map_Init`'s player loop, both ends --------------------------------

/// Decide which players `Map_Init` may reposition, and record which world the
/// live player is now in.
///
/// ```text
/// $D63C  AD DC 7A   LDA HANDOVER
/// $D63F  4A         LSR A               ; $01 -> C=1 ; $02 -> A<>0 ; $00 -> Z=1
/// $D640  B0 14      BCS handover
/// $D642  D0 0C      BNE newgame
/// ; ordinary: the live player only
/// $D644  AE 26 07   LDX Player_Current
/// $D647  AD 27 07   LDA World_Num
/// $D64A  9D DA 7A   STA PLAYER_WORLD,X
/// $D64D  4C 3E A2   JMP $A23E           ; the body, one pass at X = current
/// ; new game: both, as vanilla
/// $D650  AE 2B 07   newgame: LDX Total_Players
/// $D653  4C 3D A2   JMP $A23D           ; vanilla's DEX
/// ; hand-over: nobody
/// $D656  4C xx xx   handover: JMP LOOP_TAIL's exit
/// ```
///
/// **The `LSR` is the three-way branch.** `$01` shifts its bit into carry and
/// leaves `A` zero; `$02` leaves `A` non-zero with carry clear; `$00` leaves
/// both clear. So `BCS` then `BNE` separates all three in four bytes, and the
/// resting value `$00` — the one nothing has to set — lands on the arm that is
/// right for every world change this module has never heard of.
///
/// **The hand-over arm jumps into [`LOOP_TAIL`] rather than past the loop**, so
/// the flag clear, the `X = $FF` and the jump to `$A274` are written once
/// instead of twice. That is seven bytes, and it is why this fits its row.
///
/// `Y` survives untouched on every arm — vanilla's `LDY World_Num` sits
/// immediately before the displaced `LDX` and the body indexes `Map_Y_Starts`
/// with it.
#[rustfmt::skip]
const MAP_INIT_GATE: [u8; 29] = [
    0xAD, HANDOVER as u8, (HANDOVER >> 8) as u8,        //  0: LDA HANDOVER
    0x4A,                                               //  3: LSR A
    0xB0, 0x14,                                         //  4: BCS +20 -> handover
    0xD0, 0x0C,                                         //  6: BNE +12 -> newgame

    0xAE, PLAYER_CURRENT as u8,
          (PLAYER_CURRENT >> 8) as u8,                  //  8: LDX Player_Current
    0xAD, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,      // 11: LDA World_Num
    0x9D, PLAYER_WORLD as u8, (PLAYER_WORLD >> 8) as u8, // 14: STA PLAYER_WORLD,X
    0x4C, MAP_INIT_BODY_CPU as u8,
          (MAP_INIT_BODY_CPU >> 8) as u8,               // 17: JMP $A23E

    0xAE, TOTAL_PLAYERS as u8,
          (TOTAL_PLAYERS >> 8) as u8,                   // 20: LDX Total_Players  ; newgame
    0x4C, MAP_INIT_DEX_CPU as u8,
          (MAP_INIT_DEX_CPU >> 8) as u8,                // 23: JMP $A23D

    0x4C, LOOP_TAIL_EXIT_CPU as u8,
          (LOOP_TAIL_EXIT_CPU >> 8) as u8,              // 26: JMP exit  ; handover
];

/// Stop the loop after the live player's pass — and let a new game have its
/// second.
///
/// ```text
/// $D6B4  CA         DEX                 ; the displaced DEX
/// $D6B5  30 08      BMI exit            ; vanilla's own exit
/// $D6B6  AD DC 7A   LDA HANDOVER
/// $D6B9  F0 03      BEQ exit            ; $00: one pass was the whole job
/// $D6BB  4C 3E A2   JMP $A23E           ; $02: the other player too
/// $D6BE  A9 00      exit: LDA #$00
/// $D6C0  8D DC 7A   STA HANDOVER
/// $D6C3  A2 FF      LDX #$FF
/// $D6C5  4C 74 A2   JMP $A274
/// ```
///
/// Vanilla's `BPL` is what this replaces, so the loop no longer decides its own
/// length. The three cases:
///
/// * **one pass at `X = 0`** (one-player, or Mario live) — `DEX` goes negative
///   and `BMI` exits, exactly as vanilla did.
/// * **one pass at `X = 1`** (Luigi live) — `DEX` leaves 0, which vanilla would
///   have looped on and clobbered Mario with. `HANDOVER` is `$00`, so it exits.
/// * **both passes** (new game) — `HANDOVER` is `$02`, so the first exit test
///   falls through and jumps back to the body; the second pass exits on `BMI`.
///
/// `X = $FF` and `A = 0` are restored at the exit because the instructions after
/// the loop read them: `STX Map_2PVsGame` wants `$FF` (vanilla's comment says
/// so) and the `Map_WhiteHouse` / `Map_CoinShip` clears want `A` zero. Every
/// path leaves both correct — including the `BMI` arm, where the body's own
/// trailing `LDA #$00` already zeroed `A`.
///
/// The flag is cleared here rather than at the head so that all four arms —
/// three of this routine's and the gate's hand-over — share one clear.
#[rustfmt::skip]
const LOOP_TAIL: [u8; 21] = [
    0xCA,                                               //  0: DEX          ; displaced
    0x30, 0x08,                                         //  1: BMI +8 -> exit
    0xAD, HANDOVER as u8, (HANDOVER >> 8) as u8,        //  3: LDA HANDOVER
    0xF0, 0x03,                                         //  6: BEQ +3 -> exit
    0x4C, MAP_INIT_BODY_CPU as u8,
          (MAP_INIT_BODY_CPU >> 8) as u8,               //  8: JMP $A23E

    0xA9, 0x00,                                         // 11: LDA #$00     ; exit
    0x8D, HANDOVER as u8, (HANDOVER >> 8) as u8,        // 13: STA HANDOVER
    0xA2, 0xFF,                                         // 16: LDX #$FF
    0x4C, MAP_INIT_AFTER_LOOP_CPU as u8,
          (MAP_INIT_AFTER_LOOP_CPU >> 8) as u8,         // 18: JMP $A274
];

// --- The wand-return cutscene's camera wipe -----------------------------

/// Zero the map camera for the live player only, inside the king's-wand-return
/// cutscene.
///
/// ```text
/// $BD5F  EC 26 07   CPX Player_Current
/// $BD62  D0 06      BNE skip
/// $BD64  9D 22 07   STA Map_Prev_XOff,X   ; A is already $00
/// $BD67  9D 24 07   STA Map_Prev_XHi,X
/// $BD6A  60         skip: RTS
/// ```
///
/// **The third vanilla routine that assumes both players share a world.**
/// `PRG030_9062` runs after an airship or castle clear — and after nothing else
/// — and it walks *both* player slots:
///
/// ```text
///         JSR Clear_RAM_thru_ZeroPage   ; $0000-$06FF, live Horz_Scroll included
///         LDX Total_Players
///         DEX
/// $9062:  STA Player_FallToKing,X
///         STA Map_ReturnStatus
///         STA Map_Prev_XOff,X           ; <- the pair this replaces
///         STA Map_Prev_XHi,X
///         DEX
///         BPL $9062
/// ```
///
/// Vanilla is right to: `Map_Init` then puts both players on the new world's
/// start tile, where a zeroed camera is the correct framing. With independent
/// worlds the partner's *position* survives and their *camera* did not, so they
/// came back standing in the right place with the viewport on page 0 — the
/// sprite drawn at the right offset against the wrong screen, and movement that
/// felt correct because it was.
///
/// Only the two camera stores are gated. `Player_FallToKing,X` and
/// `Map_ReturnStatus` still clear for both players exactly as vanilla, because
/// they are not state the per-player world model has any claim on, and
/// narrowing the change narrows the risk.
///
/// `A` is `$00` from the loop's own setup and this routine only stores, so it
/// survives for the next iteration; `X` is the loop index and is untouched.
#[rustfmt::skip]
const CAMERA_KEEP: [u8; 12] = [
    0xEC, PLAYER_CURRENT as u8, (PLAYER_CURRENT >> 8) as u8, //  0: CPX Player_Current
    0xD0, 0x06,                                         //  3: BNE +6 -> skip

    0x9D, MAP_PREV_XOFF as u8, (MAP_PREV_XOFF >> 8) as u8, //  5: STA Map_Prev_XOff,X
    0x9D, MAP_PREV_XHI as u8, (MAP_PREV_XHI >> 8) as u8,  //  8: STA Map_Prev_XHi,X

    0x60,                                               // 11: RTS  ; skip
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

    // Both ends of `Map_Init`'s player loop. Order between them does not
    // matter; neither hook site overlaps the other, nor the store at
    // `MAP_INIT_SCROLL_SITE` that `start_airship_swap` may splice immediately
    // before the tail.
    rom.write_range(FS_MAP_INIT_GATE, &MAP_INIT_GATE);
    rom.write_range(MAP_INIT_LOOP_OFFSET, &jmp(0x4C, MAP_INIT_GATE_CPU));

    rom.write_range(FS_LOOP_TAIL, &LOOP_TAIL);
    rom.write_range(MAP_INIT_TAIL_OFFSET, &jmp(0x4C, LOOP_TAIL_CPU));

    // The wand-return cutscene's camera wipe. Six vanilla bytes become a
    // three-byte call and three `NOP`s rather than a tighter splice: the pair
    // has to stay six bytes wide because the loop's `BPL` counts back to
    // `$9062` and a shorter body would move the branch target.
    rom.write_range(FS_CAMERA_KEEP, &CAMERA_KEEP);
    let mut camera_hook = [0xEAu8; 6];
    camera_hook[..3].copy_from_slice(&jmp(0x20, CAMERA_KEEP_CPU));
    rom.write_range(WAND_CUTSCENE_CAMERA_OFFSET, &camera_hook);
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
    /// Vanilla at [`MAP_INIT_LOOP_OFFSET`]: `LDX Total_Players`.
    const LOOP_HEAD_VANILLA: [u8; 3] = [0xAE, 0x2B, 0x07];
    /// Vanilla at [`MAP_INIT_TAIL_OFFSET`]: `DEX / BPL PRG011_A23E`.
    const LOOP_TAIL_VANILLA: [u8; 3] = [0xCA, 0x10, 0xCA];
    /// Vanilla at [`WAND_CUTSCENE_CAMERA_OFFSET`]: the two camera stores.
    #[rustfmt::skip]
    const CAMERA_VANILLA: [u8; 6] = [0x9D, 0x22, 0x07, 0x9D, 0x24, 0x07];

    #[test]
    fn the_camera_keep_is_well_formed() {
        let mut hook = [0xEAu8; 6];
        hook[..3].copy_from_slice(&jmp(0x20, CAMERA_KEEP_CPU));
        asm::check(&CAMERA_KEEP)
            .allocation(FS_CAMERA_KEEP)
            .origin(CAMERA_KEEP_CPU)
            .hook(&CAMERA_VANILLA, 0, &hook)
            .assert_ok();
    }

    /// The gated pair has to be the *same two stores* vanilla made, or the
    /// cutscene stops clearing the live player's camera and they come back
    /// framed on wherever they were before the airship.
    #[test]
    fn the_camera_keep_replays_both_stores_it_displaced() {
        assert_eq!(CAMERA_KEEP[5..11], CAMERA_VANILLA, "both camera stores must be replayed");
    }

    #[test]
    fn the_map_init_gate_is_well_formed() {
        asm::check(&MAP_INIT_GATE)
            .allocation(FS_MAP_INIT_GATE)
            .origin(MAP_INIT_GATE_CPU)
            .hook(&LOOP_HEAD_VANILLA, 0, &jmp(0x4C, MAP_INIT_GATE_CPU))
            .assert_ok();
    }

    #[test]
    fn the_loop_tail_is_well_formed() {
        asm::check(&LOOP_TAIL)
            .allocation(FS_LOOP_TAIL)
            .origin(LOOP_TAIL_CPU)
            .hook(&LOOP_TAIL_VANILLA, 0, &jmp(0x4C, LOOP_TAIL_CPU))
            .assert_ok();
    }

    /// The gate's hand-over arm jumps into the middle of [`LOOP_TAIL`], so the
    /// offset it names has to be that routine's `LDA #$00` and not some byte
    /// inside an instruction. Editing either array would otherwise land the
    /// jump mid-instruction with nothing to notice.
    #[test]
    fn the_handover_arm_lands_on_the_shared_exit() {
        assert_eq!(
            LOOP_TAIL[LOOP_TAIL_EXIT_OFF as usize..LOOP_TAIL_EXIT_OFF as usize + 2],
            [0xA9, 0x00],
            "LOOP_TAIL_EXIT_OFF must name the exit's LDA #$00",
        );
        assert_eq!(
            u16::from_le_bytes([MAP_INIT_GATE[27], MAP_INIT_GATE[28]]),
            LOOP_TAIL_EXIT_CPU,
            "the gate's hand-over arm must jump to that exit",
        );
    }

    /// The tail replays the `DEX` it displaced, and keeps vanilla's own
    /// negative-exit. A tail that dropped either would run the loop body a
    /// second time for the same player, or never exit at all.
    #[test]
    fn the_tail_replays_the_dex_and_keeps_the_vanilla_exit() {
        assert_eq!(LOOP_TAIL[0], LOOP_TAIL_VANILLA[0], "the displaced DEX must be replayed");
        assert_eq!(LOOP_TAIL[1], 0x30, "vanilla's negative exit must survive as a BMI");
    }

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
        assert_eq!(
            rom.data[MAP_INIT_LOOP_OFFSET..MAP_INIT_LOOP_OFFSET + 3],
            LOOP_HEAD_VANILLA,
            "Map_Init's player loop no longer opens with LDX Total_Players",
        );
        assert_eq!(
            rom.data[MAP_INIT_TAIL_OFFSET..MAP_INIT_TAIL_OFFSET + 3],
            LOOP_TAIL_VANILLA,
            "Map_Init's player loop no longer ends with DEX / BPL",
        );
        // The `BPL` really does close *this* loop: its operand has to land on
        // the body the gate also jumps to. A tail hook aimed at some other
        // loop's exit would pass every other check here.
        let bpl_target =
            (MAP_INIT_TAIL_OFFSET + 3) as isize + (LOOP_TAIL_VANILLA[2] as i8) as isize;
        assert_eq!(
            bpl_target as usize, 0x1624E,
            "the displaced BPL does not branch to PRG011_A23E",
        );
        assert_eq!(
            rom.data[WAND_CUTSCENE_CAMERA_OFFSET..WAND_CUTSCENE_CAMERA_OFFSET + 6],
            CAMERA_VANILLA,
            "the wand-return cutscene no longer zeroes the camera with two indexed stores",
        );
    }

    /// `start_airship_swap` splices its own `JSR` over the store immediately
    /// before the tail hook. Byte-adjacent is fine; overlapping would mean one
    /// patch silently eating the other, and which won would depend on the
    /// option being on.
    #[test]
    fn the_tail_hook_clears_the_airship_swap_splice() {
        use crate::randomize::rom_data::MAP_INIT_SCROLL_SITE;
        assert_eq!(
            MAP_INIT_SCROLL_SITE + 3,
            MAP_INIT_TAIL_OFFSET,
            "start_airship_swap's Map_Init splice must end exactly where the tail hook starts",
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
