//! Enemy randomization: swap object IDs within CHR-compatible classes across the
//! structured level/sub-area enemy data, plus the wild-injection pass.
//!
//! Split across submodules: `tables` (class data), `sprite_bank` (CHR model),
//! `class_modes` (mode/pool resolution), `picking` (selection mechanics),
//! `segments` (the parsed segment, CHR pins, proximity groups, HB Wild
//! picks), `injection` (wild injection). This module
//! holds the orchestration entry points and the main object-data walker.

use std::borrow::Cow;

use rand::Rng;
use rand::seq::IndexedRandom;

use crate::pipeline::{EnemyMode, HazardLimit, Options, WildChaser};
use crate::randomize::enemies::protections::{
    EntryProtection, WalkerSegmentRule, entry_protection_at, rewrites_hammer_bro,
    walker_segment_rule_at,
};
use crate::randomize::rom_data::{
    ENEMY_DATA_END, ENEMY_DATA_START, HAMMER_BRO_ID, HB_NEEDS_SHELL_ENEMIES, STOMPABLE_ENEMIES,
    TREASURE_BOX_APPEAR, enemy_ptr_to_file_offset, level_index,
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

#[cfg(test)]
mod chr_stats;
#[cfg(test)]
mod tests;

/// Randomize enemies by parsing the structured object data and only swapping
/// object IDs that belong to a known enemy class. Position bytes and all
/// special objects (end-level cards, pipes, platforms, bosses, powerups,
/// autoscroll triggers, cannons, etc.) are never modified.
pub(crate) fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R, opts: &Options) {
    let starts = stream_starts(rom);
    randomize_object_data(rom, rng, opts, &starts);
}

/// Randomize Big ? Blocks by swapping their IDs among the set of Big ? Block
/// types. Every block rolls, including the eight in Unused Level 5.
///
/// Nothing is exempt. 7-F1 needs flight, but which room its pipe opens is drawn
/// per seed by `big_q_rooms`, which runs after this and forces *that* room's
/// block — so the guarantee does not depend on any offset being skipped here.
pub(crate) fn randomize_big_q_blocks<R: Rng>(rom: &mut Rom, rng: &mut R) {
    let mut rooms = std::collections::BTreeMap::new();
    for area in level_index::every_level(rom).iter().flatten() {
        if let Some(objects) = level_index::read_objects(rom, area.objects_ptr, area.tileset) {
            rooms.insert(area.objects_ptr, objects);
        }
    }
    reroll_big_q_blocks(rom, rng, rooms.values().flatten().map(|o| o.offset));
}

/// Reroll every Big ? Block among the ID-byte offsets `ids`, in order.
fn reroll_big_q_blocks<R: Rng>(rom: &mut Rom, rng: &mut R, ids: impl IntoIterator<Item = usize>) {
    for offset in ids {
        if BIG_Q_BLOCKS.contains(&rom.read_byte(offset)) {
            rom.write_byte(offset, *BIG_Q_BLOCKS.choose(rng).unwrap());
        }
    }
}

/// Where each object stream the pass visits starts (its page byte), as an
/// index into the enemy-data buffer `data`, in file order.
///
/// The streams are the ones [`level_index::every_level`] reaches: every level
/// a flag can make the game load. A stream nothing loads (unreferenced data,
/// or an airship stream that autoscroll removal repointed away from) is not
/// visited — `every_loaded_object_stream_was_visible_to_the_enemy_pass` checks
/// that no flag makes one live later.
fn stream_starts(rom: &Rom) -> Vec<usize> {
    let mut starts = std::collections::BTreeSet::new();
    for area in level_index::every_level(rom).iter().flatten() {
        if level_index::read_objects(rom, area.objects_ptr, area.tileset).is_some() {
            starts.insert(enemy_ptr_to_file_offset(area.objects_ptr) - ENEMY_DATA_START);
        }
    }
    starts.into_iter().collect()
}

/// The swap pass over the object streams starting at `starts` (indices into
/// the enemy-data block, as [`stream_starts`] returns them).
fn randomize_object_data<R: Rng>(rom: &mut Rom, rng: &mut R, opts: &Options, starts: &[usize]) {
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
    let injection_sites = if !opts.wild_injections.is_empty() {
        collect_injection_sites(rom, opts)
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
        if is_hb_segment && opts.hb_encounters == EnemyMode::Wild {
            // `hazard_cap_full: false` on purpose — `limit_hazards` is a
            // levels-only filter. An HB room draws from its own curated pools
            // (STOMPABLE_ENEMIES / HB_NEEDS_SHELL_ENEMIES), already chosen so
            // the player can clear a closed room, and it has no vanilla enemy
            // to measure "introduced" against since the room is composed from
            // scratch.
            let limits =
                SegmentLimits { bertha_cap_full: false, no_hammer_bro, hazard_cap_full: false };
            randomize_hb_wild_segment(&mut data, &entries, &hb_modes, limits, closed_room, rng);
            continue;
        }

        // Wild injection. Runs here — after the entries are parsed, before
        // anything is committed or picked — so an injected chaser is just an
        // enemy the segment contains as far as everything below is concerned:
        // the Bertha tally counts it, the pin scan commits its page, and the
        // picks route around it. HB segments are excluded; their enemies are
        // the overworld mini-battle, not a level's.
        let injected: Vec<usize> = if injection_sites.contains(&seg_start) && !is_hb_segment {
            inject_first_enemy(&mut data, &mut entries, modes, opts, rng).into_iter().collect()
        } else {
            Vec::new()
        };

        // Split entries into proximity groups by X-position. Each group gets
        // independent CHR slot tracking — enemies more than CHR_GROUP_GAP tiles
        // apart are never live together, so they don't need compatible pages.
        let groups = chr_groups(&entries);

        // The Bertha tally counts an injected Bertha too: `entries` was updated
        // in step with `data` above. The chaser pages seed every group, since
        // Lakitu, the Angry Sun and the Big Berthas follow the player across
        // groups (see CHASER_IDS).
        let (all, chaser) = segment_pins(&entries, modes, &injected, None);
        let mut state = SegmentState {
            berthas: entries.iter().filter(|e| BERTHA_IDS.contains(&e.obj_id)).count() as u8,
            added_hazards: 0,
            all,
            chaser,
        };

        for group in &groups {
            // A group is picked, and if keeping a vanilla enemy clashed with
            // an earlier pick (no candidate fit, so the vanilla page went in
            // on top), it is picked again with that enemy fixed from the
            // start, so the other picks route around its page. Up to
            // MAX_CHR_REROLLS retries; after that the last attempt stands.
            // Each retry starts from the state the group started with.
            let saved_bytes: Vec<(u8, u8)> = group
                .iter()
                .map(|&i| (data[entries[i].data_index], data[entries[i].data_index + 2]))
                .collect();
            let saved_state = state;
            let mut kept: Vec<usize> = Vec::new();
            for attempt in 0..=MAX_CHR_REROLLS {
                if attempt > 0 {
                    for (&i, &(id, y)) in group.iter().zip(&saved_bytes) {
                        data[entries[i].data_index] = id;
                        data[entries[i].data_index + 2] = y;
                    }
                    state = saved_state;
                }

                // Pass 1: pre-commit CHR pages from pinned entries (non-swappable
                // objects, uniform-CHR classes, SkipSwap protections, enemies a
                // previous attempt had to keep), seeded with the pages of any
                // level-wide chasers in the segment.
                let mut local = state.chaser;
                for &idx in group {
                    let entry = &entries[idx];
                    if kept.contains(&idx) {
                        state.commit(entry.obj_id, &mut local);
                    } else if is_fixed(entry, modes, &injected) {
                        local.commit(entry.obj_id);
                    }
                }

                // Enemies this attempt kept although their page clashed.
                let mut clashed: Vec<usize> = Vec::new();

                // Pass 2: pick a replacement for each swappable entry. The
                // per-entry decision (pool choice, primary pick, placement
                // constraints) lives in `pick_replacement`; this loop handles the
                // special-cased swaps and the segment-level bookkeeping.
                for &idx in group {
                    let entry = &entries[idx];
                    let file_offset = ENEMY_DATA_START + entry.data_index;
                    let protection = entry_protection_at(file_offset);

                    // SkipSwap keeps its enemy; its CHR page was already pinned
                    // in Pass 1 (`is_fixed`, via `is_pinned`, covers SkipSwap). So was
                    // an enemy a previous attempt had to keep. This comes first so
                    // that a protection holds for every enemy, Boom-Booms included.
                    if protection == Some(EntryProtection::SkipSwap) || kept.contains(&idx) {
                        continue;
                    }
                    // Boom-Booms swap among their own kind and skip the class
                    // machinery entirely.
                    if BOOMBOOM_SWAP.contains(&data[entry.data_index]) {
                        data[entry.data_index] = *BOOMBOOM_SWAP.choose(rng).unwrap();
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
                        bertha_cap_full: state.berthas.saturating_sub(was_bertha as u8)
                            >= MAX_BERTHA_PER_SEGMENT,
                        no_hammer_bro,
                        hazard_cap_full: match opts.limit_hazards {
                            HazardLimit::Off => false,
                            HazardLimit::Sparse => {
                                state.added_hazards >= MAX_ADDED_HAZARDS_PER_SEGMENT
                            }
                            HazardLimit::All => true,
                        },
                    };
                    let chr = ChrCtx { local, segment: state.all };
                    let Some(chosen) =
                        pick_replacement(entry, protection, modes, wild_pool, chr, limits, rng)
                    else {
                        // No swap (protection mode off, unknown class, or no
                        // compatible candidate) — the vanilla enemy stays, so its
                        // page is a real on-screen commitment like any pick. If
                        // it is the one that breaks a slot, the group goes round
                        // again with it fixed first. A pass-1-pinned entry never
                        // trips this: its own page is already in the slot.
                        let id = entry.obj_id;
                        if local.would_conflict(id)
                            || (CHASER_IDS.contains(&id) && state.all.would_conflict(id))
                        {
                            clashed.push(idx);
                        }
                        state.commit(id, &mut local);
                        continue;
                    };

                    let chosen_is_bertha = BERTHA_IDS.contains(&chosen);
                    if was_bertha && !chosen_is_bertha {
                        state.berthas = state.berthas.saturating_sub(1);
                    } else if !was_bertha && chosen_is_bertha {
                        state.berthas = state.berthas.saturating_add(1);
                    }
                    // Same predicate `keep` blocks on, so under All this never
                    // fires and under Sparse it spends the segment's one budget.
                    if HAZARDS.excludes(chosen, entry.obj_id) {
                        state.added_hazards = state.added_hazards.saturating_add(1);
                    }
                    swap_enemy(&mut data, entry.data_index, chosen);
                    state.commit(chosen, &mut local);
                }

                if clashed.is_empty() {
                    break;
                }
                kept.extend(clashed);
            }
        }
    }

    // Write the buffer back. Only IDs (and a tall enemy's Y) changed, all
    // inside the visited streams, and `write_byte` records only the bytes that
    // differ, so this touches exactly what the pass changed. Entry order is
    // never changed: vanilla streams are not always in X order (6-6's sub-area
    // spikes), and sorting would move entries the pass never touched.
    for (k, &b) in data.iter().enumerate() {
        rom.write_byte(ENEMY_DATA_START + k, b);
    }
}
