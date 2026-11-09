//! ? block contents: every powerup block is rerolled within its class.
//!
//! Blocks come from [`level_index::every_power_block`], which reaches every
//! level through its pointers, so this module never walks level data itself.

use rand::Rng;
use rand::seq::IndexedRandom;

use crate::randomize::rom_data::level_index::{self, PowerBlock};
use crate::rom::Rom;

/// ? block powerup shapes (flower=0, leaf=1, star=2).
const QBLOCK_SHAPES: &[u8] = &[0x00, 0x01, 0x02];

/// ? block powerup shapes without star (flower=0, leaf=1).
const QBLOCK_SHAPES_NO_STAR: &[u8] = &[0x00, 0x01];

/// Brick powerup shapes (flower=6, leaf=7, star=8, 1-up=11).
const BRICK_SHAPES: &[u8] = &[0x06, 0x07, 0x08, 0x0B];

/// Brick powerup shapes that make small Mario big (flower=6, leaf=7).
const BRICK_SHAPES_FLOWER_OR_LEAF: &[u8] = &[0x06, 0x07];

/// Note block powerup shapes (flower=1, leaf=2, star=3).
const NOTE_SHAPES: &[u8] = &[0x01, 0x02, 0x03];

/// Wood block powerup shapes (flower=4, leaf=5, star=6).
const WOOD_SHAPES: &[u8] = &[0x04, 0x05, 0x06];

/// File offsets of byte2 values that must not be randomized.
///
/// 6-5 (Ice level): single Q-leaf block — leaf is required for flight.
///
/// 7-7 (Muncher level): four Q-star blocks that must stay star — stars are
/// required to cross muncher fields. Found by brute-scanning the sub-area
/// at 0x23D48–0x23F1F for group 1 fixed-size byte2=0x02 patterns.
pub(crate) const PROTECTED_OFFSETS: &[usize] = &[
    0x22D74, // 6-5 Q-leaf byte2 — leaf required for flight in ice level
    0x23D7F, // 7-7 Q-star byte2 (screen 1)
    0x23DB0, // 7-7 Q-star byte2 (screen 2)
    0x23E1F, // 7-7 Q-star byte2 (screen 5)
    0x23EA0, // 7-7 Q-star byte2 (screen 8)
];

/// ? block and brick byte2 offsets that randomize between flower and leaf only —
/// never a star or a 1-up — regardless of the airship no-star option. Both
/// flower and leaf make small Mario big, which these spots require; a star or a
/// 1-up would leave him small.
///
/// 7-F1: the ? block gating the Tanooki area. Small Mario must break bricks to
/// reach it.
///
/// 5-2's Big [?] room (BigQ5): the brick flower in front of the Big [?] Block,
/// which only big Mario can open.
///
/// 8-F: the main area's screen-2 brick flower and the sub-area's screen-2 ?
/// block flower. Each is where a small Mario gets big for what follows.
pub(crate) const FLOWER_OR_LEAF_OFFSETS: &[usize] = &[
    0x2B39E, // 7-F1 ? block — flower/leaf so small Mario can break bricks for Tanooki area
    0x1B440, // BigQ5 brick — flower/leaf so small Mario can open the Big [?] Block
    0x2B900, // 8-F brick (main area, screen 2) — flower/leaf so Mario is big to break a block later
    0x2BB18, // 8-F ? block (sub-area, screen 2) — flower/leaf so small Mario gets big
];

/// Airship Q-block byte2 offsets (all 9 powerup blocks across W1-W7 airships).
/// When hammer_vulnerable_koopalings is on, these draw from {flower, leaf} only
/// (no star) to prevent trivializing Koopaling boss fights.
pub(crate) const AIRSHIP_QBLOCK_OFFSETS: &[usize] = &[
    0x2ED22, // W5 airship
    0x2EE46, // W1 airship
    0x2EF6D, // W2 airship
    0x2F0C6, // W3 airship
    0x2F208, // W4 airship (block 1)
    0x2F2BB, // W4 airship (block 2)
    0x2F3E7, // W6 airship
    0x2F50D, // W7 airship (block 1)
    0x2F572, // W7 airship (block 2)
];

/// Pick the shape pool for a ? block at `file_offset`. Blocks that gate
/// progression on being big (`FLOWER_OR_LEAF_OFFSETS`) always
/// exclude the star; airship blocks exclude it only when `no_airship_stars`
/// is set. Everything else may become flower, leaf, or star.
fn qblock_pool(file_offset: usize, no_airship_stars: bool) -> &'static [u8] {
    let exclude_star = FLOWER_OR_LEAF_OFFSETS.contains(&file_offset)
        || (no_airship_stars && AIRSHIP_QBLOCK_OFFSETS.contains(&file_offset));
    if exclude_star { QBLOCK_SHAPES_NO_STAR } else { QBLOCK_SHAPES }
}

/// Reroll every powerup block's contents within its class: ? blocks among
/// {flower, leaf, star}, bricks among {flower, leaf, star, 1-up}, and note and
/// wood blocks among {flower, leaf, star} of their own kind. Note and wood
/// blocks only count in tilesets where those shapes are blocks at all (see
/// `LevelDataRegion::randomize_note_wood`). [`PROTECTED_OFFSETS`] are never
/// rerolled.
///
/// When `no_airship_stars` is true, airship ? blocks draw from {flower, leaf}
/// only, so star power cannot trivialize a Koopaling fight.
pub(crate) fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R, no_airship_stars: bool) {
    for block in level_index::every_power_block(rom) {
        let offset = block.shape_offset();
        if PROTECTED_OFFSETS.contains(&offset) {
            continue;
        }
        if let Some(pool) = shape_pool(&block, no_airship_stars) {
            rom.write_byte(offset, *pool.choose(rng).unwrap());
        }
    }
}

/// The shapes `block` may be rerolled to, or `None` if it holds something
/// that is not rerolled (a coin, a vine, a P-switch...).
fn shape_pool(block: &PowerBlock, no_airship_stars: bool) -> Option<&'static [u8]> {
    let shape = block.shape();
    let pool = if block.is_note_or_wood() {
        [NOTE_SHAPES, WOOD_SHAPES].into_iter().find(|p| p.contains(&shape))?
    } else if QBLOCK_SHAPES.contains(&shape) {
        qblock_pool(block.shape_offset(), no_airship_stars)
    } else if BRICK_SHAPES.contains(&shape) {
        if FLOWER_OR_LEAF_OFFSETS.contains(&block.shape_offset()) {
            BRICK_SHAPES_FLOWER_OR_LEAF
        } else {
            BRICK_SHAPES
        }
    } else {
        return None;
    };
    Some(pool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::rom_data::LEVEL_DATA_REGIONS;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn load_rom() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    /// Every byte a reroll changes is a block's shape byte, and it stays in
    /// its class: a ? block stays a ? block, a brick a brick, and so on.
    #[test]
    fn only_block_shapes_change_and_each_stays_in_its_class() {
        let Some(vanilla) = load_rom() else { return };
        let blocks: std::collections::BTreeMap<usize, PowerBlock> =
            level_index::every_power_block(&vanilla)
                .into_iter()
                .map(|b| (b.shape_offset(), b))
                .collect();
        let mut changed = 0;
        for seed in 0..20 {
            let mut rom = vanilla.clone();
            randomize(&mut rom, &mut ChaCha8Rng::seed_from_u64(seed), false);
            for i in 0..vanilla.data.len() {
                if vanilla.data[i] == rom.data[i] {
                    continue;
                }
                changed += 1;
                let block = blocks.get(&i).unwrap_or_else(|| panic!("0x{i:05X} is not a block"));
                let pool = shape_pool(block, false).expect("a rerolled block has a pool");
                assert!(pool.contains(&rom.data[i]), "0x{i:05X} left its class");
            }
        }
        assert!(changed > 0, "nothing was rerolled");
    }

    /// The vanilla block count, so a change in what the index finds is seen.
    /// 232 includes 5-2's Big [?] room, whose brick flower the index has reached
    /// since it learned the Big [?] slots.
    #[test]
    fn vanilla_has_232_rerollable_blocks() {
        let Some(rom) = load_rom() else { return };
        let n = level_index::every_power_block(&rom)
            .iter()
            .filter(|b| shape_pool(b, false).is_some())
            .count();
        assert_eq!(n, 232);
    }

    #[test]
    fn protected_blocks_never_change() {
        let Some(vanilla) = load_rom() else { return };
        for seed in 0..50 {
            let mut rom = vanilla.clone();
            randomize(&mut rom, &mut ChaCha8Rng::seed_from_u64(seed), false);
            for &o in PROTECTED_OFFSETS {
                assert_eq!(rom.read_byte(o), vanilla.read_byte(o), "seed {seed}: 0x{o:05X}");
            }
        }
    }

    /// With the airship option on, no airship ? block ever rolls a star.
    #[test]
    fn airship_blocks_never_roll_a_star_when_asked() {
        let Some(vanilla) = load_rom() else { return };
        for seed in 0..50 {
            let mut rom = vanilla.clone();
            randomize(&mut rom, &mut ChaCha8Rng::seed_from_u64(seed), true);
            for &o in AIRSHIP_QBLOCK_OFFSETS {
                assert_ne!(rom.read_byte(o), 0x02, "seed {seed}: star at 0x{o:05X}");
            }
        }
    }

    /// The blocks that must make Mario big only ever hold a flower or a leaf:
    /// ? block shapes 0/1, brick shapes 6/7.
    #[test]
    fn flower_or_leaf_blocks_only_roll_flower_or_leaf() {
        let Some(vanilla) = load_rom() else { return };
        for seed in 0..50 {
            let mut rom = vanilla.clone();
            randomize(&mut rom, &mut ChaCha8Rng::seed_from_u64(seed), false);
            for &o in FLOWER_OR_LEAF_OFFSETS {
                let shape = rom.read_byte(o);
                assert!(
                    QBLOCK_SHAPES_NO_STAR.contains(&shape)
                        || BRICK_SHAPES_FLOWER_OR_LEAF.contains(&shape),
                    "seed {seed}: shape {shape:02X} at 0x{o:05X}"
                );
            }
        }
    }

    #[test]
    fn same_seed_same_blocks() {
        let Some(vanilla) = load_rom() else { return };
        let (mut a, mut b) = (vanilla.clone(), vanilla);
        randomize(&mut a, &mut ChaCha8Rng::seed_from_u64(123), false);
        randomize(&mut b, &mut ChaCha8Rng::seed_from_u64(123), false);
        assert_eq!(a.data, b.data);
    }

    #[test]
    fn qblock_pool_7f1_is_flower_or_leaf_never_star() {
        // 7-F1's Tanooki-gate block: flower or leaf, never star, regardless of
        // the airship no-star flag.
        for no_airship_stars in [false, true] {
            let pool = qblock_pool(0x2B39E, no_airship_stars);
            assert_eq!(pool, QBLOCK_SHAPES_NO_STAR);
            assert!(pool.contains(&0x00), "flower missing from pool");
            assert!(pool.contains(&0x01), "leaf missing from pool");
            assert!(!pool.contains(&0x02), "star must never be in the 7-F1 pool");
        }
    }

    #[test]
    fn qblock_pool_airship_star_gated_by_flag() {
        // An airship block keeps the star unless the no-star option is set.
        assert_eq!(qblock_pool(0x2ED22, false), QBLOCK_SHAPES);
        assert_eq!(qblock_pool(0x2ED22, true), QBLOCK_SHAPES_NO_STAR);
    }

    #[test]
    fn qblock_pool_ordinary_block_allows_star() {
        // A normal block (not in either special list) can still roll a star.
        assert_eq!(qblock_pool(0x12345, false), QBLOCK_SHAPES);
        assert_eq!(qblock_pool(0x12345, true), QBLOCK_SHAPES);
    }

    #[test]
    fn test_regions_stay_within_one_prg_bank() {
        // Level data is addressed CPU $A000-$BFFF within a single switched
        // PRG bank, so no region may cross a 8KB bank boundary. A region
        // whose `end` overruns its bank walks into the next bank's metatile
        // quadrant table and the randomizer corrupts metatile definitions
        // (stray tiles in desert levels and HB battle scenes — issue #34).
        const INES_HEADER: usize = 0x10;
        const BANK_SIZE: usize = 0x2000;
        for region in LEVEL_DATA_REGIONS {
            let start_bank = (region.start - INES_HEADER) / BANK_SIZE;
            let end_bank = (region.end - 1 - INES_HEADER) / BANK_SIZE;
            assert_eq!(
                start_bank,
                end_bank,
                "region 0x{:05X}-0x{:05X} crosses PRG bank boundary 0x{:05X}",
                region.start,
                region.end,
                (start_bank + 1) * BANK_SIZE + INES_HEADER
            );
        }
    }
}
