//! Embed a [`Mission`] into a [`Map`]: find fort positions and lock tiles that
//! realize the mission, or `None` if the map can't host it.
//!
//! A backtracking search over (fort slot × lockable tile) assignments. Each
//! leaf is checked with [`verify::realizes`], so whatever `embed` returns is
//! correct by construction — there is no fallback that silently degrades the
//! mission (the failure mode of the geometry-first builder). Search is naive
//! (fine for the small synthetic maps this slice targets); real-map pruning is
//! a later concern.

use std::collections::HashSet;

use super::map::Map;
use super::verify::realizes;
use super::{Embedding, Mission, Role};

/// Find an embedding of `mission` into `map`, or `None`.
pub(crate) fn embed(mission: &Mission, map: &Map) -> Option<Embedding> {
    let n = mission.fort_count();
    let mut search = Search {
        mission,
        map,
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

struct Search<'a> {
    mission: &'a Mission,
    map: &'a Map,
    fort_pos: Vec<usize>,
    lock_pos: Vec<usize>,
    used_pos: HashSet<usize>,
    used_lock: HashSet<usize>,
}

impl Search<'_> {
    /// Assign fort `i`, then recurse. Returns true once a full assignment
    /// verifies. On success `fort_pos`/`lock_pos` hold the answer.
    fn assign(&mut self, i: usize) -> bool {
        if i == self.mission.fort_count() {
            let emb = Embedding {
                fort_pos: self.fort_pos.clone(),
                lock_pos: self.lock_pos.clone(),
            };
            return realizes(self.mission, self.map, &emb);
        }

        for &pos in &self.map.fort_slots {
            if self.used_pos.contains(&pos) {
                continue;
            }
            for &lock in &self.map.lockable {
                if self.used_lock.contains(&lock) || lock == pos {
                    continue;
                }
                // Cheap, position-independent prune: a GoalGate lock must gate
                // the goal; a Safe lock must not. (ChainLink needs its target's
                // position, so it's left to the leaf check.)
                if !self.lock_role_plausible(i, lock) {
                    continue;
                }

                self.fort_pos[i] = pos;
                self.lock_pos[i] = lock;
                self.used_pos.insert(pos);
                self.used_lock.insert(lock);

                if self.assign(i + 1) {
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

    fn lock_role_plausible(&self, i: usize, lock: usize) -> bool {
        let gates_goal = self.map.strands(lock, self.map.goal);
        match self.mission.roles[i] {
            Role::GoalGate => gates_goal,
            Role::Safe => !gates_goal,
            Role::ChainLink { .. } => true,
        }
    }
}

#[cfg(test)]
mod tests {
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
}
