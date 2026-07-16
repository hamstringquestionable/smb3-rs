//! Wild-injection pass: seed a level-wide chaser (Lakitu or Angry Sun) into a
//! fraction of real action levels, selected via the `node_catalog` (not raw
//! enemy pointers).
//!
//! Level-centric design (replaces the old entry-point / `enemy_entry_points`
//! approach). Each candidate is a `NodeKind::Level` — fortresses, airships and
//! Bowser are excluded *by type*, so a chaser never lands in a boss room. For
//! each candidate we replace its first enemy with a CHR-compatible chaser the
//! level does not already have. Suns are re-seeded to the vanilla screen-0
//! spawn so they engage (deep suns idle in the background).
//!
//! The pool is Lakitu + Angry Sun only. Boss Bass is deliberately excluded: it's
//! a `WATER_ENEMIES` member, so the later walker pass would reshuffle an injected
//! one into an ordinary water enemy. Lakitu and Sun belong to no class pool, so
//! the walker leaves them untouched.
//!
//! Guards: boss-type exclusion, shared-enemy-set de-dup (one physical enemy set
//! injects at most once), no-double (`has_enemy_id`), first-enemy must be a
//! real swappable/unprotected enemy (don't clobber a critical object or get
//! reverted by the walker), and CHR compatibility. All offsets use the
//! `enemy_ptr_to_file_offset` frame — the same one `has_enemy_id` and the rest
//! of the codebase use.

use std::collections::HashSet;

use rand::seq::SliceRandom;

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

/// Pick a CHR-compatible chaser this level doesn't already have. Returns `None`
/// if nothing fits.
fn pick_injection<R: Rng>(
    rom: &Rom,
    obj_ptr: u16,
    slot4: ChrSlot,
    slot5: ChrSlot,
    rng: &mut R,
) -> Option<u8> {
    let eligible: Vec<u8> = WILD_INJECTION_IDS
        .iter()
        .copied()
        .filter(|&id| {
            // CHR-compatible with the segment's pinned pages, and not a chaser
            // the level already has (no doubling — e.g. 2-Quicksand's sun).
            is_chr_compatible(id, slot4, slot5) && !has_enemy_id(rom, obj_ptr, id)
        })
        .collect();
    // Favor the sun over the (harder) Lakitu when both fit.
    eligible
        .choose_weighted(rng, |&id| {
            if id == ANGRY_SUN_ID { SUN_INJECTION_WEIGHT } else { 1 }
        })
        .ok()
        .copied()
}

/// Wild-injection pass. Shuffles the candidate levels for per-seed variety,
/// then rolls each independently at [`WILD_INJECTION_CHANCE`].
pub(super) fn inject_wild_chasers<R: Rng>(
    data: &mut [u8],
    rom: &Rom,
    bounds: &[segment_writer::SegmentBounds],
    opts: &Options,
    rng: &mut R,
) {
    let normal_modes = ClassModes::from_options(opts);
    let mut candidates = collect_candidates(rom, data, opts);
    candidates.shuffle(rng);

    for Candidate { obj_ptr, first_idx } in candidates {
        let roll: u8 = rng.random_range(..=255);
        if roll >= WILD_INJECTION_CHANCE {
            continue;
        }

        // The entry we'd replace must be a real, swappable, unprotected enemy:
        // don't clobber a critical object (find_class_pool guards that it is a
        // shuffleable enemy) and don't get reverted by the walker's protection
        // handlers (SkipSwap / forced-pool / ExcludeHazards).
        let first_id = data[first_idx];
        let fo = ENEMY_DATA_START + first_idx;
        if entry_protection_at(fo).is_some() {
            continue;
        }
        if find_class_pool(first_id, &normal_modes).is_none() {
            continue;
        }

        // Enclosing $FF segment for CHR pinning (chasers are level-wide, so the
        // whole segment's pins constrain the pick).
        let Some(seg) = bounds.iter().find(|b| {
            let s = b.file_offset + 1;
            let end = s + b.entry_count * 3;
            (s..end).contains(&first_idx)
        }) else {
            continue;
        };
        let mut s4 = ChrSlot::Free;
        let mut s5 = ChrSlot::Free;
        for k in 0..seg.entry_count {
            let off = seg.file_offset + 1 + k * 3;
            if off == first_idx {
                continue; // slot being replaced — its enemy goes away
            }
            let fo2 = ENEMY_DATA_START + off;
            if is_pinned(data[off], fo2, &normal_modes) {
                commit_chr_page(data[off], &mut s4, &mut s5);
            }
        }

        let Some(chosen) = pick_injection(rom, obj_ptr, s4, s5, rng) else {
            continue;
        };

        swap_enemy(data, first_idx, chosen);
        // The Angry Sun idles in the background unless it spawns on the first
        // screen (with Early Sun on). Injection would otherwise leave it at the
        // replaced enemy's usually-deep position, so re-seed it to the vanilla
        // 2-Quicksand spawn (screen 0, Y=0x11). The sun becomes the lowest-X
        // entry, keeping the run X-sorted for the writeback.
        if chosen == ANGRY_SUN_ID {
            data[first_idx + 1] = SUN_SPAWN_X;
            data[first_idx + 2] = SUN_SPAWN_Y;
        } else if chosen == LAKITU_ID && rng.random_range(..2u8) == 0 {
            // Lakitu works at any height, but the inherited Y is usually a low
            // ground-enemy spot (harder). Coin-flip half of them up to the
            // common vanilla Lakitu height; the other half keep the low Y.
            data[first_idx + 2] = LAKITU_ALT_Y;
        }
    }
}
