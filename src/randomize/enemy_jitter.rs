use rand::Rng;

use crate::randomize::rom_data::JITTER_SEGMENTS;
use crate::rom::Rom;

/// Apply per-seed X/Y jitter (±2 tiles) to matching entries in each configured
/// gauntlet segment. Targets memorizable projectile-hazard levels (5F-2
/// podoboos, 8B pre-Bowser fireballs) so speedrunners can't run a static
/// route every seed. Non-matching entries in the same segment (DryBones,
/// Boos, etc.) are left untouched.
///
/// The Y byte's high nibble encodes vertical page/flags and must be preserved;
/// only the low nibble (row within page) is jittered. X is a global tile
/// column and is jittered as a whole byte, clamped to the segment's width.
pub fn apply<R: Rng>(rom: &mut Rom, rng: &mut R) {
    for seg in JITTER_SEGMENTS {
        for i in 0..seg.count {
            let off = seg.file_offset + i * 3;
            let id = rom.data[off];
            if !seg.ids.contains(&id) {
                continue;
            }
            let dx: i16 = rng.random_range(-2..=2);
            let dy: i16 = rng.random_range(-2..=2);

            // X: clamp whole byte to segment width.
            let new_x = (rom.data[off + 1] as i16 + dx).clamp(0, seg.max_x as i16) as u8;

            // Y: preserve high nibble (page/flags); clamp low nibble (row) to 0..=15.
            let old_y = rom.data[off + 2];
            let old_row = (old_y & 0x0F) as i16;
            let new_row = (old_row + dy).clamp(0, 0x0F) as u8;
            let new_y = (old_y & 0xF0) | new_row;

            rom.data[off + 1] = new_x;
            rom.data[off + 2] = new_y;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    /// Build a minimal ROM with vanilla bytes at the two jitter segment
    /// offsets. Values copied directly from Super Mario Bros. 3 (USA Rev 1).
    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        // 5F-2 sub-area 1 (vanilla bytes). The high nibble of Y = vertical
        // page bit; 0x1X = page 1 (lower half), 0x0X = page 0.
        let w5f2 = &[
            0x9E, 0x06, 0x17, // Podoboo, page 1 row 7
            0x9E, 0x0B, 0x15, // Podoboo, page 1 row 5
            0x9E, 0x0D, 0x11, // Podoboo, page 1 row 1
            0x53, 0x12, 0x0F, // Ceiling podoboo, page 0 row 15
            0x53, 0x18, 0x0F, // Ceiling podoboo
            0x9E, 0x1E, 0x12, // Podoboo
            0x9E, 0x24, 0x16, // Podoboo
            0x9E, 0x2C, 0x15, // Podoboo
            0x9E, 0x2E, 0x11, // Podoboo
            0x3F, 0x28, 0x17, // DryBones (not jittered)
            0x9E, 0x32, 0x11, // Podoboo
            0x9E, 0x36, 0x12, // Podoboo
            0x53, 0x3A, 0x0F, // Ceiling podoboo
            0x65, 0x47, 0x17, // Boo (not jittered)
            0x9E, 0x4B, 0x14, // Podoboo
            0x9E, 0x4E, 0x17, // Podoboo
            0x9E, 0x51, 0x14, // Podoboo
            0x53, 0x56, 0x0F, // Ceiling podoboo
            0x53, 0x5E, 0x0F, // Ceiling podoboo
            0x9E, 0x63, 0x11, // Podoboo
            0x65, 0x6F, 0x15, // Boo (not jittered)
            0x9E, 0x6A, 0x10, // Podoboo
            0x9E, 0x71, 0x12, // Podoboo
            0x9E, 0x78, 0x13, // Podoboo
            0x53, 0x79, 0x0F, // Ceiling podoboo
            0x3F, 0x7E, 0x17, // DryBones (not jittered)
        ];
        data[0x0D2CA..0x0D2CA + w5f2.len()].copy_from_slice(w5f2);

        // 8B sub-area 1 (vanilla bytes).
        let w8b = &[
            0x3F, 0x04, 0x18, // DryBones (not jittered)
            0x3F, 0x0A, 0x18, // DryBones (not jittered)
            0x8C, 0x16, 0x10, // ThwompRightSlide (not jittered)
            0xC9, 0x40, 0x15, // CannonFire (not jittered)
            0x75, 0x62, 0x16, // BossAttack fireball
            0x75, 0x6C, 0x16, // BossAttack fireball
            0x75, 0x73, 0x17, // BossAttack fireball
            0x75, 0x7E, 0x15, // BossAttack fireball
            0xC9, 0xA3, 0x16, // CannonFire (not jittered)
            0x75, 0xD1, 0x17, // BossAttack fireball
            0x75, 0xD6, 0x16, // BossAttack fireball
            0x75, 0xD9, 0x16, // BossAttack fireball
            0x75, 0xE1, 0x14, // BossAttack fireball
            0x75, 0xE5, 0x17, // BossAttack fireball
        ];
        data[0x0D61C..0x0D61C + w8b.len()].copy_from_slice(w8b);

        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_jitter_preserves_ids_y_high_nibble_and_non_targets() {
        let original = make_test_rom();
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        apply(&mut rom, &mut rng);

        for seg in JITTER_SEGMENTS {
            for i in 0..seg.count {
                let off = seg.file_offset + i * 3;
                let id = rom.data[off];
                // obj_id byte is never changed
                assert_eq!(
                    id, original.data[off],
                    "obj_id changed at offset 0x{off:05X}"
                );
                if seg.ids.contains(&id) {
                    // Jittered X stays within clamp bounds
                    assert!(
                        rom.data[off + 1] <= seg.max_x,
                        "X out of bounds at 0x{off:05X}: 0x{:02X} > 0x{:02X}",
                        rom.data[off + 1],
                        seg.max_x
                    );
                    // Y high nibble preserved (page/flags must not change)
                    assert_eq!(
                        rom.data[off + 2] & 0xF0,
                        original.data[off + 2] & 0xF0,
                        "Y high nibble changed at 0x{off:05X}: was 0x{:02X}, now 0x{:02X}",
                        original.data[off + 2],
                        rom.data[off + 2],
                    );
                } else {
                    // Non-target entries are byte-identical
                    assert_eq!(
                        rom.data[off + 1],
                        original.data[off + 1],
                        "non-target X changed at 0x{off:05X}"
                    );
                    assert_eq!(
                        rom.data[off + 2],
                        original.data[off + 2],
                        "non-target Y changed at 0x{off:05X}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_jitter_is_deterministic() {
        let mut rom1 = make_test_rom();
        let mut rom2 = make_test_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(777);
        let mut rng2 = ChaCha8Rng::seed_from_u64(777);
        apply(&mut rom1, &mut rng1);
        apply(&mut rom2, &mut rng2);
        for seg in JITTER_SEGMENTS {
            let n = seg.count * 3;
            assert_eq!(
                rom1.read_range(seg.file_offset, n),
                rom2.read_range(seg.file_offset, n),
                "non-deterministic jitter in segment at 0x{:05X}",
                seg.file_offset
            );
        }
    }

    #[test]
    fn test_jitter_actually_changes_positions() {
        // With 22 + 8 = 30 target entries getting independent [-2..=+2]
        // jitter in X and Y, the probability of zero visible changes is
        // negligibly small. Guards against a future bug where apply()
        // silently no-ops.
        let original = make_test_rom();
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(12345);
        apply(&mut rom, &mut rng);

        let mut changed_entries = 0;
        for seg in JITTER_SEGMENTS {
            for i in 0..seg.count {
                let off = seg.file_offset + i * 3;
                if seg.ids.contains(&rom.data[off])
                    && (rom.data[off + 1] != original.data[off + 1]
                        || rom.data[off + 2] != original.data[off + 2])
                {
                    changed_entries += 1;
                }
            }
        }
        assert!(
            changed_entries > 0,
            "jitter produced no changes — apply() is a no-op"
        );
    }
}
