//! Central registry of ROM free-space allocations (assembled code + data
//! tables). The overlap test guards against collisions.

/// Free space allocation: (file_offset, size_bytes, label).
/// The overlap test checks that no two allocations in this list share any bytes.
#[cfg(test)]
pub(crate) const FREE_SPACE_ALLOCATIONS: &[(usize, usize, &str)] = &[
    // PRG030 (fixed bank, always mapped $8000–$9FFF, file 0x3C010)
    (0x3DF20, 28, "world_order: routine + tables"),
    (0x3DF3C, 20, "big_q_block: save obj_ptr trampoline"),
    // PRG031 (always mapped $E000–$FFFF, file 0x3E010)
    (0x3E924, 25, "title_screen: sprite copy routine"),
    (0x3E93D, 40, "title_screen: sprite data table"),
    (0x35572, 13, "mystery_anchor: item redirect trampoline"),
    (0x3557F, 50, "hammer_locks: tile check subroutine + tables"),
    (0x3E260, 33, "starting_items: lives + intro skip + menu music + inventory init trampoline"),
    (0x3E281, 69, "start_airship_swap: 4 tables (X/XHi/ScrL/ScrH × 8) + Map_Init seed helper"),
    (0x3E965, 13, "title_screen: intro skip + menu music routine"),
    (0x3FFF0, 26, "card_speed_clear: XOR trampoline"),
    // PRG026 (file 0x34010, CPU $A000–$BFFF)
    (0x35530, 66, "big_q_block: lookup routine + tables"),
    (0x355B1, 12, "anchor_visuals: items-vs-cards index guard trampoline"),
    // PRG027 (file 0x36010, CPU $A000–$BFFF)
    (0x379D9, 894, "king_quotes: 7 quotes + hook (7×120 + 54)"),
    // PRG010 (file 0x14010, CPU $C000–$DFFF during map)
    (0x15554, 80, "fx_screen_check: cross-screen lock patch (Fred's algorithm verbatim)"),
    (0x15DF0, 35, "canoe_fix: death respawn position save"),
    // PRG011 (file 0x16010, CPU $A000–$BFFF during map)
    (0x17C87, 36, "start_airship_swap: game-over twirl finalize helper"),
    (0x17D00, 66, "canoe_fix: backup/restore subroutines (CANOE_BACKUP_ROUTINE)"),
    (0x17D42, 8, "bros_no_hands: hand-trap tile bypass for overworld bro movement gate"),
    // PRG001 (file 0x02010, CPU $A000–$BFFF)
    (0x0382A, 23, "koopa_hits: subroutine + defeat JMP + threshold table"),
    (0x03841, 13, "koopa_collision_guard: skip collision bitmap during invuln"),
    (0x0384E, 16, "koopa_vram_clear: clear VRAM buffer on defeat"),
    (0x0385E, 12, "koopa_fire_preset: set stomp counter from threshold table for fireball defeat"),
    (0x03FD0, 22, "koopa_y_clamp: clamp Koopaling Y position to screen"),
    (0x03FE6, 36, "fire_flower: position-hash suit routine + pool table"),
    // PRG003 (file 0x06010, CPU $A000–$BFFF) — object AI bank (Boom-Boom lives here)
    (0x07FCF, 16, "boomboom_hits: per-fortress threshold table"),
    (0x07FDF, 44, "boomboom_hits: decoupled stomp-count subroutine"),
    // PRG006 (file 0x0C010, CPU $C000–$DFFF) — level enemy data bank
    (0x0DA74, 22, "hand_rooms: 2 cloned enemy streams for unique 8-Hnd treasure rooms"),
    // PRG029 (file 0x3A010, CPU $C000–$DFFF) — swim physics bank
    (0x3A600, 24, "faster_frog: Frog-Suit swim-speed boost routine"),
];

// PRG030
pub(crate) const FS_WORLD_ORDER: usize       = 0x3DF20; // 28 bytes

pub(crate) const FS_BIG_Q_SAVE: usize        = 0x3DF3C; // 20 bytes

// PRG031
pub(crate) const FS_SEED_HASH_ROUTINE: usize = 0x3E924; // 25 bytes

pub(crate) const FS_SEED_HASH_DATA: usize    = 0x3E93D; // 40 bytes

pub(crate) const FS_INTRO_SKIP: usize        = 0x3E965; // 13 bytes

pub(crate) const FS_CARD_CLEAR: usize        = 0x3FFF0; // 26 bytes

// PRG031 — start_airship_swap engine scaffolding. One ~69-byte block: 4 × 8-byte
// per-world tables followed by a single assembled seed subroutine. PRG031 is
// always-mapped at $E000-$FFFF so Map_Init / GameOver_TwirlToStart (PRG011) can
// read the tables regardless of which bank is at $A000. NOTE: the PRG031 free run
// at 0x3E281 ends at 0x3E2D0 (real code follows) — only 79 bytes; do not grow this
// block past that ceiling.
pub(crate) const FS_SAS_BLOCK: usize             = 0x3E281;       // 69 bytes used (79 max)

pub(crate) const FS_SAS_X_TABLE: usize           = FS_SAS_BLOCK;       // 8 bytes — Mario X-low pixel per world

pub(crate) const FS_SAS_XHI_TABLE: usize         = FS_SAS_BLOCK + 8;   // 8 bytes — Mario screen index per world

pub(crate) const FS_SAS_SCRL_TABLE: usize        = FS_SAS_BLOCK + 16;  // 8 bytes — camera scroll low per world ($0722 / $7986)

pub(crate) const FS_SAS_SCRH_TABLE: usize        = FS_SAS_BLOCK + 24;  // 8 bytes — camera scroll high per world ($0724 / $7988)

// Single Map_Init seed subroutine: writes Mario's start position plus the primary
// AND secondary scroll backups from the four tables (replaces the former x/xhi
// helper pair). Reached via `JSR` from the Map_Init scroll-store site.
pub(crate) const FS_SAS_SEED_HELPER: usize       = FS_SAS_BLOCK + 32;  // 37 bytes

// The game-over twirl finalize helper lives in PRG011 free space (not FS_SAS_BLOCK
// — that PRG031 run has no room for it). PRG011 is the hook's own bank, so the JSR
// is bank-local; the helper still reads the FS_SAS_* tables in always-resident
// PRG031.
pub(crate) const FS_SAS_GAMEOVER_FINALIZE: usize = 0x17C87;  // PRG011, 36 bytes — stamps World_Map_X/XHi + primary/secondary scroll backup + live Horz_Scroll/Hi at twirl finalize (clean gap before FS_CANOE_BACKUP)

// Vanilla 8-byte Map_Y_Starts table (per-world Mario spawn Y-pixel). Lives in
// PRG030's world-enter routine. The start_airship_swap module rewrites this
// in place so swapped worlds spawn Mario at the airship row instead of the
// vanilla start row.
pub(crate) const MAP_Y_STARTS_OFF: usize  = 0x3C39A;

// Map_Init inline patch site in PRG011 (CPU $A237). The start_airship_swap module
// replaces the vanilla `STA $0724,X` scroll-store with `JSR seed_helper`, which
// overwrites the whole start position + scroll block from the FS_SAS_* tables. The
// earlier vanilla `LDA #$20 / STA $797A,X / STA $7982,X` X-low store at 0x16257 is
// left intact — the seed helper re-stamps $797A/$7982 later in the same loop
// iteration (before any draw), so the vanilla value is harmlessly overwritten.
pub(crate) const MAP_INIT_SCROLL_SITE: usize = 0x1627E;   // 3 bytes — `STA $0724,X`

// GameOver_TwirlToStart finalize hook in PRG011 (CPU $A6AA). The twirl is a
// delta-animation: it spirals Mario back to the start by a per-frame X/Y delta,
// then at finalize copies World_Map_X/XHi/Y into Map_Previous_*. The delta is
// low-byte/within-screen only and has a second hardcoded column-2 ($20) for the
// skid direction, so a swapped start on a different column/screen is unreachable
// by patching the delta. Instead we let the vanilla animation play and STAMP the
// correct start position at finalize: replace `STA Map_Prev_XHi2,X` (the last
// store before the World_Map → Map_Previous copies) with `JSR finalize helper`,
// which overwrites World_Map_X ($79,X) / World_Map_XHi ($77,X), the camera scroll
// ($0722/$0724,X) and both secondary backups ($7986/$7988) from the FS_SAS_*
// tables. The displaced `STA $7988` (A=0) is intentionally dropped: the helper now
// stamps $7988 with the start screen instead of zeroing it (nothing between the
// hook and the following copies reads it).
pub(crate) const GAMEOVER_FINALIZE_SITE: usize = 0x166BA;  // 3 bytes — `STA $7988,X` (Map_Prev_XHi2,X)

// Map_Object slot 1 == the airship sprite per southbird's disassembly:
// "NOTE: Assumes Index 1 is the Airship!"
pub(crate) const AIRSHIP_OBJ_SLOT: usize = 1;

// PRG026
pub(crate) const FS_BIG_Q_LOOKUP: usize      = 0x35530; // 66 bytes

// PRG027
pub(crate) const FS_KING_QUOTES: usize       = 0x379D9; // 894 bytes

// PRG010
pub(crate) const FS_FX_SCREEN_CHECK: usize   = 0x15554; // 80 bytes (Fred's algorithm)

pub(crate) const FS_CANOE_RESPAWN: usize     = 0x15DF0; // 35 bytes

// PRG011
pub(crate) const FS_CANOE_BACKUP: usize      = 0x17D00; // 59 bytes

// 8-byte hand-trap bypass subroutine for the overworld bro movement gate
// (MaCobra52's "Bros don't stop on hands"). CPU $BD42. Sits just past the
// 66-byte FS_CANOE_BACKUP reservation; the gate hook at $B425 JSRs here.
pub(crate) const FS_BROS_NO_HANDS: usize     = 0x17D42; // 8 bytes (CPU $BD42)

// PRG026 (cont.)
pub(crate) const FS_MYSTERY_ANCHOR: usize    = 0x35572; // 13 bytes

pub(crate) const FS_HAMMER_LOCKS: usize      = 0x3557F; // 50 bytes

pub(crate) const FS_ANCHOR_ITEM_GUARD: usize = 0x355B1; // 12 bytes (CPU $B5A1)

pub(crate) const FS_STARTING_ITEMS: usize    = 0x3E260; // 33 bytes

// PRG001 (file 0x02010, CPU $A000–$BFFF)
// Koopaling stomp handler is ObjHit_Koopaling in prg001.asm (southbird disassembly).
pub(crate) const FS_KOOPA_HITS_SUB: usize    = 0x0382A; // 13 code + 3 JMP + 7 table = 23 bytes

pub(crate) const FS_KOOPA_HITS_TABLE: usize  = 0x0383A; // 7 bytes (sub + 16)

/// CPU address of the subroutine: $A000 + (0x0382A - 0x02010) = $B81A
pub(crate) const KOOPA_HITS_SUB_CPU: u16     = 0xB81A;

/// CPU address of the threshold table: $A000 + (0x0383A - 0x02010) = $B82A
pub(crate) const KOOPA_HITS_TABLE_CPU: u16   = 0xB82A;

// Koopaling collision guard — skip collision bitmap update during invulnerability.
// Source: Fred's Koopaling fixes.
pub(crate) const FS_KOOPA_COLLISION_GUARD: usize = 0x03841; // 13 bytes

pub(crate) const KOOPA_COLLISION_GUARD_CPU: u16  = 0xB831;  // $A000 + (0x03841 - 0x02010)

// Koopaling defeat VRAM buffer clear — zero $0300/$0301 on defeat to prevent
// stale PPU writes during wand/king transition in non-native worlds.
// Source: Fred's Koopaling fixes.
pub(crate) const FS_KOOPA_VRAM_CLEAR: usize = 0x0384E; // 16 bytes

pub(crate) const KOOPA_VRAM_CLEAR_CPU: u16  = 0xB83E;  // $A000 + (0x0384E - 0x02010)

// Koopaling Y-position clamp — keep bouncing Koopalings on screen in non-native rooms.
// Source: Fred's Koopaling fixes.
pub(crate) const FS_KOOPA_Y_CLAMP: usize = 0x03FD0; // 22 bytes

pub(crate) const KOOPA_Y_CLAMP_CPU: u16  = 0xBFC0;  // $A000 + (0x03FD0 - 0x02010)

// Random Fire Flower (issue #22) — injected routine that derives the granted
// power state from a seed-derived salt (the shuffled starting world) + the
// current World_Num + the level layout pointer + the flower's screen number,
// instead of the vanilla hardcoded Fire. Sits in the PRG001 bank-end gap right
// after koopa_y_clamp (which ends at 0x3FE6). Up to 36 bytes: 26-byte routine +
// a 4- or 6-byte pool table. ObjHit_FireFlower runs with PRG001 banked at
// $A000, so the JSR from the hook is bank-local.
pub(crate) const FS_FIRE_FLOWER: usize     = 0x03FE6;

pub(crate) const FIRE_FLOWER_SUB_CPU: u16  = 0xBFD6; // $A000 + (0x03FE6 - 0x02010)

// Fireball defeat preset — load per-world stomp threshold from table so the
// fireball→stomp handoff always triggers defeat after INC.
pub(crate) const FS_KOOPA_FIRE_PRESET: usize = 0x0385E; // 12 bytes

pub(crate) const KOOPA_FIRE_PRESET_CPU: u16  = 0xB84E;  // $A000 + (0x0385E - 0x02010)

// PRG003 (file 0x06010, CPU $A000–$BFFF) — Boom-Boom stomp-count randomization.
// The Boom-Boom boss AI (ObjInit_BoomBoom, BoomBoom_HitTest, the DynJump state
// machine) lives in this bank, so the stomp handler's JMP into these routines is
// bank-local. Both allocations sit in the bank-end filler gap ($BFBF–$BFFF).
//
// Layout: 16-byte threshold table first, then the 44-byte subroutine.
pub(crate) const FS_BOOMBOOM_HITS_TABLE: usize = 0x07FCF; // 16 bytes (CPU $BFBF)

pub(crate) const BOOMBOOM_HITS_TABLE_CPU: u16  = 0xBFBF;  // $A000 + (0x07FCF - 0x06010)

pub(crate) const FS_BOOMBOOM_HITS_SUB: usize   = 0x07FDF; // 44 bytes (CPU $BFCF)

pub(crate) const BOOMBOOM_HITS_SUB_CPU: u16    = 0xBFCF;  // $A000 + (0x07FDF - 0x06010)

// PRG006 — duplicated enemy streams for the W8 Hand sub-areas. Each clone is
// 11 bytes (page byte + 3 enemy entries + 0xFF terminator); two clones give
// the three Hand levels independent OBJ_TREASURESET item bytes.
pub(crate) const FS_HAND_ROOMS: usize = 0x0DA74; // 22 bytes (2 × 11)

// PRG029 (file 0x3A010, CPU $C000–$DFFF) — Frog-Suit swim-speed boost routine
// reached by a bank-local JSR $C5F0 from the swim-physics code. 24 bytes.
pub(crate) const FS_FASTER_FROG: usize = 0x3A600; // CPU $C5F0

#[cfg(test)]
mod free_space_tests {
    use super::super::*;

    #[test]
    fn test_free_space_no_overlap() {
        for (i, &(a_off, a_sz, a_label)) in FREE_SPACE_ALLOCATIONS.iter().enumerate() {
            let a_end = a_off + a_sz;
            for &(b_off, b_sz, b_label) in &FREE_SPACE_ALLOCATIONS[i + 1..] {
                let b_end = b_off + b_sz;
                assert!(
                    a_end <= b_off || b_end <= a_off,
                    "free space overlap: '{}' (0x{:05X}..0x{:05X}) vs '{}' (0x{:05X}..0x{:05X})",
                    a_label, a_off, a_end, b_label, b_off, b_end,
                );
            }
        }
    }

    #[test]
    fn test_free_space_constants_match_registry() {
        let offsets: Vec<usize> = FREE_SPACE_ALLOCATIONS.iter().map(|&(o, _, _)| o).collect();
        assert!(offsets.contains(&FS_WORLD_ORDER));
        assert!(offsets.contains(&FS_BIG_Q_SAVE));
        assert!(offsets.contains(&FS_SEED_HASH_ROUTINE));
        assert!(offsets.contains(&FS_SEED_HASH_DATA));
        assert!(offsets.contains(&FS_INTRO_SKIP));
        assert!(offsets.contains(&FS_CARD_CLEAR));
        assert!(offsets.contains(&FS_BIG_Q_LOOKUP));
        assert!(offsets.contains(&FS_KING_QUOTES));
        assert!(offsets.contains(&FS_FX_SCREEN_CHECK));
        assert!(offsets.contains(&FS_CANOE_RESPAWN));
        assert!(offsets.contains(&FS_CANOE_BACKUP));
        assert!(offsets.contains(&FS_KOOPA_HITS_SUB));
        assert!(offsets.contains(&FS_BOOMBOOM_HITS_TABLE));
        assert!(offsets.contains(&FS_BOOMBOOM_HITS_SUB));
        assert!(offsets.contains(&FS_STARTING_ITEMS));
        assert!(offsets.contains(&FS_MYSTERY_ANCHOR));
        assert!(offsets.contains(&FS_HAMMER_LOCKS));
    }

    // Ground-truth pins for the PRG bank ↔ file-offset mapping. Each pair is a
    // *known-correct* (bank, cpu, file) triple taken from a shipped patch, so
    // these tests catch drift in the mapping itself — including dropping the
    // 0x10 iNES header, the issue #14 root cause. Patch code derives operands
    // from these helpers (e.g. `jsr_into_bank`) rather than transcribing
    // address bytes, so getting the helpers right protects every hook.
    #[test]
    fn test_prg_bank_mapping_known_pairs() {
        // PRG011 canoe backup routine: file 0x17D00 ↔ CPU $BCF0 (JSR $BCF0).
        assert_eq!(prg_bank_cpu_to_file(11, 0xBCF0), 0x17D00);
        assert_eq!(prg_bank_file_to_cpu(11, 0x17D00), 0xBCF0);
        // Bank start maps to the window base, header included.
        assert_eq!(prg_bank_cpu_to_file(11, 0xA000), 0x16010);
        assert_eq!(prg_bank_file_to_cpu(11, 0x16010), 0xA000);
        // Round-trips across banks and the whole window.
        for bank in [9usize, 11, 13, 26] {
            for cpu in [0xA000u16, 0xA001, 0xB425, 0xBD32, 0xBFFF] {
                assert_eq!(prg_bank_file_to_cpu(bank, prg_bank_cpu_to_file(bank, cpu)), cpu);
            }
        }
    }

    #[test]
    fn test_jsr_into_bank_builds_correct_operand() {
        // `JSR <file 0x17D00 in bank 11>` must encode as 20 F0 BC (JSR $BCF0).
        assert_eq!(jsr_into_bank(11, 0x17D00), [0x20, 0xF0, 0xBC]);
        // Opcode is always JSR; operand is little-endian CPU address.
        let j = jsr_into_bank(11, FS_BROS_NO_HANDS);
        assert_eq!(j[0], 0x20);
        let cpu = u16::from_le_bytes([j[1], j[2]]);
        assert_eq!(prg_bank_cpu_to_file(11, cpu), FS_BROS_NO_HANDS);
    }
}
