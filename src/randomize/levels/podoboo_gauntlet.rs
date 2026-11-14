//! 5-F2 podoboo gauntlet composer.
//!
//! 5-F2 sub-area 1 (objects pointer `$D2B9`, file offset `0xD2C9`, 26
//! entries) is a long corridor of podoboos and ceiling podoboos punctuated by
//! 2 Dry Bones and 2 Boos. The podoboos are easy to memorize per-seed once
//! known, so we jitter each podoboo's (X, Y) per seed.
//!
//! It runs after the enemy pass and reads the room as that pass left it:
//! only podoboos move, and every other entry keeps the ID, X and Y the enemy
//! pass gave it — so the Boos and Dry Bones randomize like any other level's.
//!
//! Compared to the old `enemy_jitter` ±2 implementation this composer:
//! - Respects segment-wide X-sort via [`segment_writer`] (no adjacent-pair
//!   swap-over bugs on tight gaps).
//! - Picks each podoboo X within its actual gap to neighbors, not a flat
//!   ±2 window.
//! - Preserves Y page bits (ceiling podoboos stay on page 0, regular
//!   podoboos stay on page 1).

use rand::Rng;

use crate::randomize::levels::segment_writer::{self, SegmentEntry, SegmentSpec};
use crate::randomize::rom_data::enemy_ptr_to_file_offset;
use crate::randomize::rom_data::level_index::read_objects;
use crate::rom::Rom;

/// 5-F2 sub-area 1's objects pointer.
const OBJECTS_PTR: u16 = 0xD2B9;
/// Its tileset (fortress); [`read_objects`] needs it to rule out Toad Houses.
const TILESET: u8 = 2;

const PODOBOO: u8 = 0x9E;
const CEILING_PODOBOO: u8 = 0x53;

/// X bounds: 5-F2 sub-area 1 is 8 screens, so X spans 0..=0x7F.
const SEG_X_MIN: u8 = 0x02;
const SEG_X_MAX: u8 = 0x7D;

const MIN_X_GAP: u8 = 2;

/// Half-window for per-podoboo X jitter, in tiles. Each podoboo can move
/// up to this far from its vanilla X, subject to the segment-wide
/// MIN_X_GAP constraint to surrounding entries.
const X_JITTER_RADIUS: u8 = 4;

/// Vanilla X positions of podoboos pinned to their vanilla (X, Y) — they
/// are never jittered. These are the first two regular podoboos of the
/// corridor (file offsets 0xD2CA and 0xD2CD); players rely on them as a
/// fixed entry landmark. X is unique within the segment, so matching by X
/// during the sorted walk is exact.
const PINNED_X: &[u8] = &[0x06, 0x0B];

pub(crate) fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R) {
    rom.push_tag("podoboo_gauntlet");

    // The room as the enemy pass left it, sorted by X so the jitter sees a
    // canonical order (vanilla lists the Dry Bones at 0x28 after the podoboos
    // at 0x2C and 0x2E, and the Boo at 0x6F before the podoboo at 0x6A).
    let mut current: Vec<SegmentEntry> = read_objects(rom, OBJECTS_PTR, TILESET)
        .expect("podoboo_gauntlet: 5-F2 sub-area 1 has an object list")
        .iter()
        .map(|o| SegmentEntry { obj_id: o.id, x: o.x, y: o.y })
        .collect();
    current.sort_by_key(|e| e.x);

    // Walk left-to-right. For each podoboo, jitter X within the gap to its
    // neighbors (respecting MIN_X_GAP and the per-entry radius around its X).
    // Everything else stays put.
    let mut out: Vec<SegmentEntry> = Vec::with_capacity(current.len());
    for (i, entry) in current.iter().enumerate() {
        let new_entry = if is_target(entry.obj_id) && !PINNED_X.contains(&entry.x) {
            let prev_x = out.last().map(|e| e.x).unwrap_or(SEG_X_MIN.saturating_sub(MIN_X_GAP));
            let next_x =
                current.get(i + 1).map(|e| e.x).unwrap_or(SEG_X_MAX.saturating_add(MIN_X_GAP));
            let lo = prev_x
                .saturating_add(MIN_X_GAP)
                .max(entry.x.saturating_sub(X_JITTER_RADIUS))
                .max(SEG_X_MIN);
            let hi = next_x
                .saturating_sub(MIN_X_GAP)
                .min(entry.x.saturating_add(X_JITTER_RADIUS))
                .min(SEG_X_MAX);
            let new_x = if hi >= lo { rng.random_range(lo..=hi) } else { entry.x };

            // Y: preserve high nibble (page bits), jitter low nibble.
            let y_page = entry.y & 0xF0;
            let old_row = (entry.y & 0x0F) as i16;
            let dy: i16 = rng.random_range(-2..=2);
            let new_row = (old_row + dy).clamp(0, 0x0F) as u8;
            let new_y = y_page | new_row;

            SegmentEntry { obj_id: entry.obj_id, x: new_x, y: new_y }
        } else {
            *entry
        };
        out.push(new_entry);
    }

    segment_writer::write_segment(
        rom,
        &SegmentSpec {
            file_offset: enemy_ptr_to_file_offset(OBJECTS_PTR),
            original_count: out.len(),
            entries: &out,
            label: Some("5-F2 sub-area 1"),
        },
    )
    .expect("podoboo_gauntlet: segment write failed");

    rom.pop_tag();
}

fn is_target(obj_id: u8) -> bool {
    obj_id == PODOBOO || obj_id == CEILING_PODOBOO
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    const DRY_BONES: u8 = 0x3F;
    const BOO: u8 = 0x2F;

    /// Vanilla 5-F2 sub-area 1, in ROM order (26 entries).
    #[rustfmt::skip]
    const VANILLA: &[(u8, u8, u8)] = &[
        (PODOBOO, 0x06, 0x17), (PODOBOO, 0x0B, 0x15), (PODOBOO, 0x0D, 0x11),
        (CEILING_PODOBOO, 0x12, 0x0F), (CEILING_PODOBOO, 0x18, 0x0F),
        (PODOBOO, 0x1E, 0x12), (PODOBOO, 0x24, 0x16), (PODOBOO, 0x2C, 0x15),
        (PODOBOO, 0x2E, 0x11), (DRY_BONES, 0x28, 0x17), (PODOBOO, 0x32, 0x11),
        (PODOBOO, 0x36, 0x12), (CEILING_PODOBOO, 0x3A, 0x0F), (BOO, 0x47, 0x17),
        (PODOBOO, 0x4B, 0x14), (PODOBOO, 0x4E, 0x17), (PODOBOO, 0x51, 0x14),
        (CEILING_PODOBOO, 0x56, 0x0F), (CEILING_PODOBOO, 0x5E, 0x0F),
        (PODOBOO, 0x63, 0x11), (BOO, 0x6F, 0x15), (PODOBOO, 0x6A, 0x10),
        (PODOBOO, 0x71, 0x12), (PODOBOO, 0x78, 0x13), (CEILING_PODOBOO, 0x79, 0x0F),
        (DRY_BONES, 0x7E, 0x17),
    ];

    /// A stub ROM holding vanilla 5-F2 sub-area 1, with `edit` applied to
    /// its entries first (to stand in for what the enemy pass wrote).
    fn make_test_rom(edit: impl Fn(&mut Vec<(u8, u8, u8)>)) -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        let mut entries = VANILLA.to_vec();
        edit(&mut entries);
        let base = enemy_ptr_to_file_offset(OBJECTS_PTR);
        data[base] = 0x00; // page byte
        for (i, &(id, x, y)) in entries.iter().enumerate() {
            data[base + 1 + i * 3..base + 4 + i * 3].copy_from_slice(&[id, x, y]);
        }
        data[base + 1 + entries.len() * 3] = 0xFF;
        Rom::from_bytes_lax(&data, true).unwrap()
    }

    fn run(seed: u64, rom: &mut Rom) -> Vec<SegmentEntry> {
        randomize(rom, &mut ChaCha8Rng::seed_from_u64(seed));
        segment_writer::read_segment(rom, enemy_ptr_to_file_offset(OBJECTS_PTR), VANILLA.len())
    }

    #[test]
    fn output_invariants() {
        let fixed: Vec<(u8, u8, u8)> =
            VANILLA.iter().copied().filter(|&(id, ..)| !is_target(id)).collect();
        for seed in 0..50u64 {
            let out = run(seed, &mut make_test_rom(|_| {}));

            // Sorted, ascending strict, and still terminated.
            for w in out.windows(2) {
                assert!(
                    w[0].x < w[1].x,
                    "seed {seed}: not sorted: {:02X} >= {:02X}",
                    w[0].x,
                    w[1].x
                );
            }
            // Non-targets keep their ID, X and Y.
            for &(id, x, y) in &fixed {
                assert!(
                    out.iter().any(|e| (e.obj_id, e.x, e.y) == (id, x, y)),
                    "seed {seed}: {id:02X} at ({x:02X},{y:02X}) moved"
                );
            }
            // Ceiling podoboos stay on page 0 (Y high nibble == 0).
            for e in out.iter().filter(|e| e.obj_id == CEILING_PODOBOO) {
                assert_eq!(e.y & 0xF0, 0x00, "seed {seed}: ceiling podoboo crossed page");
            }
            // Regular podoboos stay on page 1.
            for e in out.iter().filter(|e| e.obj_id == PODOBOO) {
                assert_eq!(e.y & 0xF0, 0x10, "seed {seed}: podoboo crossed page");
            }
        }
    }

    /// The composer keeps whatever the enemy pass put in the non-podoboo
    /// slots: here both Boos became Hot Foots and a Dry Bones a Boo.
    #[test]
    fn keeps_the_enemy_pass_picks() {
        let rom = || {
            make_test_rom(|e| {
                for entry in e.iter_mut() {
                    entry.0 = match entry.0 {
                        BOO => 0x45,
                        DRY_BONES if entry.1 == 0x28 => BOO,
                        id => id,
                    };
                }
            })
        };
        for seed in 0..20u64 {
            let out = run(seed, &mut rom());
            let at = |x: u8| out.iter().find(|e| e.x == x).map(|e| e.obj_id);
            assert_eq!(at(0x47), Some(0x45), "seed {seed}");
            assert_eq!(at(0x6F), Some(0x45), "seed {seed}");
            assert_eq!(at(0x28), Some(BOO), "seed {seed}");
            assert_eq!(at(0x7E), Some(DRY_BONES), "seed {seed}");
        }
    }

    #[test]
    fn pinned_x_matches_vanilla_entries() {
        // PINNED_X is the first two vanilla podoboos.
        assert_eq!(PINNED_X, &[VANILLA[0].1, VANILLA[1].1]);
        assert_eq!(VANILLA[0].0, PODOBOO);
        assert_eq!(VANILLA[1].0, PODOBOO);
    }

    #[test]
    fn first_two_podoboos_pinned() {
        // The first two regular podoboos (vanilla X=0x06 Y=0x17, X=0x0B
        // Y=0x15) must never move, across all seeds.
        for seed in 0..50u64 {
            let out = run(seed, &mut make_test_rom(|_| {}));
            assert!(
                out.iter().any(|e| e.obj_id == PODOBOO && e.x == 0x06 && e.y == 0x17),
                "seed {seed}: pinned podoboo #1 (0x06,0x17) missing"
            );
            assert!(
                out.iter().any(|e| e.obj_id == PODOBOO && e.x == 0x0B && e.y == 0x15),
                "seed {seed}: pinned podoboo #2 (0x0B,0x15) missing"
            );
        }
    }

    #[test]
    fn fixes_tight_pair_collision() {
        // Vanilla has 0x9E at X=0x78 and 0x53 at X=0x79 (gap 1). The old
        // ±2 jitter could land them at the same X. The composer enforces
        // MIN_X_GAP=2.
        for seed in 0..50u64 {
            let out = run(seed, &mut make_test_rom(|_| {}));
            for w in out.windows(2) {
                assert!(
                    w[1].x - w[0].x >= MIN_X_GAP,
                    "seed {seed}: gap too small {:02X} -> {:02X}",
                    w[0].x,
                    w[1].x
                );
            }
        }
    }

    /// The stub matches the real ROM, and the pointer is an area the level
    /// index reaches. Skips without the ROM.
    #[test]
    fn stub_matches_the_rom() {
        let Some(rom) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes")
            .ok()
            .and_then(|b| Rom::from_bytes(&b).ok())
        else {
            return;
        };
        let objects = read_objects(&rom, OBJECTS_PTR, TILESET).unwrap();
        let got: Vec<(u8, u8, u8)> = objects.iter().map(|o| (o.id, o.x, o.y)).collect();
        assert_eq!(got, VANILLA);
        assert!(
            crate::randomize::rom_data::level_index::every_level(&rom)
                .iter()
                .flatten()
                .any(|a| a.objects_ptr == OBJECTS_PTR && a.tileset == TILESET)
        );
    }
}
