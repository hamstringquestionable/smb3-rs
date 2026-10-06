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
