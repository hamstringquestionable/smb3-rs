use crate::rom::Rom;
use rand_chacha::ChaCha8Rng;

/// Starting lives value byte (LDA #imm operand).
/// Both Mario and Luigi are initialized from this single byte.
const STARTING_LIVES_OFFSET: usize = 0x308E1;

/// Base of the 8-byte lives init code: LDA #lives; STA $0736; STA $0737.
const LIVES_INIT_BASE: usize = 0x308E0;

use super::rom_data::{FS_STARTING_ITEMS, FS_HAMMER_LOCKS};

// W3 drawbridge map tile offsets (2× $B2 horizontal, 2× $B1 vertical)
const W3_BRIDGE_H1: usize = 0x18777;
const W3_BRIDGE_H2: usize = 0x18779;
const W3_BRIDGE_V1: usize = 0x1880C;
const W3_BRIDGE_V2: usize = 0x188F3;

// Toggle code: LDA $07BB; EOR #$01; STA $07BB (8 bytes at 0x14A68)
const W3_TOGGLE_OFFSET: usize = 0x14A68;
const W3_TOGGLE_LEN: usize = 8;

// W2 rock blocking secret path (screen 1, row 0, col 5) — $51 → $45
const W2_SECRET_ROCK: usize = 0x186E0;

// W3 rock blocking boat path (screen 0, row 6, col 15) — $51 → $45
const W3_BOAT_ROCK: usize = 0x187DB;

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
// Trampoline in PRG030 free space — offset from rom_data::FS_BIG_Q_SAVE.
use super::rom_data::FS_BIG_Q_SAVE as BIG_Q_PRG030_OFFSET;
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
// Lookup routine in PRG026 free space — offset from rom_data::FS_BIG_Q_LOOKUP.
use super::rom_data::FS_BIG_Q_LOOKUP as BIG_Q_ROUTINE_OFFSET;
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
    let clamped = lives.min(99).max(1);
    rom.write_byte(STARTING_LIVES_OFFSET, clamped);
}

/// Write starting items into Mario's inventory via a trampoline in PRG031.
///
/// Replaces the 8-byte lives init at 0x308E0 with `JSR $E250` into a
/// routine that sets lives, does the intro skip, AND writes up to 3 items
/// to inventory ($7D80+). Must run AFTER title_screen (which hooks the same
/// region for intro skip) — this trampoline incorporates that behavior.
pub fn write_starting_items(rom: &mut Rom, lives: u8, items: &[u8]) {
    let lives = lives.min(99).max(1);
    // Build trampoline: lives init + intro skip + item writes + RTS
    // CPU $E250 = file FS_STARTING_ITEMS
    let mut buf = Vec::with_capacity(24);
    buf.extend_from_slice(&[
        0xA9, lives,         // LDA #lives
        0x8D, 0x36, 0x07,    // STA $0736
        0x8D, 0x37, 0x07,    // STA $0737
        0xA9, 0x06,          // LDA #$06       (Title_State = IntroSkip)
        0x85, 0xDE,          // STA $DE
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

/// Remove the W2 rock blocking the secret path, replacing it with horizontal path.
pub fn remove_w2_rock(rom: &mut Rom) {
    rom.write_byte(W2_SECRET_ROCK, 0x45);
}

/// Remove the W3 rock blocking the boat path, replacing it with horizontal path.
pub fn remove_w3_boat_rock(rom: &mut Rom) {
    rom.write_byte(W3_BOAT_ROCK, 0x45);
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

// Trampoline in PRG031 dead space — offset from rom_data::FS_CARD_CLEAR.
// Overwrites 3 unused $FF bytes + "SUPER MARIO 3" string + dead padding.
// 26 bytes available ($FFE0-$FFF9), routine uses 26.
use super::rom_data::FS_CARD_CLEAR as CARD_TRAMPOLINE;

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

/// Adjust hitboxes for Bowser and Koopalings so they're easier to hit.
///
/// Original IPS: "Adjust Hitboxes (Bowser and Koopalings).ips"
/// 5 records total modifying sprite collision dimensions.
const HITBOX_A_OFFSET: usize = 0x002D4;
const HITBOX_A_DATA: [u8; 4] = [0x04, 0x14, 0x0A, 0x1C];
const HITBOX_B_OFFSET: usize = 0x0031C;
const HITBOX_C_OFFSET: usize = 0x0E681;
const HITBOX_D_OFFSET: usize = 0x0E686;
const HITBOX_E_OFFSET: usize = 0x0E691;

pub fn adjust_boss_hitboxes(rom: &mut Rom) {
    rom.write_range(HITBOX_A_OFFSET, &HITBOX_A_DATA);
    rom.write_byte(HITBOX_B_OFFSET, 0x04);
    rom.write_byte(HITBOX_C_OFFSET, 0x32);
    rom.write_byte(HITBOX_D_OFFSET, 0x20);
    rom.write_byte(HITBOX_E_OFFSET, 0x18);
}

/// Fix Koopaling softlock when airships are shuffled across worlds.
///
/// Original IPS: "SMB3 - Koopaling Softlock Fix.ips"
/// Single byte in a PRG001 object init table ($A176) controls Koopaling
/// behavior state. Vanilla value 0x05 can softlock when a Koopaling loads
/// in a non-native world (airship shuffle). Changing to 0x09 prevents it.
///
/// Applied when either `shuffle_airships` or `hammer_vulnerable_koopalings`
/// is enabled (the combined IPS also writes this byte).
const KOOPALING_SOFTLOCK_OFFSET: usize = 0x02186;

pub fn fix_koopaling_softlock(rom: &mut Rom) {
    rom.write_byte(KOOPALING_SOFTLOCK_OFFSET, 0x09);
}

/// Guard Koopaling collision bitmap during invulnerability frames.
///
/// Source: Fred's Koopaling fixes.
///
/// After a stomp (but before defeat), Objects_Timer2 ($0520,X) is set to ~$80.
/// The vanilla code at CPU $B15D unconditionally jumps to the collision bitmap
/// update ($D9D3), registering the Koopaling as hittable even during
/// invulnerability. This can cause phantom double-stomps — especially impactful
/// with randomized hit counts where a race-condition skip is more noticeable.
///
/// We change `JMP $D9D3` (3 bytes at file 0x0316D) to `JSR guard_routine`.
/// The guard checks Objects_Timer2 >= $70; if so, RTS skips the collision
/// update. Otherwise PLA;PLA;JMP $D9D3 restores vanilla behavior.
///
/// Patch site: file 0x0316D (CPU $B15D), 3 bytes.
const KOOPA_COLLISION_PATCH_SITE: usize = 0x0316D;

pub fn koopaling_collision_guard(rom: &mut Rom) {
    use super::rom_data::{FS_KOOPA_COLLISION_GUARD, KOOPA_COLLISION_GUARD_CPU};

    // Subroutine (13 bytes):
    //   LDA $0520,X    ; Objects_Timer2
    //   CMP #$70
    //   BCS +5         ; timer >= $70 → skip (RTS)
    //   PLA            ; pop JSR return address
    //   PLA
    //   JMP $D9D3      ; do vanilla collision bitmap update
    //   RTS            ; skip path
    #[rustfmt::skip]
    let code: [u8; 13] = [
        0xBD, 0x20, 0x05,   // LDA $0520,X
        0xC9, 0x70,          // CMP #$70
        0xB0, 0x05,          // BCS +5 → RTS
        0x68,                // PLA
        0x68,                // PLA
        0x4C, 0xD3, 0xD9,   // JMP $D9D3
        0x60,                // RTS
    ];
    rom.write_range(FS_KOOPA_COLLISION_GUARD, &code);

    // Patch site: JMP $D9D3 → JSR guard_routine
    let lo = (KOOPA_COLLISION_GUARD_CPU & 0xFF) as u8;
    let hi = (KOOPA_COLLISION_GUARD_CPU >> 8) as u8;
    rom.write_range(KOOPA_COLLISION_PATCH_SITE, &[0x20, lo, hi]); // JSR
}

/// Clear VRAM transfer buffer on Koopaling defeat.
///
/// Source: Fred's Koopaling fixes.
///
/// The fixed-bank cleanup at $F513 only clears $0300/$0301 (PPU VRAM buffer
/// header) when Level_ExitTo ($005E) == 0. But the Koopaling defeat routine
/// sets $005E = 6 *before* cleanup runs, so the conditional clear is skipped.
/// Stale VRAM write commands persist and get processed by NMI during the
/// wand-drop/king-rescue transition, causing garbled tiles — especially when
/// airships are shuffled to non-native worlds with different CHR banks.
///
/// We hook the defeat finalization at $BFA8 (file 0x03FB8, 8 bytes) via
/// JSR to a new routine that does the original work plus zeros $0300/$0301.
///
/// Patch site: file 0x03FB8 (CPU $BFA8), 8 bytes.
const KOOPA_DEFEAT_PATCH_SITE: usize = 0x03FB8;

pub fn koopaling_vram_clear(rom: &mut Rom) {
    use super::rom_data::{FS_KOOPA_VRAM_CLEAR, KOOPA_VRAM_CLEAR_CPU};

    // Subroutine (16 bytes):
    //   LDA #$06       ; exit type = Koopaling wand
    //   STA $005E      ; Level_ExitTo
    //   LDX $CD        ; restore object slot index
    //   LDA #$00
    //   STA $0300      ; clear VRAM buffer byte 0
    //   STA $0301      ; clear VRAM buffer byte 1
    //   RTS
    #[rustfmt::skip]
    let code: [u8; 16] = [
        0xA9, 0x06,          // LDA #$06
        0x8D, 0x5E, 0x00,   // STA $005E
        0xA6, 0xCD,          // LDX $CD
        0xA9, 0x00,          // LDA #$00
        0x8D, 0x00, 0x03,   // STA $0300
        0x8D, 0x01, 0x03,   // STA $0301
        0x60,                // RTS
    ];
    rom.write_range(FS_KOOPA_VRAM_CLEAR, &code);

    // Patch site: replace 8-byte defeat finalization with JSR + NOPs + RTS
    let lo = (KOOPA_VRAM_CLEAR_CPU & 0xFF) as u8;
    let hi = (KOOPA_VRAM_CLEAR_CPU >> 8) as u8;
    rom.write_range(KOOPA_DEFEAT_PATCH_SITE, &[
        0x20, lo, hi,   // JSR vram_clear
        0xEA, 0xEA,     // NOP; NOP
        0xEA, 0xEA,     // NOP; NOP
        0x60,            // RTS
    ]);
}

/// Clamp Koopaling Y position to screen bounds ($08–$E7).
///
/// Source: Fred's Koopaling fixes.
///
/// Koopalings like Lemmy/Wendy bounce via velocity table deltas. In non-native
/// boss rooms (airship shuffle), the floor height may differ, causing the
/// accumulated Y to wrap around 0/255 — the Koopaling teleports off-screen
/// and becomes unhittable (softlock).
///
/// Hooks the movement handler at $B3F4 (file 0x03404) by replacing
/// `LDA $0679,X` with `JSR clamp_routine`. The displaced instruction
/// executes inside the subroutine before RTS, so the caller sees the
/// same accumulator value.
///
/// Patch site: file 0x03404 (CPU $B3F4), 3 bytes.
const KOOPA_Y_CLAMP_PATCH_SITE: usize = 0x03404;

pub fn koopaling_y_clamp(rom: &mut Rom) {
    use super::rom_data::{FS_KOOPA_Y_CLAMP, KOOPA_Y_CLAMP_CPU};

    // Subroutine (22 bytes):
    //   LDA $91,X      ; Objects_Y
    //   CMP #$08       ; below top bound?
    //   BCC .low       ; if < 8, clamp low
    //   CMP #$E8       ; above bottom bound?
    //   BCC .store     ; if < 232, in range
    //   LDA #$E8       ; clamp high
    //   BCS .store     ; unconditional (carry set)
    // .low:
    //   LDA #$08       ; clamp low
    // .store:
    //   STA $91,X      ; write clamped Y
    //   LDA $0679,X    ; displaced instruction from caller
    //   RTS
    #[rustfmt::skip]
    let code: [u8; 22] = [
        0xB5, 0x91,          // LDA $91,X
        0xC9, 0x08,          // CMP #$08
        0x90, 0x08,          // BCC .low (+8)
        0xC9, 0xE8,          // CMP #$E8
        0x90, 0x06,          // BCC .store (+6)
        0xA9, 0xE8,          // LDA #$E8
        0xB0, 0x02,          // BCS .store (+2)
        0xA9, 0x08,          // LDA #$08
        // .store:
        0x95, 0x91,          // STA $91,X
        0xBD, 0x79, 0x06,   // LDA $0679,X (displaced)
        0x60,                // RTS
    ];
    rom.write_range(FS_KOOPA_Y_CLAMP, &code);

    // Patch site: LDA $0679,X → JSR clamp_routine
    let lo = (KOOPA_Y_CLAMP_CPU & 0xFF) as u8;
    let hi = (KOOPA_Y_CLAMP_CPU >> 8) as u8;
    rom.write_range(KOOPA_Y_CLAMP_PATCH_SITE, &[0x20, lo, hi]); // JSR
}

/// Make Koopalings vulnerable to thrown hammers.
///
/// Original IPS: "SMB3 - Koopaling Softlock Fix + Hammers Can Hit Koopalings.ips"
/// Clears bit 7 of an object attribute byte in PRG000 ($8302), removing the
/// Koopaling hammer invulnerability flag. Vanilla 0x89 → 0x09.
const KOOPALING_HAMMER_VULN_OFFSET: usize = 0x00312;

pub fn hammer_vulnerable_koopalings(rom: &mut Rom) {
    rom.write_byte(KOOPALING_HAMMER_VULN_OFFSET, 0x09);
}

/// Randomize Koopaling identity per world via `Map_Unused7EEA` remap.
/// Source: fcoughlin (Fred).
/// See docs/smb3_rom_reference.md § "Map_Unused7EEA".
const KOOPALING_REMAP_SITES: &[usize] = &[
    0x02E30, 0x02ED4, 0x02F3B, 0x02FAE, 0x02FE5, 0x02FF6,
    0x03020, 0x03181, 0x03372, 0x033E8, 0x03612,
];
const KOOPALING_REMAP_LUT: usize = 0x16018;

pub fn random_koopalings(rom: &mut Rom, rng: &mut ChaCha8Rng) {
    use rand::seq::SliceRandom;

    let mut koopalings: [u8; 7] = [0, 1, 2, 3, 4, 5, 6];
    koopalings.shuffle(rng);

    let mut lut = [0u8; 8];
    lut[..7].copy_from_slice(&koopalings);
    lut[7] = 0x05; // W8 unchanged (Bowser)
    rom.write_range(KOOPALING_REMAP_LUT, &lut);

    for &site in KOOPALING_REMAP_SITES {
        rom.write_range(site + 1, &[0xEA, 0x7E]);
    }
}

/// Make the hammer item also break fortress lock tiles on the overworld map.
///
/// The vanilla hammer routine at PRG026 (file 0x346D5, CPU $A6C5) uses a
/// 7-byte range check: `SEC; SBC #$51; CMP #$02; BCC .found` which only
/// matches rock tiles $51–$52. We replace this with a JSR to a table-driven
/// subroutine in PRG026 free space that checks 5 tile IDs (2 rocks + 3 locks).
///
/// Patch site 1 — Range check (file 0x346D5, 7 bytes):
///   `SEC; SBC #$51; CMP #$02; BCC .found` →
///   `JSR HammerCheckTile; BCC .found; NOP; NOP`
///
/// Patch site 2 — Replacement tile load (file 0x346E9, 3 bytes):
///   `LDA $A6B1,X` → `LDA $7EB6` (load from scratch RAM set by subroutine)
///
/// New subroutine at FS_HAMMER_LOCKS (0x3557F, CPU $B56F), 47 bytes:
///   Table-driven check of breakable tiles, stores replacement tile in $7EB6,
///   saves/restores X via $7EB7, returns carry clear if breakable.
///
/// Water gap locks (0x9D → 0xB3) are intentionally excluded — bridge tiles
/// need more testing.

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

/// Randomize per-Koopaling stomp counts (1–5 hits each, independently).
///
/// The Koopaling stomp handler is `ObjHit_Koopaling` in PRG001 (southbird
/// disassembly). The vanilla code at CPU $B187 does:
///   LDA $7F,X    ; load Objects_Var4 (stomp counter)
///   CMP #$03     ; 3 hits to kill
///   BCS defeated
///
/// We replace `LDA $7F,X; CMP #$03` (3 bytes at file 0x03197) with
/// `JMP subroutine` which loads the counter, looks up a per-world threshold
/// table indexed by World_Num ($0727), and branches to the vanilla survive
/// ($B18D) or defeat ($B193) paths.
///
/// Patch sites:
///   - 0x03197: `LDA $7F,X; CMP #$03` → `JMP $B81A`
///   - FS_KOOPA_HITS_SUB (0x0382A): 13-byte subroutine
///   - FS_KOOPA_HITS_TABLE (0x03837): 7-byte per-world threshold table

/// File offset of `LDA $7F,X; CMP #$03` in ObjHit_Koopaling (3 bytes).
const KOOPA_PATCH_SITE: usize = 0x03197;
/// CPU address of the vanilla "survive" path (sets timer, RTS).
const KOOPA_SURVIVE_CPU: u16 = 0xB18D;
/// CPU address of the vanilla "defeated" path.
const KOOPA_DEFEAT_CPU: u16 = 0xB193;

use super::rom_data::{KOOPA_HITS_SUB_CPU, KOOPA_HITS_TABLE_CPU};

/// Subroutine machine code (13 bytes):
/// ```asm
///   LDA $7F,X              ; load stomp counter (original instruction)
///   LDY $0727              ; Y = World_Num (0–6)
///   CMP ($B827),Y          ; compare with per-world threshold
///   BCS +3                 ; if >= threshold → defeated
///   JMP $B18D              ; survive
///   JMP $B193              ; defeated
/// ```
const KOOPA_HITS_CODE: [u8; 13] = [
    0xB5, 0x7F,                                                  // LDA $7F,X
    0xAC, 0x27, 0x07,                                            // LDY $0727
    0xD9, KOOPA_HITS_TABLE_CPU as u8, (KOOPA_HITS_TABLE_CPU >> 8) as u8, // CMP table,Y
    0xB0, 0x03,                                                  // BCS +3 (to JMP defeat)
    0x4C, KOOPA_SURVIVE_CPU as u8, (KOOPA_SURVIVE_CPU >> 8) as u8, // JMP $B18D
];
// Note: defeat JMP ($B193) follows immediately after in the table area,
// but we can just let BCS fall through to the table bytes — instead we
// place the defeat JMP right after the code, before the table.
// Total: 13 bytes code + 3 bytes JMP defeat + 7 bytes table = 23 bytes.

pub fn randomize_koopaling_hits(rom: &mut Rom, rng: &mut ChaCha8Rng) {
    use rand::Rng;

    // Write subroutine into free space
    rom.write_range(super::rom_data::FS_KOOPA_HITS_SUB, &KOOPA_HITS_CODE);

    // Write JMP defeat right after the subroutine (at sub + 13)
    let defeat_jmp_offset = super::rom_data::FS_KOOPA_HITS_SUB + 13;
    rom.write_range(defeat_jmp_offset, &[
        0x4C, KOOPA_DEFEAT_CPU as u8, (KOOPA_DEFEAT_CPU >> 8) as u8,
    ]);

    // Build per-world threshold table: worlds 0–6 get random 1–5
    let mut table = [3u8; 7];
    for entry in table.iter_mut() {
        *entry = rng.random_range(1..=5);
    }
    rom.write_range(super::rom_data::FS_KOOPA_HITS_TABLE, &table);

    // Patch call site: replace LDA $7F,X; CMP #$03 (3 bytes) with JMP subroutine
    rom.write_range(KOOPA_PATCH_SITE, &[
        0x4C, KOOPA_HITS_SUB_CPU as u8, (KOOPA_HITS_SUB_CPU >> 8) as u8,
    ]);
}

/// Skip the wand falling cutscene after defeating a Koopaling.
///
/// Lets the player jump for the wand grab instead of watching the wand drop.
/// Original IPS: 2 bytes at 0x002EF3.
const SKIP_WAND_CUTSCENE_OFFSET: usize = 0x002EF3;

pub fn skip_wand_cutscene(rom: &mut Rom) {
    rom.write_range(SKIP_WAND_CUTSCENE_OFFSET, &[0x16, 0xB5]);
}

/// Remove N-card (N-Spade) panels from the overworld map.
///
/// Patches the map-screen code so N-Spade tiles never appear.
/// Original IPS: 3 bytes at 0x016C90 → LDA #$07; NOP.
const N_CARD_OFFSET: usize = 0x016C90;

pub fn remove_n_cards(rom: &mut Rom) {
    rom.write_range(N_CARD_OFFSET, &[0xA9, 0x07, 0xEA]);
}

/// Fix W3 canoe softlocks: save death respawn position when entering via canoe,
/// and backup/restore the map tile data the canoe overwrites.
///
/// Without this, levels placed on W3 island tiles (freed by spade game removal)
/// can softlock if the player dies — the respawn position is invalid and the map
/// data under the canoe is permanently corrupted.
///
/// Based on "SMB3 - Canoe Softlock Fixes (Open World compatible).ips".
pub fn fix_canoe_softlock(rom: &mut Rom) {
    // Record 1: Hook at 0x146FA (PRG010, CPU $C6EA) → JSR $BD0C (canoe cleanup)
    rom.write_range(0x146FA, &[0x20, 0x0C, 0xBD, 0xEA, 0xEA]);

    // Record 2: Boundary check adjustment at 0x14F23 (PRG010, CPU $CF13)
    rom.write_range(0x14F23, &[0xE0, 0xDD]);

    // Record 3: New subroutine in PRG010 free space (rom_data::FS_CANOE_RESPAWN)
    // Saves player map position as death respawn point when entering via canoe ($4B)
    rom.write_range(super::rom_data::FS_CANOE_RESPAWN, &[
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
    ]);

    // Record 4: Hook at 0x1623F (PRG011, CPU $A22F) → JSR $BCF0 (canoe backup)
    rom.write_range(0x1623F, &[0x20, 0xF0, 0xBC, 0xEA, 0xEA]);

    // Record 5: Canoe backup/restore subroutines in PRG011 free space (rom_data::FS_CANOE_BACKUP)
    // Part A ($BCF0): backs up 3 map data values before canoe overwrites them
    // Part B ($BD0C): restores backed-up values when canoe interaction ends
    rom.write_range(super::rom_data::FS_CANOE_BACKUP, &[
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
    ]);
}

/// Replace W3 drawbridge tiles with normal path tiles and NOP the toggle code.
pub fn fix_w3_drawbridges(rom: &mut Rom) {
    // Replace horizontal drawbridge tiles with bridge ($B3, horizontal path)
    rom.write_byte(W3_BRIDGE_H1, 0xB3);
    rom.write_byte(W3_BRIDGE_H2, 0xB3);
    // Replace vertical drawbridge tiles with open path ($BA, vertical-compatible)
    rom.write_byte(W3_BRIDGE_V1, 0xBA);
    rom.write_byte(W3_BRIDGE_V2, 0xBA);
    // NOP out the toggle code (LDA $07BB; EOR #$01; STA $07BB)
    rom.write_range(W3_TOGGLE_OFFSET, &[0xEA; W3_TOGGLE_LEN]);
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
        Rom::from_bytes(&data).unwrap()
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
    fn test_remove_w2_rock() {
        let mut rom = make_test_rom();
        rom.write_byte(W2_SECRET_ROCK, 0x51);
        remove_w2_rock(&mut rom);
        assert_eq!(rom.read_byte(W2_SECRET_ROCK), 0x45);
    }

    #[test]
    fn test_remove_w3_boat_rock() {
        let mut rom = make_test_rom();
        rom.write_byte(W3_BOAT_ROCK, 0x51);
        remove_w3_boat_rock(&mut rom);
        assert_eq!(rom.read_byte(W3_BOAT_ROCK), 0x45);
    }

    #[test]
    fn test_skip_wand_cutscene() {
        let mut rom = make_test_rom();
        rom.write_range(SKIP_WAND_CUTSCENE_OFFSET, &[0x00, 0x00]);
        skip_wand_cutscene(&mut rom);
        assert_eq!(rom.read_range(SKIP_WAND_CUTSCENE_OFFSET, 2), &[0x16, 0xB5]);
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
        // Place original drawbridge tiles
        rom.write_byte(W3_BRIDGE_H1, 0xB2);
        rom.write_byte(W3_BRIDGE_H2, 0xB2);
        rom.write_byte(W3_BRIDGE_V1, 0xB1);
        rom.write_byte(W3_BRIDGE_V2, 0xB1);
        // Place original toggle code
        rom.write_range(W3_TOGGLE_OFFSET, &[0xAD, 0xBB, 0x07, 0x49, 0x01, 0x8D, 0xBB, 0x07]);

        fix_w3_drawbridges(&mut rom);

        assert_eq!(rom.read_byte(W3_BRIDGE_H1), 0xB3);
        assert_eq!(rom.read_byte(W3_BRIDGE_H2), 0xB3);
        assert_eq!(rom.read_byte(W3_BRIDGE_V1), 0xBA);
        assert_eq!(rom.read_byte(W3_BRIDGE_V2), 0xBA);
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
    fn test_fix_koopaling_softlock() {
        let mut rom = make_test_rom();
        fix_koopaling_softlock(&mut rom);
        assert_eq!(rom.read_byte(KOOPALING_SOFTLOCK_OFFSET), 0x09);
    }

    #[test]
    fn test_hammer_vulnerable_koopalings() {
        let mut rom = make_test_rom();
        hammer_vulnerable_koopalings(&mut rom);
        assert_eq!(rom.read_byte(KOOPALING_HAMMER_VULN_OFFSET), 0x09);
    }

    #[test]
    fn test_adjust_boss_hitboxes() {
        let mut rom = make_test_rom();
        adjust_boss_hitboxes(&mut rom);
        assert_eq!(rom.read_range(HITBOX_A_OFFSET, 4), &HITBOX_A_DATA);
        assert_eq!(rom.read_byte(HITBOX_B_OFFSET), 0x04);
        assert_eq!(rom.read_byte(HITBOX_C_OFFSET), 0x32);
        assert_eq!(rom.read_byte(HITBOX_D_OFFSET), 0x20);
        assert_eq!(rom.read_byte(HITBOX_E_OFFSET), 0x18);
    }

    #[test]
    fn test_randomize_koopaling_hits() {
        use rand::SeedableRng;

        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize_koopaling_hits(&mut rom, &mut rng);

        // Patch site: JMP $B81A
        assert_eq!(rom.read_range(KOOPA_PATCH_SITE, 3), &[
            0x4C,
            crate::randomize::rom_data::KOOPA_HITS_SUB_CPU as u8,
            (crate::randomize::rom_data::KOOPA_HITS_SUB_CPU >> 8) as u8,
        ]);
        // Subroutine written
        assert_eq!(
            rom.read_range(crate::randomize::rom_data::FS_KOOPA_HITS_SUB, 13),
            &KOOPA_HITS_CODE,
        );
        // Defeat JMP follows subroutine
        let defeat_off = crate::randomize::rom_data::FS_KOOPA_HITS_SUB + 13;
        assert_eq!(rom.read_range(defeat_off, 3), &[0x4C, 0x93, 0xB1]);
        // Table: worlds 0–6 each in 1..=5
        let table = rom.read_range(crate::randomize::rom_data::FS_KOOPA_HITS_TABLE, 7);
        for &v in &table[..] {
            assert!((1..=5).contains(&v), "threshold {v} out of range 1–5");
        }
    }

    #[test]
    fn test_random_koopalings() {
        use rand::SeedableRng;

        let mut rom = make_test_rom();
        // Seed vanilla bytes at each patch site so the operand rewrite is visible.
        for &site in KOOPALING_REMAP_SITES {
            rom.write_range(site, &[0xAD, 0x27, 0x07]);
        }

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        random_koopalings(&mut rom, &mut rng);

        // LUT: W1–W7 permutation of 0..=6, W8 = 0x05
        let lut = rom.read_range(KOOPALING_REMAP_LUT, 8);
        let mut sorted: Vec<u8> = lut[..7].to_vec();
        sorted.sort();
        assert_eq!(sorted, vec![0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(lut[7], 0x05);

        // All 11 sites have operand bytes rewritten to EA 7E
        for &site in KOOPALING_REMAP_SITES {
            assert_eq!(
                rom.read_range(site + 1, 2),
                &[0xEA, 0x7E],
                "site 0x{site:05X} operand not patched"
            );
            // Opcode byte preserved
            assert_eq!(rom.read_byte(site), 0xAD);
        }
    }
}
