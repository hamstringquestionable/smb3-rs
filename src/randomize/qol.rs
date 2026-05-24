use crate::rom::Rom;
use super::rom_data::{
    BETA_PATCHES,
    FS_BIG_Q_LOOKUP as BIG_Q_ROUTINE_OFFSET,
    FS_BIG_Q_SAVE as BIG_Q_PRG030_OFFSET,
    FS_CANOE_BACKUP,
    FS_CANOE_RESPAWN,
    FS_CARD_CLEAR as CARD_TRAMPOLINE,
    FS_HAMMER_LOCKS,
    FS_STARTING_ITEMS,
};

/// Starting lives value byte (LDA #imm operand).
/// Both Mario and Luigi are initialized from this single byte.
const STARTING_LIVES_OFFSET: usize = 0x308E1;

/// Base of the 8-byte lives init code: LDA #lives; STA $0736; STA $0737.
const LIVES_INIT_BASE: usize = 0x308E0;

// W3 drawbridge map tile patches: (file offset, replacement tile).
// Vanilla: 2× $B2 horizontal + 2× $B1 vertical. Replace with $B3
// (horizontal bridge path) and $BA (vertical-compatible open path).
const W3_DRAWBRIDGE_TILES: [(usize, u8); 4] = [
    (0x18777, 0xB3), // H1
    (0x18779, 0xB3), // H2
    (0x1880C, 0xBA), // V1
    (0x188F3, 0xBA), // V2
];

// Toggle code: LDA $07BB; EOR #$01; STA $07BB (8 bytes at 0x14A68)
const W3_TOGGLE_OFFSET: usize = 0x14A68;
const W3_TOGGLE_LEN: usize = 8;

// W2 rock blocking secret path (screen 1, row 0, col 5) — $51 → $45
const W2_SECRET_ROCK: usize = 0x186E0;

// W3 rock blocking boat path (screen 0, row 6, col 15) — $51 → $45
const W3_BOAT_ROCK: usize = 0x187DB;

// W4 rock blocking pipe path (screen 1, row 6, col 25) — $51 → $45
const W4_PIPE_ROCK: usize = 0x18A16;

// Big ? Block bonus room patch: decouple room selection from World_Num.
//
// Two-part patch:
// Part A — PRG030 (fixed bank): During level init, save the entry-point obj_ptr
//   from $65/$66 to scratch RAM ($7EB4/$7EB5) before the W8-specific code at
//   $894C can overwrite it with a hardcoded $C033. This hook is in the fixed
//   bank so it fires for ALL entry paths (normal tile, army sprite, etc.).
//   The old PRG012 hook was insufficient — it only covered Map_PrepareLevel
//   (enter state #$03) but W8 army sprites use a different path (state #$08).
// Part B — PRG026: Replace `LDY World_Num` in LevelJct_BigQuestionBlock with a
//   JSR to a lookup routine that reads the saved obj_ptr from scratch RAM and
//   maps it to the correct per-world bonus room index.

// Part A: PRG030 (fixed bank) trampoline for level init.
// Saves the entry-point obj_ptr from $65/$66 to scratch RAM $7EB4/$7EB5.
// Hooked in PRG030 (always loaded) so it fires for ALL entry paths — normal
// tile entry, army sprite encounters, and any other mechanism.
// Replaces `CPY #$07; BNE +$18` (4 bytes) with `JMP $9F2C` + NOP.
const BIG_Q_PRG030_HOOK: usize = 0x3C958;  // file offset of CPY #$07
const BIG_Q_PRG030_JMP: [u8; 4] = [0x4C, 0x2C, 0x9F, 0xEA];
// Trampoline in PRG030 free space — offset from rom_data::FS_BIG_Q_SAVE
// (imported as BIG_Q_PRG030_OFFSET at the top of the file).
const BIG_Q_PRG030_ROUTINE: [u8; 20] = [
    0xA5, 0x65,        // LDA $65        (real obj_lo, before W8 overwrite)
    0x8D, 0xB4, 0x7E,  // STA $7EB4
    0xA5, 0x66,        // LDA $66        (real obj_hi)
    0x8D, 0xB5, 0x7E,  // STA $7EB5
    0xC0, 0x07,        // CPY #$07       (displaced: W8 check)
    0xD0, 0x03,        // BNE +3         (skip JMP for non-W8)
    0x4C, 0x4C, 0x89,  // JMP $894C      (W8 path: save + overwrite)
    0x4C, 0x64, 0x89,  // JMP $8964      (non-W8 path: skip overwrite)
];

// Part B: PRG026 lookup routine.
// Hook point: replace `LDY $0727` with `JSR $B520` in LevelJct_BigQuestionBlock.
const BIG_Q_HOOK_OFFSET: usize = 0x349F9;
const BIG_Q_JSR: [u8; 3] = [0x20, 0x20, 0xB5];
// Lookup routine in PRG026 free space — offset from rom_data::FS_BIG_Q_LOOKUP
// (imported as BIG_Q_ROUTINE_OFFSET at the top of the file).
// Reads saved entry-point obj_ptr from $7EB4/$7EB5 (not ObjPtrOrig which
// gets overwritten by sub-area junctions). Falls back to World_Num for
// levels not in the table (W1/W2 levels don't use Big ? Blocks).
const BIG_Q_ROUTINE: [u8; 66] = [
    // LDA $7EB5 (saved entry obj_hi)
    0xAD, 0xB5, 0x7E,
    // LDX #10
    0xA2, 0x0A,
    // .loop: CMP $B541,X (obj_hi table)
    0xDD, 0x41, 0xB5,
    // BNE .next (+16)
    0xD0, 0x10,
    // PHA
    0x48,
    // LDA $7EB4 (saved entry obj_lo)
    0xAD, 0xB4, 0x7E,
    // CMP $B54C,X (obj_lo table)
    0xDD, 0x4C, 0xB5,
    // BNE .no_match (+6)
    0xD0, 0x06,
    // PLA
    0x68,
    // LDA $B557,X (room index table)
    0xBD, 0x57, 0xB5,
    // TAY
    0xA8,
    // RTS
    0x60,
    // .no_match: PLA
    0x68,
    // .next: DEX
    0xCA,
    // BPL .loop (-24)
    0x10, 0xE8,
    // fallback: LDY $0727
    0xAC, 0x27, 0x07,
    // RTS
    0x60,
    // obj_hi table (11 entries): 3-5,3-9,4-F2,5-2,5-5,6-3,6-9,6-10,7-F1,7-8,8-1
    0xCD, 0xC3, 0xD5, 0xC8, 0xCB, 0xCA, 0xCD, 0xCC, 0xD4, 0xC3, 0xC4,
    // obj_lo table (11 entries)
    0xEB, 0x8F, 0x08, 0xBE, 0x0A, 0x8E, 0x2D, 0xE8, 0xE4, 0x2D, 0x24,
    // room index table (11 entries): vanilla world indices (0-indexed)
    0x02, 0x02, 0x03, 0x04, 0x04, 0x05, 0x05, 0x05, 0x06, 0x06, 0x07,
];

/// Set starting lives for both Mario and Luigi (1–99).
pub fn set_starting_lives(rom: &mut Rom, lives: u8) {
    let clamped = lives.clamp(1, 99);
    rom.write_byte(STARTING_LIVES_OFFSET, clamped);
}

/// Write starting items into Mario's inventory via a trampoline in PRG031.
///
/// Replaces the 8-byte lives init at 0x308E0 with `JSR $E250` into a
/// routine that sets lives, does the intro skip, queues the seeded menu
/// music, AND writes up to 3 items to inventory ($7D80+). Must run AFTER
/// title_screen (which hooks the same region for intro skip) — this
/// trampoline incorporates that behavior.
pub fn write_starting_items(rom: &mut Rom, seed: u64, lives: u8, items: &[u8]) {
    let lives = lives.clamp(1, 99);
    let music = super::title_screen::pick_menu_music(seed);
    // Build trampoline: lives init + intro skip + menu music + item writes + RTS
    // CPU $E250 = file FS_STARTING_ITEMS
    let mut buf = Vec::with_capacity(33);
    buf.extend_from_slice(&[
        0xA9, lives,         // LDA #lives
        0x8D, 0x36, 0x07,    // STA $0736
        0x8D, 0x37, 0x07,    // STA $0737
        0xA9, 0x06,          // LDA #$06       (Title_State = IntroSkip)
        0x85, 0xDE,          // STA $DE
        0xA9, music,         // LDA #music
        0x8D, 0xF5, 0x04,    // STA $04F5      (queue menu music)
    ]);
    for (i, &item) in items.iter().take(3).enumerate() {
        buf.extend_from_slice(&[
            0xA9, item,                      // LDA #item
            0x8D, (0x80 + i as u8), 0x7D,    // STA $7D80+i
        ]);
    }
    buf.push(0x60); // RTS
    rom.write_range(FS_STARTING_ITEMS, &buf);

    // Patch lives init: JSR $E250 + NOP×5
    rom.write_range(LIVES_INIT_BASE, &[
        0x20, 0x50, 0xE2,                    // JSR $E250
        0xEA, 0xEA, 0xEA, 0xEA, 0xEA,       // NOP ×5
    ]);
}

/// Remove the W2 secret-path, W3 boat-path, and W4 pipe-shortcut rocks,
/// replacing each with a horizontal path tile.
pub fn remove_rocks(rom: &mut Rom) {
    for offset in [W2_SECRET_ROCK, W3_BOAT_ROCK, W4_PIPE_ROCK] {
        rom.write_byte(offset, 0x45);
    }
}

/// Patch Big ? Block bonus room selection to use level identity instead of World_Num.
///
/// Part A: Saves the entry-point obj_ptr to scratch RAM ($7EB4/$7EB5) at the end of
/// Map_PrepareLevel, before any sub-area junctions can overwrite Level_ObjPtrOrig.
///
/// Part B: Installs a lookup routine in PRG026 free space that reads the saved obj_ptr
/// and maps it to the correct per-world bonus room index. Falls back to World_Num for
/// levels not in the table (W1/W2 levels don't use Big ? Blocks).
pub fn fix_big_q_block_rooms(rom: &mut Rom) {
    // Part A: PRG030 save trampoline (saves $65/$66 before W8 overwrite)
    rom.write_range(BIG_Q_PRG030_HOOK, &BIG_Q_PRG030_JMP);
    rom.write_range(BIG_Q_PRG030_OFFSET, &BIG_Q_PRG030_ROUTINE);
    // Part B: PRG026 lookup routine
    rom.write_range(BIG_Q_HOOK_OFFSET, &BIG_Q_JSR);
    rom.write_range(BIG_Q_ROUTINE_OFFSET, &BIG_Q_ROUTINE);
}

// Card speed clear: one-of-each detection via XOR check.
//
// Card code lives in a bank mapped at CPU $A000. When 3 cards are collected,
// $BCD8 checks the 3rd slot and sets up a ~255-frame animation + 1-UP.
// We hook at $BCD8 (5 bytes: LDA $7D9E,Y; BEQ $BCFF) to jump to a
// trampoline in PRG031 dead space ($FFE0, always mapped at $E000-$FFFF).
//
// The trampoline executes the displaced 3rd-card check, then XORs all 3
// card values: card[0] ^ card[1] ^ card[2]. For one-of-each (values 1,2,3
// in any order): 1^2^3 = 0. All other combos produce non-zero.
// If zero → JMP $BD5A (clear cards + RTS, no animation).
// Otherwise → execute displaced state setup and JMP $BCE1 (normal flow).
//
// Bank 9 map-screen tables are also patched as belt-and-suspenders:
// lives=0, cutscene flag=0, match indicator=0 for bitmask index 7.

// Hook point: $BCD8 in card bank at $A000 (file 0x05CE8)
// Original 5 bytes: LDA $7D9E,Y (B9 9E 7D); BEQ $BCFF (F0 22)
const CARD_HOOK: usize = 0x05CE8;

// Trampoline in PRG031 dead space — offset from rom_data::FS_CARD_CLEAR
// (imported as CARD_TRAMPOLINE at the top of the file).
// Overwrites 3 unused $FF bytes + "SUPER MARIO 3" string + dead padding.
// 26 bytes available ($FFE0-$FFF9), routine uses 26.

// Bank 9 map-screen tables (belt-and-suspenders)
const CARD_LIVES_AWARD: usize = 0x12017; // $A000[7]
const CARD_CUTSCENE_FLAG: usize = 0x1201F; // $A008[7]
const CARD_MATCH_INDICATOR: usize = 0x12027; // $A010[7]
const CARD_CLEAR_GUARD: usize = 0x12090; // BEQ at $A080

/// Patch one-of-each card collection to skip the animation entirely.
/// Cards are cleared instantly and the level ends as if < 3 cards — a speed bonus.
/// Other mixed combos and matching triples still play the normal animation.
pub fn card_speed_clear(rom: &mut Rom) {
    // Hook: replace 5 bytes at $BCD8 with JMP $FFE0; NOP; NOP
    rom.write_range(CARD_HOOK, &[
        0x4C, 0xE0, 0xFF, // JMP $FFE0
        0xEA, 0xEA,        // NOP; NOP (pad to 5 bytes)
    ]);

    // Trampoline at $FFE0 (PRG031, always mapped, 24 bytes):
    //
    // $FFE0: LDA $7D9E,Y      ; displaced: load 3rd card
    // $FFE3: BNE +3            ; if not empty → check cards
    // $FFE5: JMP $BCFF          ; displaced: 3rd card empty → card placement
    // $FFE8: EOR $7D9C,Y       ; A has card[2], XOR card[0]
    // $FFEB: EOR $7D9D,Y       ; XOR card[1]
    // $FFEE: BNE +3            ; non-zero = not one-of-each
    // $FFF0: JMP $BD5A          ; one-of-each: clear cards, RTS
    // $FFF3: LDA #$04           ; displaced: animation state = 4
    // $FFF5: STA $9A,X          ; displaced: store state
    // $FFF7: JMP $BCE1          ; return to normal animation flow
    rom.write_range(CARD_TRAMPOLINE, &[
        0xB9, 0x9E, 0x7D, // LDA $7D9E,Y (displaced)
        0xD0, 0x03,        // BNE +3 (card present)
        0x4C, 0xFF, 0xBC, // JMP $BCFF (displaced: empty → placement)
        0x59, 0x9C, 0x7D, // EOR $7D9C,Y
        0x59, 0x9D, 0x7D, // EOR $7D9D,Y
        0xD0, 0x03,        // BNE +3 (not one-of-each)
        0x4C, 0x5A, 0xBD, // JMP $BD5A (clear cards)
        0xA9, 0x04,        // LDA #$04 (displaced)
        0x95, 0x9A,        // STA $9A,X (displaced)
        0x4C, 0xE1, 0xBC, // JMP $BCE1 (continue normal)
    ]);

    // Bank 9 map-screen patches (belt-and-suspenders for map cutscene)
    rom.write_byte(CARD_LIVES_AWARD, 0x00);
    rom.write_byte(CARD_CUTSCENE_FLAG, 0x00);
    rom.write_byte(CARD_MATCH_INDICATOR, 0x00);
    rom.write_range(CARD_CLEAR_GUARD, &[0xEA, 0xEA]);
}

// Make the hammer item also break fortress lock tiles on the overworld map.
//
// The vanilla hammer routine at PRG026 (file 0x346D5, CPU $A6C5) uses a
// 7-byte range check: `SEC; SBC #$51; CMP #$02; BCC .found` which only
// matches rock tiles $51–$52. We replace this with a JSR to a table-driven
// subroutine in PRG026 free space that checks 5 tile IDs (2 rocks + 3 locks).
//
// Patch site 1 — Range check (file 0x346D5, 7 bytes):
//   `SEC; SBC #$51; CMP #$02; BCC .found` →
//   `JSR HammerCheckTile; BCC .found; NOP; NOP`
//
// Patch site 2 — Replacement tile load (file 0x346E9, 3 bytes):
//   `LDA $A6B1,X` → `LDA $7EB6` (load from scratch RAM set by subroutine)
//
// New subroutine at FS_HAMMER_LOCKS (0x3557F, CPU $B56F), 47 bytes:
//   Table-driven check of breakable tiles, stores replacement tile in $7EB6,
//   saves/restores X via $7EB7, returns carry clear if breakable.
//
// Water gap locks (0x9D → 0xB3) are intentionally excluded — bridge tiles
// need more testing.

/// File offset of the 7-byte range check in the hammer routine ($A6C5).
const HAMMER_RANGE_CHECK: usize = 0x346D5;
/// File offset of `LDA $A6B1,X` (replacement tile load) at CPU $A6D8.
const HAMMER_REPLACE_LOAD: usize = 0x346E8;
/// CPU address of the subroutine: $A000 + (0x3557F - 0x34010) = $B56F.
const HAMMER_LOCKS_SUB_CPU: u16 = 0xB56F;

pub fn hammer_breaks_tiles(rom: &mut Rom, locks: bool, bridges: bool) {
    // Build tables dynamically based on which flags are set.
    // Always include rocks (2 entries), then conditionally add locks (3) and bridge (1).
    let mut breakable: Vec<u8> = vec![0x51, 0x52]; // rocks
    let mut replace:   Vec<u8> = vec![0x45, 0x46];
    let mut tilefix:   Vec<u8> = vec![0x00, 0x01];

    if locks {
        breakable.extend_from_slice(&[0x54, 0x56, 0xE4]);
        replace.extend_from_slice(&[0x46, 0x45, 0xDA]);
        tilefix.extend_from_slice(&[0x01, 0x00, 0x00]);
    }
    if bridges {
        breakable.push(0x9D);
        replace.push(0xB3);
        tilefix.push(0x00);
    }

    let table_len = breakable.len();
    let ldx_imm = (table_len - 1) as u8;

    // Table CPU addresses start right after the 32-byte code block.
    let tbl_base = HAMMER_LOCKS_SUB_CPU + 32;
    let breakable_cpu = tbl_base;
    let replace_cpu = tbl_base + table_len as u16;
    let tilefix_cpu = tbl_base + (table_len * 2) as u16;

    // Patch site 1: JSR HammerCheckTile; BCC .found; NOP; NOP
    //
    // Original 7 bytes at 0x346D5 (CPU $A6C5):
    //   38        SEC
    //   E9 51     SBC #$51
    //   C9 02     CMP #$02
    //   90 07     BCC .found (+$07) → $A6D2 (file 0x346E2)
    //
    // .found ($A6D2) does: STX $01; LSR $01; PHA; TAX; LDA $A6B1,X
    // — the STX $01 needs the *original* X, so the subroutine preserves it.
    //
    // New BCC at $A6C8 (0x346D8) targeting $A6D2: offset = $A6D2 - $A6CA = 0x08.
    // Only 6 bytes — must preserve DEC $00 (C6 00) at 0x346DB so the outer
    // loop that checks 4 adjacent tiles still works on no-match fall-through.
    let lo = (HAMMER_LOCKS_SUB_CPU & 0xFF) as u8;
    let hi = (HAMMER_LOCKS_SUB_CPU >> 8) as u8;
    rom.write_range(HAMMER_RANGE_CHECK, &[
        0x20, lo, hi,   // JSR HammerCheckTile
        0x90, 0x08,     // BCC .found (targets $A6D2 / file 0x346E2)
        0xEA,           // NOP (1 byte padding)
    ]);

    // Patch site 2: LDA $7EB6 (absolute) instead of LDA $A6B1,X (indexed)
    // Original at 0x346E8 (CPU $A6D8): BD B1 A6 (LDA $A6B1,X)
    // New: AD B6 7E (LDA $7EB6)
    rom.write_range(HAMMER_REPLACE_LOAD, &[0xAD, 0xB6, 0x7E]);

    // Subroutine + tables at FS_HAMMER_LOCKS (CPU $B56F), up to 50 bytes.
    //
    // Saves/restores X via $7EB7 so the caller's STX $01 at .found sees the
    // original X register. Returns carry clear with A = tilefix_map (animation
    // index 0 or 1), $7EB6 = replacement tile.
    //
    // Code: 32 bytes, tables: 3 × table_len bytes.
    #[rustfmt::skip]
    let mut subroutine: Vec<u8> = vec![
        // HammerCheckTile:
        0x8E, 0xB7, 0x7E,                                      // STX $7EB7         ; save original X
        0xA2, ldx_imm,                                          // LDX #N            ; N entries (index N..0)
        // .loop:
        0xDD, breakable_cpu as u8, (breakable_cpu >> 8) as u8,  // CMP breakable,X
        0xF0, 0x08,                                             // BEQ .found (+8)
        0xCA,                                                   // DEX
        0x10, 0xF8,                                             // BPL .loop (-8)
        0xAE, 0xB7, 0x7E,                                      // LDX $7EB7         ; restore X (not found)
        0x38,                                                   // SEC
        0x60,                                                   // RTS
        // .found:
        0xBD, replace_cpu as u8, (replace_cpu >> 8) as u8,      // LDA replace,X
        0x8D, 0xB6, 0x7E,                                      // STA $7EB6         ; scratch RAM for replacement
        0xBD, tilefix_cpu as u8, (tilefix_cpu >> 8) as u8,      // LDA tilefix,X    ; tilefix_map (animation idx)
        0xAE, 0xB7, 0x7E,                                      // LDX $7EB7         ; restore original X
        0x18,                                                   // CLC               ; found
        0x60,                                                   // RTS
    ];
    subroutine.extend_from_slice(&breakable);
    subroutine.extend_from_slice(&replace);
    subroutine.extend_from_slice(&tilefix);
    rom.write_range(FS_HAMMER_LOCKS, &subroutine);
}

/// Remove N-card (N-Spade) panels from the overworld map.
///
/// Patches the map-screen code so N-Spade tiles never appear.
/// Original IPS: 3 bytes at 0x016C90 → LDA #$07; NOP.
const N_CARD_OFFSET: usize = 0x016C90;

pub fn remove_n_cards(rom: &mut Rom) {
    rom.write_range(N_CARD_OFFSET, &[0xA9, 0x07, 0xEA]);
}

// Canoe softlock fix — based on "SMB3 - Canoe Softlock Fixes (Open World
// compatible).ips". Two hooks plus two free-space subroutines.

// Hook at PRG010 CPU $C6EA → JSR FS_CANOE_RESPAWN (5 bytes incl. NOP NOP).
const CANOE_RESPAWN_HOOK: usize = 0x146FA;
// Boundary check adjustment at PRG010 CPU $CF13 (2 bytes).
const CANOE_BOUNDARY_PATCH: usize = 0x14F23;
// Hook at PRG011 CPU $A22F → JSR FS_CANOE_BACKUP (5 bytes incl. NOP NOP).
const CANOE_BACKUP_HOOK: usize = 0x1623F;

// Record 3: subroutine in PRG010 free space (FS_CANOE_RESPAWN).
// Saves player map position as death respawn point when entering via canoe ($4B).
#[rustfmt::skip]
const CANOE_RESPAWN_ROUTINE: [u8; 35] = [
    0x20, 0xFE, 0xD1, // JSR $D1FE  (original routine)
    0xC9, 0x4B,       // CMP #$4B   (canoe state?)
    0xD0, 0x1B,       // BNE +27    (skip if not canoe)
    0xB5, 0x75,       // LDA $75,X  (map obj Y)
    0x9D, 0x7E, 0x79, // STA $797E,X (death respawn Y)
    0xB5, 0x77,       // LDA $77,X  (map obj X hi)
    0x9D, 0x80, 0x79, // STA $7980,X (death respawn X hi)
    0xB5, 0x79,       // LDA $79,X  (map obj X lo)
    0x9D, 0x82, 0x79, // STA $7982,X (death respawn X lo)
    0xA5, 0xFD,       // LDA $FD    (Map_Scroll_X)
    0x9D, 0x86, 0x79, // STA $7986,X (death respawn scroll X)
    0xA5, 0x12,       // LDA $12    (Map_Scroll_XHi)
    0x9D, 0x88, 0x79, // STA $7988,X (death respawn scroll XHi)
    0xA5, 0xE5,       // LDA $E5    (reload game state)
    0x60,             // RTS
];

// Record 5: backup/restore subroutines in PRG011 free space (FS_CANOE_BACKUP).
// Part A ($BCF0): backs up 3 map data values before canoe overwrites them.
// Part B ($BD0C): restores backed-up values when canoe interaction ends.
#[rustfmt::skip]
const CANOE_BACKUP_ROUTINE: [u8; 66] = [
    // Part A: backup on canoe load
    0xC9, 0x10,       // CMP #$10   (canoe obj ID)
    0xD0, 0x12,       // BNE +18    (skip if not canoe)
    0xB9, 0xEB, 0x7E, // LDA $7EEB,Y
    0x8D, 0xF3, 0x7A, // STA $7AF3
    0xB9, 0x07, 0x7F, // LDA $7F07,Y
    0x8D, 0xF1, 0x7A, // STA $7AF1
    0xB9, 0xF9, 0x7E, // LDA $7EF9,Y
    0x8D, 0xF2, 0x7A, // STA $7AF2
    0xB1, 0x06,       // LDA ($06),Y (original instruction)
    0x99, 0x56, 0x79, // STA $7956,Y (original instruction)
    0x60,             // RTS
    // Part B: restore on canoe cleanup
    0xA0, 0x0D,       // LDY #$0D   (iterate backwards)
    0xB9, 0x15, 0x7F, // LDA $7F15,Y (map obj ID)
    0xC9, 0x10,       // CMP #$10   (canoe?)
    0xD0, 0x14,       // BNE +20    (skip if not canoe)
    0xAD, 0xF3, 0x7A, // LDA $7AF3
    0x99, 0xEB, 0x7E, // STA $7EEB,Y (restore)
    0xAD, 0xF1, 0x7A, // LDA $7AF1
    0x99, 0x07, 0x7F, // STA $7F07,Y (restore)
    0xAD, 0xF2, 0x7A, // LDA $7AF2
    0x99, 0xF9, 0x7E, // STA $7EF9,Y (restore)
    0xA0, 0x01,       // LDY #$01   (break loop)
    0x88,             // DEY
    0xD0, 0xE2,       // BNE -30    (loop)
    0xA9, 0x00,       // LDA #$00
    0x8D, 0x00, 0x05, // STA $0500  (clear game state flag)
    0x60,             // RTS
];

/// Fix W3 canoe softlocks: save death respawn position when entering via canoe,
/// and backup/restore the map tile data the canoe overwrites.
///
/// Without this, levels placed on W3 island tiles (freed by spade game removal)
/// can softlock if the player dies — the respawn position is invalid and the map
/// data under the canoe is permanently corrupted.
///
/// Based on "SMB3 - Canoe Softlock Fixes (Open World compatible).ips".
pub fn fix_canoe_softlock(rom: &mut Rom) {
    // Record 1: Hook at PRG010 CPU $C6EA → JSR $BD0C (canoe cleanup)
    rom.write_range(CANOE_RESPAWN_HOOK, &[0x20, 0x0C, 0xBD, 0xEA, 0xEA]);

    // Record 2: Boundary check adjustment at PRG010 CPU $CF13
    rom.write_range(CANOE_BOUNDARY_PATCH, &[0xE0, 0xDD]);

    // Record 3: respawn-save subroutine
    rom.write_range(FS_CANOE_RESPAWN, &CANOE_RESPAWN_ROUTINE);

    // Record 4: Hook at PRG011 CPU $A22F → JSR $BCF0 (canoe backup)
    rom.write_range(CANOE_BACKUP_HOOK, &[0x20, 0xF0, 0xBC, 0xEA, 0xEA]);

    // Record 5: backup/restore subroutines
    rom.write_range(FS_CANOE_BACKUP, &CANOE_BACKUP_ROUTINE);
}

/// Apply deterministic layout fixes for the 9 beta stages.
///
/// The vanilla ROM has broken sub-area pointers, wrong start positions, and
/// misaligned tile commands in the beta level data. These 44 byte patches
/// repair the layouts so the stages are playable.
pub fn fix_beta_stages(rom: &mut Rom) {
    for &(offset, value) in BETA_PATCHES {
        rom.write_byte(offset, value);
    }
}

/// Replace W3 drawbridge tiles with normal path tiles and NOP the toggle code.
pub fn fix_w3_drawbridges(rom: &mut Rom) {
    for (offset, tile) in W3_DRAWBRIDGE_TILES {
        rom.write_byte(offset, tile);
    }
    // NOP out the toggle code (LDA $07BB; EOR #$01; STA $07BB)
    rom.write_range(W3_TOGGLE_OFFSET, &[0xEA; W3_TOGGLE_LEN]);
}

// ---------------------------------------------------------------------------
// MaCobra patches — always-on bundle
// Consts in this section feed apply_macobra_patches() at the bottom of the
// file; that bundle ships with every randomized ROM. Opt-in MaCobra patches
// (gated by individual options) live in their own section further down.
// ---------------------------------------------------------------------------

// Forced hammer bro walk-over: NOPs `STA $053C,Y; RTS` in the map sprite
// collision check (PRG011, CPU $AEF6). Prevents hammer bros from walking
// onto the player to force a fight — player-initiated encounters still work.
const FORCED_BRO_FIGHT: usize = 0x16F06;

// Bowser upward kill glitch: changes a BNE ($D0) to BCC ($90) in PRG001
// (CPU $BEC1) to fix a glitch where Bowser can be killed from below.
const BOWSER_UPWARD_KILL: usize = 0x3ED1;

// Fire bro bump detection: adjusts collision parameters in PRG004 to add
// proper bump detection and make fire bros slightly more fair.
const FIRE_BRO_BUMP_A: usize = 0x8911;
const FIRE_BRO_BUMP_B: usize = 0x88C1;

// Hammer suit slope slide: allows hammer suit to slide on slopes (PRG000).
const HAMMER_SUIT_SLIDE: usize = 0x3F6;

// Vertical pipe clip fix: prevents an inter-level softlock caused by
// clipping through vertical pipes between areas (PRG029).
const PIPE_CLIP_FIX: usize = 0x3B5B1;

// Move after orb grab: NOPs `STY/STA $7CF4` in PRG003 (CPU $A8ED, $A903) so
// the player-input-lock flag isn't set when grabbing the fortress magic ball.
// Two 3-byte absolute stores → NOPs.
const MOVE_AFTER_ORB_STY: usize = 0x068FD; // STY $7CF4 at CPU $A8ED
const MOVE_AFTER_ORB_STA: usize = 0x06913; // STA $7CF4 at CPU $A903

// Tail attack while swimming (PRG008) — extends the swim subroutine so
// Raccoon/Tanooki Mario can tail-swipe enemies underwater. Two 5-byte hooks
// inside the vanilla swim routine plus a 285-byte replacement block that
// covers $A9D4–$AAF0.
const TAIL_SWIM_HOOK_A: usize = 0x01097B;
const TAIL_SWIM_HOOK_A_BYTES: [u8; 5] = [0xD5, 0xAA, 0xDE, 0xA9, 0xD2];
const TAIL_SWIM_HOOK_B: usize = 0x010989;
const TAIL_SWIM_HOOK_B_BYTES: [u8; 5] = [0xE5, 0xAA, 0x30, 0xAA, 0xE2];
const TAIL_SWIM_ROUTINE_OFFSET: usize = 0x0109E4;
#[rustfmt::skip]
const TAIL_SWIM_ROUTINE: [u8; 285] = [
    0x28, 0xAC, 0x20, 0x36, 0xB0, 0x20, 0xC6, 0xB0, 0x60, 0x60, 0x20, 0x44,
    0xAB, 0x20, 0x28, 0xAC, 0xAD, 0xA4, 0x06, 0xD0, 0x37, 0xA5, 0xD8, 0xF0,
    0x10, 0xAD, 0x8A, 0x05, 0x4A, 0xB0, 0x0A, 0xA9, 0x00, 0x8D, 0x13, 0x05,
    0xA0, 0x01, 0x4C, 0x1B, 0xAA, 0xAD, 0x13, 0x05, 0xD0, 0x15, 0x85, 0xBD,
    0xA5, 0x17, 0x29, 0x03, 0xF0, 0x0D, 0xAD, 0xF1, 0x04, 0x09, 0x80, 0x8D,
    0xF1, 0x04, 0xA9, 0x1F, 0x8D, 0x13, 0x05, 0x4A, 0x4A, 0x4A, 0xA8, 0xB9,
    0x69, 0xA0, 0x85, 0xEE, 0x60, 0x03, 0x07, 0x24, 0x12, 0x21, 0x02, 0x02,
    0x02, 0x01, 0x00, 0x01, 0x02, 0x02, 0x10, 0xF0, 0xA2, 0xFF, 0xA5, 0x17,
    0x29, 0x0C, 0xF0, 0x26, 0x85, 0xD8, 0x4A, 0x4A, 0x4A, 0xAA, 0xBD, 0x2E,
    0xAA, 0x10, 0x07, 0xAC, 0x44, 0x05, 0x10, 0x02, 0xA9, 0x00, 0xA4, 0x17,
    0x10, 0x01, 0x0A, 0xC9, 0xE1, 0x90, 0x06, 0xA4, 0xD8, 0xD0, 0x02, 0xA9,
    0xE0, 0x85, 0xCF, 0x4C, 0x6B, 0xAA, 0xA4, 0xCF, 0xF0, 0x09, 0xC8, 0xA5,
    0xCF, 0x30, 0x02, 0x88, 0x88, 0x84, 0xCF, 0xA5, 0x17, 0x29, 0x03, 0xF0,
    0x10, 0x4A, 0xA8, 0xB9, 0x2E, 0xAA, 0xA4, 0x17, 0x10, 0x01, 0x0A, 0x85,
    0xBD, 0xA2, 0x02, 0xD0, 0x18, 0xA4, 0xBD, 0xF0, 0x0C, 0xC8, 0xA5, 0xBD,
    0x30, 0x02, 0x88, 0x88, 0x84, 0xBD, 0x4C, 0x99, 0xAA, 0xA5, 0xD8, 0xD0,
    0x04, 0xA9, 0x15, 0xD0, 0x36, 0x8A, 0x30, 0x29, 0xA5, 0x15, 0x4A, 0x4A,
    0xA0, 0x00, 0x24, 0x17, 0x30, 0x02, 0x4A, 0xC8, 0x29, 0x07, 0xA8, 0xD0,
    0x0F, 0xA5, 0x15, 0x39, 0x21, 0xAA, 0xD0, 0x08, 0xAD, 0xF1, 0x04, 0x09,
    0x04, 0x8D, 0xF1, 0x04, 0xBD, 0x23, 0xAA, 0x18, 0x79, 0x26, 0xAA, 0xD0,
    0x0A, 0xA0, 0x1F, 0xA5, 0x15, 0x29, 0x08, 0xF0, 0x01, 0xC8, 0x98, 0x85,
    0xEE, 0x60, 0x20, 0xAE, 0xAF, 0x20, 0x44, 0xAB, 0x20, 0x28, 0xAC, 0x20,
    0x36, 0xB0, 0x20, 0xC6, 0xB0, 0x60, 0x20, 0xAE, 0xAF, 0x20, 0x02, 0xAC,
    0x20, 0x2F, 0xAD, 0x20, 0x7F, 0xAD, 0x20, 0xC6, 0xB0,
];

// Hot Foot and Chain Chomp tail vulnerability — three byte flips in
// enemy-collision tables (PRG002 / PRG004) that let the tail/spin defeat
// these enemies. Authored by MaCobra52.
const HOTFOOT_TAIL_A: usize = 0x0413C;
const HOTFOOT_TAIL_B: usize = 0x04151;
const HOTFOOT_TAIL_C: usize = 0x0814D;

// ---------------------------------------------------------------------------
// MaCobra patches — opt-in features
// Each apply_* below is gated by an individual option in randomizer.rs;
// none of these ship unless the corresponding flag is enabled.
// ---------------------------------------------------------------------------

// Early Sun (by MaCobra52) — drops the Angry Sun's pre-attack threshold
// from 5 to 0 so it begins swooping immediately on spawn instead of after
// the vanilla delay. Single byte: PRG005 CPU $AD71 = file 0xAD81, operand
// of `CMP #$05` becomes `CMP #$00`. Source:
// https://github.com/macobra52/smb3-hacks/blob/main/SMB3%20IPS/SMB3%20-%20Early%20Sun.ips
const EARLY_SUN_OFFSET: usize = 0x0AD81;

/// Apply MaCobra52's "Early Sun" patch — the Angry Sun starts attacking
/// without its vanilla pre-attack delay.
pub fn apply_early_sun(rom: &mut Rom) {
    rom.write_byte(EARLY_SUN_OFFSET, 0x00);
}

// Japanese damage system (by MaCobra52) — NOPs the `JMP $DA15` at file
// 0x019F9 so the vanilla "demote power-up by one tier" subroutine is
// skipped. Control falls through into the path that drops the player
// straight to Small Mario from any power-up state, matching the Famicom
// SMB3 damage model. Source:
// https://github.com/macobra52/smb3-hacks/blob/main/SMB3%20IPS/SMB3%20-%20Japanese%20damage%20system%20(fixed).ips
const JP_DAMAGE_OFFSET: usize = 0x019F9;
const JP_DAMAGE_BYTES: [u8; 3] = [0xEA, 0xEA, 0xEA];

/// Apply MaCobra52's "Japanese damage system (fixed)" patch — taking damage
/// from any power-up tier (Super, Fire, Raccoon, Frog, Tanooki, Hammer)
/// drops the player straight to Small Mario instead of demoting one tier
/// at a time.
pub fn apply_japanese_damage(rom: &mut Rom) {
    rom.write_range(JP_DAMAGE_OFFSET, &JP_DAMAGE_BYTES);
}

// Infinite use Mushroom Houses (by MaCobra52) — 5-byte rewrite at file
// 0x0169E5 (PRG011, CPU $A9D5) that drops the TOADHOUSE tile ($50) out of
// the "remove after use" tile list. The remaining list entries shift one
// position earlier and two NOPs are appended so the reader stops there.
// Effect: toad houses no longer disappear after entering them, so the
// reward can be collected repeatedly. Source:
// https://github.com/macobra52/smb3-hacks/blob/main/SMB3%20IPS/SMB3%20-%20Infinite%20use%20Mushroom%20Houses.ips
const INF_MUSHROOM_HOUSES_OFFSET: usize = 0x0169E5;
const INF_MUSHROOM_HOUSES_BYTES: [u8; 5] = [0xE8, 0xE6, 0xBD, 0xEA, 0xEA];

/// Apply MaCobra52's "Infinite use Mushroom Houses" patch — toad houses
/// stay on the map after entering and can be visited any number of times.
pub fn apply_infinite_mushroom_houses(rom: &mut Rom) {
    rom.write_range(INF_MUSHROOM_HOUSES_OFFSET, &INF_MUSHROOM_HOUSES_BYTES);
}

// Fast Mushroom House (by MaCobra52) — combination of two single-byte
// timer tweaks:
//   * "Move Sooner in Mushroom House (Instant)" — file 0x005234, the
//     post-entry input-lock timer (0xFF → 0x00), so the player can move
//     immediately on the chest-select screen instead of waiting for the
//     vanilla intro animation.
//   * "Exit Mushroom House Faster" — file 0x001E3F, the exit-transition
//     timer (0xFF → 0x5F), so closing the house and returning to the map
//     is roughly 60% shorter.
// Sources:
// https://github.com/macobra52/smb3-hacks/blob/main/SMB3%20IPS/SMB3%20-%20Move%20Sooner%20in%20Mushroom%20House%20(Instant).ips
// https://github.com/macobra52/smb3-hacks/blob/main/SMB3%20IPS/SMB3%20-%20Exit%20Mushroom%20House%20Faster.ips
const FAST_MUSH_MOVE_OFFSET: usize = 0x005234;
const FAST_MUSH_EXIT_OFFSET: usize = 0x001E3F;

/// Apply MaCobra52's "Fast Mushroom House" — combines the "Move Sooner"
/// and "Exit Faster" timer tweaks: skip the entry-input-lock and shorten
/// the exit transition.
pub fn apply_fast_mushroom_house(rom: &mut Rom) {
    rom.write_byte(FAST_MUSH_MOVE_OFFSET, 0x00);
    rom.write_byte(FAST_MUSH_EXIT_OFFSET, 0x5F);
}

// Faster Tail Speed (by MaCobra52) — bundles three writes:
//
//   1. Reduced tail slowdown. File 0x110A6 ← 0x29. Shortens the
//      post-swipe slowdown frames so the tail attack is less punishing
//      to use mid-run.
//   2. Slightly reduced raccoon/Tanooki flight time. File 0x10CAA
//      ← 0x78. The faster tail makes building meter cheaper, which
//      otherwise opens a known cheese skip in 8-1 by flying over a
//      large section of the level; trimming flight duration cancels
//      the cheese without removing flight outright.
//   3. Lower the 7-6 fly-strat wall. File 0x1F36A ← {0x42, 0x14, 0xBD}
//      (3-byte tile payload). The shortened flight time from (2) would
//      otherwise leave the intended 7-6 fly route unreachable; this
//      retunes the wall height so the strat still clears at the new
//      flight duration.
//
// Source: MaCobra52 (no public IPS link).
const FASTER_TAIL_SLOWDOWN_OFFSET: usize = 0x110A6;
const FASTER_TAIL_FLIGHT_OFFSET: usize = 0x10CAA;
const FASTER_TAIL_W76_WALL_OFFSET: usize = 0x1F36A;
const FASTER_TAIL_W76_WALL_BYTES: [u8; 3] = [0x42, 0x14, 0xBD];

/// Apply MaCobra52's "Faster Tail Speed" — reduces tail-swipe slowdown,
/// trims raccoon/Tanooki flight time to neutralize the 8-1 cheese the
/// faster tail enables, and lowers the 7-6 wall so the intended fly
/// strat still clears at the new flight duration.
pub fn apply_faster_tail_speed(rom: &mut Rom) {
    rom.write_byte(FASTER_TAIL_SLOWDOWN_OFFSET, 0x29);
    rom.write_byte(FASTER_TAIL_FLIGHT_OFFSET, 0x78);
    rom.write_range(FASTER_TAIL_W76_WALL_OFFSET, &FASTER_TAIL_W76_WALL_BYTES);
}

// No Game Over Penalty (by MaCobra52) — four writes verified byte-for-byte
// against the upstream IPS at `SMB3 - No Game Over Penalty.ips` in this
// repo. After a Game Over the player keeps their reserve inventory,
// world map state, and card progress instead of having them wiped.
//
//   1. File 0x016A0F: 2 bytes — JSR operand redirected to the new
//      subroutine at CPU $BD40.
//   2. File 0x017A82: 8 bytes — `JSR $BD46` + 5 NOPs replacing the
//      vanilla "reset on Game Over" call sequence.
//   3. File 0x017D50: 26 bytes — new subroutine in PRG011 free space
//      at $BD40-$BD59 that decides which state is allowed to reset
//      (returns 0 to skip the wipe, 1 to allow it) based on the
//      current map tile being checked against $50 / $E0 / $E8.
//   4. File 0x03D314: 3 NOPs killing the vanilla decrement/clear
//      instruction that ran unconditionally on Game Over.
const NGO_HOOK_A_OFFSET: usize = 0x016A0F;
const NGO_HOOK_A_BYTES: [u8; 2] = [0x40, 0xBD];
const NGO_HOOK_B_OFFSET: usize = 0x017A82;
const NGO_HOOK_B_BYTES: [u8; 8] = [0x20, 0x46, 0xBD, 0xEA, 0xEA, 0xEA, 0xEA, 0xEA];
const NGO_ROUTINE_OFFSET: usize = 0x017D50;
#[rustfmt::skip]
const NGO_ROUTINE: [u8; 26] = [
    0x20, 0xFE, 0xD1, 0x85, 0xE6, 0x60, 0xA5, 0xE6, 0xC9, 0x50, 0xF0, 0x0B,
    0xC9, 0xE0, 0xF0, 0x07, 0xC9, 0xE8, 0xF0, 0x03, 0xA9, 0x00, 0x60, 0xA9,
    0x01, 0x60,
];
const NGO_NOP_OFFSET: usize = 0x03D314;
const NGO_NOP_BYTES: [u8; 3] = [0xEA, 0xEA, 0xEA];

/// Apply MaCobra52's "No Game Over Penalty" patch — Game Overs no longer
/// wipe the player's reserve inventory, world map progress, or card
/// state.
pub fn apply_no_game_over_penalty(rom: &mut Rom) {
    rom.write_range(NGO_HOOK_A_OFFSET, &NGO_HOOK_A_BYTES);
    rom.write_range(NGO_HOOK_B_OFFSET, &NGO_HOOK_B_BYTES);
    rom.write_range(NGO_ROUTINE_OFFSET, &NGO_ROUTINE);
    rom.write_range(NGO_NOP_OFFSET, &NGO_NOP_BYTES);
}

/// Apply MaCobra's always-on bugfixes and fairness patches.
pub fn apply_macobra_patches(rom: &mut Rom) {
    // Prevent forced hammer bro fights (4 NOPs)
    rom.write_range(FORCED_BRO_FIGHT, &[0xEA; 4]);

    // Fix Bowser upward kill glitch
    rom.write_byte(BOWSER_UPWARD_KILL, 0x90);

    // Add proper fire bro bump detection and make them more fair
    rom.write_range(FIRE_BRO_BUMP_A, &[0x13, 0xAB]);
    rom.write_byte(FIRE_BRO_BUMP_B, 0x40);

    // Enable hammer suit to slide on slopes
    rom.write_byte(HAMMER_SUIT_SLIDE, 0x00);

    // Fix inter-level vertical pipe clip softlock
    rom.write_byte(PIPE_CLIP_FIX, 0x00);

    // Allow Mario to keep moving after grabbing the fortress orb / magic ball.
    rom.write_range(MOVE_AFTER_ORB_STY, &[0xEA; 3]);
    rom.write_range(MOVE_AFTER_ORB_STA, &[0xEA; 3]);

    // Tail attack while swimming (Raccoon/Tanooki tail-swipes underwater).
    rom.write_range(TAIL_SWIM_HOOK_A, &TAIL_SWIM_HOOK_A_BYTES);
    rom.write_range(TAIL_SWIM_HOOK_B, &TAIL_SWIM_HOOK_B_BYTES);
    rom.write_range(TAIL_SWIM_ROUTINE_OFFSET, &TAIL_SWIM_ROUTINE);

    // Make Hot Foot and Chain Chomp tail-vulnerable.
    rom.write_byte(HOTFOOT_TAIL_A, 0x00);
    rom.write_byte(HOTFOOT_TAIL_B, 0x00);
    rom.write_byte(HOTFOOT_TAIL_C, 0x25);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::Rom;

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        data[STARTING_LIVES_OFFSET] = 0x04;
        Rom::from_bytes_lax(&data, true).unwrap()
    }

    #[test]
    fn test_starting_lives() {
        let mut rom = make_test_rom();
        assert_eq!(rom.read_byte(STARTING_LIVES_OFFSET), 0x04);
        set_starting_lives(&mut rom, 99);
        assert_eq!(rom.read_byte(STARTING_LIVES_OFFSET), 99);
    }

    #[test]
    fn test_starting_lives_clamped() {
        let mut rom = make_test_rom();
        set_starting_lives(&mut rom, 255);
        assert_eq!(rom.read_byte(STARTING_LIVES_OFFSET), 99);
        set_starting_lives(&mut rom, 0);
        assert_eq!(rom.read_byte(STARTING_LIVES_OFFSET), 1);
    }

    #[test]
    fn test_fix_big_q_block_rooms() {
        let mut rom = make_test_rom();
        // Place original bytes at hook points
        rom.write_range(BIG_Q_HOOK_OFFSET, &[0xAC, 0x27, 0x07]);
        rom.write_range(BIG_Q_PRG030_HOOK, &[0xC0, 0x07, 0xD0, 0x18]);

        fix_big_q_block_rooms(&mut rom);

        // Part A: PRG030 save trampoline
        assert_eq!(rom.read_range(BIG_Q_PRG030_HOOK, 4), &BIG_Q_PRG030_JMP);
        assert_eq!(
            rom.read_range(BIG_Q_PRG030_OFFSET, BIG_Q_PRG030_ROUTINE.len()),
            &BIG_Q_PRG030_ROUTINE
        );
        // Spot-check: trampoline reads $65 (zp obj_lo)
        assert_eq!(rom.read_byte(BIG_Q_PRG030_OFFSET), 0xA5);
        assert_eq!(rom.read_byte(BIG_Q_PRG030_OFFSET + 1), 0x65);

        // Part B: PRG026 lookup routine
        assert_eq!(rom.read_range(BIG_Q_HOOK_OFFSET, 3), &BIG_Q_JSR);
        assert_eq!(
            rom.read_range(BIG_Q_ROUTINE_OFFSET, BIG_Q_ROUTINE.len()),
            &BIG_Q_ROUTINE
        );
        // Spot-check: routine reads $7EB5 (not $7EBC)
        assert_eq!(rom.read_byte(BIG_Q_ROUTINE_OFFSET + 1), 0xB5);
        assert_eq!(rom.read_byte(BIG_Q_ROUTINE_OFFSET + 2), 0x7E);
        // Spot-check: first obj_hi entry is $CD (3-5), last room index is $07 (8-1)
        assert_eq!(rom.read_byte(BIG_Q_ROUTINE_OFFSET + 33), 0xCD);
        assert_eq!(rom.read_byte(BIG_Q_ROUTINE_OFFSET + 65), 0x07);
    }

    #[test]
    fn test_remove_rocks() {
        let mut rom = make_test_rom();
        for offset in [W2_SECRET_ROCK, W3_BOAT_ROCK, W4_PIPE_ROCK] {
            rom.write_byte(offset, 0x51);
        }
        remove_rocks(&mut rom);
        for offset in [W2_SECRET_ROCK, W3_BOAT_ROCK, W4_PIPE_ROCK] {
            assert_eq!(rom.read_byte(offset), 0x45);
        }
    }

    #[test]
    fn test_remove_n_cards() {
        let mut rom = make_test_rom();
        rom.write_range(N_CARD_OFFSET, &[0x00, 0x00, 0x00]);
        remove_n_cards(&mut rom);
        assert_eq!(rom.read_range(N_CARD_OFFSET, 3), &[0xA9, 0x07, 0xEA]);
    }

    #[test]
    fn test_fix_w3_drawbridges() {
        let mut rom = make_test_rom();
        for (offset, _) in W3_DRAWBRIDGE_TILES {
            rom.write_byte(offset, 0x00);
        }
        rom.write_range(W3_TOGGLE_OFFSET, &[0xAD, 0xBB, 0x07, 0x49, 0x01, 0x8D, 0xBB, 0x07]);

        fix_w3_drawbridges(&mut rom);

        for (offset, tile) in W3_DRAWBRIDGE_TILES {
            assert_eq!(rom.read_byte(offset), tile);
        }
        assert_eq!(rom.read_range(W3_TOGGLE_OFFSET, W3_TOGGLE_LEN), &[0xEA; 8]);
    }

    #[test]
    fn test_card_speed_clear() {
        let mut rom = make_test_rom();
        // Place vanilla values
        rom.write_range(CARD_HOOK, &[0xB9, 0x9E, 0x7D, 0xF0, 0x22]);
        rom.write_byte(CARD_LIVES_AWARD, 0x01);
        rom.write_byte(CARD_CUTSCENE_FLAG, 0x40);
        rom.write_byte(CARD_MATCH_INDICATOR, 0x01);
        rom.write_range(CARD_CLEAR_GUARD, &[0xF0, 0x0D]);

        card_speed_clear(&mut rom);

        // Hook: JMP $FFE0; NOP; NOP
        assert_eq!(rom.read_range(CARD_HOOK, 5), &[0x4C, 0xE0, 0xFF, 0xEA, 0xEA]);
        // Trampoline: 24 bytes at PRG031 dead space
        assert_eq!(rom.read_byte(CARD_TRAMPOLINE), 0xB9); // LDA $7D9E,Y
        assert_eq!(rom.read_byte(CARD_TRAMPOLINE + 2), 0x7D);
        // One-of-each path: JMP $BD5A
        assert_eq!(rom.read_range(CARD_TRAMPOLINE + 16, 3), &[0x4C, 0x5A, 0xBD]);
        // Normal path tail: JMP $BCE1
        assert_eq!(rom.read_range(CARD_TRAMPOLINE + 23, 3), &[0x4C, 0xE1, 0xBC]);
        // Bank 9 belt-and-suspenders
        assert_eq!(rom.read_byte(CARD_LIVES_AWARD), 0x00);
        assert_eq!(rom.read_byte(CARD_CUTSCENE_FLAG), 0x00);
        assert_eq!(rom.read_byte(CARD_MATCH_INDICATOR), 0x00);
        assert_eq!(rom.read_range(CARD_CLEAR_GUARD, 2), &[0xEA, 0xEA]);
    }

    #[test]
    fn test_macobra_tail_swim_writes() {
        let mut rom = make_test_rom();
        apply_macobra_patches(&mut rom);

        assert_eq!(rom.read_range(TAIL_SWIM_HOOK_A, 5), &TAIL_SWIM_HOOK_A_BYTES);
        assert_eq!(rom.read_range(TAIL_SWIM_HOOK_B, 5), &TAIL_SWIM_HOOK_B_BYTES);
        assert_eq!(
            rom.read_range(TAIL_SWIM_ROUTINE_OFFSET, TAIL_SWIM_ROUTINE.len()),
            &TAIL_SWIM_ROUTINE
        );
    }

    #[test]
    fn test_macobra_hotfoot_chainchomp_tail() {
        let mut rom = make_test_rom();
        apply_macobra_patches(&mut rom);

        assert_eq!(rom.read_byte(HOTFOOT_TAIL_A), 0x00);
        assert_eq!(rom.read_byte(HOTFOOT_TAIL_B), 0x00);
        assert_eq!(rom.read_byte(HOTFOOT_TAIL_C), 0x25);
    }

    #[test]
    fn test_macobra_no_game_over_penalty_writes() {
        let mut rom = make_test_rom();
        apply_no_game_over_penalty(&mut rom);

        assert_eq!(rom.read_range(NGO_HOOK_A_OFFSET, NGO_HOOK_A_BYTES.len()), &NGO_HOOK_A_BYTES);
        assert_eq!(rom.read_range(NGO_HOOK_B_OFFSET, NGO_HOOK_B_BYTES.len()), &NGO_HOOK_B_BYTES);
        assert_eq!(rom.read_range(NGO_ROUTINE_OFFSET, NGO_ROUTINE.len()), &NGO_ROUTINE);
        assert_eq!(rom.read_range(NGO_NOP_OFFSET, NGO_NOP_BYTES.len()), &NGO_NOP_BYTES);
    }
}
