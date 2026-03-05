use rand::Rng;
use rand::seq::IndexedRandom;

use crate::rom::Rom;

/// Enemy/object data block: 0x0BFD8–0x0E00D.
///
/// Format: each level's enemy set is a sequence of segments separated by 0xFF.
/// Each segment starts with a 1-byte page flag, then zero or more 3-byte
/// entries: [object_id, x_pos, y_pos], terminated by 0xFF.
///
/// We parse this structure properly and only randomize the object_id byte
/// of entries whose ID is in our explicit allowlist of swappable enemies.
const ENEMY_DATA_START: usize = 0x0BFD8;
const ENEMY_DATA_END: usize = 0x0E00D;

// Object IDs from the Southbird SMB3 disassembly (smb3.asm).
// Only IDs that are actual enemies safe to swap are included.
// Special objects (end-level card, pipes, platforms, bosses, powerups,
// autoscroll, event triggers, cannons, etc.) are NOT listed and will
// never be modified.

/// Ground-walking enemies (no shell). These can be freely swapped with each other.
const GROUND_ENEMIES: &[u8] = &[
    0x29, // OBJ_SPIKE
    0x2A, // OBJ_PATOOIE
    0x33, // OBJ_NIPPER (stationary)
    0x39, // OBJ_NIPPERHOPPING
    0x3F, // OBJ_DRYBONES
    0x40, // OBJ_BUSTERBEATLE
    0x55, // OBJ_BOBOMB
    0x6B, // OBJ_PILEDRIVER (micro goomba)
    0x71, // OBJ_SPINY
    0x72, // OBJ_GOOMBA
];

/// Shell-producing enemies — kept in their own class because some levels require
/// shells to progress. Swapping these with non-shell enemies could make levels unbeatable.
const SHELL_ENEMIES: &[u8] = &[
    0x6C, // OBJ_GREENTROOPA
    0x6D, // OBJ_REDTROOPA
    0x70, // OBJ_BUZZYBEATLE
];

/// Big enemies (Giant World variants). Swap only among themselves.
const BIG_ENEMIES: &[u8] = &[
    0x7A, // OBJ_BIGGREENTROOPA
    0x7B, // OBJ_BIGREDTROOPA
    0x7C, // OBJ_BIGGOOMBA
    0x7E, // OBJ_BIGGREENHOPPER
];

/// Flying/hopping enemies that can be swapped with each other.
const FLYING_ENEMIES: &[u8] = &[
    0x6E, // OBJ_PARATROOPAGREENHOP
    0x6F, // OBJ_FLYINGREDPARATROOPA
    0x73, // OBJ_PARAGOOMBA
    0x74, // OBJ_PARAGOOMBAWITHMICROS
    0x80, // OBJ_FLYINGGREENPARATROOPA
];

/// Water enemies that can be swapped with each other.
const WATER_ENEMIES: &[u8] = &[
    0x61, // OBJ_BLOOPERWITHKIDS
    0x62, // OBJ_BLOOPER
    0x63, // OBJ_BIGBERTHABIRTHER
    0x64, // OBJ_CHEEPCHEEPHOPPER
    0x6A, // OBJ_BLOOPERCHILDSHOOT
];

/// Hammer/Boomerang/Fire Bros — swap among themselves.
const BRO_ENEMIES: &[u8] = &[
    0x81, // OBJ_HAMMERBRO
    0x82, // OBJ_BOOMERANGBRO
    0x86, // OBJ_HEAVYBRO
    0x87, // OBJ_FIREBRO
];

/// Piranha plant variants — swap among themselves.
const PIRANHAS: &[u8] = &[
    0xA0, // OBJ_GREENPIRANHA
    0xA2, // OBJ_REDPIRANHA
    0xA4, // OBJ_GREENPIRANHA_FIRE
    0xA6, // OBJ_VENUSFIRETRAP
];
/// Piranha Ceiling / Flipped variants
const PIRANHASC: &[u8] = &[
    0xA1, // OBJ_GREENPIRANHA_FLIPPED
    0xA3, // OBJ_REDPIRANHA_FLIPPED
    0xA5, // OBJ_GREENPIRANHA_FIREC
    0xA7, // OBJ_VENUSFIRETRAP_CEIL
];

/// Cheep cheep variants (overworld jumping types).
const CHEEPS: &[u8] = &[
    0x77, // OBJ_GREENCHEEP
    0x88, // OBJ_ORANGECHEEP
];

// ---------------------------------------------------------------------------
// CHR sprite bank data (from Southbird disassembly ObjectGroup PatTableSel)
// ---------------------------------------------------------------------------
//
// Each enemy requests a 1KB CHR page be loaded into one of two sprite bank
// slots: PatTable_BankSel+4 (PPU $1800-$1BFF) or +5 (PPU $1C00-$1FFF).
// If two on-screen enemies request different CHR pages for the same slot,
// one renders with garbled sprites (the last one rendered wins).
//
// We track CHR page commitments per enemy data segment (= one level area)
// and only allow swaps that are compatible with already-committed pages.

/// CHR sprite bank requirement for an enemy.
struct SpriteBank {
    chr_page: u8, // CHR ROM page number
    slot: u8,     // 4 or 5 (PatTable_BankSel index)
}

/// Look up the CHR sprite bank requirement for a swappable enemy.
/// Returns `None` for enemies that use NOCHANGE (no bank switch).
fn sprite_bank(id: u8) -> Option<SpriteBank> {
    match id {
        // GROUND — page $0A, slot +4
        0x29 | 0x2A | 0x33 | 0x39 | 0x40 | 0x55 =>
            Some(SpriteBank { chr_page: 0x0A, slot: 4 }),
        // GROUND — page $0B, slot +4 (Spiny)
        0x71 => Some(SpriteBank { chr_page: 0x0B, slot: 4 }),
        // GROUND — page $13, slot +5 (Dry Bones)
        0x3F => Some(SpriteBank { chr_page: 0x13, slot: 5 }),
        // GROUND — page $4F, slot +5 (Goomba, Piledriver)
        0x6B | 0x72 => Some(SpriteBank { chr_page: 0x4F, slot: 5 }),
        // SHELL — page $0B, slot +4 (Buzzy Beetle)
        0x70 => Some(SpriteBank { chr_page: 0x0B, slot: 4 }),
        // SHELL — page $4F, slot +5 (Koopas)
        0x6C | 0x6D => Some(SpriteBank { chr_page: 0x4F, slot: 5 }),
        // BIG — all page $3D, slot +4
        0x7A | 0x7B | 0x7C | 0x7E =>
            Some(SpriteBank { chr_page: 0x3D, slot: 4 }),
        // FLYING — all page $4F, slot +5
        0x6E | 0x6F | 0x73 | 0x74 | 0x80 =>
            Some(SpriteBank { chr_page: 0x4F, slot: 5 }),
        // WATER — page $1A, slot +4 (Bloopers, Big Bertha)
        0x61 | 0x62 | 0x63 | 0x6A =>
            Some(SpriteBank { chr_page: 0x1A, slot: 4 }),
        // WATER — page $4F, slot +5 (CheepCheep Hopper)
        0x64 => Some(SpriteBank { chr_page: 0x4F, slot: 5 }),
        // BRO — all page $4E, slot +4
        0x81 | 0x82 | 0x86 | 0x87 =>
            Some(SpriteBank { chr_page: 0x4E, slot: 4 }),
        // PIRANHAS — all page $4F, slot +5
        0xA0..=0xA7 => Some(SpriteBank { chr_page: 0x4F, slot: 5 }),
        // CHEEPS — Orange is $4F/+5, Green is NOCHANGE
        0x88 => Some(SpriteBank { chr_page: 0x4F, slot: 5 }),
        // 0x77 (Green Cheep) = NOCHANGE, falls through to None
        _ => None,
    }
}

/// Check whether an enemy is compatible with the current CHR page commitments.
fn is_chr_compatible(id: u8, slot4: Option<u8>, slot5: Option<u8>) -> bool {
    match sprite_bank(id) {
        None => true, // NOCHANGE — always compatible
        Some(sb) => match sb.slot {
            4 => slot4.is_none() || slot4 == Some(sb.chr_page),
            5 => slot5.is_none() || slot5 == Some(sb.chr_page),
            _ => true,
        },
    }
}

/// Big ? Block IDs — these can be swapped with each other to randomize
/// which suit/powerup the player gets from Big ? Blocks.
const BIG_Q_BLOCKS: &[u8] = &[
    0x94, // OBJ_BIGQBLOCK_3UP
    0x95, // OBJ_BIGQBLOCK_MUSHROOM
    0x96, // OBJ_BIGQBLOCK_FIREFLOWER
    0x97, // OBJ_BIGQBLOCK_SUPERLEAF
    0x98, // OBJ_BIGQBLOCK_TANOOKI
    0x99, // OBJ_BIGQBLOCK_FROG
    0x9A, // OBJ_BIGQBLOCK_HAMMER
];

/// File offset of the Tanooki Big ? Block in World 7-F1.
/// This block must NOT be randomized — flying/Tanooki is required to beat the level.
const W7F1_TANOOKI_OFFSET: usize = 0x0C336;

/// Bonus room enemy/object data range (file offsets).
///
/// The 8 per-world Big ? Block bonus rooms store their enemy data inside the
/// main enemy data region (PRG006). Each bonus room contains Big ? Block enemy
/// IDs (0x94–0x9A) that determine what powerup the player receives. These must
/// NOT be randomized, or the bonus room powerups become scrambled.
///
/// The bonus room pointers (PRG026, 0x3492B) index into this contiguous block.
/// W1 starts at file 0x0C988, W8 ends at file 0x0C9C2 (FF terminator).
const BONUS_ROOM_DATA_START: usize = 0x0C986;
const BONUS_ROOM_DATA_END: usize = 0x0C9C3;

/// All swap classes collected for lookup.
const ALL_CLASSES: &[&[u8]] = &[
    GROUND_ENEMIES,
    SHELL_ENEMIES,
    BIG_ENEMIES,
    FLYING_ENEMIES,
    WATER_ENEMIES,
    BRO_ENEMIES,
    PIRANHAS,
    PIRANHASC,
    CHEEPS,
];

/// Find which class an enemy ID belongs to, if any.
fn find_class(id: u8) -> Option<&'static [u8]> {
    for class in ALL_CLASSES {
        if class.contains(&id) {
            return Some(class);
        }
    }
    None
}

/// Randomize enemies by parsing the structured object data and only swapping
/// object IDs that belong to a known enemy class. Position bytes and all
/// special objects (end-level cards, pipes, platforms, bosses, powerups,
/// autoscroll triggers, cannons, etc.) are never modified.
pub fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R) {
    randomize_object_data(rom, rng, false);
}

/// Randomize Big ? Blocks by swapping their IDs among the set of Big ? Block
/// types. The Tanooki block in World 7-F1 is protected because flying is
/// required to beat that level.
pub fn randomize_big_q_blocks<R: Rng>(rom: &mut Rom, rng: &mut R) {
    randomize_object_data(rom, rng, true);
}

/// Record a CHR page commitment for the chosen enemy's bank slot.
fn commit_chr_page(id: u8, slot4: &mut Option<u8>, slot5: &mut Option<u8>) {
    if let Some(sb) = sprite_bank(id) {
        match sb.slot {
            4 => *slot4 = Some(sb.chr_page),
            5 => *slot5 = Some(sb.chr_page),
            _ => {}
        }
    }
}

fn randomize_object_data<R: Rng>(rom: &mut Rom, rng: &mut R, big_q_only: bool) {
    let len = ENEMY_DATA_END - ENEMY_DATA_START;
    let mut data = rom.read_range(ENEMY_DATA_START, len).to_vec();

    // Per-segment CHR page commitments. Reset at each 0xFF boundary.
    let mut committed_slot4: Option<u8> = None;
    let mut committed_slot5: Option<u8> = None;

    let mut i = 0;
    while i < data.len() {
        // 0xFF = segment boundary — reset CHR commitments
        if data[i] == 0xFF {
            committed_slot4 = None;
            committed_slot5 = None;
            i += 1;
            continue;
        }

        // First non-FF byte after a terminator is the page/flag byte
        let _page_flag = data[i];
        i += 1;

        // Now parse 3-byte entries until we hit 0xFF or end of data
        while i + 2 < data.len() && data[i] != 0xFF {
            let obj_id = data[i];
            let file_offset = ENEMY_DATA_START + i;

            if big_q_only {
                // Only randomize Big ? Blocks. Skip the 7-F1 Tanooki (flight
                // required) and the bonus room data region (the bonus rooms
                // store their own Big ? Block IDs that determine what powerup
                // each world's bonus room gives — randomizing them scrambles
                // the powerup the player receives).
                let in_bonus_room =
                    file_offset >= BONUS_ROOM_DATA_START && file_offset < BONUS_ROOM_DATA_END;
                if BIG_Q_BLOCKS.contains(&obj_id)
                    && file_offset != W7F1_TANOOKI_OFFSET
                    && !in_bonus_room
                {
                    data[i] = *BIG_Q_BLOCKS.choose(rng).unwrap();
                }
            } else if let Some(class) = find_class(obj_id) {
                // Filter class to CHR-compatible candidates
                let compatible: Vec<u8> = class
                    .iter()
                    .copied()
                    .filter(|&c| is_chr_compatible(c, committed_slot4, committed_slot5))
                    .collect();

                if !compatible.is_empty() {
                    let chosen = *compatible.choose(rng).unwrap();
                    data[i] = chosen;
                    commit_chr_page(chosen, &mut committed_slot4, &mut committed_slot5);
                }
                // else: no compatible candidates, keep original (safe fallback)
            } else {
                // Non-swappable enemy — still track its CHR commitment so
                // later randomized enemies in this segment respect it.
                commit_chr_page(obj_id, &mut committed_slot4, &mut committed_slot5);
            }

            // Advance past the 3-byte entry (id, x, y)
            i += 3;
        }
    }

    rom.write_range(ENEMY_DATA_START, &data);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        // iNES header
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16; // PRG pages
        data[5] = 16; // CHR pages
        data[6] = 0x40; // mapper flags

        // Set up a realistic enemy data segment at ENEMY_DATA_START:
        // FF terminator, then a segment with page flag + entries + FF
        let seg = &[
            0xFF, // leading terminator
            0x01, // page flag
            0x72, 0x0E, 0x19, // Goomba at (0x0E, 0x19)
            0x6C, 0x24, 0x16, // Green Troopa at (0x24, 0x16)
            0xA6, 0x16, 0x17, // Venus Fire Trap at (0x16, 0x17)
            0x41, 0xA8, 0x15, // End Level Card at (0xA8, 0x15) — must not change
            0xD3, 0x00, 0x50, // Autoscroll — must not change
            0xFF, // terminator
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);

        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_enemies_stay_in_class() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng);

        // Read back the segment (skip FF + page flag = offset 2)
        let base = ENEMY_DATA_START + 2;
        let result = rom.read_range(base, 15);

        // Goomba should be replaced with a ground enemy
        assert!(
            GROUND_ENEMIES.contains(&result[0]),
            "Goomba replaced with non-ground: 0x{:02X}",
            result[0]
        );
        // Position bytes must be unchanged
        assert_eq!(result[1], 0x0E);
        assert_eq!(result[2], 0x19);

        // Green Troopa should be replaced with a shell enemy
        assert!(
            SHELL_ENEMIES.contains(&result[3]),
            "Green Troopa replaced with non-shell enemy: 0x{:02X}",
            result[3]
        );
        assert_eq!(result[4], 0x24);
        assert_eq!(result[5], 0x16);

        // Venus Fire Trap should be replaced with a piranha
        assert!(
            PIRANHAS.contains(&result[6]),
            "Venus replaced with non-piranha: 0x{:02X}",
            result[6]
        );

        // End Level Card must NOT be changed
        assert_eq!(result[9], 0x41, "End Level Card was modified!");
        assert_eq!(result[10], 0xA8);
        assert_eq!(result[11], 0x15);

        // Autoscroll must NOT be changed
        assert_eq!(result[12], 0xD3, "Autoscroll was modified!");
    }

    #[test]
    fn test_deterministic() {
        let mut rom1 = make_test_rom();
        let mut rom2 = make_test_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(77);
        let mut rng2 = ChaCha8Rng::seed_from_u64(77);

        randomize(&mut rom1, &mut rng1);
        randomize(&mut rom2, &mut rng2);

        let len = ENEMY_DATA_END - ENEMY_DATA_START;
        assert_eq!(
            rom1.read_range(ENEMY_DATA_START, len),
            rom2.read_range(ENEMY_DATA_START, len),
        );
    }

    fn make_bigq_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        // Segment with a regular Big ? Block (should be randomized)
        let seg1_start = ENEMY_DATA_START;
        let seg1 = &[
            0xFF,
            0x01, // page flag
            0x94, 0x18, 0x05, // BIGQBLOCK_3UP
            0x98, 0x16, 0x14, // BIGQBLOCK_TANOOKI
            0x41, 0xA8, 0x15, // ENDLEVELCARD (must not change)
            0xFF,
        ];
        data[seg1_start..seg1_start + seg1.len()].copy_from_slice(seg1);

        // Place the protected 7-F1 Tanooki at its exact file offset
        // W7F1_TANOOKI_OFFSET = 0x0C336, which is the ID byte of the entry.
        // We need: [FF] [page] [0x98, x, y] [0x41, x, y] [FF]
        // So page byte at 0x0C335, entry at 0x0C336
        let w7f1_seg_start = W7F1_TANOOKI_OFFSET - 2; // FF + page byte before the entry
        data[w7f1_seg_start] = 0xFF;
        data[w7f1_seg_start + 1] = 0x01; // page flag
        data[W7F1_TANOOKI_OFFSET] = 0x98; // BIGQBLOCK_TANOOKI
        data[W7F1_TANOOKI_OFFSET + 1] = 0x0A;
        data[W7F1_TANOOKI_OFFSET + 2] = 0x13;
        data[W7F1_TANOOKI_OFFSET + 3] = 0x41; // ENDLEVELCARD
        data[W7F1_TANOOKI_OFFSET + 4] = 0x48;
        data[W7F1_TANOOKI_OFFSET + 5] = 0x15;
        data[W7F1_TANOOKI_OFFSET + 6] = 0xFF;

        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_big_q_blocks_randomized() {
        let mut rom = make_bigq_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize_big_q_blocks(&mut rom, &mut rng);

        // Regular Big ? Blocks should be randomized to some Big ? Block ID
        let base = ENEMY_DATA_START + 2; // skip FF + page
        let result = rom.read_range(base, 9);
        assert!(
            BIG_Q_BLOCKS.contains(&result[0]),
            "Big Q block not replaced with Big Q: 0x{:02X}",
            result[0]
        );
        assert!(
            BIG_Q_BLOCKS.contains(&result[3]),
            "Big Q block not replaced with Big Q: 0x{:02X}",
            result[3]
        );
        // End level card must not change
        assert_eq!(result[6], 0x41);
    }

    #[test]
    fn test_chr_compatibility_enforced() {
        // Place a Goomba ($4F/+5) and Dry Bones ($13/+5) in the same segment.
        // After randomization, both must use compatible CHR pages on slot +5.
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        let seg = &[
            0xFF,
            0x01, // page flag
            0x72, 0x10, 0x19, // Goomba (slot +5, page $4F)
            0x3F, 0x20, 0x19, // Dry Bones (slot +5, page $13)
            0x29, 0x30, 0x19, // Spike (slot +4, page $0A)
            0x71, 0x40, 0x19, // Spiny (slot +4, page $0B)
            0xFF,
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);
        let rom = Rom::from_bytes(&data).unwrap();

        // Run many times to exercise different random paths
        for seed in 0..200u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng);

            let base = ENEMY_DATA_START + 2;
            let result = rom_copy.read_range(base, 12);
            let enemy1 = result[0]; // was Goomba
            let enemy2 = result[3]; // was Dry Bones
            let enemy3 = result[6]; // was Spike
            let enemy4 = result[9]; // was Spiny

            // All must still be ground enemies
            assert!(GROUND_ENEMIES.contains(&enemy1), "seed {seed}: enemy1 0x{enemy1:02X}");
            assert!(GROUND_ENEMIES.contains(&enemy2), "seed {seed}: enemy2 0x{enemy2:02X}");
            assert!(GROUND_ENEMIES.contains(&enemy3), "seed {seed}: enemy3 0x{enemy3:02X}");
            assert!(GROUND_ENEMIES.contains(&enemy4), "seed {seed}: enemy4 0x{enemy4:02X}");

            // Check CHR compatibility: no two enemies in the same segment
            // should request different CHR pages for the same bank slot.
            let enemies = [enemy1, enemy2, enemy3, enemy4];
            let mut seen_slot4: Option<u8> = None;
            let mut seen_slot5: Option<u8> = None;
            for &e in &enemies {
                if let Some(sb) = sprite_bank(e) {
                    match sb.slot {
                        4 => {
                            if let Some(prev) = seen_slot4 {
                                assert_eq!(
                                    prev, sb.chr_page,
                                    "seed {seed}: slot +4 conflict: 0x{prev:02X} vs 0x{:02X} (enemy 0x{e:02X})",
                                    sb.chr_page
                                );
                            }
                            seen_slot4 = Some(sb.chr_page);
                        }
                        5 => {
                            if let Some(prev) = seen_slot5 {
                                assert_eq!(
                                    prev, sb.chr_page,
                                    "seed {seed}: slot +5 conflict: 0x{prev:02X} vs 0x{:02X} (enemy 0x{e:02X})",
                                    sb.chr_page
                                );
                            }
                            seen_slot5 = Some(sb.chr_page);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    #[test]
    fn test_chr_resets_across_segments() {
        // Two segments: first has a Goomba ($4F/+5), second has a Dry Bones ($13/+5).
        // They should be able to choose independently since they're in different segments.
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        let seg = &[
            0xFF,
            0x01,             // page flag
            0x72, 0x10, 0x19, // Goomba (slot +5, page $4F)
            0xFF,             // segment boundary
            0x01,             // page flag
            0x3F, 0x20, 0x19, // Dry Bones (slot +5, page $13)
            0xFF,
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);
        let rom = Rom::from_bytes(&data).unwrap();

        // Run many times — Dry Bones in second segment should freely choose
        // any ground enemy, not be constrained by first segment's Goomba.
        let mut saw_slot5_4f_in_seg2 = false;
        let mut saw_slot5_13_in_seg2 = false;
        for seed in 0..200u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng);

            // Second segment's enemy is at offset: FF(1) + page(1) + entry(3) + FF(1) + page(1) = 7
            let enemy2 = rom_copy.read_byte(ENEMY_DATA_START + 7);
            assert!(GROUND_ENEMIES.contains(&enemy2), "seed {seed}: 0x{enemy2:02X}");

            if let Some(sb) = sprite_bank(enemy2) {
                if sb.slot == 5 && sb.chr_page == 0x4F {
                    saw_slot5_4f_in_seg2 = true;
                }
                if sb.slot == 5 && sb.chr_page == 0x13 {
                    saw_slot5_13_in_seg2 = true;
                }
            }
        }
        // Over 200 seeds, we should see both CHR page variants in segment 2
        assert!(
            saw_slot5_4f_in_seg2 && saw_slot5_13_in_seg2,
            "Segment 2 should not be constrained by segment 1's CHR choice"
        );
    }

    #[test]
    fn test_7f1_tanooki_protected() {
        let mut rom = make_bigq_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(99);
        randomize_big_q_blocks(&mut rom, &mut rng);

        // The 7-F1 Tanooki must remain 0x98
        let protected = rom.read_byte(W7F1_TANOOKI_OFFSET);
        assert_eq!(
            protected, 0x98,
            "7-F1 Tanooki was changed to 0x{:02X}!",
            protected
        );
    }
}
