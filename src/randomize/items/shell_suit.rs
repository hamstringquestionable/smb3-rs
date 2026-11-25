//! The Shell Suit: a Special Suit Wild suit blocks and Big ? boxes hand out.
//!
//! It is the Hammer Suit with the black swapped for green, the duck that
//! blocks fireballs, and the Kuribo's Shoe's stomp — and walks on munchers —
//! but no hammers. Only what Mario could not stomp dies the shoe's way: a
//! Koopa or a Buzzy Beetle still drops its shell, and a Shell still kicks it.
//!
//! "Like the shoe" is the whole promise, and the limit. The stomp lives in
//! `Player_HitEnemy`; an enemy whose touch skips it hurts the shoe and the
//! Shell alike. Boo, Hot Foot, Ptooie, the Nippers and the charging and
//! pool-to-pool Cheep Cheeps (PRG002) touch with a bare `Player_GetHurt`;
//! Thwomps, Rotodiscs, Podoboos and the bosses run their own collision. All
//! of those were left as they are on purpose.
//!
//! ## Suit 6 plus a marker
//!
//! The engine looks things up by `Player_Suit` in about 44 places, in tables
//! of exactly seven entries packed back to back, so a real suit 7 would mean
//! moving all of them. Instead a Shell **is** suit 6: every table, the
//! movement, the duck pose and the fireball block (`prg007.asm` ~5220) are the
//! Hammer Suit's. What tells them apart is one byte per player in cartridge
//! RAM, [`MARKER`], indexed by `Player_Current`.
//!
//! The marker is written by the one place in a level that hands out a suit 6
//! — the star's suit grant, which Big `?` boxes and every suit block reach —
//! and holds the suit just granted, with bit 7 set for a Shell. On the map the
//! only source is using a suit from the inventory, which writes the new map
//! power-up there too, bit 7 clear. So a stale marker (a Shell hurt down to
//! Big) is harmless: the marker only counts while `Player_Suit` is 6, and the
//! next way back to 6 rewrites it. The RAM is never cleared by the console,
//! so it lives through levels, the map, world travel and 2-player turns with
//! no save-and-restore code; power-on garbage can at worst make the first
//! suit of a game poof where it would have played the power-up sound.
//!
//! ## What the marker changes
//!
//! | Where | Site | Change |
//! |---|---|---|
//! | Stomp | PRG000 `$D267` | "shoe or statue" becomes "shoe, statue or Shell": the shoe's stomp |
//! | Stomp | PRG000 `$D27C` | the shoe's kill-on-stomp, for a Shell only on what can't be stomped |
//! | Munchers | PRG008 `$BDA4` | "shoe" becomes "shoe or Shell" |
//! | Hammers | PRG007 `$A247` | a Shell looks Big to the throw check |
//! | Palette | PRG008 `$A550` | a Shell uses `PowerUp_Palettes` row 3 |
//! | Map, after a move | PRG010 `$CE57` | green accent |
//! | Map, first load | PRG027 `$B8DD` | green accent |
//! | Suit grant | PRG001 `$A818` | keeps the marker, swaps Hammer and Shell, and makes an upside-down Big ? suit a Shell |
//! | Big ? suit | PRG001 `$A7D2` | flips the suit when the box's flag says Shell |
//! | Inventory | PRG026 `$A608` | clears the marker |
//!
//! ## The Big ? box
//!
//! `OBJ_BIGQBLOCK_MUSHROOM` (`$95`) is never placed in a vanilla level; only
//! the Big ? Blocks shuffle deals it. With the Shell Suit on it becomes the
//! Shell box: its entries in `BigQBlock_Item` / `BigQBlock_StarManFlash`
//! (PRG005) become the Starman and `$07` — frame 3, the Hammer Suit's picture,
//! plus bit 2, which no other code reads. The star's init flips a suit that
//! carries bit 2 upside-down (`Objects_FlipBits` bit 7, which its draw never
//! rewrites), and the grant hands out a Shell for an upside-down suit. So the
//! art is the Hammer Suit's, the way the poison mushroom is the 1-Up's.
//!
//! Row 3 is the Super Leaf's, which vanilla never reads (a Leaf wears Mario's
//! or Luigi's own colours, `Level_SetPlayerPUpPal`), so it is free for the
//! Shell: the Hammer row as it stands after the palette randomizer, with the
//! accent — the black — made green.
//!
//! The spike check sits right beside the muncher one and is left alone, on
//! the user's call: munchers only. Bob-ombs are left alone too: the shoe kills
//! one on touch because it cannot hold it, and a Shell can.

use crate::randomize::rom_data::{
    FS_SHELL_FLIP, FS_SHELL_GRANT, FS_SHELL_INVENTORY, FS_SHELL_MAP_LOAD, FS_SHELL_MAP_MOVE,
    FS_SHELL_TEST, FS_SHELL_THROW, prg_bank_file_to_cpu, prg010_file_to_cpu, prg030_file_to_cpu,
};
use crate::rom::Rom;

/// The marker, `$7BD0` Mario / `$7BD1` Luigi: the head of the 80 bytes at
/// `$7BD0-$7C1F` the disassembly declares unused. Nothing in the crate claims
/// them, and no code in the vanilla ROM addresses them.
pub(crate) const MARKER: u16 = 0x7BD0;
const MARKER_LO: u8 = MARKER as u8;
const MARKER_HI: u8 = (MARKER >> 8) as u8;

/// What the Shell's mystery object hands the suit grant: suit 6, bit 7 set.
pub(crate) const SHELL_GRANT: u8 = 0x86;
/// The Shell's mystery object: an `ObjInit_DoNothing` group-0 slot, like
/// Wild's `$0F` and `$10`.
pub(crate) const SHELL_ID: u8 = 0x11;

/// The Shell's accent, in place of the Hammer Suit's black.
const GREEN: u8 = 0x1A;

// --- Engine addresses ---------------------------------------------------

const PLAYER_SUIT: u8 = 0xED; // zero page
const PLAYER_KURIBO: [u8; 2] = [0x77, 0x05];
const PLAYER_STATUE: [u8; 2] = [0x7A, 0x05];
const PLAYER_CURRENT: [u8; 2] = [0x26, 0x07];

// --- PRG030: the shared test ---------------------------------------------

const TEST_CPU: u16 = prg030_file_to_cpu(FS_SHELL_TEST);
/// "Shoe, statue or Shell?", carry set if so. The stomp's two sites.
const STOMP_ALL_CPU: u16 = TEST_CPU;
/// "Shoe or Shell?", carry set if so. The muncher site.
const SHOE_OR_SHELL_CPU: u16 = TEST_CPU + 5;
/// "Shell?", carry set if so.
const IS_SHELL_CPU: u16 = TEST_CPU + 10;
/// `A` = `Player_Suit` and `Y` = its palette row, 3 for a Shell.
const SHELL_PALETTE_CPU: u16 = TEST_CPU + 30;
/// "Does this stomp kill?", carry set if so: always for the shoe or statue,
/// and for a Shell only when the enemy cannot be stomped. `Y` is the
/// enemy's group index, as `PRG000_D272` left it.
const STOMP_KILL_CPU: u16 = TEST_CPU + 41;

/// `ObjectGroup_Attributes3`, in the object bank at `$A000`, and its
/// `OA3_NOTSTOMPABLE` bit (5).
const OBJECT_ATTRIBUTES3: u16 = 0xA120;

const fn lo(cpu: u16) -> u8 {
    cpu as u8
}
const fn hi(cpu: u16) -> u8 {
    (cpu >> 8) as u8
}

/// The answer is in carry so every caller keeps `A`'s next load as it was and
/// can branch on it in the two bytes vanilla's own `BNE`/`BEQ` took. `X` (the
/// object, or the muncher loop) and `Y` (the object's group index, or the
/// muncher table) both come back as they went in. `Player_Suit` is 0-6, so a
/// suit other than 6 leaves `CMP #6` with carry clear: the "no" exit is the
/// `RTS` the branch lands on.
#[rustfmt::skip]
const TEST: [u8; 61] = [
    // StompAll:
    0xAD, PLAYER_STATUE[0], PLAYER_STATUE[1], // LDA Player_Statue
    0xD0, 0x17,                               // BNE yes
    // ShoeOrShell:
    0xAD, PLAYER_KURIBO[0], PLAYER_KURIBO[1], // LDA Player_Kuribo
    0xD0, 0x12,                               // BNE yes
    // IsShell:
    0xA5, PLAYER_SUIT,                        // LDA <Player_Suit
    0xC9, 0x06,                               // CMP #6
    0xD0, 0x0B,                               // BNE out   (C clear)
    0x98,                                     // TYA
    0x48,                                     // PHA
    0xAC, PLAYER_CURRENT[0], PLAYER_CURRENT[1], // LDY Player_Current
    0xB9, MARKER_LO, MARKER_HI,               // LDA marker,Y
    0x0A,                                     // ASL A     (C = Shell)
    0x68,                                     // PLA
    0xA8,                                     // TAY
    // out:
    0x60,                                     // RTS
    // yes:
    0x38,                                     // SEC
    0x60,                                     // RTS
    // ShellPalette:
    0x20, lo(IS_SHELL_CPU), hi(IS_SHELL_CPU), // JSR IsShell
    0xA5, PLAYER_SUIT,                        // LDA <Player_Suit
    0xA8,                                     // TAY
    0x90, 0x02,                               // BCC +2
    0xA0, 0x03,                               // LDY #3    (the Leaf's unused row)
    0x60,                                     // RTS
    // StompKill:
    0xAD, PLAYER_STATUE[0], PLAYER_STATUE[1], // LDA Player_Statue
    0x0D, PLAYER_KURIBO[0], PLAYER_KURIBO[1], // ORA Player_Kuribo
    0xD0, 0xEB,                               // BNE yes   (vanilla's own test)
    0x20, lo(IS_SHELL_CPU), hi(IS_SHELL_CPU), // JSR IsShell
    0x90, 0x06,                               // BCC done  (not a Shell: C clear)
    0xB9, lo(OBJECT_ATTRIBUTES3), hi(OBJECT_ATTRIBUTES3), // LDA ObjectGroup_Attributes3,Y
    0x0A, 0x0A, 0x0A,                         // ASL x3    (C = OA3_NOTSTOMPABLE)
    // done:
    0x60,                                     // RTS
];

// --- PRG000 / PRG008: the hooks into the test --------------------------

/// `PRG000_D267`: `LDA Player_Kuribo / ORA Player_Statue / BNE PRG000_D272`,
/// then `JMP PRG000_D355` (hurt). Both targets reload `A` first.
const STOMP_SITE: usize = 0x01277;
const STOMP_VANILLA: [u8; 8] = [0xAD, 0x77, 0x05, 0x0D, 0x7A, 0x05, 0xD0, 0x03];
#[rustfmt::skip]
const STOMP_PATCH: [u8; 8] = [
    0x20, lo(STOMP_ALL_CPU), hi(STOMP_ALL_CPU), // JSR StompAll
    0xB0, 0x06,                                 // BCS PRG000_D272
    0xEA, 0xEA, 0xEA,                           // NOP x3
];

/// `PRG000_D27C`: `LDA Player_Statue / ORA Player_Kuribo / BEQ PRG000_D29B` —
/// set, and the stomp is the shoe's (kill, or squash what squashes). A Shell
/// takes the shoe's only for what Mario could not stomp at all; a Koopa or a
/// Buzzy Beetle gets the ordinary stomp, and its shell.
const KILL_SITE: usize = 0x0128C;
const KILL_VANILLA: [u8; 8] = [0xAD, 0x7A, 0x05, 0x0D, 0x77, 0x05, 0xF0, 0x17];
#[rustfmt::skip]
const KILL_PATCH: [u8; 8] = [
    0x20, lo(STOMP_KILL_CPU), hi(STOMP_KILL_CPU), // JSR StompKill
    0x90, 0x1A,                                 // BCC PRG000_D29B
    0xEA, 0xEA, 0xEA,                           // NOP x3
];

/// `PRG008_BDA4`: `LDA Player_Kuribo / BEQ PRG008_BDAE` (get hurt). Inside the
/// muncher loop, so `X` and `Y` must survive — they do.
const MUNCHER_SITE: usize = 0x11DB4;
const MUNCHER_VANILLA: [u8; 5] = [0xAD, 0x77, 0x05, 0xF0, 0x05];
#[rustfmt::skip]
const MUNCHER_PATCH: [u8; 5] = [
    0x20, lo(SHOE_OR_SHELL_CPU), hi(SHOE_OR_SHELL_CPU), // JSR ShoeOrShell
    0x90, 0x05,                                         // BCC PRG008_BDAE
];

/// `Level_SetPlayerPUpPal`'s `LDA <Player_Suit / TAY` (PRG008 `$A550`). What
/// follows is `CMP #3 / BEQ / CMP #2 / BPL`, which a Shell (`A` = 6) passes
/// straight through with its row in `Y`.
const PALETTE_SITE: usize = 0x10560;
const PALETTE_VANILLA: [u8; 3] = [0xA5, PLAYER_SUIT, 0xA8];
const PALETTE_PATCH: [u8; 3] = [0x20, lo(SHELL_PALETTE_CPU), hi(SHELL_PALETTE_CPU)];

/// `PowerUp_Palettes` (PRG008 `$A529`): rows of four, `[unused, body, face,
/// accent]`.
const POWERUP_PALETTES: usize = 0x10539;
const LEAF_ROW: usize = POWERUP_PALETTES + 4 * 3;
const HAMMER_ROW: usize = POWERUP_PALETTES + 4 * 6;

// --- PRG007: no hammers --------------------------------------------------

const THROW_CPU: u16 = prg_bank_file_to_cpu(7, FS_SHELL_THROW);
/// Ends on the displaced `CMP #6`, whose flags the `BEQ` after the hook reads.
#[rustfmt::skip]
const THROW: [u8; 12] = [
    0x20, lo(IS_SHELL_CPU), hi(IS_SHELL_CPU), // JSR IsShell
    0xA5, PLAYER_SUIT,                        // LDA <Player_Suit
    0x90, 0x02,                               // BCC +2
    0xA9, 0x01,                               // LDA #1   (a Shell throws like Big: not at all)
    0xC9, 0x06,                               // CMP #6
    0x60,                                     // RTS
];
/// `PRG007_A247`: `LDA <Player_Suit / CMP #6`, then `BEQ PRG007_A251` (throw)
/// and `CMP #2 / BNE PRG007_A268` (no throw).
const THROW_SITE: usize = 0x0E257;
const THROW_VANILLA: [u8; 4] = [0xA5, PLAYER_SUIT, 0xC9, 0x06];
const THROW_PATCH: [u8; 4] = [0x20, lo(THROW_CPU), hi(THROW_CPU), 0xEA];

// --- PRG001: the suit grant ----------------------------------------------

const GRANT_CPU: u16 = prg_bank_file_to_cpu(1, FS_SHELL_GRANT);
/// Where the mystery objects' Hit entries go instead of `$A81C`, with the suit
/// (bit 7 for a Shell) in `A`.
pub(crate) const SUIT_GRANT_CPU: u16 = GRANT_CPU + 11;
/// `PUp_StarOrSuitFrames`.
const STAR_OR_SUIT_FRAMES: u16 = 0xA7FA;
/// `PRG001_A818` + 4: `CMP <Player_Suit / BEQ` — "already wearing it?".
const ALREADY_WEARING_CPU: u16 = 0xA81C;
/// `PRG001_A818` + 8: `TAY / INY / STY Player_QueueSuit`, then the poof.
const QUEUE_SUIT_CPU: u16 = 0xA820;

/// `FrameSuit` is the four bytes the hook displaces, so a Big `?` box's suit
/// arrives at `SuitGrant` the way a mystery object's does — and an upside-down
/// one, which only the Shell box makes, arrives as a Shell.
///
/// `SuitGrant` stores the new suit as the player's marker, and compares bit 7
/// with the old marker on the way: when it differs — Hammer to Shell, or
/// Shell to anything — the grant skips "already wearing it?" and queues the
/// suit, so the palette reloads and the poof plays even with suit 6 to suit 6.
#[rustfmt::skip]
const GRANT: [u8; 33] = [
    // FrameSuit:
    0xA8,                                             // TAY
    0xB9, lo(STAR_OR_SUIT_FRAMES), hi(STAR_OR_SUIT_FRAMES), // LDA PUp_StarOrSuitFrames,Y
    0xBC, OBJECTS_FLIP_BITS[0], OBJECTS_FLIP_BITS[1], // LDY Objects_FlipBits,X
    0x10, 0x02,                                       // BPL grant (right way up)
    0xA9, SHELL_GRANT,                                // LDA #Shell
    // SuitGrant:
    0xAC, PLAYER_CURRENT[0], PLAYER_CURRENT[1],       // LDY Player_Current
    0x48,                                             // PHA
    0x59, MARKER_LO, MARKER_HI,                       // EOR marker,Y
    0x0A,                                             // ASL A   (C = Shell changed)
    0x68,                                             // PLA
    0x99, MARKER_LO, MARKER_HI,                       // STA marker,Y
    0x29, 0x07,                                       // AND #7  (the suit)
    0xB0, 0x03,                                       // BCS queue
    0x4C, lo(ALREADY_WEARING_CPU), hi(ALREADY_WEARING_CPU), // JMP PRG001_A818+4
    // queue:
    0x4C, lo(QUEUE_SUIT_CPU), hi(QUEUE_SUIT_CPU),     // JMP PRG001_A818+8
];
/// `PRG001_A818`: `TAY / LDA PUp_StarOrSuitFrames,Y`. The fourth byte is never
/// reached — the mystery objects jump to `$A81C` straight after it, and a
/// Shell's to [`SUIT_GRANT_CPU`] — but a `NOP` keeps the listing honest.
const FRAME_SITE: usize = 0x02828;
const FRAME_VANILLA: [u8; 4] = [0xA8, 0xB9, 0xFA, 0xA7];
const FRAME_PATCH: [u8; 4] = [0x4C, lo(GRANT_CPU), hi(GRANT_CPU), 0xEA];

// --- PRG005 / PRG001: the Big ? box -------------------------------------

/// `Objects_FlipBits`: bit 7 draws the object upside-down. The Starman's draw
/// (`Object_ShakeAndDrawMirrored`) rewrites only bit 6, so it stays put — the
/// poison mushroom and the first Suit Blocks build both relied on it.
const OBJECTS_FLIP_BITS: [u8; 2] = [0x79, 0x06];
/// `BigQBlock_Item` and `BigQBlock_StarManFlash` (PRG005), entry 1:
/// `OBJ_BIGQBLOCK_MUSHROOM` (`$95`), which no vanilla level places. With the
/// Shell Suit on it is the Shell box: a Starman, frame 3 (the Hammer Suit's
/// picture) with bit 2 set, which nothing else reads.
const BIG_Q_MUSHROOM_ITEM: (usize, u8) = (0x0B6E3, 0x0D);
const BIG_Q_MUSHROOM_FLASH: (usize, u8) = (0x0B6EA, 0x01);
const STARMAN_ID: u8 = 0x0C;
const SHELL_BOX_FLASH: u8 = 0x07;

const FLIP_CPU: u16 = prg_bank_file_to_cpu(1, FS_SHELL_FLIP);
/// The end of `ObjInit_StarOrSuit`: the store it displaced, then the flip when
/// `PUp_StarManFlash` carries the Shell box's bit 2. Every other spawn of the
/// star leaves that bit clear (`$00`, `$80`, or a Big ? box's 1-3).
#[rustfmt::skip]
const FLIP: [u8; 16] = [
    0x9D, 0xE7, 0x7F,                         // STA Objects_SprAttr,X  (displaced)
    0xAD, 0x86, 0x05,                         // LDA PUp_StarManFlash
    0x29, 0x04,                               // AND #4
    0xF0, 0x05,                               // BEQ done
    0xA9, 0x80,                               // LDA #SPR_VFLIP
    0x9D, OBJECTS_FLIP_BITS[0], OBJECTS_FLIP_BITS[1], // STA Objects_FlipBits,X
    // done:
    0x60,                                     // RTS
];
/// `ObjInit_StarOrSuit`'s last store, `STA Objects_SprAttr,X` (PRG001 `$A7D2`).
/// Its `RTS` at `$A7D5` stays, unreached.
const FLIP_SITE: usize = 0x027E2;
const FLIP_VANILLA: [u8; 3] = [0x9D, 0xE7, 0x7F];
const FLIP_PATCH: [u8; 3] = [0x4C, lo(FLIP_CPU), hi(FLIP_CPU)];

// --- PRG026: the inventory -----------------------------------------------

const INVENTORY_CPU: u16 = prg_bank_file_to_cpu(26, FS_SHELL_INVENTORY);
/// `X` is `Player_Current`. The new map power-up has bit 7 clear.
#[rustfmt::skip]
const INVENTORY: [u8; 7] = [
    0x9D, 0x46, 0x07,           // STA World_Map_Power,X  (the store we displaced)
    0x9D, MARKER_LO, MARKER_HI, // STA marker,X
    0x60,                       // RTS
];
/// `PRG026_A608`: `STA World_Map_Power,X`.
const INVENTORY_SITE: usize = 0x34618;
const INVENTORY_VANILLA: [u8; 3] = [0x9D, 0x46, 0x07];
const INVENTORY_PATCH: [u8; 3] = [0x20, lo(INVENTORY_CPU), hi(INVENTORY_CPU)];

// --- PRG010 / PRG027: the map -------------------------------------------

/// The map power-up's accent load, or green for a Shell, with `X` the player
/// and `Y` the index the load uses. `Y` survives: nothing here writes it.
#[rustfmt::skip]
const fn map_accent(hammer_index: u8, table: u16) -> [u8; 16] {
    [
        0xC0, hammer_index,         // CPY #hammer
        0xD0, 0x08,                 // BNE plain
        0xBD, MARKER_LO, MARKER_HI, // LDA marker,X
        0x10, 0x03,                 // BPL plain
        0xA9, GREEN,                // LDA #green
        0x60,                       // RTS
        // plain:
        0xB9, lo(table), hi(table), // LDA table,Y  (the load we displaced)
        0x60,                       // RTS
    ]
}

const MAP_MOVE_CPU: u16 = prg010_file_to_cpu(FS_SHELL_MAP_MOVE);
/// `Map_PostJC_PUpPP2`, indexed by the map power-up.
const MAP_MOVE: [u8; 16] = map_accent(6, 0xCDC3);
/// `PRG010_CE57`: `LDA Map_PostJC_PUpPP2,Y`, the accent restored after each
/// move.
const MAP_MOVE_SITE: usize = 0x14E67;
const MAP_MOVE_VANILLA: [u8; 3] = [0xB9, 0xC3, 0xCD];
const MAP_MOVE_PATCH: [u8; 3] = [0x20, lo(MAP_MOVE_CPU), hi(MAP_MOVE_CPU)];
/// `two_player_vs`'s `JMP PRG010_CEBF` at `$CE87`, which makes the bytes the
/// routine sits on unreachable.
const VS_JUMP: (usize, [u8; 3]) = (0x14E97, [0x4C, 0xBF, 0xCE]);

const MAP_LOAD_CPU: u16 = prg_bank_file_to_cpu(27, FS_SHELL_MAP_LOAD);
/// `InitPals_Per_MapPUp` + 3, indexed by four times the row.
const MAP_LOAD: [u8; 16] = map_accent(4 * 6, 0xB81E);
/// `PRG027_B8DD`: `LDA InitPals_Per_MapPUp+3,Y`, when the map is drawn.
const MAP_LOAD_SITE: usize = 0x378ED;
const MAP_LOAD_VANILLA: [u8; 3] = [0xB9, 0x1E, 0xB8];
const MAP_LOAD_PATCH: [u8; 3] = [0x20, lo(MAP_LOAD_CPU), hi(MAP_LOAD_CPU)];

/// Every in-place hook: file offset, vanilla bytes, patch.
const HOOKS: [(usize, &[u8], &[u8]); 10] = [
    (FLIP_SITE, &FLIP_VANILLA, &FLIP_PATCH),
    (STOMP_SITE, &STOMP_VANILLA, &STOMP_PATCH),
    (KILL_SITE, &KILL_VANILLA, &KILL_PATCH),
    (MUNCHER_SITE, &MUNCHER_VANILLA, &MUNCHER_PATCH),
    (PALETTE_SITE, &PALETTE_VANILLA, &PALETTE_PATCH),
    (THROW_SITE, &THROW_VANILLA, &THROW_PATCH),
    (FRAME_SITE, &FRAME_VANILLA, &FRAME_PATCH),
    (INVENTORY_SITE, &INVENTORY_VANILLA, &INVENTORY_PATCH),
    (MAP_MOVE_SITE, &MAP_MOVE_VANILLA, &MAP_MOVE_PATCH),
    (MAP_LOAD_SITE, &MAP_LOAD_VANILLA, &MAP_LOAD_PATCH),
];

/// Install everything but the mystery object, which
/// [`super::block_spawn`] adds to Wild's pool. Run after the palette
/// randomizer, whose Hammer row the Shell's is copied from, and after
/// `two_player_vs`, whose jump frees the map-move routine's bytes.
pub(crate) fn apply(rom: &mut Rom) {
    assert_eq!(rom.read_range(VS_JUMP.0, 3), VS_JUMP.1, "shell_suit: 2P Vs not disabled first");
    for (site, vanilla, patch) in HOOKS {
        assert_eq!(rom.read_range(site, vanilla.len()), vanilla, "shell_suit: hook 0x{site:05X}");
        rom.write_range(site, patch);
    }
    rom.write_range(FS_SHELL_TEST, &TEST);
    rom.write_range(FS_SHELL_FLIP, &FLIP);
    for ((at, vanilla), new) in
        [(BIG_Q_MUSHROOM_ITEM, STARMAN_ID), (BIG_Q_MUSHROOM_FLASH, SHELL_BOX_FLASH)]
    {
        assert_eq!(rom.read_byte(at), vanilla, "shell_suit: Big ? table 0x{at:05X}");
        rom.write_byte(at, new);
    }
    rom.write_range(FS_SHELL_THROW, &THROW);
    rom.write_range(FS_SHELL_GRANT, &GRANT);
    rom.write_range(FS_SHELL_INVENTORY, &INVENTORY);
    rom.write_range(FS_SHELL_MAP_MOVE, &MAP_MOVE);
    rom.write_range(FS_SHELL_MAP_LOAD, &MAP_LOAD);

    assert_eq!(rom.read_range(LEAF_ROW, 4), [0; 4], "shell_suit: the Leaf's row is in use");
    let mut row = rom.read_range(HAMMER_ROW, 4).to_vec();
    row[3] = GREEN;
    rom.write_range(LEAF_ROW, &row);
}

/// Whether a finished ROM carries the Shell Suit: every hook in place.
#[cfg(test)]
pub(crate) fn installed(rom: &Rom) -> bool {
    HOOKS.iter().all(|&(site, _, patch)| rom.read_range(site, patch.len()) == patch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mos6502::memory::Bus;

    type Cpu = mos6502::cpu::CPU<mos6502::memory::Memory, mos6502::instruction::Ricoh2a03>;

    fn cpu_with(origin: u16, code: &[u8]) -> Cpu {
        let mut cpu = Cpu::new(mos6502::memory::Memory::new(), mos6502::instruction::Ricoh2a03);
        cpu.memory.set_bytes(origin, code);
        cpu.registers.stack_pointer = mos6502::registers::StackPointer(0xFF);
        cpu
    }

    /// `JSR entry` from `$0200` and run to the return. Returns the CPU.
    fn call(mut cpu: Cpu, entry: u16) -> Cpu {
        cpu.memory.set_bytes(0x0200, &[0x20, lo(entry), hi(entry)]);
        cpu.registers.program_counter = 0x0200;
        for _ in 0..200 {
            cpu.single_step();
            if cpu.registers.program_counter == 0x0203 {
                return cpu;
            }
        }
        panic!("shell_suit: routine at {entry:04X} never returned");
    }

    fn carry(cpu: &Cpu) -> bool {
        cpu.registers.status.contains(mos6502::registers::Status::PS_CARRY)
    }

    /// One state of the player — suit, marker, shoe, statue, which player —
    /// and the stomped enemy's `ObjectGroup_Attributes3` byte.
    #[derive(Clone, Copy, Debug)]
    struct State {
        suit: u8,
        marker: u8,
        kuribo: u8,
        statue: u8,
        player: u8,
        attributes3: u8,
    }

    /// The group index `Y` holds, as `PRG000_D272` leaves it.
    const GROUP_INDEX: u8 = 0x5B;
    const NOT_STOMPABLE: u8 = 0x20;

    fn test_cpu(s: State) -> Cpu {
        let mut cpu = cpu_with(TEST_CPU, &TEST);
        cpu.memory.set_byte(u16::from(PLAYER_SUIT), s.suit);
        cpu.memory.set_byte(0x0577, s.kuribo);
        cpu.memory.set_byte(0x057A, s.statue);
        cpu.memory.set_byte(0x0726, s.player);
        cpu.memory.set_byte(MARKER + u16::from(s.player), s.marker);
        // The other player's marker says the opposite, so a wrong index shows.
        cpu.memory.set_byte(MARKER + u16::from(s.player ^ 1), s.marker ^ 0x80);
        cpu.memory.set_byte(OBJECT_ATTRIBUTES3 + u16::from(GROUP_INDEX), s.attributes3);
        // The neighbours say the opposite, so a wrong index shows.
        for neighbour in [GROUP_INDEX - 1, GROUP_INDEX + 1] {
            let other = s.attributes3 ^ NOT_STOMPABLE;
            cpu.memory.set_byte(OBJECT_ATTRIBUTES3 + u16::from(neighbour), other);
        }
        cpu.registers.index_x = 0x37;
        cpu.registers.index_y = GROUP_INDEX;
        cpu
    }

    fn states() -> Vec<State> {
        let mut out = Vec::new();
        for suit in 0..=6 {
            for marker in [0x00, 0x06, 0x86, 0x81, 0xFF] {
                for (kuribo, statue) in [(0, 0), (1, 0), (0, 0xC0)] {
                    for player in [0, 1] {
                        // Stompable, not, and not with every other bit set.
                        for attributes3 in [0x00, NOT_STOMPABLE, 0xFF, !NOT_STOMPABLE] {
                            out.push(State { suit, marker, kuribo, statue, player, attributes3 });
                        }
                    }
                }
            }
        }
        out
    }

    fn shell(s: State) -> bool {
        s.suit == 6 && s.marker & 0x80 != 0
    }

    /// Every entry answers in carry and gives `X` and `Y` back untouched.
    #[test]
    fn the_test_answers_in_carry_and_keeps_x_and_y() {
        for s in states() {
            for (entry, want) in [
                (STOMP_ALL_CPU, s.kuribo != 0 || s.statue != 0 || shell(s)),
                (SHOE_OR_SHELL_CPU, s.kuribo != 0 || shell(s)),
                (IS_SHELL_CPU, shell(s)),
                // The shoe and statue kill whatever they land on; a Shell only
                // what Mario could not stomp, so a Koopa still gives a shell.
                (
                    STOMP_KILL_CPU,
                    s.kuribo != 0
                        || s.statue != 0
                        || (shell(s) && s.attributes3 & NOT_STOMPABLE != 0),
                ),
            ] {
                let cpu = call(test_cpu(s), entry);
                assert_eq!(carry(&cpu), want, "{s:?} entry {entry:04X}");
                assert_eq!(cpu.registers.index_x, 0x37, "{s:?}: X");
                assert_eq!(cpu.registers.index_y, GROUP_INDEX, "{s:?}: Y");
            }
        }
    }

    /// `A` comes back as the suit, `Y` as the row: 3 for a Shell, else the suit.
    #[test]
    fn the_palette_entry_picks_row_3_for_a_shell() {
        for s in states() {
            let cpu = call(test_cpu(s), SHELL_PALETTE_CPU);
            assert_eq!(cpu.registers.accumulator, s.suit, "{s:?}");
            let row = if shell(s) { 3 } else { s.suit };
            assert_eq!(cpu.registers.index_y, row, "{s:?}");
        }
    }

    /// The throw check sees a Shell as Big, and everything else as itself,
    /// with the flags of `CMP #6`.
    #[test]
    fn a_shell_never_throws() {
        for s in states() {
            let mut cpu = test_cpu(s);
            cpu.memory.set_bytes(THROW_CPU, &THROW);
            let cpu = call(cpu, THROW_CPU);
            let looks = if shell(s) { 1 } else { s.suit };
            assert_eq!(cpu.registers.accumulator, looks, "{s:?}");
            let zero = cpu.registers.status.contains(mos6502::registers::Status::PS_ZERO);
            assert_eq!(zero, looks == 6, "{s:?}: BEQ throw");
        }
    }

    /// Run `SuitGrant` with `granted` in `A`; returns where it jumped, the
    /// suit in `A` and the marker afterwards.
    fn grant(old_marker: u8, granted: u8, player: u8, entry: u16) -> (u16, u8, u8) {
        grant_flipped(old_marker, granted, player, entry, 0x00)
    }

    /// [`grant`], with the object's `Objects_FlipBits`.
    fn grant_flipped(
        old_marker: u8,
        granted: u8,
        player: u8,
        entry: u16,
        flip: u8,
    ) -> (u16, u8, u8) {
        let mut cpu = cpu_with(GRANT_CPU, &GRANT);
        cpu.memory.set_byte(u16::from_le_bytes(OBJECTS_FLIP_BITS) + 0x04, flip);
        cpu.memory.set_bytes(STAR_OR_SUIT_FRAMES, &[0x00, 0x05, 0x04, 0x06]);
        cpu.memory.set_byte(0x0726, player);
        cpu.memory.set_byte(MARKER + u16::from(player), old_marker);
        cpu.memory.set_byte(MARKER + u16::from(player ^ 1), 0x55);
        cpu.registers.accumulator = granted;
        cpu.registers.index_x = 0x04;
        cpu.registers.program_counter = entry;
        for _ in 0..20 {
            let pc = cpu.registers.program_counter;
            if pc == ALREADY_WEARING_CPU || pc == QUEUE_SUIT_CPU {
                assert_eq!(cpu.registers.index_x, 0x04, "X is the power-up's slot");
                assert_eq!(cpu.memory.get_byte(MARKER + u16::from(player ^ 1)), 0x55);
                return (
                    pc,
                    cpu.registers.accumulator,
                    cpu.memory.get_byte(MARKER + u16::from(player)),
                );
            }
            cpu.single_step();
        }
        panic!("shell_suit: the grant never reached the star's code");
    }

    /// The suit grant always records what it gave, hands the star's code the
    /// plain suit, and goes around "already wearing it?" exactly when the
    /// Shell bit changes.
    #[test]
    fn the_grant_keeps_the_marker_and_swaps_hammer_and_shell() {
        for player in [0, 1] {
            for old in [0x00, 0x01, 0x06, 0x86, 0x85, 0xFF] {
                for granted in [2, 3, 4, 5, 6, SHELL_GRANT] {
                    let (to, suit, marker) = grant(old, granted, player, SUIT_GRANT_CPU);
                    assert_eq!(suit, granted & 7, "old {old:02X} granted {granted:02X}");
                    assert_eq!(marker, granted, "old {old:02X} granted {granted:02X}");
                    let changed = (old ^ granted) & 0x80 != 0;
                    let want = if changed { QUEUE_SUIT_CPU } else { ALREADY_WEARING_CPU };
                    assert_eq!(to, want, "old {old:02X} granted {granted:02X}");
                }
            }
        }
        // A Big ? box's frame goes through the frames table: Hammer is a real
        // Hammer Suit, and turns a Shell back into one.
        for (frame, suit) in [(1, 5), (2, 4), (3, 6)] {
            let (to, a, marker) = grant(0x86, frame, 0, GRANT_CPU);
            assert_eq!((to, a, marker), (QUEUE_SUIT_CPU, suit, suit), "frame {frame}");
        }
        // The Shell box's suit is upside-down: a Shell, from a Hammer or a
        // Shell alike; only the flip bit (7) counts, not the H-flip (6).
        for player in [0, 1] {
            let (to, a, marker) = grant_flipped(0x06, 3, player, GRANT_CPU, 0x80);
            assert_eq!((to, a, marker), (QUEUE_SUIT_CPU, 6, SHELL_GRANT));
            let (to, _, marker) = grant_flipped(0x86, 3, player, GRANT_CPU, 0xC0);
            assert_eq!((to, marker), (ALREADY_WEARING_CPU, SHELL_GRANT));
            let (_, _, marker) = grant_flipped(0x86, 3, player, GRANT_CPU, 0x40);
            assert_eq!(marker, 6, "H-flipped is still a Hammer Suit");
        }
    }

    /// The star's init flips the suit exactly when the Shell box set bit 2,
    /// keeps the store it displaced, and leaves `X` and `Y` alone.
    #[test]
    fn only_the_shell_box_comes_out_upside_down() {
        let flip_bits = u16::from_le_bytes(OBJECTS_FLIP_BITS) + 0x03;
        for flash in [0x00, 0x01, 0x02, 0x03, 0x80, SHELL_BOX_FLASH] {
            let mut cpu = cpu_with(FLIP_CPU, &FLIP);
            cpu.memory.set_byte(0x0586, flash);
            cpu.memory.set_byte(flip_bits, 0x40);
            cpu.registers.accumulator = 0x02; // the palette ObjInit picked
            cpu.registers.index_x = 0x03;
            cpu.registers.index_y = 0x03;
            let mut cpu = call(cpu, FLIP_CPU);
            assert_eq!(cpu.memory.get_byte(0x7FE7 + 3), 0x02, "{flash:02X}: SprAttr");
            let want = if flash == SHELL_BOX_FLASH { 0x80 } else { 0x40 };
            assert_eq!(cpu.memory.get_byte(flip_bits), want, "{flash:02X}: FlipBits");
            assert_eq!((cpu.registers.index_x, cpu.registers.index_y), (3, 3));
        }
        // Frame 3 is the Hammer Suit's picture; bit 7 would colour-cycle it.
        assert_eq!(SHELL_BOX_FLASH & 3, 3);
        assert_eq!(SHELL_BOX_FLASH & 0x80, 0);
    }

    /// The map's accent: green only for a Shell's suit 6, by player, with `Y`
    /// kept for the loads after it.
    #[test]
    fn the_map_shows_green_only_for_a_shell() {
        for (code, origin, hammer, table) in
            [(MAP_MOVE, MAP_MOVE_CPU, 6u8, 0xCDC3u16), (MAP_LOAD, MAP_LOAD_CPU, 24, 0xB81E)]
        {
            for player in [0u8, 1] {
                for marker in [0x00, 0x06, 0x86, 0x81] {
                    for index in [0, 1, 2, 3, hammer, hammer + 1] {
                        let mut cpu = cpu_with(origin, &code);
                        cpu.memory.set_byte(table + u16::from(index), 0x40 + index);
                        cpu.memory.set_byte(MARKER + u16::from(player), marker);
                        cpu.memory.set_byte(MARKER + u16::from(player ^ 1), marker ^ 0x80);
                        cpu.registers.index_x = player;
                        cpu.registers.index_y = index;
                        let cpu = call(cpu, origin);
                        let green = index == hammer && marker & 0x80 != 0;
                        let want = if green { GREEN } else { 0x40 + index };
                        assert_eq!(
                            cpu.registers.accumulator, want,
                            "{origin:04X} {player} {marker:02X} {index}"
                        );
                        assert_eq!(cpu.registers.index_y, index);
                    }
                }
            }
        }
    }

    /// On the real ROM (after `two_player_vs`): every hook lands, and the
    /// Shell's row is the Hammer row with a green accent.
    #[test]
    fn install_on_the_rom() {
        let Ok(data) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            return;
        };
        let mut rom = Rom::from_bytes(&data).unwrap();
        crate::randomize::qol::two_player_vs::apply(&mut rom);
        let hammer = rom.read_range(HAMMER_ROW, 4).to_vec();
        apply(&mut rom);
        for (site, _, patch) in HOOKS {
            assert_eq!(rom.read_range(site, patch.len()), patch);
        }
        assert_eq!(rom.read_range(LEAF_ROW, 4), [hammer[0], hammer[1], hammer[2], GREEN]);
        assert_eq!(hammer, [0x00, 0x30, 0x27, 0x0F], "vanilla Hammer row");
        // The Mushroom box is now the Shell box.
        assert_eq!(rom.read_byte(BIG_Q_MUSHROOM_ITEM.0), STARMAN_ID);
        assert_eq!(rom.read_byte(BIG_Q_MUSHROOM_FLASH.0), SHELL_BOX_FLASH);
        // The frames table the grant reads is where the hook says.
        let frames = crate::randomize::rom_data::prg_bank_cpu_to_file(1, STAR_OR_SUIT_FRAMES);
        assert_eq!(rom.read_range(frames, 4), [0x00, 0x05, 0x04, 0x06]);
    }
}

#[cfg(test)]
mod asm_checks {
    use super::*;
    use crate::randomize::rom_data::asm;

    #[test]
    fn the_shell_test_is_well_formed() {
        asm::check(&TEST)
            .allocation(FS_SHELL_TEST)
            .origin(TEST_CPU)
            .zero_page(0x02, &[PLAYER_SUIT])
            .assert_ok();
    }

    /// Every in-place hook replaces whole instructions with whole instructions.
    #[test]
    fn the_hooks_are_well_formed() {
        for (_, vanilla, patch) in HOOKS {
            asm::check(patch).fragment().hook(vanilla, 0, patch).assert_ok();
        }
    }

    #[test]
    fn the_throw_check_is_well_formed() {
        asm::check(&THROW)
            .allocation(FS_SHELL_THROW)
            .origin(THROW_CPU)
            .hook(&THROW_VANILLA, 0, &THROW_PATCH)
            .zero_page(0x02, &[PLAYER_SUIT])
            .assert_ok();
    }

    #[test]
    fn the_grant_is_well_formed() {
        asm::check(&GRANT)
            .allocation(FS_SHELL_GRANT)
            .origin(GRANT_CPU)
            .hook(&FRAME_VANILLA, 0, &FRAME_PATCH)
            .zero_page(0x02, &[])
            .assert_ok();
    }

    #[test]
    fn the_flip_is_well_formed() {
        asm::check(&FLIP)
            .allocation(FS_SHELL_FLIP)
            .origin(FLIP_CPU)
            .hook(&FLIP_VANILLA, 0, &FLIP_PATCH)
            .zero_page(0x02, &[])
            .assert_ok();
    }

    #[test]
    fn the_inventory_store_is_well_formed() {
        asm::check(&INVENTORY)
            .allocation(FS_SHELL_INVENTORY)
            .origin(INVENTORY_CPU)
            .hook(&INVENTORY_VANILLA, 0, &INVENTORY_PATCH)
            .zero_page(0x02, &[])
            .assert_ok();
    }

    #[test]
    fn the_map_accents_are_well_formed() {
        asm::check(&MAP_MOVE)
            .allocation(FS_SHELL_MAP_MOVE)
            .origin(MAP_MOVE_CPU)
            .hook(&MAP_MOVE_VANILLA, 0, &MAP_MOVE_PATCH)
            .zero_page(0x02, &[])
            .assert_ok();
        asm::check(&MAP_LOAD)
            .allocation(FS_SHELL_MAP_LOAD)
            .origin(MAP_LOAD_CPU)
            .hook(&MAP_LOAD_VANILLA, 0, &MAP_LOAD_PATCH)
            .zero_page(0x02, &[])
            .assert_ok();
    }
}
