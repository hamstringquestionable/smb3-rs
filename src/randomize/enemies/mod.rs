//! Enemy randomization: swap object IDs within CHR-compatible classes across the
//! structured level/sub-area enemy data, plus the wild-injection pass.
//!
//! Split across submodules: `tables` (class data), `sprite_bank` (CHR model),
//! `class_modes` (mode/pool resolution), `picking` (selection mechanics),
//! `segments` (HB wild segments), `injection` (wild injection). This module
//! holds the orchestration entry points and the main object-data walker.

use std::borrow::Cow;

use rand::Rng;
use rand::seq::IndexedRandom;

use crate::pipeline::{EnemyMode, HazardLimit, Options, WildChaser};
use crate::randomize::enemies::protections::{
    EntryProtection, WalkerSegmentRule, entry_protection_at, rewrites_hammer_bro,
    walker_segment_rule_at,
};
use crate::randomize::levels::segment_writer::{self, SegmentEntry as WriterEntry, SortMode};
use crate::randomize::rom_data::{
    ENEMY_DATA_END, ENEMY_DATA_START, HAMMER_BRO_ID, HB_NEEDS_SHELL_ENEMIES, LEVEL_DATA_REGIONS,
    STOMPABLE_ENEMIES, TREASURE_BOX_APPEAR, enemy_ptr_to_file_offset, level_index,
};
use crate::rom::Rom;

mod class_modes;
mod injection;
mod picking;
mod segments;
mod sprite_bank;
mod tables;

// Which entries the swap pass must leave alone, and why.
mod protections;

// --- Enemy and boss behaviour patches, applied outside the swap pass ---
pub(crate) mod koopalings;
pub(crate) mod stomp_fairness;
pub(crate) mod water_stomp;

use class_modes::*;
use injection::*;
use picking::*;
use segments::*;
use sprite_bank::*;
use tables::*;

// Public API consumed by the randomizer and the chr_stats integration test.
pub use class_modes::wild_pool_for;
pub use injection::enemy_entry_points;
pub use sprite_bank::{SpriteBank, sprite_bank};

#[cfg(test)]
mod tests;

/// Randomize enemies by parsing the structured object data and only swapping
/// object IDs that belong to a known enemy class. Position bytes and all
/// special objects (end-level cards, pipes, platforms, bosses, powerups,
/// autoscroll triggers, cannons, etc.) are never modified.
pub(crate) fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R, opts: &Options) {
    let starts = stream_starts(rom);
    randomize_object_data(rom, rng, false, opts, &starts);
}

/// Randomize Big ? Blocks by swapping their IDs among the set of Big ? Block
/// types. Every block rolls, including the eight in Unused Level 5.
///
/// Nothing is exempt. 7-F1 needs flight, but which room its pipe opens is drawn
/// per seed by `big_q_rooms`, which runs after this and forces *that* room's
/// block — so the guarantee does not depend on any offset being skipped here.
pub(crate) fn randomize_big_q_blocks<R: Rng>(rom: &mut Rom, rng: &mut R) {
    let starts = stream_starts(rom);
    randomize_object_data(rom, rng, true, &big_q_only_options(), &starts);
}

/// All enemy classes off, so the walker swaps Big ? Blocks and nothing else.
fn big_q_only_options() -> Options {
    Options {
        ground: EnemyMode::Off,
        shell: EnemyMode::Off,
        flying: EnemyMode::Off,
        piranhas: EnemyMode::Off,
        ghosts: EnemyMode::Off,
        thwomps: EnemyMode::Off,
        rotodiscs: EnemyMode::Off,
        cannons: EnemyMode::Off,
        water: EnemyMode::Off,
        bros: EnemyMode::Off,
        hb_encounters: EnemyMode::Off,
        wild_injections: Vec::new(),
        ..Options::default()
    }
}

/// Where each object stream the pass visits starts (its page byte), as an
/// index into the enemy-data buffer `data`, in file order.
///
/// The streams are the ones [`level_index::every_level`] reaches: every level
/// a flag can make the game load. A stream nothing loads is not visited.
///
/// TEMPORARY (step 2 of the index migration): the segments a block-wide walk
/// finds that no level loads are visited too, so this change can be proven
/// byte-identical. The next step drops them.
fn stream_starts(rom: &Rom) -> Vec<usize> {
    let data = rom.read_range(ENEMY_DATA_START, ENEMY_DATA_END - ENEMY_DATA_START);
    let mut starts = std::collections::BTreeSet::new();
    for area in level_index::every_level(rom).iter().flatten() {
        if level_index::read_objects(rom, area.objects_ptr, area.tileset).is_some() {
            starts.insert(enemy_ptr_to_file_offset(area.objects_ptr) - ENEMY_DATA_START);
        }
    }
    let spoiled: Vec<core::ops::Range<usize>> =
        crate::randomize::levels::autoscroll::SPOILED_SEGMENT_RANGES
            .iter()
            .map(|r| (r.start - ENEMY_DATA_START)..(r.end - ENEMY_DATA_START))
            .collect();
    for b in segment_writer::walk_segments(data, 0, data.len(), &spoiled) {
        starts.insert(b.file_offset);
    }
    starts.into_iter().collect()
}

/// The swap pass over the object streams starting at `starts` (indices into
/// the enemy-data block, as [`stream_starts`] returns them).
fn randomize_object_data<R: Rng>(
    rom: &mut Rom,
    rng: &mut R,
    big_q_only: bool,
    opts: &Options,
    starts: &[usize],
) {
    let len = ENEMY_DATA_END - ENEMY_DATA_START;
    let mut data = rom.read_range(ENEMY_DATA_START, len).to_vec();

    // Build class modes and wild pools
    let normal_modes = ClassModes::from_options(opts);
    let normal_wild_pool = normal_modes.build_wild_pool();
    let hb_modes = hb_class_modes(opts.hb_encounters);
    let hb_wild_pool = hb_modes.build_wild_pool();

    // Wild injection happens inside the segment loop below, not as a pass
    // around it — the walker is what knows a segment's real CHR pin set, and a
    // pass on either side of it can only re-derive that. All it needs up front
    // is where each level's first enemy lives (header-pointed enemy_ptr values,
    // via node_catalog), which no shuffle can move.
    let injection_sites = if !opts.wild_injections.is_empty() && !big_q_only {
        collect_injection_sites(rom, &data, opts)
    } else {
        InjectionSites::new()
    };

    for &seg_start in starts {
        // The page/flag byte; the entries follow it.
        let seg_file_offset = ENEMY_DATA_START + seg_start;
        let mut i = seg_start + 1;

        let segment_rule = walker_segment_rule_at(seg_file_offset);

        if segment_rule == WalkerSegmentRule::Skip {
            continue;
        }

        let is_hb_segment = segment_rule == WalkerSegmentRule::HammerBro;
        let (modes, wild_pool) = if is_hb_segment {
            (&hb_modes, hb_wild_pool.as_slice()) // HB Wild is batch-handled below
        } else {
            (&normal_modes, normal_wild_pool.as_slice())
        };

        // Collect all entries in this segment
        let mut entries: Vec<SegmentEntry> = Vec::new();
        while i + 2 < data.len() && data[i] != 0xFF {
            entries.push(SegmentEntry { data_index: i, obj_id: data[i], x_pos: data[i + 1] });
            i += 3;
        }

        // A treasure box in the room means `Level_Event = 7`, which arms the
        // Hammer Bro placeholder rewrite — and also that the room is *closed*:
        // the player cannot leave until it is empty, which is what makes a
        // floor-ignoring enemy a softlock rather than a nuisance (see
        // `place_hb_pick`). Decided once from the segment's own vanilla
        // contents, before any pick, so every path below agrees.
        let closed_room = entries.iter().any(|e| e.obj_id == TREASURE_BOX_APPEAR);
        let no_hammer_bro = rewrites_hammer_bro(seg_file_offset, closed_room);

        // HB Wild: batch-assign enemies with stompability constraints.
        if is_hb_segment && opts.hb_encounters == EnemyMode::Wild && !big_q_only {
            // `hazard_cap_full: false` on purpose — `limit_hazards` is a
            // levels-only filter. An HB room draws from its own curated pools
            // (STOMPABLE_ENEMIES / HB_NEEDS_SHELL_ENEMIES), already chosen so
            // the player can clear a closed room, and it has no vanilla enemy
            // to measure "introduced" against since the room is composed from
            // scratch.
            let limits = SegmentLimits { cap_full: false, no_hammer_bro, hazard_cap_full: false };
            randomize_hb_wild_segment(&mut data, &entries, &hb_modes, limits, closed_room, rng);
            continue;
        }

        // Wild injection. Runs here — after the entries are parsed, before
        // anything is committed or picked — so an injected chaser is just an
        // enemy the segment contains as far as everything below is concerned:
        // the Bertha tally counts it, the pin scan commits its page, and the
        // picks route around it. HB segments are excluded; their enemies are
        // the overworld mini-battle, not a level's.
        let injected = if injection_sites.is_empty() || is_hb_segment {
            Vec::new()
        } else {
            inject_segment_chasers(&mut data, rom, &mut entries, &injection_sites, modes, opts, rng)
        };

        // Track Boss Bass count for this segment so the per-segment cap is
        // enforced during class swaps. An injected Bertha is already reflected
        // here — `entries` was updated in step with `data` above.
        let mut bertha_count: u8 =
            entries.iter().filter(|e| BERTHA_IDS.contains(&e.obj_id)).count() as u8;

        // Hazards this pass has *added* to this segment, for HazardLimit::Sparse.
        // Starts at zero rather than counting what's already here: the rule is
        // additive-only, so a segment Nintendo filled with nippers keeps them and
        // still has its full budget for one more. Monotonic — a pick that
        // replaces an added hazard with a non-hazard doesn't refund it, which
        // keeps the budget a ceiling on introductions rather than on the
        // final count.
        let mut added_hazards: u8 = 0;

        // Split entries into proximity groups by X-position. Each group gets
        // independent CHR slot tracking — enemies more than CHR_GROUP_GAP tiles
        // apart can never be on-screen together, so they don't need compatible
        // CHR pages.
        let groups = chr_groups(&entries);

        // Level-wide chaser bookkeeping (see CHASER_IDS): Lakitu, the Angry
        // Sun, and the Big Berthas follow the player across proximity groups,
        // so the one-screen assumption behind chr_groups doesn't hold for them.
        //  - seg_all accumulates every commitment in the segment: pinned pages
        //    from all groups up front, then every pick as it's made. A chaser
        //    candidate must be compatible with all of it (checked in
        //    pick_replacement via ChrCtx::segment).
        //  - seg_chaser holds the pages of chasers present in the segment
        //    (pinned now, or picked as we go); it seeds every group's local
        //    slots so ordinary picks stay compatible with a chaser that will
        //    follow the player to them.
        let seg = if big_q_only {
            SegmentPins::none()
        } else {
            segment_pins(&entries, modes, &injected, None)
        };
        let (mut seg_all4, mut seg_all5) = seg.all;
        let (mut seg_chaser4, mut seg_chaser5) = seg.chaser;

        for group in &groups {
            // Two-pass approach per CHR group:
            // Pass 1: pre-commit CHR pages from pinned entries (non-swappable
            // objects, uniform-CHR classes, SkipSwap protections), seeded with
            // the pages of any level-wide chasers in the segment.
            let mut committed_slot4 = seg_chaser4;
            let mut committed_slot5 = seg_chaser5;

            if !big_q_only {
                for &idx in group {
                    let entry = &entries[idx];
                    if is_fixed(entry, modes, &injected) {
                        commit_chr_page(entry.obj_id, &mut committed_slot4, &mut committed_slot5);
                    }
                }
            }

            // Pass 2: pick a replacement for each swappable entry. The
            // per-entry decision (pool choice, primary pick, placement
            // constraints) lives in `pick_replacement`; this loop handles the
            // special-cased swaps and the segment-level bookkeeping.
            for &idx in group {
                let entry = &entries[idx];
                let file_offset = ENEMY_DATA_START + entry.data_index;
                let protection = entry_protection_at(file_offset);

                // Big ? blocks and Boom-Booms swap among their own kind and skip
                // the class machinery entirely.
                if big_q_only {
                    if BIG_Q_BLOCKS.contains(&entry.obj_id) {
                        data[entry.data_index] = *BIG_Q_BLOCKS.choose(rng).unwrap();
                    }
                    continue;
                }
                if BOOMBOOM_SWAP.contains(&data[entry.data_index]) {
                    data[entry.data_index] = *BOOMBOOM_SWAP.choose(rng).unwrap();
                    continue;
                }
                // SkipSwap keeps its enemy; its CHR page was already pinned
                // in Pass 1 (is_pinned covers SkipSwap protections).
                if protection == Some(EntryProtection::SkipSwap) {
                    continue;
                }
                // A wild injection settled this entry. Not covered by the
                // SkipSwap check above, and not covered by is_pinned either:
                // `should_precommit` is false for anything in a Wild pool, so
                // without this an injected Boss Bass would be swapped straight
                // back out whenever water is Wild.
                if injected.contains(&entry.data_index) {
                    continue;
                }

                let was_bertha = BERTHA_IDS.contains(&data[entry.data_index]);
                let limits = SegmentLimits {
                    cap_full: bertha_count.saturating_sub(was_bertha as u8)
                        >= MAX_BERTHA_PER_SEGMENT,
                    no_hammer_bro,
                    hazard_cap_full: match opts.limit_hazards {
                        HazardLimit::Off => false,
                        HazardLimit::Sparse => added_hazards >= MAX_ADDED_HAZARDS_PER_SEGMENT,
                        HazardLimit::All => true,
                    },
                };
                let chr = ChrCtx {
                    local: (committed_slot4, committed_slot5),
                    segment: (seg_all4, seg_all5),
                };
                let Some(chosen) =
                    pick_replacement(entry, protection, modes, wild_pool, chr, limits, rng)
                else {
                    // No swap (protection mode off, unknown class, or no
                    // compatible candidate) — the vanilla enemy stays, so its
                    // page is a real on-screen commitment like any pick.
                    // (Redundant for pass-1-pinned entries; it matters when a
                    // swappable entry found no compatible replacement.)
                    commit_chr_page(entry.obj_id, &mut committed_slot4, &mut committed_slot5);
                    commit_chr_page(entry.obj_id, &mut seg_all4, &mut seg_all5);
                    if CHASER_IDS.contains(&entry.obj_id) {
                        commit_chr_page(entry.obj_id, &mut seg_chaser4, &mut seg_chaser5);
                    }
                    continue;
                };

                let chosen_is_bertha = BERTHA_IDS.contains(&chosen);
                if was_bertha && !chosen_is_bertha {
                    bertha_count = bertha_count.saturating_sub(1);
                } else if !was_bertha && chosen_is_bertha {
                    bertha_count = bertha_count.saturating_add(1);
                }
                // Same predicate `keep` blocks on, so under All this never
                // fires and under Sparse it spends the segment's one budget.
                if HAZARDS.excludes(chosen, entry.obj_id) {
                    added_hazards = added_hazards.saturating_add(1);
                }
                swap_enemy(&mut data, entry.data_index, chosen);
                commit_chr_page(chosen, &mut committed_slot4, &mut committed_slot5);
                commit_chr_page(chosen, &mut seg_all4, &mut seg_all5);
                if CHASER_IDS.contains(&chosen) {
                    commit_chr_page(chosen, &mut seg_chaser4, &mut seg_chaser5);
                }
            }
        }
    }

    // Write each visited stream back through segment_writer with
    // SortMode::Preserve: only IDs (and a tall enemy's Y) changed, and vanilla
    // streams are not always in X order (6-6's sub-area spikes), so sorting
    // would move entries the pass never touched.
    for &seg_start in starts {
        let count = (0..)
            .map(|k| seg_start + 1 + k * 3)
            .take_while(|&e| e + 2 < data.len() && data[e] != 0xFF)
            .count();
        let entries: Vec<WriterEntry> = (0..count)
            .map(|k| {
                let off = seg_start + 1 + k * 3;
                WriterEntry { obj_id: data[off], x: data[off + 1], y: data[off + 2] }
            })
            .collect();
        let rom_offset = ENEMY_DATA_START + seg_start;
        segment_writer::write_segment(
            rom,
            &segment_writer::SegmentSpec {
                file_offset: rom_offset,
                original_count: count,
                entries: &entries,
                label: None,
                sort_mode: SortMode::Preserve,
            },
        )
        .expect("enemies: segment write failed");
    }
}
