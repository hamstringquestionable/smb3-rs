//! Where a key can be put, and how to put it there.
//!
//! One list, built once from a finished build, of every spot a key could sit.
//! Each entry pairs the **cell the player stands on** — so the solver can ask
//! whether it is inside a reach — with the **write that puts an item there**.
//!
//! Deliberately split from the pass that chooses. The chooser works in reaches
//! and never learns a ROM offset; this list works in offsets and never learns
//! what a reach is. A third kind of site changes this file alone.
//!
//! # Only sources the player cannot miss
//!
//! Reaching the cell has to *mean* holding the item. A source that can be
//! walked past makes the model looser than the game, and that difference is
//! what strands somebody. Two qualify, and they are the two that need no
//! plumbing beyond a byte:
//!
//! - **Hammer Bro** — beating the encounter hands the reward over. The cell is
//!   the sprite's home tile and the reward is a model field the writer stamps.
//! - **Princess letter** — finishing the world hands it over. The cell is that
//!   world's target.
//!
//! # Why no Toad House
//!
//! Not rejected — deferred, for three reasons worth not re-deriving:
//!
//! 1. **It buys nothing measurable.** `anchor_keyability_census` puts a Hammer
//!    Bro or a letter in front of the water on 100% of boat-required seeds,
//!    both arms, 400 seeds. Houses move 100% to 100%.
//! 2. **It is the expensive sink.** A house's cell is not known here — which
//!    entry lands on which slot is `assign_pool`'s call, downstream — so it
//!    needs a `SlotAssignment::pin`, *and* the pin read back from
//!    `WorldAssignments::unmet_pins`, because `friendlier_levels`, Deja Vu and
//!    the top-up all reshape the deck afterwards. A pin that did not take is
//!    not a key, and believing otherwise is exactly the stranding bug.
//! 3. **The supply is thinner than it looks.** Only 5 of the 22 houses hand
//!    over a fixed item; the other 17 roll it from a 3-wide window when the box
//!    is opened (see [`rom_data::toad_house_reward_is_fixed`]). Those 5 share a
//!    byte per treasure type — 2 Frog houses read one byte, 2 Tanooki another,
//!    1 Hammer a third — so it is 3 distinct levers, not 5.
//!
//! What would bring it back: a gate whose reach is *tight* enough to hold no
//! bro and no airship (the canoe's never is — it seals a pocket, not a world),
//! or a decision that keys should turn up in more than two kinds of place.
//! Step 1 already taught the catalog which houses are fixed, so adding the sink
//! is writing the sink, not re-deriving which ones are usable.
//!
//! An in-level chest does **not** qualify at any price: a level can be beaten
//! without ever opening its chest. Extra keys dealt into chests are a kindness
//! the model must not lean on.

use super::item_keys::Key;
use super::maze::GlobalState;
use super::maze::walk::MazePos;
use super::overworld_build::BuildResult;
use crate::rom::Rom;

/// A spot a key can go, and the write that puts one there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct KeySite {
    /// The cell the player stands on to collect it.
    pub pos: MazePos,
    pub sink: Sink,
}

/// How an item reaches a site. Both arms are a single byte; the difference is
/// only whether it lands in the model or in the ROM.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Sink {
    /// `BuiltWorld::hb_sprites[idx].reward`, which the writer stamps later.
    HammerBro { world: usize, idx: usize },
    /// `LetterItem_ByWorld[world]`, written straight into the ROM.
    Letter { world: usize },
}

impl KeySite {
    /// **What this site already holds.** A site whose item is worth less than
    /// the key is the cheaper one to spend — a Hammer Bro left with no reward
    /// costs nothing to overwrite, where one holding a P-Wing costs a P-Wing.
    ///
    /// Reported rather than acted on: which site to spend is the chooser's
    /// policy, not this list's.
    // Reason: the chooser is the caller, and lands with the placement pass.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn current_item(&self, rom: &Rom, build: &BuildResult) -> u8 {
        match self.sink {
            Sink::HammerBro { world, idx } => build
                .worlds
                .iter()
                .find(|w| w.world_idx == world)
                .map_or(0, |w| w.hb_sprites[idx].reward),
            Sink::Letter { world } => super::items::princess_reward(rom, world),
        }
    }
}

/// Every site on this maze, in a stable order.
///
/// Worlds off the spine are skipped: a world nothing requires is a bonus, and
/// a key hidden there is a key the player may never be sent to look for.
// Reason: the placement pass is the caller, and lands next.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn sites(rom: &Rom, build: &BuildResult, state: &GlobalState) -> Vec<KeySite> {
    let mut out = Vec::new();

    for w in build.worlds.iter().filter(|w| state.in_maze[w.world_idx]) {
        for idx in 0..w.hb_sprites.len() {
            out.push(KeySite {
                pos: (w.world_idx, w.hb_sprites[idx].grid_pos),
                sink: Sink::HammerBro { world: w.world_idx, idx },
            });
        }
    }

    for w in state.worlds.iter().filter(|w| state.in_maze[w.world_idx]) {
        // World 8 ends in Bowser, not a letter.
        let Some(pos) = w.target.filter(|_| w.world_idx < LETTER_WORLDS) else {
            continue;
        };
        // A world whose letter is `$00` grants nothing in vanilla, and
        // `items::randomize` leaves those alone on purpose. Writing a key
        // there would invent a reward the game never had, so it is not a site.
        if super::items::princess_reward(rom, w.world_idx) == 0 {
            continue;
        }
        out.push(KeySite { pos: (w.world_idx, pos), sink: Sink::Letter { world: w.world_idx } });
    }

    out
}

/// Worlds that end in a Princess letter: W1-W7.
const LETTER_WORLDS: usize = 7;

/// Put `key` at `site`.
///
/// Both writes are idempotent and destroy whatever the site held, which is
/// why [`KeySite::current_item`] exists for the caller to look first.
// Reason: the placement pass is the caller, and lands next.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn grant(rom: &mut Rom, build: &mut BuildResult, site: &KeySite, key: Key) {
    match site.sink {
        Sink::HammerBro { world, idx } => {
            let w = build
                .worlds
                .iter_mut()
                .find(|w| w.world_idx == world)
                .expect("a site names a world of this build");
            w.hb_sprites[idx].reward = key.item_byte();
        }
        Sink::Letter { world } => super::items::set_princess_reward(rom, world, key.item_byte()),
    }
}

/// Shared by this module's tests and `key_placement`'s: both need a real
/// maze, and building one is forty lines of pipeline.
#[cfg(test)]
pub(crate) mod test_support {
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    use super::*;
    use crate::randomize::maze::{self, IDENTITY_SPINE};
    use crate::randomize::node_catalog::NodeCatalog;
    use crate::randomize::overworld_build::{
        BuildFlags, OverworldData, SECRET_EXIT_SLOTS_NEEDED, build,
    };
    use crate::randomize::overworld_pickup::{PickupFlags, pick_up};
    use crate::randomize::{items, start_airship_swap};

    pub(crate) fn load_rom() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    /// One seed, built the way a shipping run builds: the item tables roll
    /// first, so a Hammer Bro's reward is knowable from the model.
    pub(crate) fn one_maze(raw: &Rom, seed: u64) -> (Rom, BuildResult, maze::GlobalState) {
        let mut rom = raw.clone();
        let mut item_rng = ChaCha8Rng::seed_from_u64(seed ^ 0x4954_454D_535F_5631);
        items::randomize(&mut rom, &mut item_rng, false, false);

        let mut catalog = NodeCatalog::build(&rom, false);
        let mut swap_rng = ChaCha8Rng::seed_from_u64(seed);
        start_airship_swap::pick_swaps(&mut catalog, &mut swap_rng);
        let pickup = pick_up(
            &rom,
            &catalog,
            PickupFlags {
                shuffle_spade_games: true,
                shuffle_toad_houses: true,
                shuffle_hammer_bros: true,
            },
        );
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let result = build(
            &rom,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            BuildFlags {
                shuffle_toad_houses: true,
                shuffle_hammer_bros: true,
                ..Default::default()
            },
        );
        let mut mrng = ChaCha8Rng::seed_from_u64(seed ^ 0x5EED_1234);
        let (state, _report) = maze::generate(
            &result,
            &IDENTITY_SPINE,
            maze::DEFAULT_WANDS_REQUIRED,
            &Default::default(),
            SECRET_EXIT_SLOTS_NEEDED,
            &mut mrng,
        );
        (rom, result, state)
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{load_rom, one_maze};
    use super::*;
    use crate::randomize::items;

    /// **Both kinds of site turn up, and each points where it says.**
    ///
    /// The list is only worth anything if the cell it reports is the cell the
    /// player actually stands on — that cell is what gets tested against a
    /// reach, so a site pointing somewhere else is a key placed somewhere
    /// else.
    #[test]
    fn every_site_points_at_its_own_cell() {
        let Some(raw) = load_rom() else { return };
        let (rom, result, state) = one_maze(&raw, 1);
        let sites = sites(&rom, &result, &state);

        let mut bros = 0;
        let mut letters = 0;
        for s in &sites {
            assert!(state.in_maze[s.pos.0], "a site off the spine: {s:?}");
            match s.sink {
                Sink::HammerBro { world, idx } => {
                    bros += 1;
                    let w = result.worlds.iter().find(|w| w.world_idx == world).unwrap();
                    assert_eq!(s.pos, (world, w.hb_sprites[idx].grid_pos), "bro cell");
                }
                Sink::Letter { world } => {
                    letters += 1;
                    let w = state.worlds.iter().find(|w| w.world_idx == world).unwrap();
                    assert_eq!(s.pos, (world, w.target.unwrap()), "letter cell");
                    assert!(world < LETTER_WORLDS, "W8 ends in Bowser, not a letter");
                }
            }
        }
        // Seed 1 measures 15 bros + 6 letters = 21 sites. Six rather than
        // seven because vanilla W7's letter is $00, which is not a site. The
        // floors are loose on purpose: the point is that the supply is deep
        // enough for several gates, not that it is exactly this.
        assert!(bros >= 5, "only {bros} hammer bro sites");
        assert!(letters >= 3, "only {letters} letter sites");
    }

    /// **A world that grants nothing is not a site.**
    ///
    /// `items::randomize` leaves a `$00` letter alone on purpose — that world
    /// hands out no item in vanilla. Offering it as a site would invent a
    /// reward rather than redirect one.
    #[test]
    fn a_world_granting_nothing_is_not_a_site() {
        let Some(raw) = load_rom() else { return };
        let (mut rom, result, state) = one_maze(&raw, 1);

        let victim = match sites(&rom, &result, &state).iter().find_map(|s| match s.sink {
            Sink::Letter { world } => Some(world),
            _ => None,
        }) {
            Some(w) => w,
            None => return,
        };

        items::set_princess_reward(&mut rom, victim, 0);
        let after = sites(&rom, &result, &state);
        assert!(
            !after.iter().any(|s| s.sink == Sink::Letter { world: victim }),
            "W{} grants nothing now and should not be offered",
            victim + 1
        );
    }

    /// **Granting writes where the site points, and nowhere else.**
    #[test]
    fn granting_lands_at_the_site() {
        let Some(raw) = load_rom() else { return };
        let (mut rom, mut result, state) = one_maze(&raw, 1);
        let sites = sites(&rom, &result, &state);

        for kind in [0, 1] {
            let site = *sites
                .iter()
                .find(|s| matches!(s.sink, Sink::HammerBro { .. }) == (kind == 0))
                .expect("one site of each kind");
            let others: Vec<(KeySite, u8)> = sites
                .iter()
                .filter(|s| **s != site)
                .map(|s| (*s, s.current_item(&rom, &result)))
                .collect();

            grant(&mut rom, &mut result, &site, Key::Anchor);
            assert_eq!(
                site.current_item(&rom, &result),
                Key::Anchor.item_byte(),
                "the granted site holds the key: {site:?}"
            );
            for (other, before) in others {
                assert_eq!(
                    other.current_item(&rom, &result),
                    before,
                    "granting at {site:?} disturbed {other:?}"
                );
            }
        }
    }
}
