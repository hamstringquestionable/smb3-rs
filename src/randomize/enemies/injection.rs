//! Wild injection: seed a level-wide chaser into a fraction of real action
//! levels, selected via the `node_catalog` (not raw enemy pointers).
//!
//! Level-centric design (replaces the old entry-point / `enemy_entry_points`
//! approach). Each candidate is a `NodeKind::Level` — fortresses, airships and
//! Bowser are excluded *by type*, so a chaser never lands in a boss room. For
//! each candidate we replace its first enemy with a CHR-compatible chaser the
//! level does not already have. Suns are re-seeded to the vanilla screen-0
//! spawn so they engage (deep suns idle in the background).
//!
//! **This is not a pass.** It used to be one, running before the walker, which
//! meant re-deriving the CHR state the walker was about to compute — a second
//! copy of a model that had to stay in step with the first. It now happens
//! inside the walker's segment prologue ([`inject_segment_chasers`], called
//! from `randomize_object_data`), reading the same [`segment_pins`] the walker
//! seeds itself from. `collect_candidates` still runs up front, because *where*
//! a level's first enemy lives has nothing to do with the shuffle.
//!
//! Injecting during the prologue rather than after the walk keeps the property
//! that made the old ordering worth its cost: the chaser is fixed before any
//! pick is made, so every other enemy in the segment is chosen around it.
//!
//! Guards: boss-type exclusion, shared-enemy-set de-dup (one physical enemy set
//! injects at most once), no-double (`has_enemy_id`), first-enemy must be a
//! real swappable/unprotected enemy (don't clobber a critical object), and CHR
//! compatibility. All offsets use the `enemy_ptr_to_file_offset` frame — the
//! same one `has_enemy_id` and the rest of the codebase use.

use std::collections::HashSet;

use crate::randomize::node_catalog::{NodeCatalog, NodeKind};
use crate::randomize::rom_data::{enemy_ptr_to_file_offset, has_enemy_id};

use super::*;

/// Collect every `enemy_ptr` value (bytes 2-3 of every 9-byte level
/// header) from every region in [`LEVEL_DATA_REGIONS`]. Retained for the
/// `chr_stats` integration test's distribution analysis; the injection pass
/// itself no longer drives off this (it uses the node catalog).
///
/// Returned values are unique and in first-seen order.
pub fn enemy_entry_points(rom: &Rom) -> Vec<u16> {
    const LEVEL_HEADER_SIZE: usize = 9;
    let mut pts: Vec<u16> = Vec::new();
    let mut seen: std::collections::HashSet<u16> = std::collections::HashSet::new();
    for region in LEVEL_DATA_REGIONS {
        let len = region.end - region.start;
        let data = rom.read_range(region.start, len);
        let mut i = 0usize;
        while i + LEVEL_HEADER_SIZE < data.len() {
            let ep = (data[i + 2] as u16) | ((data[i + 3] as u16) << 8);
            if seen.insert(ep) {
                pts.push(ep);
            }
            i += LEVEL_HEADER_SIZE;
            while i + 2 < data.len() {
                if data[i] == 0xFF {
                    i += 1;
                    break;
                }
                i += region.command_size(data[i], data[i + 2]);
            }
        }
    }
    pts
}

/// Where a level's first enemy sits in the `data` buffer, mapped to the level's
/// `obj_ptr`. The walker consults this by entry index to learn "this byte is
/// some level's first enemy, and here is the level it belongs to" — which it
/// cannot work out itself, because one `$FF` segment routinely spans several
/// levels' enemy pointers.
pub(super) type InjectionSites = std::collections::HashMap<usize, u16>;

/// Build the injectable-level map from the node catalog: real action levels
/// only (`NodeKind::Level`), de-duped by enemy-data location so a shared enemy
/// set is a single candidate. Consumes no RNG and reads no CHR state — it is
/// pure level geography, which is why it can run before the walk.
pub(super) fn collect_injection_sites(rom: &Rom, data: &[u8], opts: &Options) -> InjectionSites {
    collect_candidates(rom, data, opts)
        .into_iter()
        .map(|c| (c.first_idx, c.obj_ptr))
        .collect()
}

/// One candidate level for injection: its enemy-data location as an index into
/// the `data` buffer (the first enemy entry, after any page byte).
struct Candidate {
    obj_ptr: u16,
    /// Index into `data` of the first enemy entry (byte 0 = obj_id).
    first_idx: usize,
}

/// Build the list of injectable levels from the node catalog: real action
/// levels only (`NodeKind::Level`), de-duped by enemy-data location so a shared
/// enemy set is a single candidate.
fn collect_candidates(rom: &Rom, data: &[u8], opts: &Options) -> Vec<Candidate> {
    let catalog = NodeCatalog::build(rom, opts.include_beta_stages);
    let mut out = Vec::new();
    let mut seen: HashSet<usize> = HashSet::new();
    for e in &catalog.entries {
        if !matches!(e.kind, NodeKind::Level) {
            continue; // fortresses / airships / Bowser / non-levels excluded by type
        }
        let Some(le) = &e.level_entry else { continue };
        let obj_ptr = ((le.obj_hi as u16) << 8) | le.obj_lo as u16;
        if obj_ptr < 0xC000 {
            continue;
        }
        let file_off = enemy_ptr_to_file_offset(obj_ptr);
        if !(ENEMY_DATA_START..ENEMY_DATA_END).contains(&file_off) {
            continue;
        }
        let page_idx = file_off - ENEMY_DATA_START;
        if page_idx >= data.len() {
            continue;
        }
        // Skip the leading page-flag byte if present (real obj_ids never
        // overlap 0x00/0x01, so the value is an unambiguous discriminator).
        let first_idx = if matches!(data[page_idx], 0x00 | 0x01) {
            page_idx + 1
        } else {
            page_idx
        };
        if first_idx >= data.len() || data[first_idx] == 0xFF {
            continue; // empty level
        }
        if seen.insert(first_idx) {
            out.push(Candidate { obj_ptr, first_idx });
        }
    }
    out
}

/// Which chaser an obj_id is, for matching against the player's allowed set.
fn chaser_of(id: u8) -> Option<WildChaser> {
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
    rom: &Rom,
    obj_ptr: u16,
    slot4: ChrSlot,
    slot5: ChrSlot,
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
                && is_chr_compatible(id, slot4, slot5)
                && !has_enemy_id(rom, obj_ptr, id)
        })
        .collect();
    // Favor the sun over the harder two when more than one fits.
    eligible
        .choose_weighted(rng, |&id| {
            if id == ANGRY_SUN_ID { SUN_INJECTION_WEIGHT } else { 1 }
        })
        .ok()
        .copied()
}

/// Inject chasers into any of this segment's entries that a level points at as
/// its first enemy. Called from the walker's segment prologue, before the pin
/// scan and before any pick, so a chaser that lands here constrains every pick
/// that follows it.
///
/// Rolls each site independently at [`WILD_INJECTION_CHANCE`] in data order.
/// Returns the `data_index` of every entry it wrote, which the walker treats as
/// fixed for the rest of the segment (see [`is_fixed`]).
///
/// `entries` is updated in step with `data` so the caller's later reads — the
/// Bertha tally, the pin scan, the picks — see the injected enemy.
pub(super) fn inject_segment_chasers<R: Rng>(
    data: &mut [u8],
    rom: &Rom,
    entries: &mut [SegmentEntry],
    sites: &InjectionSites,
    modes: &ClassModes,
    opts: &Options,
    rng: &mut R,
) -> Vec<usize> {
    let mut injected: Vec<usize> = Vec::new();
    for k in 0..entries.len() {
        let first_idx = entries[k].data_index;
        let Some(&obj_ptr) = sites.get(&first_idx) else {
            continue; // not a level's first enemy
        };
        let roll: u8 = rng.random_range(..=255);
        if roll >= WILD_INJECTION_CHANCE {
            continue;
        }

        // The entry we'd replace must be a real, swappable, unprotected enemy —
        // don't clobber a critical object. `find_class_pool` is the test for
        // "the shuffle would have been allowed to touch this anyway".
        if entry_protection_at(ENEMY_DATA_START + first_idx).is_some() {
            continue;
        }
        if find_class_pool(entries[k].obj_id, modes).is_none() {
            continue;
        }

        // Pages the segment has already committed. Chasers are level-wide, so
        // the whole segment constrains the pick — and the entry being replaced
        // is skipped, since its enemy is on its way out.
        let pins = segment_pins(entries, modes, &injected, Some(first_idx));
        // Same exclusion for the Bertha tally: the enemy leaving doesn't count,
        // whatever it was. The walker's own cap can only see swaps, so an
        // injected Bass has to check the budget here.
        let bertha_full = entries
            .iter()
            .filter(|e| e.data_index != first_idx && BERTHA_IDS.contains(&e.obj_id))
            .count() as u8
            >= MAX_BERTHA_PER_SEGMENT;
        let Some(chosen) = pick_injection(
            rom,
            obj_ptr,
            pins.all.0,
            pins.all.1,
            &opts.wild_injections,
            bertha_full,
            rng,
        ) else {
            continue;
        };

        swap_enemy(data, first_idx, chosen);
        entries[k].obj_id = chosen;
        // The Angry Sun idles in the background unless it spawns on the first
        // screen (with Early Sun on). Injection would otherwise leave it at the
        // replaced enemy's usually-deep position, so re-seed it to the vanilla
        // 2-Quicksand spawn (screen 0, Y=0x11).
        if chosen == ANGRY_SUN_ID {
            data[first_idx + 1] = SUN_SPAWN_X;
            data[first_idx + 2] = SUN_SPAWN_Y;
            entries[k].x_pos = SUN_SPAWN_X;
        } else if chosen == LAKITU_ID && rng.random_range(..2u8) == 0 {
            // Lakitu works at any height, but the inherited Y is usually a low
            // ground-enemy spot (harder). Coin-flip half of them up to the
            // common vanilla Lakitu height; the other half keep the low Y.
            data[first_idx + 2] = LAKITU_ALT_Y;
        }
        // Boss Bass keeps the replaced enemy's position: it homes in on the
        // player from wherever it starts, so unlike the sun there is no spawn
        // that makes it work and no height that makes it fair. Whether it wants
        // a rule of its own is a playtest question.
        injected.push(first_idx);
    }
    injected
}
