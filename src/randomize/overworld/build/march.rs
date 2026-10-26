//! The Hammer-Bro march graph — where a wandering map sprite can actually go.
//!
//! Wandering sprites do not walk the player's map graph. Their movement is
//! `Map_Object_March` (PRG011), and three ROM facts shape it:
//!
//! - **A march is exactly two tiles.** `Map_March_InitValues` seeds the
//!   counter with `$20` = 32, `Map_Object_Travel_X/Y` moves one unit per
//!   tick, and a map tile is 16 pixels. Every leg is 2 tiles in one of the
//!   four directions — never one, never a diagonal.
//! - **The tile travelled over must be a path tile of the matching
//!   orientation.** `Map_MarchValidateTravel` checks the adjacent tile
//!   against `Map_Object_Valid_Left/Right` (horizontal) or
//!   `Map_Object_Valid_Down/Up` (vertical). A level number, a fortress, a
//!   toad house or a lock all stop a bro dead — the graph is the *path
//!   network*, not the walkable map.
//! - **Landing on another sprite costs a whole extra march.** When a bro's
//!   counter reaches zero it scans the other map-object slots for one at
//!   rest on the identical tile; on a match `PRG011_AF85` sets *both*
//!   counters back to `$20`. `MO_HammerBroMarch` (PRG010) will not end the
//!   map turn until every counter is zero, so each collision buys another 32
//!   frames for two sprites, with no cap. Bros placed close together collide
//!   repeatedly and the march phase visibly drags.
//!
//! That last fact is why placement measures distance *here* rather than on
//! the grid. Chebyshev distance answers the wrong question twice over: two
//! tiles six columns apart down one corridor are three legs apart, while two
//! tiles six columns apart in different corridors may be unable to reach each
//! other at all — and a pair that cannot meet can never collide.
//!
//! # Deliberate over-approximation
//!
//! The graph models the *travel* check and skips two later ones: the
//! forbidden-landing table (`Map_Object_Forbid_LandingTiles` +
//! `Map_MarchXtraForbidTiles`) and the enterable-landing extension that
//! doubles the counter to `$40` so a bro hops over a node. Both only ever
//! *remove* resting places, so ignoring them can only make the graph more
//! connected than the engine's — which makes distances shorter and separation
//! stricter. The error is on the safe side, and the check that matters for
//! corridor separation (the mid tile) is exact.
//!
//! Locks are treated as open for the same reason: a lock the player has not
//! opened yet will open later, and two bros separated only by a closed lock
//! would drift together the moment it does.

use super::*;

/// Tiles a marching sprite may travel over moving left or right. Mirror of
/// `Map_Object_Valid_Left` / `Map_Object_Valid_Right` in PRG010 (the two are
/// byte-identical; Nintendo duplicated them to leave room for one-way paths).
const MARCH_VALID_HORIZONTAL: [u8; 9] = [
    0x45, // horizontal path
    0xB2, // horizontal drawbridge
    0xB3, // bridge
    0xAC, // horizontal path over water
    0xB7, // ... land on left end
    0xB8, // ... land on right end
    0xDA, // sky horizontal path
    0xB9, // ... land on both ends
    0xE6, // W8 hand trap
];

/// Tiles a marching sprite may travel over moving up or down. Mirror of
/// `Map_Object_Valid_Down` / `Map_Object_Valid_Up`, minus the two repeats of
/// `0x46` the ROM uses to pad all four tables to the same length.
const MARCH_VALID_VERTICAL: [u8; 7] = [
    0x46, // vertical path
    0xB1, // vertical drawbridge
    0xAA, // vertical path over water, land upper
    0xAB, // ... land lower
    0xB0, // vertical path over water
    0xDB, // sky vertical path
    0xBA, // ... land on both ends
];

/// Tiles per march leg — `$20` counter ticks at 1 unit each, over a 16-pixel
/// tile. See the module docs.
const MARCH_LEG: usize = 2;

/// Minimum separation between two Hammer Bro sprites, in march legs.
///
/// A leg is 2 tiles, so 1 means "can land on the other bro in a single
/// march" — which is what the old Chebyshev `>= 2` rule permitted, i.e. the
/// collision distance itself. 3 legs (6 tiles of travel) is satisfiable in
/// 99.6% of multi-bro worlds; 4 legs drops to 94.2% and 5 to 84.2%, measured
/// over 200 seeds. [`pick_spread_positions`] relaxes one rung at a time
/// rather than failing, so the floor costs no placements.
pub(super) const MIN_HB_SEPARATION_LEGS: u32 = 3;

/// Can a sprite travel over the tile at `pos`? `blocked` holds the positions
/// the writer will stamp as nodes (levels, fortresses, pipes, toad houses,
/// spades) — those are still blank path tiles on the build grid, so the tile
/// byte alone would wrongly say yes.
fn traversable(grid: &Grid, blocked: &HashSet<Pos>, pos: Pos, vertical: bool) -> bool {
    if blocked.contains(&pos) {
        return false;
    }
    let tile = grid.get(pos.0, pos.1);
    if vertical {
        MARCH_VALID_VERTICAL.contains(&tile)
    } else {
        MARCH_VALID_HORIZONTAL.contains(&tile)
    }
}

/// Every tile a sprite starting at `start` can reach, and in how many march
/// legs. A tile absent from the map is unreachable — two sprites in that
/// relationship can never collide, however close they look on the grid.
pub(super) fn march_distances(
    grid: &Grid,
    blocked: &HashSet<Pos>,
    start: Pos,
) -> HashMap<Pos, u32> {
    let mut dist: HashMap<Pos, u32> = HashMap::new();
    dist.insert(start, 0);
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(start);

    while let Some(pos) = queue.pop_front() {
        let legs = dist[&pos] + 1;
        let (r, c) = pos;
        // (mid, landing, vertical) for each of the four directions, skipping
        // any that would step off the grid.
        let mut steps: Vec<(Pos, Pos, bool)> = Vec::with_capacity(4);
        if c + MARCH_LEG < grid.cols {
            steps.push(((r, c + 1), (r, c + MARCH_LEG), false));
        }
        if c >= MARCH_LEG {
            steps.push(((r, c - 1), (r, c - MARCH_LEG), false));
        }
        if r + MARCH_LEG < grid.rows() {
            steps.push(((r + 1, c), (r + MARCH_LEG, c), true));
        }
        if r >= MARCH_LEG {
            steps.push(((r - 1, c), (r - MARCH_LEG, c), true));
        }
        for (mid, land, vertical) in steps {
            if !traversable(grid, blocked, mid, vertical) {
                continue;
            }
            if dist.contains_key(&land) {
                continue;
            }
            dist.insert(land, legs);
            queue.push_back(land);
        }
    }
    dist
}

/// Pick `count` positions from `candidates`, spread out in the march graph:
/// greedily accept a shuffled candidate that is at least
/// [`MIN_HB_SEPARATION_LEGS`] legs from every position already chosen — or
/// unreachable from all of them, which is better still.
///
/// Best-effort by design. A cramped world may not hold `count` sprites at the
/// full separation, so the floor is relaxed one leg at a time and, failing
/// that, the remainder is topped up from the leftovers. Placing every
/// encounter matters more than placing them well; the relaxation only bites
/// where the map leaves no choice.
pub(super) fn pick_spread_positions<R: Rng>(
    candidates: &[Pos],
    count: usize,
    grid: &Grid,
    blocked: &HashSet<Pos>,
    rng: &mut R,
) -> Vec<Pos> {
    let count = count.min(candidates.len());
    let mut shuffled = candidates.to_vec();
    shuffled.shuffle(rng);

    let mut chosen: Vec<Pos> = Vec::with_capacity(count);
    // One reachability map per chosen position, computed on acceptance — at
    // most `count` BFS passes per world.
    let mut reach: Vec<HashMap<Pos, u32>> = Vec::with_capacity(count);

    for floor in (1..=MIN_HB_SEPARATION_LEGS).rev() {
        if chosen.len() == count {
            break;
        }
        for &pos in &shuffled {
            if chosen.len() == count {
                break;
            }
            if chosen.contains(&pos) {
                continue;
            }
            // `is_none_or`: absent from a reachability map means that sprite
            // can never reach this tile, which trivially clears the floor.
            if reach.iter().all(|d| d.get(&pos).is_none_or(|&legs| legs >= floor)) {
                reach.push(march_distances(grid, blocked, pos));
                chosen.push(pos);
            }
        }
    }
    // Spacing may still have rejected too many; top up from the rest.
    for &pos in &shuffled {
        if chosen.len() == count {
            break;
        }
        if !chosen.contains(&pos) {
            chosen.push(pos);
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Options, Tri};

    /// Every blank node a Hammer Bro sprite can march onto has a pointer-table
    /// row. The player cannot enter a blank, so a blank no sprite reaches may
    /// go without one — but a bro that lands on one hands the engine a lookup
    /// that matches nothing.
    ///
    /// Read off finished ROMs, with every lock, rock and water gap open: they
    /// open in play, and a sprite then marches through.
    #[test]
    fn every_blank_a_sprite_reaches_has_a_row() {
        let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            return;
        };
        let base = || Options { palettes: false, palette_themed: false, ..Options::default() };
        let arms = [
            ("default", base()),
            ("8s wild", Options { eights_are_wild: Tri::On, ..base() }),
            ("hammer rocks", Options { more_hammer_rocks: Tri::On, ..base() }),
            ("no hb shuffle", Options { shuffle_hammer_bros: false, ..base() }),
            ("generated w1", Options { generated_worlds: 1, ..base() }),
            ("maze", Options { world_maze: true, ..base() }),
        ];
        let mut stranded = Vec::new();
        for (name, opts) in &arms {
            for seed in 0..4 {
                let out = crate::generate_patched_rom(&bytes, seed, opts, None).expect("generate");
                let rom = Rom::from_bytes_lax(&out, true).expect("readable ROM");
                for wi in 0..8 {
                    let world = &rom_data::WORLDS[wi];
                    let rows: HashSet<Pos> = (0..world.entry_count)
                        .map(|i| rom_data::entry_grid_position(&rom, world, i))
                        .collect();
                    let grid = opened(rom_data::read_tile_grid(&rom, wi));
                    // A bro that lands on a sprite at rest marches again
                    // (`PRG011_AF85`), so it never stops on the army's cells.
                    let bros = rom_data::read_hb_sprite_positions(&rom, wi);
                    let parked: HashSet<Pos> = rom_data::read_map_sprite_positions(&rom, wi)
                        .into_iter()
                        .filter(|p| !bros.contains(p))
                        .collect();
                    for &bro in &bros {
                        for pos in march_distances(&grid, &HashSet::new(), bro).into_keys() {
                            let tile = grid.get(pos.0, pos.1);
                            if rom_data::VALID_BLANK_TILES.contains(&tile)
                                && !rows.contains(&pos)
                                && !parked.contains(&pos)
                            {
                                stranded.push(format!("{name} seed {seed} W{} {pos:?}", wi + 1));
                            }
                        }
                    }
                }
            }
        }
        stranded.sort();
        stranded.dedup();
        assert!(stranded.is_empty(), "blank nodes a bro reaches with no row: {stranded:#?}");
    }

    /// `grid` with every lock, breakable rock and water gap replaced by the
    /// path it opens to. Each tile says which way it runs (`$54` and `$52`
    /// vertical; `$56`, `$51` and the bridge `$9D` is cut from horizontal)
    /// except the sky lock `$E4`, which runs across when a neighbour on
    /// either side is a path or a blank.
    fn opened(mut grid: Grid) -> Grid {
        let across = |grid: &Grid, r: usize, c: usize| {
            let joins = |t: u8| {
                MARCH_VALID_HORIZONTAL.contains(&t) || rom_data::VALID_BLANK_TILES.contains(&t)
            };
            (c > 0 && joins(grid.get(r, c - 1))) || (c + 1 < grid.cols && joins(grid.get(r, c + 1)))
        };
        for r in 0..grid.rows() {
            for c in 0..grid.cols {
                let open = match grid.get(r, c) {
                    0x54 | rom_data::ROCK_BREAK_V => 0x46,
                    0x56 | rom_data::ROCK_BREAK_H | rom_data::WATER_GAP_TILE => 0x45,
                    0xE4 if across(&grid, r, c) => 0x45,
                    0xE4 => 0x46,
                    _ => continue,
                };
                grid.set(r, c, open);
            }
        }
        grid
    }
}
