//! Embed a [`Mission`] into a map: find fort positions and lock tiles that make
//! the mission true, or `None` if the map can't host it.
//!
//! The whole job is filling in one table. For a chain "fort 0 gates fort 1,
//! fort 1 gates the goal":
//!
//! | fort | its lock must cut off… | pick a lock that does + a spot in front of it |
//! |------|------------------------|-----------------------------------------------|
//! | 0    | fort 1                 | …                                             |
//! | 1    | the goal               | …                                             |
//!
//! We try (spot, lock) pairs until every row is filled, then confirm the whole
//! layout is actually beatable with [`verify::realizes`]. Whatever `embed`
//! returns is correct by construction — there is no "give up and place a
//! meaningless lock" fallback (the failure mode of the geometry-first builder).
//!
//! Two things keep the search fast on real worlds:
//! - `processing_order` places a `ChainLink`'s target BEFORE the fort that gates
//!   it, so when we pick that fort's lock we already know where the target is.
//! - `lock_role_plausible` rejects hopeless (spot, lock) pairs immediately,
//!   using `GridMap`'s precomputed "what does each lock cut off" table.
//!
//! The `<M: MapView>` on `embed` just means "works for any map type" — see the
//! header of `map.rs` for why that trait exists.

use std::collections::HashSet;

use super::map::MapView;
use super::verify::realizes;
use super::{Embedding, Mission, Role};

/// Find an embedding of `mission` into `map`, or `None`.
pub(crate) fn embed<M: MapView>(mission: &Mission, map: &M) -> Option<Embedding> {
    let n = mission.fort_count();
    let Some(order) = processing_order(mission) else {
        return None; // cyclic mission — not embeddable
    };
    let mut search = Search {
        mission,
        map,
        order,
        fort_pos: vec![usize::MAX; n],
        lock_pos: vec![usize::MAX; n],
        used_pos: HashSet::new(),
        used_lock: HashSet::new(),
    };
    search.assign(0).then(|| Embedding {
        fort_pos: search.fort_pos.clone(),
        lock_pos: search.lock_pos.clone(),
    })
}

/// Order forts so a `ChainLink`'s targets are ALL assigned BEFORE the fort
/// that gates them — then, when we pick the gating fort's lock, every target's
/// position is already known and we can prune to locks that strand them all.
/// `None` if the gating relation is cyclic (no valid order). For a chain
/// `0->1->..->goal` this is simply the reverse: goal-fort first; for a fork,
/// the terminal group first, then its entrance link.
fn processing_order(mission: &Mission) -> Option<Vec<usize>> {
    let n = mission.fort_count();
    let mut placed = vec![false; n];
    let mut order = Vec::with_capacity(n);
    while order.len() < n {
        let mut progressed = false;
        for i in 0..n {
            if placed[i] {
                continue;
            }
            let ready = match &mission.roles[i] {
                Role::ChainLink { targets } => targets.iter().all(|t| placed[*t]),
                _ => true,
            };
            if ready {
                placed[i] = true;
                order.push(i);
                progressed = true;
            }
        }
        if !progressed {
            return None; // cycle
        }
    }
    Some(order)
}

struct Search<'a, M: MapView> {
    mission: &'a Mission,
    map: &'a M,
    /// Fort indices in assignment order (targets before their gating forts).
    order: Vec<usize>,
    fort_pos: Vec<usize>,
    lock_pos: Vec<usize>,
    used_pos: HashSet<usize>,
    used_lock: HashSet<usize>,
}

impl<M: MapView> Search<'_, M> {
    /// Assign the fort at `order[step]`, then recurse. Returns true once a full
    /// assignment verifies; on success `fort_pos`/`lock_pos` hold the answer.
    fn assign(&mut self, step: usize) -> bool {
        if step == self.order.len() {
            let emb = Embedding {
                fort_pos: self.fort_pos.clone(),
                lock_pos: self.lock_pos.clone(),
            };
            return realizes(self.mission, self.map, &emb);
        }
        let i = self.order[step];

        for &pos in self.map.fort_slots() {
            if self.used_pos.contains(&pos) {
                continue;
            }
            for &lock in self.map.lockable() {
                if self.used_lock.contains(&lock) || lock == pos {
                    continue;
                }
                if !self.lock_role_plausible(i, pos, lock) {
                    continue;
                }

                self.fort_pos[i] = pos;
                self.lock_pos[i] = lock;
                self.used_pos.insert(pos);
                self.used_lock.insert(lock);

                if self.assign(step + 1) {
                    return true;
                }

                self.used_pos.remove(&pos);
                self.used_lock.remove(&lock);
            }
        }
        self.fort_pos[i] = usize::MAX;
        self.lock_pos[i] = usize::MAX;
        false
    }

    /// Necessary conditions on fort `i`'s lock, checked before recursing. The
    /// leaf `realizes` does the full check; these just prune the search.
    fn lock_role_plausible(&self, i: usize, pos: usize, lock: usize) -> bool {
        // No fort can sit behind its own lock (it must stay reachable to beat).
        if self.map.strands(lock, pos) {
            return false;
        }
        match &self.mission.roles[i] {
            Role::GoalGate => self.map.strands(lock, self.map.goal()),
            Role::Safe => !self.map.strands(lock, self.map.goal()),
            // Targets are already placed (processing order guarantees it), so
            // the lock must strand every one of their positions.
            Role::ChainLink { targets } => targets.iter().all(|t| {
                let tpos = self.fort_pos[*t];
                tpos != usize::MAX && self.map.strands(lock, tpos)
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::map::Map;
    use super::*;

    /// Single fort gating the goal on a linear corridor:
    ///   0=start — 1 — 2 — 3 — 4=goal ; fort slots {1,2}, lockable {2,3}.
    #[test]
    fn single_gate_one_fort() {
        let mut m = Map::new(5, 0, 4);
        m.edge(0, 1).edge(1, 2).edge(2, 3).edge(3, 4);
        m.fort_slots = vec![1, 2];
        m.lockable = vec![2, 3];

        let mission = Mission { roles: vec![Role::GoalGate] };
        let emb = embed(&mission, &m).expect("should embed");

        // The lock must actually gate the goal, and the fort sits before it.
        assert!(m.strands(emb.lock_pos[0], m.goal));
        assert!(!m.strands(emb.lock_pos[0], emb.fort_pos[0]));
    }

    /// One GoalGate + one Safe decoy, on a map with a goal branch and a
    /// separate dead-end branch:
    ///   0=start — 1 — 2 — 3=goal        (lock 2 gates the goal)
    ///   0 — 4                           (a second reachable fort slot)
    ///   1 — 5 — 6                       (lock 5 gates {6}: a Safe decoy region)
    #[test]
    fn single_gate_with_safe_decoy() {
        let mut m = Map::new(7, 0, 3);
        m.edge(0, 1).edge(1, 2).edge(2, 3).edge(0, 4).edge(1, 5).edge(5, 6);
        m.fort_slots = vec![1, 4];
        m.lockable = vec![2, 5];

        let mission = Mission { roles: vec![Role::GoalGate, Role::Safe] };
        let emb = embed(&mission, &m).expect("should embed");

        // Fort 0 is the GoalGate: its lock gates the goal.
        assert!(m.strands(emb.lock_pos[0], m.goal));
        // Fort 1 is Safe: its lock gates neither the goal nor any fort.
        let safe_strand = m.strand_set(emb.lock_pos[1]);
        assert!(!safe_strand.contains(&m.goal));
        assert!(!safe_strand.contains(&emb.fort_pos[0]));
        assert!(!safe_strand.contains(&emb.fort_pos[1]));
    }

    /// The goal has two independent approaches, so no single lock can gate it —
    /// a GoalGate mission is unembeddable and `embed` must say so (rather than
    /// silently degrade).
    ///   0=start — 1 — 3=goal, and 0 — 2 — 3.
    #[test]
    fn ungateable_goal_returns_none() {
        let mut m = Map::new(4, 0, 3);
        m.edge(0, 1).edge(1, 3).edge(0, 2).edge(2, 3);
        m.fort_slots = vec![1, 2];
        m.lockable = vec![1, 2];

        let mission = Mission { roles: vec![Role::GoalGate] };
        assert!(embed(&mission, &m).is_none());
    }

    /// Two-fort chain: fort 0's lock gates fort 1, fort 1's lock gates the goal.
    ///   0=start — 1 — 2 — 3 — 4 — 5=goal ; forts {1,3}, locks {2,4}.
    #[test]
    fn two_fort_chain() {
        let mut m = Map::new(6, 0, 5);
        m.edge(0, 1).edge(1, 2).edge(2, 3).edge(3, 4).edge(4, 5);
        m.fort_slots = vec![1, 3];
        m.lockable = vec![2, 4];

        let mission = Mission {
            roles: vec![Role::ChainLink { targets: vec![1] }, Role::GoalGate],
        };
        let emb = embed(&mission, &m).expect("should embed");

        assert!(m.strands(emb.lock_pos[0], emb.fort_pos[1])); // link gates next fort
        assert!(m.strands(emb.lock_pos[1], m.goal)); // last gates the goal
    }

    /// Three-fort chain along a longer corridor. `embed` must place the forts in
    /// depth order (a nearer fort can't be gated behind a farther one without
    /// stranding itself), which it finds by search.
    #[test]
    fn three_fort_chain() {
        let mut m = Map::new(8, 0, 7);
        m.edge(0, 1).edge(1, 2).edge(2, 3).edge(3, 4).edge(4, 5).edge(5, 6).edge(6, 7);
        m.fort_slots = vec![1, 3, 5];
        m.lockable = vec![2, 4, 6];

        let mission = Mission {
            roles: vec![
                Role::ChainLink { targets: vec![1] },
                Role::ChainLink { targets: vec![2] },
                Role::GoalGate,
            ],
        };
        let emb = embed(&mission, &m).expect("should embed");

        assert!(m.strands(emb.lock_pos[0], emb.fort_pos[1]));
        assert!(m.strands(emb.lock_pos[1], emb.fort_pos[2]));
        assert!(m.strands(emb.lock_pos[2], m.goal));
    }

    /// Fork behind a chain prefix: fort 0's lock gates the terminal group
    /// {1, 2} as a unit; inside the group, fort 1 is the GoalGate and fort 2 a
    /// Safe decoy.
    ///   0=start — 1(fort) — 2(lock) — 3(hub)
    ///   3 — 4(fort) — 5(lock) — 6=goal    (real branch)
    ///   3 — 7(fort) — 8(lock, dead end)   (decoy branch)
    #[test]
    fn fork_with_prefix() {
        let mut m = Map::new(9, 0, 6);
        m.edge(0, 1).edge(1, 2).edge(2, 3);
        m.edge(3, 4).edge(4, 5).edge(5, 6);
        m.edge(3, 7).edge(7, 8);
        m.fort_slots = vec![1, 4, 7];
        m.lockable = vec![2, 5, 8];

        let mission = Mission {
            roles: vec![
                Role::ChainLink { targets: vec![1, 2] },
                Role::GoalGate,
                Role::Safe,
            ],
        };
        let emb = embed(&mission, &m).expect("should embed");

        // The entrance link strands BOTH terminal forts at once.
        assert!(m.strands(emb.lock_pos[0], emb.fort_pos[1]));
        assert!(m.strands(emb.lock_pos[0], emb.fort_pos[2]));
        // GoalGate gates the goal; the Safe decoy's lock gates nothing.
        assert!(m.strands(emb.lock_pos[1], m.goal));
        let safe_strand = m.strand_set(emb.lock_pos[2]);
        assert!(!safe_strand.contains(&m.goal));
        assert!(!safe_strand.contains(&emb.fort_pos[0]));
        assert!(!safe_strand.contains(&emb.fort_pos[1]));
    }
}
