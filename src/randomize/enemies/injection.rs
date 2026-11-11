//! Wild injection: seed a level-wide chaser into a fraction of real action
//! levels, selected via the `node_catalog` (not raw enemy pointers).
//!
//! Each candidate is a `NodeKind::Level` — fortresses, airships and
//! Bowser are excluded *by type*, so a chaser never lands in a boss room. For
//! each candidate we replace its first enemy with a CHR-compatible chaser the
//! level does not already have. Suns are re-seeded to the vanilla screen-0
//! spawn so they engage (deep suns idle in the background).
//!
//! **This is not a pass.** It runs inside the walker's segment prologue
//! ([`inject_first_enemy`], called from `randomize_object_data`), reading
//! the same [`segment_pins`] the walker seeds itself from, before any pick is
//! made, so every other enemy in the segment is chosen around the chaser.
//! [`collect_injection_sites`] runs up front, because *which* segments a level
//! loads has nothing to do with the shuffle.
//!
//! Guards: boss-type exclusion, one roll per segment however many levels share
//! it, no doubling (a chaser the segment already has), first enemy must be a
//! real swappable/unprotected enemy (don't clobber a critical object), and CHR
//! compatibility.

use std::collections::HashSet;

use crate::randomize::overworld::node_catalog::{NodeCatalog, NodeKind};
use crate::randomize::rom_data::enemy_ptr_to_file_offset;

use super::*;

/// The segments (page-byte index into the `data` buffer) that a real action
/// level loads. Wild injection may replace such a segment's first enemy.
pub(super) type InjectionSites = HashSet<usize>;

/// Build the injectable segments from the node catalog: real action levels
/// only (`NodeKind::Level`), so fortresses, airships and Bowser are excluded by
/// type. A segment several levels share is one site. Consumes no RNG and reads
/// no CHR state — it is pure level geography, which is why it can run before
/// the walk.
pub(super) fn collect_injection_sites(rom: &Rom, opts: &Options) -> InjectionSites {
    let catalog = NodeCatalog::build(rom, opts.include_beta_stages);
    let mut sites = InjectionSites::new();
    for e in &catalog.entries {
        if !matches!(e.kind, NodeKind::Level) {
            continue;
        }
        let Some(le) = &e.level_entry else { continue };
        let obj_ptr = ((le.obj_hi as u16) << 8) | le.obj_lo as u16;
        if obj_ptr < 0xC000 {
            continue;
        }
        let file_off = enemy_ptr_to_file_offset(obj_ptr);
        if (ENEMY_DATA_START..ENEMY_DATA_END).contains(&file_off) {
            sites.insert(file_off - ENEMY_DATA_START);
        }
    }
    sites
}

/// Which chaser an obj_id is, for matching against the player's allowed set.
pub(super) fn chaser_of(id: u8) -> Option<WildChaser> {
    match id {
        ANGRY_SUN_ID => Some(WildChaser::Sun),
        LAKITU_ID => Some(WildChaser::Lakitu),
        BOSS_BASS_ID => Some(WildChaser::Bass),
        _ => None,
    }
}

/// Pick a CHR-compatible chaser this level doesn't already have, from the set
/// the player allowed. Returns `None` if nothing fits — a level that can't take
/// an allowed chaser gets none, rather than one the player didn't ask for.
fn pick_injection<R: Rng>(
    entries: &[SegmentEntry],
    pinned: Pages,
    allowed: &[WildChaser],
    bertha_full: bool,
    rng: &mut R,
) -> Option<u8> {
    let eligible: Vec<u8> = WILD_INJECTION_IDS
        .iter()
        .copied()
        .filter(|&id| {
            // Allowed by the player, CHR-compatible with the segment's pinned
            // pages, not a chaser the level already has (no doubling — e.g.
            // 2-Quicksand's sun), and within the segment's Bertha budget: a
            // Boss Bass is sprite-heavy enough that stacking them starves other
            // objects of slots (see MAX_BERTHA_PER_SEGMENT).
            chaser_of(id).is_some_and(|c| allowed.contains(&c))
                && !(bertha_full && BERTHA_IDS.contains(&id))
                && pinned.fits(id)
                && !entries.iter().any(|e| e.obj_id == id)
        })
        .collect();
    // Favor the sun over the harder two when more than one fits.
    eligible
        .choose_weighted(rng, |&id| if id == ANGRY_SUN_ID { SUN_INJECTION_WEIGHT } else { 1 })
        .ok()
        .copied()
}

/// Maybe inject a chaser into this segment's first enemy; the walker calls it
/// for segments in [`InjectionSites`], from the segment prologue, before the
/// pin scan and before any pick, so a chaser that lands here constrains every
/// pick that follows it.
///
/// Rolls once at [`WILD_INJECTION_CHANCE`]. Returns the `data_index` of the
/// entry it wrote, if any, which the walker treats as fixed for the rest of
/// the segment (see [`is_fixed`]).
///
/// `entries` is updated in step with `data` so the caller's later reads — the
/// Bertha tally, the pin scan, the picks — see the injected enemy.
pub(super) fn inject_first_enemy<R: Rng>(
    data: &mut [u8],
    entries: &mut [SegmentEntry],
    modes: &ClassModes,
    opts: &Options,
    rng: &mut R,
) -> Option<usize> {
    let first = entries.first()?;
    let first_idx = first.data_index;
    let roll: u8 = rng.random_range(..=255);
    if roll >= WILD_INJECTION_CHANCE {
        return None;
    }

    // The entry we'd replace must be a real, swappable, unprotected enemy —
    // don't clobber a critical object. `find_class_pool` is the test for
    // "the shuffle would have been allowed to touch this anyway".
    if entry_protection_at(ENEMY_DATA_START + first_idx).is_some()
        || find_class_pool(first.obj_id, modes).is_none()
    {
        return None;
    }

    // Pages the segment has already committed. Chasers are level-wide, so the
    // whole segment constrains the pick — and the entry being replaced is
    // skipped, since its enemy is on its way out.
    let (pinned, _) = segment_pins(entries, modes, &[], Some(first_idx));
    // Same exclusion for the Bertha tally: the enemy leaving doesn't count,
    // whatever it was. The walker's own cap can only see swaps, so an injected
    // Bass has to check the budget here.
    let bertha_full = entries
        .iter()
        .filter(|e| e.data_index != first_idx && BERTHA_IDS.contains(&e.obj_id))
        .count() as u8
        >= MAX_BERTHA_PER_SEGMENT;
    let chosen = pick_injection(entries, pinned, &opts.wild_injections, bertha_full, rng)?;

    swap_enemy(data, first_idx, chosen);
    entries[0].obj_id = chosen;
    // The Angry Sun idles in the background unless it spawns on the first
    // screen (with Early Sun on). Injection would otherwise leave it at the
    // replaced enemy's usually-deep position, so re-seed it to the vanilla
    // 2-Quicksand spawn (screen 0, Y=0x11).
    if chosen == ANGRY_SUN_ID {
        data[first_idx + 1] = SUN_SPAWN_X;
        data[first_idx + 2] = SUN_SPAWN_Y;
        entries[0].x_pos = SUN_SPAWN_X;
    } else if chosen == LAKITU_ID && rng.random_range(..2u8) == 0 {
        // Lakitu works at any height, but the inherited Y is usually a low
        // ground-enemy spot (harder). Coin-flip half of them up to the common
        // vanilla Lakitu height; the other half keep the low Y.
        data[first_idx + 2] = LAKITU_ALT_Y;
    }
    // Boss Bass keeps the replaced enemy's position: it homes in on the player
    // from wherever it starts, so unlike the sun there is no spawn that makes
    // it work and no height that makes it fair. Whether it wants a rule of its
    // own is a playtest question.
    Some(first_idx)
}
