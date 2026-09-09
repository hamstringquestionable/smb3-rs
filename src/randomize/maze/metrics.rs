//! What a finished maze costs a player, in the unit the player actually feels:
//! **levels beaten**.
//!
//! Nothing in the per-world route scorer answers this. `route_choice` prices a
//! route in abstract points to compare two routes on one map; the question here
//! is a whole-game number a player would recognise — "this seed takes about
//! twenty levels" — and it has to be answered across eight grids with the
//! lock/key dependency in the middle of it.
//!
//! Two numbers, and they measure different things:
//!
//! * [`completion_cost`] — how many levels and fortresses a play-through
//!   actually beats. This is the headline: the length of the game.
//! * [`required_levels`] — how many levels the player has **no choice** about,
//!   in the strict sense that removing one makes the game unwinnable. This is
//!   the mandatory core; everything else is a route decision.
//!
//! The gap between them is the size of the choice the maze offers.

use std::collections::HashSet;

use super::super::overworld_build::SlotKind;
use super::walk::{MazePos, walk_maze, walk_maze_cost};
use super::{FortRef, GlobalState};

/// A play-through's price, in content beaten.
#[derive(Clone, Debug, Default)]
pub(crate) struct CompletionCost {
    /// Levels and fortresses beaten on the way to the castle.
    pub content: usize,
    /// How many of those were fortresses — the keys, as opposed to the road.
    pub forts: usize,
    /// Fortresses the run had to beat that were NOT on the direct route: the
    /// detours the lock/key structure forced.
    pub detours: usize,
    /// Airships the run had to clear purely to satisfy the wand gate.
    pub wand_detours: usize,
    /// False when the castle was never reached.
    pub reached: bool,
}

/// Simulate a player who beats exactly what they must.
///
/// A level tile in SMB3 cannot be walked past — the player has to clear it —
/// so "how far is the castle" is naturally measured in levels, and a Dijkstra
/// that charges 1 for stepping onto an uncleared level or fortress and 0 for
/// everything else answers it directly.
///
/// The lock/key dependency is what makes it more than one Dijkstra. The loop
/// is a lazy player: price the route to the castle with the keys currently
/// held; if it is not reachable, go and beat the **cheapest** fortress that is,
/// which opens its lock, and price it again. Every cell on a route taken is
/// marked cleared, so a level is charged once however many times it is walked
/// over afterwards — which is what the engine does.
///
/// It is an **upper bound** on the true minimum, because a lazy player takes
/// the cheapest next fort rather than the one that leads to the cheapest whole
/// run. Getting the exact minimum means searching over key orders, which is
/// exponential; the bound is tight enough to answer "about how long is this
/// game" and it is honest about which way it errs.
pub(crate) fn completion_cost(state: &GlobalState) -> CompletionCost {
    let bases = state.base_grids(&HashSet::new());
    let links = state.links();

    // Content cells, and which of them is a fortress.
    let mut charged: HashSet<MazePos> = HashSet::new();
    let mut fort_at: Vec<(FortRef, MazePos)> = Vec::new();
    for w in state.worlds.iter().filter(|w| state.in_maze[w.world_idx]) {
        for slot in &w.slots {
            match slot.kind {
                SlotKind::Level => {
                    charged.insert((w.world_idx, slot.pos));
                }
                SlotKind::Fortress => {
                    charged.insert((w.world_idx, slot.pos));
                    fort_at.push((
                        FortRef { world: w.world_idx, section: slot.section },
                        (w.world_idx, slot.pos),
                    ));
                }
                _ => {}
            }
        }
    }

    // Airships are wands, and a wand is a thing the player has to go and get.
    // Everything but the spine's last world (which holds the castle) has one.
    let wand_tiles: Vec<MazePos> = state.wand_tiles();

    let mut cleared: HashSet<MazePos> = HashSet::new();
    let mut beaten: HashSet<FortRef> = HashSet::new();
    let mut held: HashSet<MazePos> = HashSet::new();
    let mut out = CompletionCost::default();

    loop {
        let shut = state.shut_locks(&beaten);
        let view = state.view(&bases, &shut);
        let reach = walk_maze(&view, &links, state.start);
        let cost = walk_maze_cost(&view, &links, state.start, &reach, |p| {
            u32::from(charged.contains(&p) && !cleared.contains(&p))
        });

        // Wands, when the gate still wants them and one is within reach. The
        // gate on World 8's bridge will not lift without K of them, so a run
        // that could walk to the castle still has to go and clear airships —
        // which is the whole reason it exists: without it a pad chain finishes
        // some seeds in ONE level (measured).
        //
        // "Within reach" is the load-bearing half. An airship that is still
        // behind a lock is not a dead end, it is a fortress away, so this
        // falls through to the fort loop below rather than giving up. Getting
        // that wrong made K >= 1 look unwinnable in two seeds out of three.
        if held.len() < state.wands_required as usize {
            let next = wand_tiles
                .iter()
                .filter(|p| !held.contains(p))
                .filter_map(|&p| cost.get(p).map(|c| (c, p)))
                .min_by_key(|&(c, p)| (c, p));
            if let Some((price, pos)) = next {
                out.content += price as usize;
                out.wand_detours += 1;
                for p in cost.path_to(pos) {
                    cleared.insert(p);
                }
                held.insert(pos);
                continue;
            }
        } else if let Some(price) = cost.get(state.goal) {
            out.content += price as usize;
            out.forts += cost
                .path_to(state.goal)
                .iter()
                .filter(|p| fort_at.iter().any(|(_, fp)| fp == *p) && !cleared.contains(p))
                .count();
            out.reached = true;
            return out;
        }

        // The castle is gated. Beat the cheapest fortress still standing.
        let next = fort_at
            .iter()
            .filter(|(f, _)| !beaten.contains(f))
            .filter_map(|&(f, pos)| cost.get(pos).map(|c| (c, f, pos)))
            .min_by_key(|&(c, f, _)| (c, f.world, f.section));
        let Some((price, fort, pos)) = next else {
            return out; // nothing left to open: unsolvable
        };
        out.content += price as usize;
        out.forts += 1;
        out.detours += 1;
        for p in cost.path_to(pos) {
            cleared.insert(p);
        }
        beaten.insert(fort);
    }
}

/// Levels the player has no choice about: blocking one makes the game
/// unwinnable.
///
/// The same cut-vertex question `WorldState::forced_forts` asks of fortresses,
/// asked of every level across the whole maze. A level tile is made
/// impassable — not merely uncleared, but a wall — and the global fixpoint is
/// re-run. If the castle can no longer be reached, or a fortress can no longer
/// be beaten, that level was mandatory.
///
/// Expensive: one global fixpoint per level, ~62 per seed. A census
/// instrument, not something to call in a build.
pub(crate) fn required_levels(state: &GlobalState) -> usize {
    let levels: Vec<MazePos> = state
        .worlds
        .iter()
        .filter(|w| state.in_maze[w.world_idx])
        .flat_map(|w| {
            w.slots.iter().filter(|s| s.kind == SlotKind::Level).map(move |s| (w.world_idx, s.pos))
        })
        .collect();

    let mut count = 0;
    let mut blocked = HashSet::new();
    for &cell in &levels {
        blocked.clear();
        blocked.insert(cell);
        if !state.spheres_with_blocked(&blocked).solvable {
            count += 1;
        }
    }
    count
}
