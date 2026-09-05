//! The null model's telepad placer: uniform-random, roles ignored.
//!
//! This is deliberately the crudest thing that is still a maze. The charter's
//! pad ROLES — hub, island, gated — are queries against machinery that already
//! exists (`islands.rs`, `forced_positions`, `locks.rs`), but a role placer is
//! a lever, and a lever with no measured baseline to beat is taste. So: place
//! pads uniformly, census the result, and argue every later placer as a delta
//! against that.

use rand::Rng;
use rand::seq::{IndexedRandom, SliceRandom};

use super::super::overworld_build::{SlotKind, WorldState};
use super::super::rom_data::{self, Pos};
use super::MazeEdge;

// The budget and the per-world roll live in `graph`, which is the production
// placer. The null model shares them deliberately: a baseline that dealt pads
// at a different rate from the real placer would not be a baseline, it would
// be a different experiment.
pub(crate) use super::graph::{PAD_BUDGET, roll_count};

/// Place pads across all eight worlds, uniformly at random, respecting
/// [`PAD_BUDGET`].
///
/// Worlds are visited in a shuffled order so the budget does not systematically
/// starve W8, and a world's roll is clamped to what is left.
pub(crate) fn place_pads_uniform<R: Rng>(worlds: &[WorldState], rng: &mut R) -> Vec<MazeEdge> {
    let landings: Vec<Vec<Pos>> = worlds.iter().map(landing_candidates).collect();
    let mut order: Vec<usize> = (0..worlds.len()).collect();
    order.shuffle(rng);

    let mut out = Vec::new();
    let mut budget = PAD_BUDGET;
    for wi in order {
        if budget == 0 {
            break;
        }
        let mut sites = pad_sites(&worlds[wi]);
        let want = roll_count(rng).min(budget).min(sites.len());
        sites.shuffle(rng);
        for &pos in sites.iter().take(want) {
            // A destination world is drawn uniformly, same world included —
            // position keying gives a same-world hop for free, so there is no
            // reason for the null model to exclude it.
            let dest_world = rng.random_range(..worlds.len());
            let Some(&dest) = landings[dest_world].choose(rng) else { continue };
            out.push(MazeEdge::Pad { from: (wi, pos), to: (dest_world, dest) });
            budget -= 1;
        }
    }
    out
}

/// Where a pad TILE may go in a finished world.
///
/// Two pools, and the second is the interesting one:
///
/// * **HammerBro slots** — the leftover blank pool by construction
///   (`HammerBroFill` claims every reachable blank the other phases did not),
///   so this is "an ordinary walkable cell holding the least valuable content".
/// * **Blanks the fill never claimed** — i.e. cells the world's own walk does
///   not reach. A pad there is the charter's *island pad*, the shape the whole
///   mode is for, and it falls out of the crude placer for free.
///
/// A pad tile takes no completion bit (the hardware playtest confirmed a pad on
/// a spade panel stays a spade panel, because diverting at enter time means
/// `MO_DoLevelClear` never runs), so the row-7/8 rule does not apply to it.
fn pad_sites(w: &WorldState) -> Vec<Pos> {
    let taken: Vec<Pos> = w.slots.iter().map(|s| s.pos).collect();
    let mut out: Vec<Pos> =
        w.slots.iter().filter(|s| s.kind == SlotKind::HammerBro).map(|s| s.pos).collect();
    for r in 0..w.grid.rows() {
        for c in 0..w.grid.cols {
            let pos = (r, c);
            if rom_data::VALID_BLANK_TILES.contains(&w.grid.get(r, c))
                && !taken.contains(&pos)
                && !w.fixed.contains(&pos)
                && Some(pos) != w.start
                && Some(pos) != w.target
            {
                out.push(pos);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Where a pad may DEPOSIT the player: any placed slot, or the world's start
/// tile. All of these sit on the node lattice, which the 2-tile movement model
/// requires — an arrival on a path tile would leave the player off-grid.
fn landing_candidates(w: &WorldState) -> Vec<Pos> {
    let mut out: Vec<Pos> = w.slots.iter().map(|s| s.pos).collect();
    out.extend(w.start);
    out.sort_unstable();
    out.dedup();
    out
}
