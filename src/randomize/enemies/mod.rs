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

use crate::randomize::enemy_protections::{
    entry_protection_at, walker_segment_rule_at, EntryProtection, WalkerSegmentRule,
};
use crate::randomize::rom_data::{
    ENEMY_DATA_END, ENEMY_DATA_START, HB_NEEDS_SHELL_ENEMIES, LEVEL_DATA_REGIONS, STOMPABLE_ENEMIES,
    TANK_BRO_POOL,
};
use crate::randomize::segment_writer::{self, SegmentEntry as WriterEntry, SortMode};
use crate::randomizer::{EnemyMode, Options};
use crate::rom::Rom;

mod class_modes;
mod injection;
mod picking;
mod segments;
mod sprite_bank;
mod tables;

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
pub fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R, opts: &Options) {
    randomize_object_data(rom, rng, false, opts);
}

/// Randomize Big ? Blocks by swapping their IDs among the set of Big ? Block
/// types. The Tanooki block in World 7-F1 is protected because flying is
/// required to beat that level.
pub fn randomize_big_q_blocks<R: Rng>(rom: &mut Rom, rng: &mut R) {
    // All enemy classes off — only Big ? Blocks get randomized
    let no_flags = Options {
        ground: EnemyMode::Off, shell: EnemyMode::Off, flying: EnemyMode::Off,
        piranhas: EnemyMode::Off, ghosts: EnemyMode::Off,
        thwomps: EnemyMode::Off, rotodiscs: EnemyMode::Off,
        cannons: EnemyMode::Off, water: EnemyMode::Off, bros: EnemyMode::Off,
        hb_encounters: EnemyMode::Off, wild_injections: false,
        ..Options::default()
    };
    randomize_object_data(rom, rng, true, &no_flags);
}

fn randomize_object_data<R: Rng>(rom: &mut Rom, rng: &mut R, big_q_only: bool, opts: &Options) {
    let len = ENEMY_DATA_END - ENEMY_DATA_START;
    let mut data = rom.read_range(ENEMY_DATA_START, len).to_vec();

    // Spoiled segments left by upstream passes (e.g. disable_autoscroll
    // inserts $FF mid-segment to neutralize an autoscroll entry — the
    // level loader for that obj_ptr stops at the early $FF and is happy,
    // but a block-wide greedy walker mis-parses the orphaned bytes as a
    // "ghost" segment that swallows the next real segment's page byte +
    // first entry). Translated from ROM file offsets to local-buffer
    // indices so the walker can jump past them.
    let skip_ranges: Vec<core::ops::Range<usize>> = super::autoscroll::SPOILED_SEGMENT_RANGES
        .iter()
        .map(|r| (r.start - ENEMY_DATA_START)..(r.end - ENEMY_DATA_START))
        .collect();
    let in_skip_range = |idx: usize| -> Option<usize> {
        skip_ranges.iter().find(|r| r.contains(&idx)).map(|r| r.end)
    };

    // Build class modes and wild pools
    let normal_modes = ClassModes::from_options(opts);
    let normal_wild_pool = normal_modes.build_wild_pool();
    let hb_modes = hb_class_modes(opts.hb_encounters);
    let hb_wild_pool = hb_modes.build_wild_pool();

    // Wild injection runs in its own pass driven by *level entry points*
    // (header-pointed enemy_ptr values), not by walker-segments. This
    // guarantees every injection lands on a byte the SMB3 level loader
    // actually reads. See inject_at_entry_points doc for details. Segment
    // bounds are passed so the injection's CHR pin-scan can cover the whole
    // $FF-bounded segment, not just the ep's own run (runs nest, and outer
    // levels see the injected enemy too).
    if opts.wild_injections && !big_q_only {
        let bounds = segment_writer::walk_segments(&data, 0, data.len(), &skip_ranges);
        inject_wild_chasers(&mut data, rom, &bounds, opts, rng);
    }

    let mut i = 0;
    while i < data.len() {
        // Jump past spoiled byte ranges (see skip_ranges comment above).
        if let Some(end) = in_skip_range(i) {
            i = end;
            continue;
        }
        // 0xFF = segment boundary
        if data[i] == 0xFF {
            i += 1;
            continue;
        }

        // First non-FF byte after a terminator is the page/flag byte
        let seg_start = i;
        let seg_file_offset = ENEMY_DATA_START + seg_start;
        i += 1;

        let segment_rule = walker_segment_rule_at(seg_file_offset);

        // Skip entire segment if it's protected
        if segment_rule == WalkerSegmentRule::Skip {
            while i + 2 < data.len() && data[i] != 0xFF {
                i += 3;
            }
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
            entries.push(SegmentEntry {
                data_index: i,
                obj_id: data[i],
                x_pos: data[i + 1],
            });
            i += 3;
        }

        // HB Wild: batch-assign enemies with stompability constraints.
        if is_hb_segment && opts.hb_encounters == EnemyMode::Wild && !big_q_only {
            randomize_hb_wild_segment(&mut data, &entries, &hb_modes, rng);
            continue;
        }

        // Track Boss Bass count for this segment so the per-segment cap is
        // enforced during class swaps. If a wild injection (run earlier in
        // its own pass) added a Bertha to this segment, that's already
        // reflected here because we re-read obj_ids from `data`.
        let mut bertha_count: u8 = entries.iter()
            .filter(|e| BERTHA_IDS.contains(&e.obj_id))
            .count() as u8;

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
        let mut seg_all4 = ChrSlot::Free;
        let mut seg_all5 = ChrSlot::Free;
        let mut seg_chaser4 = ChrSlot::Free;
        let mut seg_chaser5 = ChrSlot::Free;
        if !big_q_only {
            for entry in &entries {
                let fo = ENEMY_DATA_START + entry.data_index;
                if is_pinned(entry.obj_id, fo, modes) {
                    commit_chr_page(entry.obj_id, &mut seg_all4, &mut seg_all5);
                    if CHASER_IDS.contains(&entry.obj_id) {
                        commit_chr_page(entry.obj_id, &mut seg_chaser4, &mut seg_chaser5);
                    }
                }
            }
        }

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
                    let fo = ENEMY_DATA_START + entry.data_index;
                    if is_pinned(entry.obj_id, fo, modes) {
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
                    if BIG_Q_BLOCKS.contains(&entry.obj_id) && file_offset != W7F1_TANOOKI_OFFSET {
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

                let was_bertha = BERTHA_IDS.contains(&data[entry.data_index]);
                let cap_full = bertha_count.saturating_sub(was_bertha as u8)
                    >= MAX_BERTHA_PER_SEGMENT;
                let chr = ChrCtx {
                    local: (committed_slot4, committed_slot5),
                    segment: (seg_all4, seg_all5),
                };
                let Some(chosen) = pick_replacement(
                    entry, protection, modes, wild_pool, chr, cap_full, rng,
                ) else {
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
                swap_enemy(&mut data, entry.data_index, chosen);
                commit_chr_page(chosen, &mut committed_slot4, &mut committed_slot5);
                commit_chr_page(chosen, &mut seg_all4, &mut seg_all5);
                if CHASER_IDS.contains(&chosen) {
                    commit_chr_page(chosen, &mut seg_chaser4, &mut seg_chaser5);
                }
            }

        }
    }

    // Route the final write through segment_writer per segment using
    // SortMode::Preserve. Sorting would be wrong here: walker segments
    // often span multiple logical levels (different enemy_ptrs pointing
    // at different positions in the same $FF-bounded run), each with its
    // own X sequence. A segment-wide X-sort can move entries across
    // logical-level boundaries the walker can't see, displacing wild
    // injections off their target ep and reordering vanilla bytes the
    // class-swap pass didn't touch. Preserve mode writes byte-for-byte
    // from the local `data` buffer, which already holds the desired
    // post-injection + post-class-swap state.
    //
    // Spoiled-segment skip ranges are honored so the walker doesn't
    // mis-parse autoscroll-clobbered bytes as ghost segments and
    // scramble adjacent real data.
    let bounds = segment_writer::walk_segments(&data, 0, data.len(), &skip_ranges);
    for b in bounds {
        let entries: Vec<WriterEntry> = (0..b.entry_count).map(|i| {
            let off = b.file_offset + 1 + i * 3;
            WriterEntry { obj_id: data[off], x: data[off + 1], y: data[off + 2] }
        }).collect();
        let rom_offset = ENEMY_DATA_START + b.file_offset;
        segment_writer::write_segment(rom, &segment_writer::SegmentSpec {
            file_offset: rom_offset,
            original_count: b.entry_count,
            entries: &entries,
            label: None,
            sort_mode: SortMode::Preserve,
        }).expect("enemies: segment write failed");
    }
}
