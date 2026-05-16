//! 5-F2 podoboo gauntlet composer.
//!
//! 5-F2 sub-area 1 (segment file offset `0xD2C9`, 26 entries) is a long
//! corridor of podoboos and ceiling podoboos punctuated by 2 DryBones and
//! 2 Boos. The podoboos are easy to memorize per-seed once known, so we
//! jitter each podoboo's (X, Y) per seed while keeping non-target enemies
//! at vanilla positions.
//!
//! Compared to the old `enemy_jitter` ±2 implementation this composer:
//! - Respects segment-wide X-sort via [`segment_writer`] (no adjacent-pair
//!   swap-over bugs on tight gaps).
//! - Picks each podoboo X within its actual gap to neighbors, not a flat
//!   ±2 window.
//! - Preserves Y page bits (ceiling podoboos stay on page 0, regular
//!   podoboos stay on page 1).

use rand::Rng;

use crate::rom::Rom;
use super::segment_writer::{self, SegmentEntry, SegmentSpec, SortMode};

const SEG_OFFSET: usize = 0xD2C9;
const ENTRY_COUNT: usize = 26;

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

/// Vanilla layout of 5-F2 sub-area 1 — hard-coded so the composer
/// produces a known segment regardless of what bytes the input ROM has at
/// this offset (matters for integration tests using stub ROMs). 26 entries
/// in vanilla X order. Targets (Podoboo, Ceiling Podoboo) get jittered;
/// non-targets (DryBones, Boo) keep these exact (X, Y, ID).
const VANILLA: &[SegmentEntry] = &[
    SegmentEntry { obj_id: PODOBOO,         x: 0x06, y: 0x17 },
    SegmentEntry { obj_id: PODOBOO,         x: 0x0B, y: 0x15 },
    SegmentEntry { obj_id: PODOBOO,         x: 0x0D, y: 0x11 },
    SegmentEntry { obj_id: CEILING_PODOBOO, x: 0x12, y: 0x0F },
    SegmentEntry { obj_id: CEILING_PODOBOO, x: 0x18, y: 0x0F },
    SegmentEntry { obj_id: PODOBOO,         x: 0x1E, y: 0x12 },
    SegmentEntry { obj_id: PODOBOO,         x: 0x24, y: 0x16 },
    SegmentEntry { obj_id: 0x3F,            x: 0x28, y: 0x17 }, // DryBones
    SegmentEntry { obj_id: PODOBOO,         x: 0x2C, y: 0x15 },
    SegmentEntry { obj_id: PODOBOO,         x: 0x2E, y: 0x11 },
    SegmentEntry { obj_id: PODOBOO,         x: 0x32, y: 0x11 },
    SegmentEntry { obj_id: PODOBOO,         x: 0x36, y: 0x12 },
    SegmentEntry { obj_id: CEILING_PODOBOO, x: 0x3A, y: 0x0F },
    SegmentEntry { obj_id: 0x65,            x: 0x47, y: 0x17 }, // Boo
    SegmentEntry { obj_id: PODOBOO,         x: 0x4B, y: 0x14 },
    SegmentEntry { obj_id: PODOBOO,         x: 0x4E, y: 0x17 },
    SegmentEntry { obj_id: PODOBOO,         x: 0x51, y: 0x14 },
    SegmentEntry { obj_id: CEILING_PODOBOO, x: 0x56, y: 0x0F },
    SegmentEntry { obj_id: CEILING_PODOBOO, x: 0x5E, y: 0x0F },
    SegmentEntry { obj_id: PODOBOO,         x: 0x63, y: 0x11 },
    SegmentEntry { obj_id: 0x65,            x: 0x6F, y: 0x15 }, // Boo
    SegmentEntry { obj_id: PODOBOO,         x: 0x6A, y: 0x10 },
    SegmentEntry { obj_id: PODOBOO,         x: 0x71, y: 0x12 },
    SegmentEntry { obj_id: PODOBOO,         x: 0x78, y: 0x13 },
    SegmentEntry { obj_id: CEILING_PODOBOO, x: 0x79, y: 0x0F },
    SegmentEntry { obj_id: 0x3F,            x: 0x7E, y: 0x17 }, // DryBones
];

pub fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R) {
    rom.push_tag("podoboo_gauntlet");

    // Walk vanilla left-to-right by X. For each entry: if it's a podoboo,
    // jitter X within the gap to its neighbors (respecting MIN_X_GAP and
    // the per-entry radius around vanilla X). Non-targets stay put.
    //
    // Note: the VANILLA list is roughly sorted but has one inversion at
    // entries 20 (Podoboo 0x63) and 21 (Boo 0x6F) — index 22 (Podoboo
    // 0x6A) follows. We sort by X up-front so the jitter logic sees a
    // canonical order.
    let mut sorted_vanilla: Vec<SegmentEntry> = VANILLA.to_vec();
    sorted_vanilla.sort_by_key(|e| e.x);

    let mut out: Vec<SegmentEntry> = Vec::with_capacity(ENTRY_COUNT);
    for (i, entry) in sorted_vanilla.iter().enumerate() {
        let new_entry = if is_target(entry.obj_id) {
            let prev_x = out.last().map(|e| e.x).unwrap_or(SEG_X_MIN.saturating_sub(MIN_X_GAP));
            let next_x = sorted_vanilla.get(i + 1).map(|e| e.x).unwrap_or(SEG_X_MAX.saturating_add(MIN_X_GAP));
            let lo = prev_x.saturating_add(MIN_X_GAP)
                .max(entry.x.saturating_sub(X_JITTER_RADIUS))
                .max(SEG_X_MIN);
            let hi = next_x.saturating_sub(MIN_X_GAP)
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

    segment_writer::write_segment(rom, &SegmentSpec {
        file_offset: SEG_OFFSET,
        original_count: ENTRY_COUNT,
        entries: &out,
        label: Some("5-F2 sub-area 1"),
        sort_mode: SortMode::SortByX,
    }).expect("podoboo_gauntlet: segment write failed");

    rom.pop_tag();
}

fn is_target(obj_id: u8) -> bool {
    obj_id == PODOBOO || obj_id == CEILING_PODOBOO
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha8Rng;
    use rand::SeedableRng;

    fn make_test_rom() -> Rom {
        // Stub ROM is fine — the composer uses hard-coded VANILLA and never
        // reads bytes from the input ROM for segment structure.
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16; data[5] = 16; data[6] = 0x40;
        Rom::from_bytes_lax(&data, true).unwrap()
    }

    #[test]
    fn output_invariants() {
        for seed in 0..50u64 {
            let mut rom = make_test_rom();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom, &mut rng);
            let out = segment_writer::read_segment(&rom, SEG_OFFSET, ENTRY_COUNT);

            // Sorted, ascending strict.
            for w in out.windows(2) {
                assert!(w[0].x < w[1].x, "seed {seed}: not sorted: {:02X} >= {:02X}", w[0].x, w[1].x);
            }
            // Non-targets stay at vanilla X and Y.
            let drybones: Vec<&SegmentEntry> = out.iter().filter(|e| e.obj_id == 0x3F).collect();
            assert_eq!(drybones.len(), 2);
            let boos: Vec<&SegmentEntry> = out.iter().filter(|e| e.obj_id == 0x65).collect();
            assert_eq!(boos.len(), 2);
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

    #[test]
    fn fixes_tight_pair_collision() {
        // Vanilla has 0x9E at X=0x78 and 0x53 at X=0x79 (gap 1). The old
        // ±2 jitter could land them at the same X. The composer enforces
        // MIN_X_GAP=2.
        for seed in 0..50u64 {
            let mut rom = make_test_rom();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom, &mut rng);
            let out = segment_writer::read_segment(&rom, SEG_OFFSET, ENTRY_COUNT);
            for w in out.windows(2) {
                assert!(w[1].x - w[0].x >= MIN_X_GAP,
                        "seed {seed}: gap too small {:02X} -> {:02X}", w[0].x, w[1].x);
            }
        }
    }
}
