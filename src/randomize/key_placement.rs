//! Choosing which site carries which key.
//!
//! Runs on a finished maze with gates already installed, and makes the maze
//! winnable again by putting each gate's key somewhere the player can reach
//! before meeting it.
//!
//! # Place at the stall
//!
//! The solver works in rounds, and normally each round opens more map. When a
//! round opens nothing new and the castle is still out of reach, it has
//! **stalled** — and it hands back the one thing this pass needs: everywhere
//! the player got to before getting stuck. That set is exactly where the key
//! may go, because anything beyond it is behind the very gate the key opens.
//!
//! So "where can this key go" is never searched for. The stall answers it, and
//! the answer is a set to draw from:
//!
//! ```text
//! run solver -> stalled, three worlds unreachable behind water
//!               reached so far: W1, W2, half of W5
//! place      -> an Anchor onto a Hammer Bro in W2
//! run solver -> finished
//! ```
//!
//! **A key therefore cannot land behind its own gate.** Not because anything
//! checks for it, but because the only sites offered were already proven
//! reachable without it. That holds at any number of gates, which is why this
//! is a loop over stalls rather than a search over placements — contrast
//! `item_layer`'s candidate-cut loop on `refactor/items-before-pickup`, which
//! is generate-and-test because it is choosing *where to cut*. Keys need no
//! such thing.
//!
//! # A gate that cannot be keyed is dropped, not the mode
//!
//! If the stalled reach holds no free site, this uninstalls that one gate and
//! carries on. The alternative — vetoing the whole mode, as `item_layer`'s
//! `Placement::installed` does — is survivable for one gate and collapses for
//! several: with N gates each keyable with probability p, the mode installs at
//! roughly p^N. Dropping one gate degrades into exactly today's behaviour for
//! that gate and leaves the rest doing their job.
//!
//! **The caller must honour `dropped`.** The ROM patches gate the water
//! whatever the model decided, so a gate this pass gave up on has to be left
//! uninstalled on the ROM side too. Gating without a model that agrees is how
//! a seed strands a player.

use std::collections::HashSet;

use rand::Rng;
use rand::seq::IndexedRandom;

use super::item_keys::Key;
use super::key_sites::{self, KeySite};
use super::maze::{Gate, GlobalState, KeySource};
use super::overworld_build::BuildResult;
use crate::rom::Rom;

/// What a run of [`place`] decided.
#[derive(Default, Debug, Clone)]
pub(crate) struct Placement {
    /// Keys written, in the order they were placed.
    pub placed: Vec<Placed>,
    /// Gates no reachable site could key. **The caller must not install
    /// these on the ROM.**
    pub dropped: Vec<Gate>,
    /// Sites that already held a key the gates want, credited without a
    /// write. Most seeds have one: ~79% hold an Anchor somewhere already.
    pub credited: Vec<Placed>,
}

/// One key, and where it went.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Placed {
    pub key: Key,
    pub site: KeySite,
}

/// Belt and braces. Each pass either places a key (one fewer missing key) or
/// drops a gate (one fewer gate), so the loop cannot run forever — but a
/// future gate kind that violates that would hang rather than fail, and a
/// hang is the worst way to find out.
const MAX_PASSES: usize = 64;

/// Key every installed gate, dropping the ones that cannot be keyed.
///
/// Three parts, in order:
///
/// 1. **Credit what the seed already has.** A site holding a key the gates
///    want *is* a source, and saying so costs nothing and writes nothing.
///    Most seeds are keyed by luck already, so this is usually the whole job.
/// 2. **The stall loop**, for gates luck did not cover.
/// 3. **Availability.** Every surviving gate's keys must exist *somewhere*,
///    even when the solver never stalled on that gate. A gate whose key is
///    nowhere is a permanent wall, and `solvable` would not notice: it asks
///    that the castle be reachable and every fortress beatable, not that
///    every level be. A canoe gated with no Anchor anywhere would seal the
///    levels behind the water and report a fine seed.
///
/// `sites` is the pool to draw from; a spent site is removed, since a second
/// write to it would overwrite the first key. Takes it as an argument rather
/// than building it so a caller can narrow the pool and a test can empty it.
///
/// Sites are drawn **uniformly**. Preferring sites that hold nothing — a
/// Hammer Bro left with no reward costs nothing to overwrite, where one
/// holding a P-Wing costs a P-Wing — is available via
/// [`KeySite::current_item`] but is not done here: it is a policy, and no
/// measurement has asked for it yet.
// Reason: the `world_maze` wiring is the production caller, and lands with the
// option.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn place<R: Rng>(
    rom: &mut Rom,
    build: &mut BuildResult,
    state: &mut GlobalState,
    mut sites: Vec<KeySite>,
    rng: &mut R,
) -> Placement {
    let mut out = Placement::default();

    // 1. Credit keys the seed already holds. A site is removed once credited:
    //    writing another key over it would destroy the one already there.
    let wanted: Vec<Key> = state
        .gates
        .iter()
        .flat_map(|g| g.needs.iter().copied())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let mut i = 0;
    while i < sites.len() {
        let held = sites[i].current_item(rom, build);
        match wanted.iter().copied().find(|k| k.item_byte() == held) {
            Some(key) => {
                let site = sites.swap_remove(i);
                state.sources.push(KeySource { pos: site.pos, key });
                out.credited.push(Placed { key, site });
            }
            None => i += 1,
        }
    }

    // 2. The stall loop.
    for _ in 0..MAX_PASSES {
        let (spheres, reach) = state.spheres_and_reach();
        if spheres.solvable {
            ensure_available(rom, build, state, &mut sites, rng, &mut out);
            return out;
        }

        // Which gate came to rest shut, and which of its keys is missing.
        let stuck =
            state.gates.iter().position(|g| !g.needs.iter().all(|k| spheres.found.contains(k)));
        let Some(gi) = stuck else {
            // Unsolvable with every gate open. Not this pass's doing and not
            // its to fix — the generator guarantees a solvable maze.
            ensure_available(rom, build, state, &mut sites, rng, &mut out);
            return out;
        };
        let key = *state.gates[gi]
            .needs
            .iter()
            .find(|k| !spheres.found.contains(k))
            .expect("the gate is shut, so a key is missing");

        // Only sites the stall already reached. This is the whole guarantee.
        let reachable: Vec<usize> =
            (0..sites.len()).filter(|&i| reach.contains(sites[i].pos)).collect();

        match reachable.choose(rng) {
            Some(&i) => {
                let site = sites.swap_remove(i);
                key_sites::grant(rom, build, &site, key);
                state.sources.push(KeySource { pos: site.pos, key });
                out.placed.push(Placed { key, site });
            }
            None => out.dropped.push(state.gates.remove(gi)),
        }
    }

    debug_assert!(false, "key placement did not settle in {MAX_PASSES} passes");
    out
}

/// Make sure every surviving gate's keys exist somewhere reachable.
///
/// The stall loop only keys gates that *stopped* the solver. A gate the run
/// never had to pass still needs its key to exist, or it is a wall on whatever
/// it guards — and for the canoe that is every level behind the water, which
/// `Spheres::solvable` does not protect.
///
/// A gate whose key cannot be put anywhere reachable is dropped, exactly as in
/// the stall loop: a gate nothing can open is worse than no gate.
fn ensure_available<R: Rng>(
    rom: &mut Rom,
    build: &mut BuildResult,
    state: &mut GlobalState,
    sites: &mut Vec<KeySite>,
    rng: &mut R,
    out: &mut Placement,
) {
    let mut gi = 0;
    while gi < state.gates.len() {
        let missing: Vec<Key> = state.gates[gi]
            .needs
            .iter()
            .copied()
            .filter(|k| !state.sources.iter().any(|s| s.key == *k))
            .collect();
        if missing.is_empty() {
            gi += 1;
            continue;
        }

        let (_, reach) = state.spheres_and_reach();
        let mut dropped = false;
        for key in missing {
            let reachable: Vec<usize> =
                (0..sites.len()).filter(|&i| reach.contains(sites[i].pos)).collect();
            match reachable.choose(rng) {
                Some(&i) => {
                    let site = sites.swap_remove(i);
                    key_sites::grant(rom, build, &site, key);
                    state.sources.push(KeySource { pos: site.pos, key });
                    out.placed.push(Placed { key, site });
                }
                None => {
                    out.dropped.push(state.gates.remove(gi));
                    dropped = true;
                    break;
                }
            }
        }
        if !dropped {
            gi += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    use super::*;
    use crate::randomize::key_sites::test_support::{load_rom, one_maze};
    use crate::randomize::maze::GateTarget;

    /// The first seed in `0..60` whose maze genuinely needs the boat, with
    /// every canoe already gated on the Anchor.
    fn a_boat_required_seed() -> Option<(Rom, BuildResult, GlobalState)> {
        let raw = load_rom()?;
        for seed in 0..60u64 {
            let (rom, build, mut state) = one_maze(&raw, seed);
            if !state.spheres().solvable {
                continue;
            }
            state.gate_every_canoe(Key::Anchor);
            if !state.spheres().solvable {
                return Some((rom, build, state));
            }
        }
        panic!("no seed in 0..60 required the boat — the census says ~18% should");
    }

    /// **The gated maze ends up winnable, with an Anchor the stall could
    /// reach.**
    ///
    /// Whether that Anchor was written or was already there does not matter to
    /// the player, so the assertion is on the outcome: a source inside the
    /// reach the run came to rest at. Most seeds take the credited path —
    /// ~79% already hold an Anchor somewhere.
    #[test]
    fn the_pass_reopens_a_stalled_maze() {
        let Some((mut rom, mut build, mut state)) = a_boat_required_seed() else { return };
        let (_, before) = state.spheres_and_reach();

        let sites = crate::randomize::key_sites::sites(&rom, &build, &state);
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let out = place(&mut rom, &mut build, &mut state, sites, &mut rng);

        assert!(out.dropped.is_empty(), "nothing should be undoable here: {out:?}");
        assert!(state.spheres().solvable, "the maze should be winnable again");
        assert!(
            state.sources.iter().any(|s| s.key == Key::Anchor && before.contains(s.pos)),
            "no Anchor the stall could reach: {out:?}"
        );
    }

    /// **With no Anchor to inherit, the pass writes one — inside the stall.**
    ///
    /// The seed is offered only sites that hold no Anchor, so the credit pass
    /// finds nothing and the stall loop has to do the work. That is the ~18%
    /// of boat-required seeds luck does not cover, which is the case this
    /// whole feature exists for.
    #[test]
    fn with_nothing_to_inherit_the_pass_writes_a_key() {
        let Some((mut rom, mut build, mut state)) = a_boat_required_seed() else { return };
        let (_, before) = state.spheres_and_reach();

        let anchor = Key::Anchor.item_byte();
        let sites: Vec<KeySite> = crate::randomize::key_sites::sites(&rom, &build, &state)
            .into_iter()
            .filter(|s| s.current_item(&rom, &build) != anchor)
            .collect();
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let out = place(&mut rom, &mut build, &mut state, sites, &mut rng);

        assert!(out.credited.is_empty(), "no site held an Anchor to credit: {out:?}");
        assert_eq!(out.placed.len(), 1, "one Anchor opens every canoe: {out:?}");
        assert!(before.contains(out.placed[0].site.pos), "written outside the stall");
        assert!(state.spheres().solvable, "the maze should be winnable again");
    }

    /// **A site behind the gate is never drawn, even when it is the only one.**
    ///
    /// The guarantee this whole pass exists for, and it needs a decoy to be
    /// tested at all: measured on real seeds *every* site sits inside the
    /// canoe-free reach (21 of 21 on the boat-required seed in `0..40`),
    /// because the canoe seals a pocket rather than a world. So a pass that
    /// ignored the reach entirely would still draw a legal site by luck and
    /// every other test here would stay green. Confirmed by mutation:
    /// deleting the reach filter passed the suite until this existed.
    ///
    /// **One gate, deliberately.** With every canoe gated, dropping one frees
    /// that boat and can legitimately bring a formerly stranded cell into
    /// reach for the next gate — correct behaviour, but it muddies what this
    /// is asserting. Gating a single canoe removes the cascade.
    #[test]
    fn a_site_behind_the_gate_is_never_drawn() {
        let Some((mut rom, mut build, mut state)) = a_boat_required_seed() else { return };

        // One canoe whose gate alone stalls the run.
        let lone = (0..state.worlds.len())
            .find(|&w| {
                state.gates = vec![Gate { target: GateTarget::Canoe(w), needs: vec![Key::Anchor] }];
                !state.spheres().solvable
            })
            .expect("some single canoe is required on a boat-required seed");
        state.gates = vec![Gate { target: GateTarget::Canoe(lone), needs: vec![Key::Anchor] }];

        let (_, reach) = state.spheres_and_reach();
        let stranded = state
            .worlds
            .iter()
            .flat_map(|w| w.slots.iter().map(move |s| (w.world_idx, s.pos)))
            .find(|p| !reach.contains(*p))
            .expect("a stalled maze has content it could not reach");

        // A sink holding something other than an Anchor, so the decoy has to
        // be *drawn* to be used rather than merely credited.
        let anchor = Key::Anchor.item_byte();
        let real = *crate::randomize::key_sites::sites(&rom, &build, &state)
            .iter()
            .find(|s| s.current_item(&rom, &build) != anchor)
            .expect("some site holds no Anchor");
        let decoy = KeySite { pos: stranded, sink: real.sink };

        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let out = place(&mut rom, &mut build, &mut state, vec![decoy], &mut rng);

        assert!(out.placed.is_empty(), "a key was placed behind its own gate: {out:?}");
        assert_eq!(out.dropped.len(), 1, "with no usable site the gate must be dropped");
        assert!(state.gates.is_empty(), "the dropped gate was left installed");
    }

    /// **With nowhere to put a key, the gate is dropped — not the mode.**
    ///
    /// And the maze is winnable afterwards, which is the point: a gate the
    /// pass gave up on leaves the seed exactly as it would have been without
    /// that gate, rather than failing the whole run.
    #[test]
    fn a_gate_with_no_site_is_dropped() {
        let Some((mut rom, mut build, mut state)) = a_boat_required_seed() else { return };
        let gated = state.gates.len();
        let mut rng = ChaCha8Rng::seed_from_u64(7);

        let out = place(&mut rom, &mut build, &mut state, Vec::new(), &mut rng);

        assert!(out.placed.is_empty(), "no sites, so nothing can be placed");
        assert!(!out.dropped.is_empty(), "the blocking gate should have been dropped");
        assert!(
            out.dropped.iter().all(|g| matches!(g.target, GateTarget::Canoe(_))),
            "only canoe gates exist here: {out:?}"
        );
        assert_eq!(state.gates.len(), gated - out.dropped.len(), "dropped gates left installed");
        assert!(state.spheres().solvable, "dropping the gate should restore the seed");
    }

    /// **A seed the boat is optional on still gets an Anchor.**
    ///
    /// The rule the availability pass exists for. Nothing stalls here, so the
    /// stall loop does nothing — but the canoes are still gated, and a gate
    /// whose key exists nowhere is a permanent wall. `Spheres::solvable` would
    /// not notice: it asks that the castle be reachable and every fortress
    /// beatable, not that every level be, so the water could seal off ordinary
    /// levels and the seed would still report fine.
    ///
    /// Offered only sites holding no Anchor, so there is nothing to inherit
    /// and the pass has to write one.
    #[test]
    fn an_unstalled_maze_still_gets_an_anchor() {
        let Some(raw) = load_rom() else { return };
        for seed in 0..60u64 {
            let (mut rom, mut build, mut state) = one_maze(&raw, seed);
            if !state.spheres().solvable {
                continue;
            }
            state.gate_every_canoe(Key::Anchor);
            if !state.spheres().solvable {
                continue; // boat required — the stall tests' business
            }

            let anchor = Key::Anchor.item_byte();
            let sites: Vec<KeySite> = crate::randomize::key_sites::sites(&rom, &build, &state)
                .into_iter()
                .filter(|s| s.current_item(&rom, &build) != anchor)
                .collect();
            let mut rng = ChaCha8Rng::seed_from_u64(7);
            let out = place(&mut rom, &mut build, &mut state, sites, &mut rng);

            assert!(out.dropped.is_empty(), "nothing needed dropping: {out:?}");
            assert!(out.credited.is_empty(), "no site held an Anchor: {out:?}");
            assert_eq!(out.placed.len(), 1, "an Anchor must exist even unstalled: {out:?}");
            assert!(
                state.sources.iter().any(|s| s.key == Key::Anchor),
                "the model must know the Anchor it wrote: {out:?}"
            );
            assert!(state.spheres().solvable, "placing a key cannot break a seed");
            return;
        }
        panic!("no seed in 0..60 left the boat optional — the census says ~82% should");
    }

    /// **An Anchor the seed already holds is credited, not duplicated.**
    #[test]
    fn an_inherited_anchor_is_not_written_again() {
        let Some(raw) = load_rom() else { return };
        for seed in 0..60u64 {
            let (mut rom, mut build, mut state) = one_maze(&raw, seed);
            if !state.spheres().solvable {
                continue;
            }
            state.gate_every_canoe(Key::Anchor);
            let sites = crate::randomize::key_sites::sites(&rom, &build, &state);
            let anchor = Key::Anchor.item_byte();
            if !sites.iter().any(|s| s.current_item(&rom, &build) == anchor) {
                continue; // this seed has nothing to inherit
            }

            let mut rng = ChaCha8Rng::seed_from_u64(7);
            let out = place(&mut rom, &mut build, &mut state, sites, &mut rng);

            assert!(!out.credited.is_empty(), "the Anchor already there was not credited");
            assert!(out.placed.is_empty(), "wrote an Anchor over a seed that had one: {out:?}");
            assert!(state.spheres().solvable);
            return;
        }
        panic!("no seed in 0..60 held an Anchor at a site — the census says ~79% should");
    }
}
