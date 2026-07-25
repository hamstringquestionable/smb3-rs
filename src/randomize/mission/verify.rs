//! Does an [`Embedding`] actually realize its [`Mission`]?
//!
//! Two independent checks:
//! - **Role correctness** — each lock, closed alone, gates exactly what its role
//!   demands (a `GoalGate` gates the goal and no fort; a `Safe` gates neither;
//!   a `ChainLink` gates its target fort). This mirrors `place_locks`' role_ok.
//! - **Completability** — with every lock closed, there is an order in which
//!   every fort can be reached and beaten (opening its lock) until the goal is
//!   reachable. This catches bad *interactions* between locks that the
//!   per-lock role check can't see.

use std::collections::HashSet;

use super::map::Map;
use super::{Embedding, Mission, Role};

/// Nodes reachable while the locks of not-yet-`beaten` forts are closed.
fn reachable_with_beaten(map: &Map, emb: &Embedding, beaten: &HashSet<usize>) -> HashSet<usize> {
    let blocked: HashSet<usize> = (0..emb.lock_pos.len())
        .filter(|i| !beaten.contains(i))
        .map(|i| emb.lock_pos[i])
        .collect();
    map.reachable_blocking(&blocked)
}

/// Simulate progression to a fixpoint: repeatedly reach every fort you can,
/// beat it (opening its lock), and see what that unlocks. Returns the set of
/// forts beaten and whether the goal ends up reachable.
fn analyze(map: &Map, emb: &Embedding) -> (HashSet<usize>, bool) {
    let n = emb.fort_pos.len();
    let mut beaten: HashSet<usize> = HashSet::new();
    loop {
        let reach = reachable_with_beaten(map, emb, &beaten);
        let mut progressed = false;
        for i in 0..n {
            if !beaten.contains(&i) && reach.contains(&emb.fort_pos[i]) {
                beaten.insert(i);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    let goal_reachable = reachable_with_beaten(map, emb, &beaten).contains(&map.goal);
    (beaten, goal_reachable)
}

/// True iff `emb` is a valid, role-correct, completable realization of
/// `mission` on `map`.
pub(crate) fn realizes(mission: &Mission, map: &Map, emb: &Embedding) -> bool {
    let n = mission.fort_count();
    if emb.fort_pos.len() != n || emb.lock_pos.len() != n {
        return false;
    }

    // Structural: forts and locks are distinct valid nodes, and no lock sits on
    // a fort tile.
    let fort_set: HashSet<usize> = emb.fort_pos.iter().copied().collect();
    let lock_set: HashSet<usize> = emb.lock_pos.iter().copied().collect();
    if fort_set.len() != n || lock_set.len() != n {
        return false;
    }
    if !emb.fort_pos.iter().all(|p| map.fort_slots.contains(p)) {
        return false;
    }
    if !emb.lock_pos.iter().all(|l| map.lockable.contains(l)) {
        return false;
    }
    if emb.fort_pos.iter().any(|p| lock_set.contains(p)) {
        return false;
    }

    // Role correctness: each lock, closed alone, gates exactly its role.
    for (i, role) in mission.roles.iter().enumerate() {
        let strand = map.strand_set(emb.lock_pos[i]);
        let strands_a_fort = strand.iter().any(|s| fort_set.contains(s));
        match role {
            Role::GoalGate => {
                if !strand.contains(&map.goal) || strands_a_fort {
                    return false;
                }
            }
            Role::Safe => {
                if strand.contains(&map.goal) || strands_a_fort {
                    return false;
                }
            }
            Role::ChainLink { target } => {
                if !strand.contains(&emb.fort_pos[*target]) {
                    return false;
                }
            }
        }
    }

    // Completability under all locks closed.
    let (beaten, goal_reachable) = analyze(map, emb);
    beaten.len() == n && goal_reachable
}
