//! The map interface for lock-and-key reachability, plus a synthetic
//! adjacency-backed implementation for tests.
//!
//! [`MapView`] is the whole contract `embed`/`verify` need. Its one required
//! method is `reachable_blocking` — which nodes are reachable from the start
//! with a given set of locks closed. `strand_set` ("what does this lock gate?")
//! and `strands` fall out as defaults. A real world map satisfies this via
//! `walk_map` (see `gridmap.rs`); the synthetic [`Map`] here satisfies it with a
//! plain BFS over hand-written graphs you can verify by eye.

use std::collections::{HashSet, VecDeque};

/// A read-only lock-and-key map: nodes are `usize` ids, with a start, a goal,
/// candidate fort slots, and lockable path nodes. A "lock" closed on a node
/// makes it impassable.
pub(crate) trait MapView {
    fn start(&self) -> usize;
    fn goal(&self) -> usize;
    fn fort_slots(&self) -> &[usize];
    fn lockable(&self) -> &[usize];

    /// Nodes reachable from the start with every node in `blocked` closed.
    fn reachable_blocking(&self, blocked: &HashSet<usize>) -> HashSet<usize>;

    /// The set of nodes a lock at `lock` gates: reachable with it open but not
    /// once it is closed (excluding `lock` itself). Difference form, so nodes
    /// already unreachable are never miscredited to the lock. Empty = the lock
    /// sits on a redundant path and gates nothing.
    fn strand_set(&self, lock: usize) -> HashSet<usize> {
        let open = self.reachable_blocking(&HashSet::new());
        let closed = self.reachable_blocking(&HashSet::from([lock]));
        open.difference(&closed).copied().filter(|v| *v != lock).collect()
    }

    /// Does a lock at `lock` strand `target` — reachable with the lock open but
    /// not once it is closed?
    fn strands(&self, lock: usize, target: usize) -> bool {
        self.reachable_blocking(&HashSet::new()).contains(&target)
            && !self.reachable_blocking(&HashSet::from([lock])).contains(&target)
    }
}

/// A synthetic map: nodes `0..n`, undirected walk edges, built directly in
/// tests. The reference implementation of [`MapView`].
pub(crate) struct Map {
    adj: Vec<Vec<usize>>,
    pub start: usize,
    pub goal: usize,
    pub fort_slots: Vec<usize>,
    pub lockable: Vec<usize>,
}

impl Map {
    pub fn new(n: usize, start: usize, goal: usize) -> Self {
        Map {
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
}

impl MapView for Map {
    fn start(&self) -> usize {
        self.start
    }
    fn goal(&self) -> usize {
        self.goal
    }
    fn fort_slots(&self) -> &[usize] {
        &self.fort_slots
    }
    fn lockable(&self) -> &[usize] {
        &self.lockable
    }

    fn reachable_blocking(&self, blocked: &HashSet<usize>) -> HashSet<usize> {
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
