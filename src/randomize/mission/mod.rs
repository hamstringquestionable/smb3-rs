//! Mission-first overworld generation (standalone, work in progress).
//!
//! A self-contained lock-and-key mission engine: it decides an abstract
//! progression — which fort's lock gates which fort or the goal — independent
//! of any map geometry, and (in later slices) embeds that mission into a map by
//! placing forts and locks to realize it. See `docs/mission_first_overworld.md`.
//!
//! This module is deliberately NOT wired into the shipping overworld builder.
//! It is developed and tested in isolation; nothing consumes its output until
//! the embed/verify slices and the writer adapter land. Keeping it parallel is
//! the whole point — the current builder stays untouched until this is proven.

// Reason: standalone builder under construction, intentionally not yet consumed
// by the pipeline. Every item here is exercised by this module's own unit tests
// and will be wired in once the embed/verify/adapter slices land (see the
// module docs). The allow comes off the moment the module is consumed.
#![allow(dead_code)]

mod embed;
mod gridmap;
mod map;
mod verify;

// The engine's public surface for the (in-progress) mission-first builder.
pub(crate) use embed::embed;
pub(crate) use gridmap::GridMap;
pub(crate) use map::MapView;

/// A realized mission: each fort placed on a map node, each fort's lock on a
/// map node. Indexed by the same mission-local fort index as [`Mission::roles`]
/// — `fort_pos[i]` is fort `i`'s position, `lock_pos[i]` the lock its defeat
/// opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Embedding {
    pub fort_pos: Vec<usize>,
    pub lock_pos: Vec<usize>,
}

/// The role a fort's lock plays in a mission. Forts are identified by
/// mission-local index (`0..n`); embedding maps each index to a map position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// This fort's lock gates every fort in `targets` — you must beat this
    /// fort to reach them. A plain chain link has one target; a fork's
    /// entrance link gates the whole terminal group at once (one lock, many
    /// forts), so the group becomes reachable together and no fort is
    /// telegraphed as "the one that appeared last".
    ChainLink { targets: Vec<usize> },
    /// This fort's lock gates the goal (airship/Bowser), stranding no fort.
    GoalGate,
    /// This fort's lock gates nothing important — a decoy or optional fort.
    Safe,
}

/// An abstract, geometry-free progression: one [`Role`] per fort.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Mission {
    pub roles: Vec<Role>,
}

impl Mission {
    /// A chain of `n` forts: fort 0 gates fort 1, 1 gates 2, …, and the last
    /// gates the goal. `n == 0` is the empty mission; `n == 1` is a lone
    /// `GoalGate`.
    pub fn chain(n: usize) -> Mission {
        let roles = (0..n)
            .map(|i| {
                if i + 1 < n {
                    Role::ChainLink { targets: vec![i + 1] }
                } else {
                    Role::GoalGate
                }
            })
            .collect();
        Mission { roles }
    }

    pub fn fort_count(&self) -> usize {
        self.roles.len()
    }

    /// Well-formedness of the abstract mission (independent of any map):
    /// every `ChainLink` has at least one target, each in range and not the
    /// fort itself, and the gating relation is acyclic — a cycle would be an
    /// unbeatable deadlock (fort A needs B beaten, B needs A beaten).
    pub fn is_well_formed(&self) -> bool {
        let n = self.roles.len();
        for (i, r) in self.roles.iter().enumerate() {
            if let Role::ChainLink { targets } = r
                && (targets.is_empty() || targets.iter().any(|t| *t >= n || *t == i))
            {
                return false;
            }
        }
        // Acyclic iff a processing order exists: a fort is placeable once all
        // its targets are placed (the same order `embed` assigns in).
        let mut placed = vec![false; n];
        let mut placed_count = 0;
        loop {
            let mut progressed = false;
            for i in 0..n {
                if placed[i] {
                    continue;
                }
                let ready = match &self.roles[i] {
                    Role::ChainLink { targets } => targets.iter().all(|t| placed[*t]),
                    _ => true,
                };
                if ready {
                    placed[i] = true;
                    placed_count += 1;
                    progressed = true;
                }
            }
            if !progressed {
                break;
            }
        }
        placed_count == n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_is_well_formed() {
        // 0 -> 1 -> 2, and fort 2 gates the goal.
        let m = Mission {
            roles: vec![
                Role::ChainLink { targets: vec![1] },
                Role::ChainLink { targets: vec![2] },
                Role::GoalGate,
            ],
        };
        assert!(m.is_well_formed());
    }

    #[test]
    fn fork_is_well_formed() {
        // 0 gates the terminal group {1, 2}: one GoalGate + one decoy.
        let m = Mission {
            roles: vec![
                Role::ChainLink { targets: vec![1, 2] },
                Role::GoalGate,
                Role::Safe,
            ],
        };
        assert!(m.is_well_formed());
    }

    #[test]
    fn empty_targets_rejected() {
        let m = Mission {
            roles: vec![Role::ChainLink { targets: vec![] }, Role::GoalGate],
        };
        assert!(!m.is_well_formed());
    }

    #[test]
    fn single_gate_is_well_formed() {
        let m = Mission {
            roles: vec![Role::Safe, Role::GoalGate, Role::Safe],
        };
        assert!(m.is_well_formed());
    }

    #[test]
    fn self_gate_rejected() {
        let m = Mission {
            roles: vec![Role::ChainLink { targets: vec![0] }],
        };
        assert!(!m.is_well_formed());
    }

    #[test]
    fn out_of_range_target_rejected() {
        let m = Mission {
            roles: vec![Role::ChainLink { targets: vec![5] }],
        };
        assert!(!m.is_well_formed());
    }

    #[test]
    fn cycle_rejected() {
        // 0 -> 1 -> 0 : deadlock.
        let m = Mission {
            roles: vec![
                Role::ChainLink { targets: vec![1] },
                Role::ChainLink { targets: vec![0] },
            ],
        };
        assert!(!m.is_well_formed());
    }

    #[test]
    fn multi_target_cycle_rejected() {
        // 0 gates {1, 2}, but 2 gates 0 — deadlock through the group.
        let m = Mission {
            roles: vec![
                Role::ChainLink { targets: vec![1, 2] },
                Role::GoalGate,
                Role::ChainLink { targets: vec![0] },
            ],
        };
        assert!(!m.is_well_formed());
    }
}
