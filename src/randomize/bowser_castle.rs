//! 8-Bowser sub-area 1 composer.
//!
//! The pre-Bowser corridor (segment at file offset `0xD61B`, 14 entries)
//! contains:
//! - 2× DryBones (`0x3F`) — kept at vanilla positions
//! - 1× ThwompRightSlide (`0x8C`) — kept at vanilla position
//! - 2× CFIRE_LASER (`0xD0`) — repositioned to a random pair from a
//!   curated 9-position statue pool
//! - 9× OBJ_BOSSATTACK (`0x75`, the Bowser-statue fireballs) — repositioned
//!   per-seed to random `(X, Y)` within the remaining X gaps, Y in the
//!   `0x11..=0x17` band fred uses
//!
//! All 14 entries are composed together and routed through
//! [`segment_writer`] so the final segment is X-sorted and collision-free.

use rand::Rng;
use rand::seq::IndexedRandom;

use crate::rom::Rom;
use super::segment_writer::{self, SegmentEntry, SegmentSpec, SortMode};

/// File offset of the segment's page/header byte (entries start at +1).
const SEG_OFFSET: usize = 0xD61B;
const ENTRY_COUNT: usize = 14;

const LASER: u8 = 0xD0;
const FIREBALL: u8 = 0x75;
const DRY_BONES: u8 = 0x3F;
const THWOMP_SLIDE: u8 = 0x8C;

/// 9 curated statue positions in 8-Bowser sub-area 1, `(x, y)`.
/// Sourced from observed output of the fred SMB3 randomizer — positions
/// where a laser visually aligns with a Bowser statue head.
const STATUE_POOL: &[(u8, u8)] = &[
    (0x40, 0x15),
    (0x45, 0x16),
    (0x4C, 0x14),
    (0x52, 0x15),
    (0x7C, 0x11),
    (0xA3, 0x16),
    (0xA9, 0x13),
    (0xB0, 0x13),
    (0xBC, 0x13),
];

/// Y range for fireballs (matches fred's observed band).
const FIREBALL_Y_MIN: u8 = 0x11;
const FIREBALL_Y_MAX: u8 = 0x17;

/// Minimum X gap between any two segment entries — keeps fireballs from
/// stacking on each other or on a fixed anchor.
const MIN_X_GAP: u8 = 2;

/// X bounds for fireball placement. The lower bound is a safety floor:
/// vanilla's leftmost fireball sits at 0x62, and entries any further left
/// spawn on top of the player walking in from the door at X≈0x02, so they
/// can't be reacted to. Sit a little under vanilla to keep some variance
/// without re-creating that no-win spawn.
const SEG_X_MIN: u8 = 0x55;
const SEG_X_MAX: u8 = 0xEF;  // a few past last vanilla fireball at 0xE5

/// Vanilla fixed entries (DryBones + Thwomp) — hard-coded so the composer
/// produces the same 14-entry segment regardless of what bytes the input
/// ROM has at this offset (matters for integration tests using stub ROMs).
const FIXED_ENTRIES: &[SegmentEntry] = &[
    SegmentEntry { obj_id: DRY_BONES, x: 0x04, y: 0x18 },
    SegmentEntry { obj_id: DRY_BONES, x: 0x0A, y: 0x18 },
    SegmentEntry { obj_id: THWOMP_SLIDE, x: 0x16, y: 0x10 },
];
const LASER_COUNT: usize = 2;
const FIREBALL_COUNT: usize = 9;

pub fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R) {
    rom.push_tag("bowser_castle");

    // 1. Lasers: pick 2 distinct positions from the 9-pool.
    let picks: Vec<&(u8, u8)> = STATUE_POOL.choose_multiple(rng, LASER_COUNT).collect();
    let lasers: Vec<SegmentEntry> = picks.into_iter()
        .map(|&(x, y)| SegmentEntry { obj_id: LASER, x, y })
        .collect();

    // 2. Fireballs: greedy constraint-driven placement. The pool of
    //    anchors grows as fireballs are placed so each new fireball
    //    respects MIN_X_GAP to every prior entry.
    let mut anchors: Vec<u8> = FIXED_ENTRIES.iter().map(|e| e.x)
        .chain(lasers.iter().map(|e| e.x))
        .collect();
    anchors.sort();

    let mut fireballs: Vec<SegmentEntry> = Vec::with_capacity(FIREBALL_COUNT);
    for _ in 0..FIREBALL_COUNT {
        let x = pick_fireball_x(&anchors, rng);
        let y = rng.random_range(FIREBALL_Y_MIN..=FIREBALL_Y_MAX);
        fireballs.push(SegmentEntry { obj_id: FIREBALL, x, y });
        let pos = anchors.binary_search(&x).unwrap_or_else(|p| p);
        anchors.insert(pos, x);
    }

    // 3. Combine and write. segment_writer handles sort and validation.
    let mut entries: Vec<SegmentEntry> = Vec::with_capacity(ENTRY_COUNT);
    entries.extend_from_slice(FIXED_ENTRIES);
    entries.extend(lasers);
    entries.extend(fireballs);

    segment_writer::write_segment(rom, &SegmentSpec {
        file_offset: SEG_OFFSET,
        original_count: ENTRY_COUNT,
        entries: &entries,
        label: Some("8-Bowser sub-area 1"),
        sort_mode: SortMode::SortByX,
    }).expect("bowser_castle: segment write failed");

    rom.pop_tag();
}

/// Pick a fireball X within the widest available gap between anchors.
/// `anchors` is the sorted list of already-claimed X positions.
fn pick_fireball_x<R: Rng>(anchors: &[u8], rng: &mut R) -> u8 {
    // Build gap intervals: virtual boundaries at SEG_X_MIN - MIN_X_GAP and
    // SEG_X_MAX + MIN_X_GAP so the actual usable range is [SEG_X_MIN..=SEG_X_MAX].
    let sentinel = SEG_X_MAX.saturating_add(MIN_X_GAP);
    let mut prev = SEG_X_MIN.saturating_sub(MIN_X_GAP);
    let mut gaps: Vec<(u8, u8)> = Vec::new(); // usable (lo, hi) intervals
    for &a in anchors.iter().chain(std::iter::once(&sentinel)) {
        let lo = prev.saturating_add(MIN_X_GAP).max(SEG_X_MIN);
        let hi = a.saturating_sub(MIN_X_GAP).min(SEG_X_MAX);
        if hi >= lo {
            gaps.push((lo, hi));
        }
        prev = a;
    }

    // Widest gap wins; ties keep the leftmost gap (`rev()` turns max_by_key's
    // last-max-wins tie rule into first-max-wins in original order).
    let &(lo, hi) = gaps
        .iter()
        .rev()
        .max_by_key(|(lo, hi)| hi - lo)
        .expect("bowser_castle: no fireball gap available");
    rng.random_range(lo..=hi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha8Rng;
    use rand::SeedableRng;

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16; data[5] = 16; data[6] = 0x40;
        // Vanilla 8B sub-area 1 bytes (page byte + 14 entries).
        let seg = &[
            0x00,                  // page byte
            0x3F, 0x04, 0x18,
            0x3F, 0x0A, 0x18,
            0x8C, 0x16, 0x10,
            0xD0, 0x40, 0x15,
            0x75, 0x62, 0x16,
            0x75, 0x6C, 0x16,
            0x75, 0x73, 0x17,
            0x75, 0x7E, 0x15,
            0xD0, 0xA3, 0x16,
            0x75, 0xD1, 0x17,
            0x75, 0xD6, 0x16,
            0x75, 0xD9, 0x16,
            0x75, 0xE1, 0x14,
            0x75, 0xE5, 0x17,
        ];
        data[SEG_OFFSET..SEG_OFFSET + seg.len()].copy_from_slice(seg);
        Rom::from_bytes_lax(&data, true).unwrap()
    }

    #[test]
    fn output_invariants() {
        for seed in 0..200u64 {
            let mut rom = make_test_rom();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom, &mut rng);

            let out = segment_writer::read_segment(&rom, SEG_OFFSET, ENTRY_COUNT);

            // Count by id
            let n_dry = out.iter().filter(|e| e.obj_id == DRY_BONES).count();
            let n_thwomp = out.iter().filter(|e| e.obj_id == THWOMP_SLIDE).count();
            let n_laser = out.iter().filter(|e| e.obj_id == LASER).count();
            let n_fireball = out.iter().filter(|e| e.obj_id == FIREBALL).count();
            assert_eq!(n_dry, 2, "seed {seed}: dry bones count");
            assert_eq!(n_thwomp, 1, "seed {seed}: thwomp count");
            assert_eq!(n_laser, 2, "seed {seed}: laser count");
            assert_eq!(n_fireball, 9, "seed {seed}: fireball count");

            // Fixed entries preserve vanilla X.
            let dry_xs: Vec<u8> = out.iter().filter(|e| e.obj_id == DRY_BONES).map(|e| e.x).collect();
            assert!(dry_xs.contains(&0x04));
            assert!(dry_xs.contains(&0x0A));
            let thwomp_x = out.iter().find(|e| e.obj_id == THWOMP_SLIDE).unwrap().x;
            assert_eq!(thwomp_x, 0x16);

            // Sort order + min gap.
            for w in out.windows(2) {
                assert!(w[0].x < w[1].x, "seed {seed}: not sorted");
            }
            // Min gap holds among MOVABLE entries (fireballs vs fireballs/lasers).
            // Vanilla DryBones at 0x04 and 0x0A are gap 6 — fine.

            // Laser positions came from the pool.
            for e in out.iter().filter(|e| e.obj_id == LASER) {
                assert!(STATUE_POOL.contains(&(e.x, e.y)),
                        "seed {seed}: laser ({:02X},{:02X}) not in pool", e.x, e.y);
            }

            // Fireball Y in band.
            for e in out.iter().filter(|e| e.obj_id == FIREBALL) {
                assert!((FIREBALL_Y_MIN..=FIREBALL_Y_MAX).contains(&e.y),
                        "seed {seed}: fireball y={:02X} out of band", e.y);
                assert!(e.x >= SEG_X_MIN && e.x <= SEG_X_MAX);
            }
        }
    }

    #[test]
    fn determinism() {
        let mut r1 = make_test_rom();
        let mut r2 = make_test_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(12345);
        let mut rng2 = ChaCha8Rng::seed_from_u64(12345);
        randomize(&mut r1, &mut rng1);
        randomize(&mut r2, &mut rng2);
        assert_eq!(
            segment_writer::read_segment(&r1, SEG_OFFSET, ENTRY_COUNT),
            segment_writer::read_segment(&r2, SEG_OFFSET, ENTRY_COUNT),
        );
    }

    #[test]
    fn uses_seventh_pool_position() {
        // Across enough seeds, 0x7C (the position that needed coordination
        // with the surrounding fireballs) should appear at least once.
        let mut saw_7c = false;
        for seed in 0..200u64 {
            let mut rom = make_test_rom();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom, &mut rng);
            let out = segment_writer::read_segment(&rom, SEG_OFFSET, ENTRY_COUNT);
            if out.iter().any(|e| e.obj_id == LASER && e.x == 0x7C) {
                saw_7c = true;
                break;
            }
        }
        assert!(saw_7c, "0x7C never picked across 200 seeds — pool wiring broken");
    }
}
