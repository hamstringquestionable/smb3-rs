use rand::Rng;

use crate::rom::Rom;

use super::rom_data::{KOOPA_HITS_SUB_CPU, KOOPA_HITS_TABLE_CPU};

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

/// Immediate operands of the two `CMP #$imm` checks in `Koopaling_DetectWorld`
/// (file 0x03612 / CPU $B602) that gate the heavy-physics effect (enhanced
/// gravity, floor-shake, player paralysis). Vanilla compares against the Roy
/// (0x04) and Ludwig (0x06) identity values; rewriting these operands moves the
/// effect onto any two identities. See docs/smb3_rom_reference.md § "Map_Unused7EEA".
const KOOPALING_HEAVY_CMP_ROY: usize = 0x03616;
const KOOPALING_HEAVY_CMP_LUDWIG: usize = 0x0361A;

/// Immediate operands of the three checks that together make up Wendy's ring
/// attack: the ring-vs-wand projectile choice (`CMP` at 0x02FB2), the firing
/// cadence (`CMP` at 0x02FFA), and the straight-aim / skip-homing branch
/// (`CPY` at 0x03024). All three test the same identity (vanilla 0x02 = Wendy),
/// so they must be rewritten *together* to the same value to move the whole
/// ring package coherently onto another body.
const KOOPALING_RING_CMP_SITES: [usize; 3] = [0x02FB2, 0x02FFA, 0x03024];

/// `KoopalingPatSet5` — the per-identity sprite CHR page loaded into `PatTable_BankSel+5`
/// (the projectile window) by `ObjNorm_Koopaling` (`$AEC4: LDY $7EEA; LDA $AE79,Y`).
/// 7 bytes, indexed by remapped Koopaling identity. Slot +4 (`KoopalingPatSet4`, the
/// body window) needs no change — each identity already loads its own body page.
const KOOPALING_PATSET5: usize = 0x02E89;
/// CHR page holding Wendy's ring projectile tiles (vanilla: only Wendy's identity loads it).
const CHR_PAGE_RING: u8 = 0x4A;
/// CHR page holding Lemmy's ball tiles.
const CHR_PAGE_BALL: u8 = 0x48;
/// CHR page holding the plain wand-blast tiles (every non-ring, non-ball boss).
const CHR_PAGE_WAND: u8 = 0x37;
/// Lemmy's Koopaling identity value.
const LEMMY_IDENTITY: usize = 0x05;

pub fn random_koopalings<R: Rng>(rom: &mut Rom, rng: &mut R) {
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

    // Reassign the heavy-physics effect (vanilla: Roy + Ludwig) to two random
    // identities. The two DetectWorld compares are equality tests, so the picks
    // must be distinct to keep exactly two heavy bosses. Lemmy (0x05) is
    // excluded from the pool: his AI is replaced wholesale by the ball routine,
    // so it's unverified whether DetectWorld's heavy branch even runs for him —
    // keeping him out guarantees the effect always lands on two live bosses.
    let mut heavy: [u8; 6] = [0, 1, 2, 3, 4, 6];
    heavy.shuffle(rng);
    rom.write_byte(KOOPALING_HEAVY_CMP_ROY, heavy[0]);
    rom.write_byte(KOOPALING_HEAVY_CMP_LUDWIG, heavy[1]);

    // Move Wendy's ring attack onto a random identity's body. There is exactly
    // one ring boss (as in vanilla); randomizing the compare value picks which
    // body carries it. All three ring sites take the SAME value to stay
    // coherent. Lemmy (0x05) is excluded: his ball AI replaces the wand-fire
    // path the ring gate lives on, so the ring would never fire on his body.
    let mut ring: [u8; 6] = [0, 1, 2, 3, 4, 6];
    ring.shuffle(rng);
    for &site in &KOOPALING_RING_CMP_SITES {
        rom.write_byte(site, ring[0]);
    }

    // Move the ring's *graphics* to match the moved *behavior*. The projectile
    // lives in its own 1KB sprite CHR page loaded into BankSel slot +5
    // (KoopalingPatSet5); vanilla only maps the ring page (0x4A) for Wendy's
    // identity, so retargeting the ring behavior without this leaves the new
    // ring boss loading the plain wand-blast page and rendering garbled tiles.
    // Rewrite the per-identity table so the projectile page follows assignment:
    // the ring identity gets the ring page, Lemmy keeps his ball page, everyone
    // else gets the wand-blast page. This also corrects the reverse case — the
    // old Wendy identity no longer loads 0x4A, so her wand blast renders right.
    // ring[0] is drawn from a pool excluding Lemmy (0x05), so it never collides.
    let mut patset5 = [CHR_PAGE_WAND; 7];
    patset5[LEMMY_IDENTITY] = CHR_PAGE_BALL;
    patset5[ring[0] as usize] = CHR_PAGE_RING;
    rom.write_range(KOOPALING_PATSET5, &patset5);
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

// Randomize per-Koopaling stomp counts (1–5 hits each, independently).
//
// The Koopaling stomp handler is `ObjHit_Koopaling` in PRG001 (southbird
// disassembly). The vanilla code at CPU $B187 does:
//   LDA $7F,X    ; load Objects_Var4 (stomp counter)
//   CMP #$03     ; 3 hits to kill
//   BCS defeated
//
// We replace `LDA $7F,X; CMP #$03` (3 bytes at file 0x03197) with
// `JMP subroutine` which loads the counter, looks up a per-world threshold
// table indexed by World_Num ($0727), and branches to the vanilla survive
// ($B18D) or defeat ($B193) paths.
//
// Patch sites:
//   - 0x03197: `LDA $7F,X; CMP #$03` → `JMP $B81A`
//   - FS_KOOPA_HITS_SUB (0x0382A): 13-byte subroutine
//   - FS_KOOPA_HITS_TABLE (0x03837): 7-byte per-world threshold table

/// File offset of `LDA $7F,X; CMP #$03` in ObjHit_Koopaling (3 bytes).
const KOOPA_PATCH_SITE: usize = 0x03197;
/// CPU address of the vanilla "survive" path (sets timer, RTS).
const KOOPA_SURVIVE_CPU: u16 = 0xB18D;
/// CPU address of the vanilla "defeated" path.
const KOOPA_DEFEAT_CPU: u16 = 0xB193;

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
// Layout: the defeat JMP ($B193) sits right after the code (sub + 13),
// followed by the 7-byte threshold table.
// Total: 13 bytes code + 3 bytes JMP defeat + 7 bytes table = 23 bytes.

/// File offset of fireball→stomp handoff: `LDA #$02; STA $7F,X` (4 bytes).
///
/// When Objects_HitCount ($7CF6) reaches 0 from fireball hits, vanilla sets
/// the stomp counter ($7F,X) to 2 and jumps into the stomp handler at $B17B,
/// which does INC $7F,X → 3, then CMP #$03 → defeat. With random thresholds
/// > 3, the hardcoded 2 never reaches defeat — permanent softlock.
///
/// We replace these 4 bytes with `JSR fire_preset; NOP`. The fire_preset
/// subroutine loads the per-world threshold from our table, subtracts 1,
/// and stores to $7F,X. After INC at $B185, the counter exactly equals the
/// threshold, guaranteeing defeat.
const KOOPA_FIRE_HANDOFF: usize = 0x03035;

pub fn randomize_koopaling_hits<R: Rng>(rom: &mut Rom, rng: &mut R) {
    use super::rom_data::{FS_KOOPA_FIRE_PRESET, KOOPA_FIRE_PRESET_CPU};

    // Write stomp threshold subroutine into free space
    rom.write_range(super::rom_data::FS_KOOPA_HITS_SUB, &KOOPA_HITS_CODE);

    // Write JMP defeat right after the subroutine (at sub + 13)
    let defeat_jmp_offset = super::rom_data::FS_KOOPA_HITS_SUB + 13;
    rom.write_range(defeat_jmp_offset, &[
        0x4C, KOOPA_DEFEAT_CPU as u8, (KOOPA_DEFEAT_CPU >> 8) as u8,
    ]);

    // Build per-world threshold table: worlds 0–6 get random 1–5
    let table: [u8; 7] = std::array::from_fn(|_| rng.random_range(1..=5));
    rom.write_range(super::rom_data::FS_KOOPA_HITS_TABLE, &table);

    // Patch stomp call site: replace LDA $7F,X; CMP #$03 (3 bytes) with JMP subroutine
    rom.write_range(KOOPA_PATCH_SITE, &[
        0x4C, KOOPA_HITS_SUB_CPU as u8, (KOOPA_HITS_SUB_CPU >> 8) as u8,
    ]);

    // Write fireball preset subroutine (12 bytes):
    //   LDY $0727        ; World_Num
    //   LDA table,Y      ; per-world threshold
    //   SEC
    //   SBC #$01         ; threshold - 1
    //   STA $7F,X        ; store so INC at $B185 → exactly threshold
    //   RTS
    #[rustfmt::skip]
    let fire_code: [u8; 12] = [
        0xAC, 0x27, 0x07,                                              // LDY $0727
        0xB9, KOOPA_HITS_TABLE_CPU as u8, (KOOPA_HITS_TABLE_CPU >> 8) as u8, // LDA table,Y
        0x38,                                                           // SEC
        0xE9, 0x01,                                                     // SBC #$01
        0x95, 0x7F,                                                     // STA $7F,X
        0x60,                                                           // RTS
    ];
    rom.write_range(FS_KOOPA_FIRE_PRESET, &fire_code);

    // Patch fireball handoff: LDA #$02; STA $7F,X (4 bytes) → JSR fire_preset; NOP
    let lo = (KOOPA_FIRE_PRESET_CPU & 0xFF) as u8;
    let hi = (KOOPA_FIRE_PRESET_CPU >> 8) as u8;
    rom.write_range(KOOPA_FIRE_HANDOFF, &[0x20, lo, hi, 0xEA]); // JSR + NOP
}

// Randomize per-fortress Boom-Boom stomp counts (1–5 hits each).
//
// Boom-Boom's boss AI lives in PRG003. Unlike the Koopaling, its stomp count is
// entangled with its attack-state machine: `Objects_Var5` ($9A,X) is *both* the
// hit counter *and* the index the DynJump dispatcher uses to pick the current
// attack (2=Primary, 3=Secondary, 4=Final, 5=Death). The vanilla stomp handler
// at CPU $AE68 does:
//   LDA $9A,X ; INC $9A,X ; CMP #$04 ; BEQ death   (death when Var5 reaches 5)
// Var5 starts the fight at 2, so 3 stomps (2→3→4→5) kill it.
//
// We can't just change the compare: death *requires* Var5 to reach the death
// state (5), and letting Var5 climb past 5 would index off the 6-entry jump
// table (crash). Instead we DECOUPLE the count from the state:
//
//   * `Objects_Var12` ($7CD2,X) — cleared to 0 on spawn by Level_PrepareNewObject
//     and never touched by any Boom-Boom routine — is our real stomp tally.
//   * Var5 keeps advancing (2→3→4) so the boss still cycles through its attacks;
//     when it would hit the death state (5) but the tally hasn't reached the
//     threshold yet, we bounce it back to Primary (state 2) so it keeps fighting.
//   * Only when the tally reaches this fortress's threshold do we force Var5=5
//     and take the vanilla death path.
//
// The per-fortress threshold comes from a 16-byte table indexed by
// `(World_Num << 2 + ordinal) & $0F`, where the ordinal (1–4) is Boom-Boom's
// fortress number within its world, sitting in `Objects_Var4` ($7F,X) at the
// moment of the stomp. That index makes every fortress *within a world* distinct
// (only far-apart cross-world fortresses can share a table slot, which is
// invisible in play).
//
// Fireball defeat (37 fireballs via Objects_HitCount) is a separate path and is
// intentionally left unchanged — only the stomp count is randomized.

/// File offset of the vanilla Boom-Boom stomp handler
/// `LDA $9A,X; INC $9A,X; CMP #$04; BEQ +$12` in BoomBoom_HitTest (8 bytes,
/// CPU $AE68). We overwrite it with `JMP subroutine` + NOP padding.
const BOOMBOOM_PATCH_SITE: usize = 0x06E78;
/// CPU address of the vanilla "survive" tail (clears state vars, sets Timer2, RTS).
const BOOMBOOM_SURVIVE_CPU: u16 = 0xAE70;
/// CPU address of the vanilla "death" tail (sets death Timer, RTS).
const BOOMBOOM_DEATH_CPU: u16 = 0xAE82;

pub fn randomize_boomboom_hits<R: Rng>(rom: &mut Rom, rng: &mut R) {
    use super::rom_data::{
        BOOMBOOM_HITS_SUB_CPU, BOOMBOOM_HITS_TABLE_CPU, FS_BOOMBOOM_HITS_SUB,
        FS_BOOMBOOM_HITS_TABLE,
    };

    // 16-entry threshold table: each fortress index gets an independent 1–5.
    let table: [u8; 16] = std::array::from_fn(|_| rng.random_range(1..=5));
    rom.write_range(FS_BOOMBOOM_HITS_TABLE, &table);

    let tbl_lo = BOOMBOOM_HITS_TABLE_CPU as u8;
    let tbl_hi = (BOOMBOOM_HITS_TABLE_CPU >> 8) as u8;
    let surv_lo = BOOMBOOM_SURVIVE_CPU as u8;
    let surv_hi = (BOOMBOOM_SURVIVE_CPU >> 8) as u8;
    let death_lo = BOOMBOOM_DEATH_CPU as u8;
    let death_hi = (BOOMBOOM_DEATH_CPU >> 8) as u8;

    // Subroutine (44 bytes, CPU $BFCF):
    //   INC $7CD2,X          ; Objects_Var12 — stomp tally (self-zeroed on spawn)
    //   INC $9A,X            ; Objects_Var5  — advance attack state
    //   LDA $0727            ; World_Num
    //   ASL ; ASL            ; world * 4
    //   CLC ; ADC $7F,X      ; + ordinal (Objects_Var4, 1–4)
    //   AND #$0F             ; -> table index 0..15
    //   TAY
    //   LDA $7CD2,X          ; tally
    //   CMP table,Y          ; tally - threshold
    //   BCS .death           ; tally >= threshold -> defeat
    //   LDA $9A,X            ; else keep Var5 a valid attack state:
    //   CMP #$05
    //   BCC .surv            ;   still 2–4 -> fine
    //   LDA #$02 ; STA $9A,X ;   would be Death -> bounce back to Primary
    // .surv:
    //   JMP $AE70            ; vanilla survive tail
    // .death:
    //   LDA #$05 ; STA $9A,X ; force Death state
    //   JMP $AE82            ; vanilla death tail
    #[rustfmt::skip]
    let code: [u8; 44] = [
        0xFE, 0xD2, 0x7C,               // INC $7CD2,X
        0xF6, 0x9A,                     // INC $9A,X
        0xAD, 0x27, 0x07,               // LDA $0727
        0x0A,                           // ASL
        0x0A,                           // ASL
        0x18,                           // CLC
        0x75, 0x7F,                     // ADC $7F,X
        0x29, 0x0F,                     // AND #$0F
        0xA8,                           // TAY
        0xBD, 0xD2, 0x7C,               // LDA $7CD2,X
        0xD9, tbl_lo, tbl_hi,           // CMP table,Y
        0xB0, 0x0D,                     // BCS .death (+$0D)
        0xB5, 0x9A,                     // LDA $9A,X
        0xC9, 0x05,                     // CMP #$05
        0x90, 0x04,                     // BCC .surv (+$04)
        0xA9, 0x02,                     // LDA #$02
        0x95, 0x9A,                     // STA $9A,X
        0x4C, surv_lo, surv_hi,         // .surv:  JMP $AE70
        0xA9, 0x05,                     // .death: LDA #$05
        0x95, 0x9A,                     // STA $9A,X
        0x4C, death_lo, death_hi,       // JMP $AE82
    ];
    rom.write_range(FS_BOOMBOOM_HITS_SUB, &code);

    // Patch the stomp handler: replace the 8-byte vanilla block with
    // `JMP subroutine` + 5 NOPs (the NOPs are unreachable — the JMP is taken
    // unconditionally — but keep the disassembly clean).
    let sub_lo = BOOMBOOM_HITS_SUB_CPU as u8;
    let sub_hi = (BOOMBOOM_HITS_SUB_CPU >> 8) as u8;
    rom.write_range(BOOMBOOM_PATCH_SITE, &[
        0x4C, sub_lo, sub_hi,           // JMP subroutine
        0xEA, 0xEA, 0xEA, 0xEA, 0xEA,   // NOP × 5
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::Rom;
    use rand_chacha::ChaCha8Rng;

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        Rom::from_bytes_lax(&data, true).unwrap()
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
        for &v in table {
            assert!((1..=5).contains(&v), "threshold {v} out of range 1–5");
        }

        // Fireball handoff: JSR fire_preset + NOP
        assert_eq!(rom.read_byte(KOOPA_FIRE_HANDOFF), 0x20); // JSR opcode
        assert_eq!(rom.read_byte(KOOPA_FIRE_HANDOFF + 3), 0xEA); // NOP

        // Fire preset subroutine written
        let fire = rom.read_range(crate::randomize::rom_data::FS_KOOPA_FIRE_PRESET, 12);
        assert_eq!(fire[0], 0xAC); // LDY abs
        assert_eq!(fire[11], 0x60); // RTS
    }

    #[test]
    fn test_randomize_boomboom_hits() {
        use rand::SeedableRng;
        use crate::randomize::rom_data::{
            BOOMBOOM_HITS_SUB_CPU, FS_BOOMBOOM_HITS_SUB, FS_BOOMBOOM_HITS_TABLE,
        };

        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize_boomboom_hits(&mut rom, &mut rng);

        // Patch site: JMP subroutine + 5 NOPs.
        assert_eq!(rom.read_byte(BOOMBOOM_PATCH_SITE), 0x4C);
        assert_eq!(rom.read_range(BOOMBOOM_PATCH_SITE + 1, 2), &[
            BOOMBOOM_HITS_SUB_CPU as u8,
            (BOOMBOOM_HITS_SUB_CPU >> 8) as u8,
        ]);
        assert_eq!(rom.read_range(BOOMBOOM_PATCH_SITE + 3, 5), &[0xEA; 5]);

        // Subroutine head: INC $7CD2,X ; INC $9A,X ; and it ends in JMP $AE82.
        assert_eq!(rom.read_range(FS_BOOMBOOM_HITS_SUB, 5), &[0xFE, 0xD2, 0x7C, 0xF6, 0x9A]);
        assert_eq!(rom.read_range(FS_BOOMBOOM_HITS_SUB + 41, 3), &[
            0x4C,
            BOOMBOOM_DEATH_CPU as u8,
            (BOOMBOOM_DEATH_CPU >> 8) as u8,
        ]);

        // Threshold table: 16 entries, each a valid 1–5 hit count.
        let table = rom.read_range(FS_BOOMBOOM_HITS_TABLE, 16);
        for &v in table {
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

        // Heavy-physics compares reassigned to two distinct identities drawn
        // from the pool {0,1,2,3,4,6} (Lemmy/0x05 excluded).
        let a = rom.read_byte(KOOPALING_HEAVY_CMP_ROY);
        let b = rom.read_byte(KOOPALING_HEAVY_CMP_LUDWIG);
        assert_ne!(a, b, "heavy-physics identities must be distinct");
        for id in [a, b] {
            assert!(
                [0, 1, 2, 3, 4, 6].contains(&id),
                "heavy-physics identity 0x{id:02X} outside pool (Lemmy excluded)"
            );
        }

        // Ring attack: all three sites rewritten to the SAME identity, drawn
        // from the pool {0,1,2,3,4,6} (Lemmy/0x05 excluded).
        let ring: Vec<u8> = KOOPALING_RING_CMP_SITES
            .iter()
            .map(|&s| rom.read_byte(s))
            .collect();
        assert!(
            ring.iter().all(|&id| id == ring[0]),
            "ring sites must all hold the same identity, got {ring:02X?}"
        );
        assert!(
            [0, 1, 2, 3, 4, 6].contains(&ring[0]),
            "ring identity 0x{:02X} outside pool (Lemmy excluded)",
            ring[0]
        );
    }

    #[test]
    fn test_skip_wand_cutscene() {
        let mut rom = make_test_rom();
        rom.write_range(SKIP_WAND_CUTSCENE_OFFSET, &[0x00, 0x00]);
        skip_wand_cutscene(&mut rom);
        assert_eq!(rom.read_range(SKIP_WAND_CUTSCENE_OFFSET, 2), &[0x16, 0xB5]);
    }
}

#[cfg(test)]
mod asm_checks {
    //! Decode each assembled routine and check the structural properties no
    //! assembler was around to enforce. See [`crate::randomize::rom_data::asm`].
    use super::*;
    use crate::randomize::rom_data::asm;

    #[test]
    fn koopa_hits_is_well_formed() {
        asm::check(&KOOPA_HITS_CODE).allocation(crate::randomize::rom_data::FS_KOOPA_HITS_SUB).fragment().assert_ok();
    }

    fn vanilla() -> Option<Vec<u8>> {
        std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()
    }

    #[test]
    fn koopa_arena_mark_is_well_formed() {
        asm::check(&KOOPA_ARENA_MARK)
            .allocation(crate::randomize::rom_data::FS_KOOPA_ARENA_MARK)
            .origin(crate::randomize::rom_data::KOOPA_ARENA_MARK_CPU)
            .assert_ok();
    }

    #[test]
    fn koopa_grab_height_is_well_formed() {
        asm::check(&KOOPA_GRAB_HEIGHT)
            .allocation(crate::randomize::rom_data::FS_KOOPA_GRAB_HEIGHT)
            .origin(crate::randomize::rom_data::KOOPA_GRAB_HEIGHT_CPU)
            .assert_ok();
    }

    /// Both hooks displace exactly one whole instruction, and each `JSR` names
    /// its routine's origin. Needs the ROM, so it skips where the ROM is absent.
    #[test]
    fn grab_height_hooks_displace_whole_instructions() {
        let Some(v) = vanilla() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        let mark = crate::randomize::rom_data::KOOPA_ARENA_MARK_CPU.to_le_bytes();
        let mark_hook = [0x20, mark[0], mark[1]];
        asm::check(&KOOPA_ARENA_MARK)
            .origin(crate::randomize::rom_data::KOOPA_ARENA_MARK_CPU)
            .hook(&v, KOOPA_ARENA_MARK_SITE, &mark_hook)
            .assert_ok();

        let height = crate::randomize::rom_data::KOOPA_GRAB_HEIGHT_CPU.to_le_bytes();
        let height_hook = [0x20, height[0], height[1]];
        asm::check(&KOOPA_GRAB_HEIGHT)
            .origin(crate::randomize::rom_data::KOOPA_GRAB_HEIGHT_CPU)
            .hook(&v, KOOPA_GRAB_HEIGHT_SITE, &height_hook)
            .assert_ok();
    }

    /// The two hook sites, against the bytes the disassembly says are there.
    /// A drifted offset would silently splice a `JSR` into the wrong routine.
    #[test]
    fn grab_height_patch_sites_match_vanilla() {
        let Some(v) = vanilla() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        // ObjNorm_Koopaling ($AEC4): LDY World_Num
        assert_eq!(&v[KOOPA_ARENA_MARK_SITE..KOOPA_ARENA_MARK_SITE + 3], &[0xAC, 0x27, 0x07]);
        // StatusBar_Fill_Score ($B191): STA Player_Score, followed by STA <Temp_Var3.
        // The routine returns the high byte in A precisely so that next store lands it.
        assert_eq!(&v[KOOPA_GRAB_HEIGHT_SITE..KOOPA_GRAB_HEIGHT_SITE + 3], &[0x8D, 0x15, 0x07]);
        assert_eq!(&v[KOOPA_GRAB_HEIGHT_SITE + 3..KOOPA_GRAB_HEIGHT_SITE + 5], &[0x85, 0x02]);
    }

    /// Execute the routine on an emulated CPU. It is a self-contained
    /// calculation — it reads five RAM bytes, calls nothing, and returns — so
    /// unlike most patches here its *meaning* can be checked, not just its
    /// shape. Returns `(displayed_value, wrote_temps)`.
    fn run(arena: u8, wand_state: u8, y_hi: u8, y: u8, frac: u8) -> u32 {
        use mos6502::cpu::CPU;
        use mos6502::instruction::{Instruction, Ricoh2a03};
        use mos6502::memory::{Bus, Memory};
        use mos6502::Variant;

        let origin = crate::randomize::rom_data::KOOPA_GRAB_HEIGHT_CPU;
        let mut mem = Memory::new();
        mem.set_bytes(origin, &KOOPA_GRAB_HEIGHT);
        let mut cpu = CPU::new(mem, Ricoh2a03);

        cpu.memory.set_byte(0x0086, arena);
        cpu.memory.set_byte(0x07BD, wand_state);
        cpu.memory.set_byte(0x0087, y_hi);
        cpu.memory.set_byte(0x00A2, y);
        cpu.memory.set_byte(0x075F, frac);
        cpu.memory.set_byte(0x0000, 0xAA); // Temp_Var1 poison
        cpu.memory.set_byte(0x0001, 0xAA); // Temp_Var2 poison
        cpu.registers.program_counter = origin;

        // Bounded by the multiply loop: at most 9 iterations of ~10 instructions.
        for _ in 0..256 {
            let op = cpu.memory.get_byte(cpu.registers.program_counter);
            if matches!(Ricoh2a03::decode(op), Some((Instruction::RTS, _))) {
                let hi = u32::from(cpu.registers.accumulator);
                let mid = u32::from(cpu.memory.get_byte(0x0001));
                let lo = u32::from(cpu.memory.get_byte(0x0000));
                return hi * 65536 + mid * 256 + lo;
            }
            cpu.single_step();
        }
        panic!("routine never reached RTS");
    }

    /// The displayed number is `arena * 10000 + position`, where position is
    /// the vertical position in 1/16 px. Checked across the whole input space
    /// the routine can actually see in a Koopaling room.
    #[test]
    fn displayed_value_is_arena_prefix_plus_sixteenths() {
        for arena in 1..=8u8 {
            for y_hi in 0..=1u8 {
                for y in [0u8, 1, 15, 16, 127, 128, 254, 255] {
                    for frac in [0x00u8, 0x10, 0x50, 0xF0, 0xFF] {
                        let got = run(arena, 0, y_hi, y, frac);
                        let position =
                            (u32::from(y_hi) * 256 + u32::from(y)) * 16 + u32::from(frac >> 4);
                        let want = u32::from(arena) * 10000 + position;
                        assert_eq!(
                            got, want,
                            "arena={arena} yhi={y_hi} y={y} frac={frac:#04X}"
                        );
                    }
                }
            }
        }
    }

    /// Sub-pixel actually reaches the display: two positions one sixteenth of a
    /// pixel apart must differ by exactly one. This is what breaks ties between
    /// two players who grabbed on the same pixel.
    #[test]
    fn one_sixteenth_of_a_pixel_moves_the_number_by_one() {
        for frac in 0..15u8 {
            let a = run(3, 0, 0, 100, frac << 4);
            let b = run(3, 0, 0, 100, (frac + 1) << 4);
            assert_eq!(b - a, 1, "sub-pixel {frac} -> {} did not move the readout", frac + 1);
        }
        // ...and the low nibble of $075F is never significant: the engine only
        // ever accumulates into bits 7-4, so it must not perturb the reading.
        assert_eq!(run(3, 0, 0, 100, 0x50), run(3, 0, 0, 100, 0x5F));
    }

    /// Outside a Koopaling room the routine must be transparent: it returns the
    /// real score's high byte and leaves the converter's temps for vanilla.
    #[test]
    fn arena_flag_zero_leaves_the_score_alone() {
        use mos6502::cpu::CPU;
        use mos6502::instruction::{Instruction, Ricoh2a03};
        use mos6502::memory::{Bus, Memory};
        use mos6502::Variant;

        let origin = crate::randomize::rom_data::KOOPA_GRAB_HEIGHT_CPU;
        let mut mem = Memory::new();
        mem.set_bytes(origin, &KOOPA_GRAB_HEIGHT);
        let mut cpu = CPU::new(mem, Ricoh2a03);
        cpu.memory.set_byte(0x0086, 0x00); // not a Koopaling room
        cpu.memory.set_byte(0x0715, 0x42); // Player_Score high byte
        cpu.memory.set_byte(0x0000, 0xAA);
        cpu.memory.set_byte(0x0001, 0xBB);
        cpu.registers.program_counter = origin;
        cpu.registers.accumulator = 0x42; // as the displaced STA would have it

        for _ in 0..256 {
            let op = cpu.memory.get_byte(cpu.registers.program_counter);
            if matches!(Ricoh2a03::decode(op), Some((Instruction::RTS, _))) {
                assert_eq!(cpu.registers.accumulator, 0x42, "real score MSB not returned");
                assert_eq!(cpu.memory.get_byte(0x0000), 0xAA, "Temp_Var1 was clobbered");
                assert_eq!(cpu.memory.get_byte(0x0001), 0xBB, "Temp_Var2 was clobbered");
                return;
            }
            cpu.single_step();
        }
        panic!("routine never reached RTS");
    }

    /// From wand state 3 on, the reading is frozen: moving the player must not
    /// change it. This is the whole point — the number you read is the number
    /// at the grab.
    #[test]
    fn reading_freezes_once_the_wand_is_grabbed() {
        use mos6502::cpu::CPU;
        use mos6502::instruction::{Instruction, Ricoh2a03};
        use mos6502::memory::{Bus, Memory};
        use mos6502::Variant;

        let origin = crate::randomize::rom_data::KOOPA_GRAB_HEIGHT_CPU;
        let mut mem = Memory::new();
        mem.set_bytes(origin, &KOOPA_GRAB_HEIGHT);
        let mut cpu = CPU::new(mem, Ricoh2a03);
        cpu.memory.set_byte(0x0086, 5); // arena 5

        let step = |cpu: &mut CPU<Memory, Ricoh2a03>, state: u8, y: u8, frac: u8| -> u32 {
            cpu.memory.set_byte(0x07BD, state);
            cpu.memory.set_byte(0x0087, 0);
            cpu.memory.set_byte(0x00A2, y);
            cpu.memory.set_byte(0x075F, frac);
            cpu.registers.program_counter = origin;
            for _ in 0..256 {
                let op = cpu.memory.get_byte(cpu.registers.program_counter);
                if matches!(Ricoh2a03::decode(op), Some((Instruction::RTS, _))) {
                    return u32::from(cpu.registers.accumulator) * 65536
                        + u32::from(cpu.memory.get_byte(0x0001)) * 256
                        + u32::from(cpu.memory.get_byte(0x0000));
                }
                cpu.single_step();
            }
            panic!("routine never reached RTS");
        };

        // Fighting: the readout tracks the player.
        let mid_fight = step(&mut cpu, 0, 100, 0x40);
        assert_eq!(mid_fight, 5 * 10000 + 100 * 16 + 4);
        // The grab frame sets the value that must survive.
        let at_grab = step(&mut cpu, 0, 60, 0x80);
        assert_eq!(at_grab, 5 * 10000 + 60 * 16 + 8);
        // Wand grabbed (state 3+): the player falls, the reading must not move.
        for (state, y) in [(3u8, 90u8), (4, 150), (5, 200), (6, 220), (7, 255)] {
            assert_eq!(step(&mut cpu, state, y, 0xF0), at_grab, "state {state} moved the reading");
        }
    }

    /// The displayed number must never reach the 6-digit overflow the vanilla
    /// converter guards with `CMP #$FA`. Worst case is world 9 at the bottom of
    /// a two-page room, and even that stays under 100000.
    #[test]
    fn displayed_value_cannot_overflow_the_score_field() {
        let max_position = (1u32 << 13) - 1; // (YHi:Y) << 4 | frac, YHi <= 1
        assert_eq!(max_position, 8191);
        let max_displayed = 9 * 10000 + max_position; // World_Num 8 -> arena id 9
        assert!(max_displayed < 100000, "{max_displayed} would overflow to all-9s");
    }
}

// ---------------------------------------------------------------------------
// Koopaling grab-height readout
// ---------------------------------------------------------------------------

/// Replace the score field with the player's vertical position during a
/// Koopaling fight, frozen at the moment the wand is grabbed.
///
/// The point is a per-arena competition: with the wand cutscene skipped (see
/// [`skip_wand_cutscene`]) the player jumps for the wand, and this reports
/// exactly where they caught it so runs can be compared. Raw Y is deliberate —
/// it needs no per-room floor constant, so the number means the same thing in
/// every arena. It counts *down*: a smaller number is a higher grab.
///
/// # The value
///
/// SMB3 has no player-Y fractional byte, but it does have sub-pixel precision.
/// `Object_AddVelFrac` (PRG000 $DCFB) takes the 4.4FP velocity, shifts the
/// fractional nibble up into bits 7-4, accumulates it in `Player_YVelFrac`
/// ($075F) and carries into the pixel byte. Despite its name that byte is the
/// sub-*pixel position*, so the full position is:
///
/// ```text
///     Player_YHi ($87) : Player_Y ($A2) . Player_YVelFrac ($075F) >> 4
/// ```
///
/// i.e. 1/16-pixel resolution. We display it in 1/16-px units, which is exact
/// and lossless (ties break in the sub-pixel) and spans 0..8191 — four digits.
/// The score field has six, so the top two carry the arena number:
///
/// ```text
///     displayed = (World_Num + 1) * 10000 + position
/// ```
///
/// The status bar appends a fixed `0` tile of its own, so world 5 at position
/// 4720 reads `0514720`. That trailing zero is part of the static status-bar
/// layout, not one of the six buffered tiles, so it cannot be dropped without
/// degrading the real score display everywhere.
///
/// # Why the score is never touched
///
/// `StatusBar_Fill_Score` (PRG026 $B175) folds `Score_Earned` into
/// `Player_Score`, copies the three bytes to `Temp_Var1/2/3`, and then converts
/// **from the temps**. Hooking after the copy and overwriting the temps leaves
/// `Player_Score` completely alone — the real score keeps accumulating
/// underneath and reappears by itself when the flag clears. There is no backup
/// and no restore to get wrong.
///
/// The hook displaces `STA Player_Score` (3 bytes) and the routine performs
/// that store itself, then returns the value's high byte in `A` so vanilla's
/// very next instruction — `STA <Temp_Var3` — does the third store for free.
///
/// # Arena detection and the latch
///
/// `ObjNorm_Koopaling` (PRG001 $AEC4) opens with `LDY World_Num`, another whole
/// 3-byte instruction, so the marker hook displaces it and reuses the same load
/// for the arena id. It stamps `$86` — unused zero page — with `World_Num + 1`
/// every frame the Koopaling AI runs. Zero page is cleared by
/// `Clear_RAM_thru_ZeroPage` on every level exit, so the flag needs no clear
/// hook of its own.
///
/// `Level_GetWandState` ($07BD) supplies the freeze: state 3 is "wand grabbed".
/// Below 3 the latch is recomputed each frame; from 3 on it is only replayed,
/// so the reading holds through the grab flash, the time bonus and the fall.
/// Merely *stopping* the overwrite would snap the display back to the real
/// score instead of freezing, which is why the value is latched into
/// `$0758`/`$0759` rather than recomputed.
const KOOPA_ARENA_MARK_SITE: usize = 0x02ED4; // ObjNorm_Koopaling: LDY World_Num
const KOOPA_GRAB_HEIGHT_SITE: usize = 0x351A1; // StatusBar_Fill_Score: STA Player_Score

/// Marker: stamp the arena id, restore `Y`, return. 8 bytes.
#[rustfmt::skip]
const KOOPA_ARENA_MARK: [u8; 8] = [
    0xAC, 0x27, 0x07, // LDY World_Num     ; displaced instruction
    0xC8,             // INY               ; 1-based, so 0 stays "not an arena"
    0x84, 0x86,       // STY $86           ; arena id (zero page: 2 bytes, auto-cleared)
    0x88,             // DEY               ; restore Y = World_Num for the caller
    0x60,             // RTS
];

/// Score-field override. 101 bytes.
#[rustfmt::skip]
const KOOPA_GRAB_HEIGHT: [u8; 101] = [
    0x8D, 0x15, 0x07,       // STA Player_Score      ; displaced — real score preserved
    0xA4, 0x86,             // LDY $86               ; arena id; 0 = not a Koopaling room
    0xF0, 0x5A,             // BEQ out               ; -> vanilla behaviour
    0xAD, 0xBD, 0x07,       // LDA Level_GetWandState
    0xC9, 0x03,             // CMP #$03
    0xB0, 0x2B,             // BCS show              ; wand grabbed -> replay frozen latch
    // latch = (YHi:Y) << 4 | (frac >> 4)   — position in 1/16 px
    0xA5, 0xA2,             // LDA <Player_Y
    0x0A, 0x0A, 0x0A, 0x0A, // ASL A x4              ; Y << 4
    0x8D, 0x58, 0x07,       // STA $0758
    0xAD, 0x5F, 0x07,       // LDA Player_YVelFrac   ; sub-pixel in bits 7-4
    0x4A, 0x4A, 0x4A, 0x4A, // LSR A x4              ; -> 0..15
    0x0D, 0x58, 0x07,       // ORA $0758
    0x8D, 0x58, 0x07,       // STA $0758             ; latch lo
    0xA5, 0xA2,             // LDA <Player_Y
    0x4A, 0x4A, 0x4A, 0x4A, // LSR A x4              ; Y >> 4
    0x8D, 0x59, 0x07,       // STA $0759
    0xA5, 0x87,             // LDA <Player_YHi
    0x0A, 0x0A, 0x0A, 0x0A, // ASL A x4              ; YHi << 4
    0x0D, 0x59, 0x07,       // ORA $0759
    0x8D, 0x59, 0x07,       // STA $0759             ; latch hi
    // show: publish the latch into the converter's temps
    0xAD, 0x58, 0x07,       // LDA $0758
    0x85, 0x00,             // STA <Temp_Var1
    0xAD, 0x59, 0x07,       // LDA $0759
    0x85, 0x01,             // STA <Temp_Var2
    0xA9, 0x00,             // LDA #$00
    0x8D, 0x5A, 0x07,       // STA $075A             ; 24-bit high accumulator
    // value += 10000 * arena_id  (Y = 1..8, so at most 8 adds)
    0xA5, 0x00,             // mul: LDA <Temp_Var1
    0x18,                   // CLC
    0x69, 0x10,             // ADC #<10000
    0x85, 0x00,             // STA <Temp_Var1
    0xA5, 0x01,             // LDA <Temp_Var2
    0x69, 0x27,             // ADC #>10000
    0x85, 0x01,             // STA <Temp_Var2
    0x90, 0x03,             // BCC nc
    0xEE, 0x5A, 0x07,       // INC $075A
    0x88,                   // nc: DEY
    0xD0, 0xEB,             // BNE mul
    0xAD, 0x5A, 0x07,       // LDA $075A             ; -> vanilla's STA <Temp_Var3
    0x60,                   // RTS
    0xAD, 0x15, 0x07,       // out: LDA Player_Score ; real MSB for vanilla's STA <Temp_Var3
    0x60,                   // RTS
];

pub fn koopaling_grab_height(rom: &mut Rom) {
    use super::rom_data::{
        FS_KOOPA_ARENA_MARK, FS_KOOPA_GRAB_HEIGHT, KOOPA_ARENA_MARK_CPU, KOOPA_GRAB_HEIGHT_CPU,
    };

    rom.write_range(FS_KOOPA_ARENA_MARK, &KOOPA_ARENA_MARK);
    rom.write_range(FS_KOOPA_GRAB_HEIGHT, &KOOPA_GRAB_HEIGHT);

    let mark = KOOPA_ARENA_MARK_CPU.to_le_bytes();
    rom.write_range(KOOPA_ARENA_MARK_SITE, &[0x20, mark[0], mark[1]]); // JSR arena_mark

    let height = KOOPA_GRAB_HEIGHT_CPU.to_le_bytes();
    rom.write_range(KOOPA_GRAB_HEIGHT_SITE, &[0x20, height[0], height[1]]); // JSR grab_height
}
