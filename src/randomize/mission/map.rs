//! Read-only map view for lock-and-key reachability reasoning.
//!
//! An abstract graph — nodes `0..n`, undirected walk edges, a start and goal,
//! candidate fort slots, and lockable path nodes. It is constructed directly in
//! tests (small maps you can verify by eye) and, in a later slice, adapted from
//! the real `Grid` / `walk_map`. The one primitive it provides is `strand_set`:
//! given a closed lock, which nodes does it wall off from the start?

use std::collections::{HashSet, VecDeque};

/// An abstract lock-and-key map. A "lock" is a path node that, when closed,
/// becomes impassable (cannot be entered or walked through).
pub(crate) struct Map {
    n: usize,
    adj: Vec<Vec<usize>>,
    pub start: usize,
    pub goal: usize,
    /// Nodes a fort may be placed on.
    pub fort_slots: Vec<usize>,
    /// Nodes that can hold a lock (walkable path tiles).
    pub lockable: Vec<usize>,
}

impl Map {
    pub fn new(n: usize, start: usize, goal: usize) -> Self {
        Map {
            n,
            adj: vec![Vec::new(); n],
            start,
            goal,
            fort_slots: Vec::new(),
            lockable: Vec::new(),
        }
    }

    /// Add an undirected walk edge. Returns `&mut self` for chaining.
    pub fn edge(&mut self, a: usize, b: usize) -> &mut Self {
        self.adj[a].push(b);
        self.adj[b].push(a);
        self
    }

    /// Nodes reachable from `start` with every node in `blocked` closed (a
    /// closed lock is impassable — it can neither be entered nor traversed).
    pub fn reachable_blocking(&self, blocked: &HashSet<usize>) -> HashSet<usize> {
        let mut seen = HashSet::new();
        if blocked.contains(&self.start) {
            return seen; // start itself blocked — nothing is reachable
        }
        seen.insert(self.start);
        let mut q = VecDeque::from([self.start]);
        while let Some(u) = q.pop_front() {
            for &v in &self.adj[u] {
                if blocked.contains(&v) || seen.contains(&v) {
                    continue;
                }
                seen.insert(v);
                q.push_back(v);
            }
        }
        seen
    }

    /// Nodes reachable from `start`, optionally with one node `removed`.
    pub fn reachable(&self, removed: Option<usize>) -> HashSet<usize> {
        self.reachable_blocking(&removed.into_iter().collect())
    }

    /// The set of nodes a lock at `lock` gates: those reachable from the start
    /// with the lock open but *not* once it is closed (excluding `lock`
    /// itself). Defined as a difference so nodes that were already unreachable
    /// (orphans) are never miscredited to the lock. An empty set means the lock
    /// sits on a redundant path and gates nothing.
    pub fn strand_set(&self, lock: usize) -> HashSet<usize> {
        let open = self.reachable(None);
        let closed = self.reachable(Some(lock));
        open.difference(&closed).copied().filter(|v| *v != lock).collect()
    }

    /// Does a lock at `lock` strand `target` — i.e. is `target` reachable with
    /// the lock open but not once it is closed?
    pub fn strands(&self, lock: usize, target: usize) -> bool {
        self.reachable(None).contains(&target) && !self.reachable(Some(lock)).contains(&target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Linear corridor: 0=start — 1 — 2 — 3 — 4=goal.
    fn linear() -> Map {
        let mut m = Map::new(5, 0, 4);
        m.edge(0, 1).edge(1, 2).edge(2, 3).edge(3, 4);
        m
    }

    #[test]
    fn linear_lock_gates_everything_downstream() {
        let m = linear();
        assert_eq!(m.strand_set(2), HashSet::from([3, 4]));
        assert_eq!(m.strand_set(1), HashSet::from([2, 3, 4]));
        assert!(m.strands(2, 4)); // gates the goal
        assert!(!m.strands(2, 1)); // upstream stays reachable
    }

    /// Hub with two independent branches:
    ///   0=start — 1
    ///   1 — 2 — 3=goal   (goal branch)
    ///   1 — 4 — 5        (decoy branch)
    fn forked() -> Map {
        let mut m = Map::new(6, 0, 3);
        m.edge(0, 1).edge(1, 2).edge(2, 3).edge(1, 4).edge(4, 5);
        m
    }

    #[test]
    fn fork_branches_gate_independently() {
        let m = forked();
        assert_eq!(m.strand_set(2), HashSet::from([3])); // gates only the goal
        assert_eq!(m.strand_set(4), HashSet::from([5])); // gates only the decoy
        assert!(m.strands(2, 3));
        assert!(!m.strands(2, 5)); // the two regions don't interfere
    }

    /// Diamond — two routes to the goal region:
    ///   0=start — 1 — 3, and 0 — 2 — 3, then 3 — 4=goal.
    fn diamond() -> Map {
        let mut m = Map::new(5, 0, 4);
        m.edge(0, 1).edge(1, 3).edge(0, 2).edge(2, 3).edge(3, 4);
        m
    }

    #[test]
    fn redundant_path_gates_nothing() {
        let m = diamond();
        assert!(m.strand_set(1).is_empty()); // route via 2 keeps all reachable
        assert!(m.strand_set(2).is_empty());
        assert_eq!(m.strand_set(3), HashSet::from([4])); // the join gates the goal
    }
}
