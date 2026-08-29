//! Hammer-Bro "wild" segment randomization with stompability constraints.

use super::*;

/// A parsed 3-byte entry from the enemy data block.
pub(super) struct SegmentEntry {
    /// Index into the segment data buffer (points to the obj_id byte)
    pub(super) data_index: usize,
    /// The object ID
    pub(super) obj_id: u8,
    /// X tile position (byte 2 of the 3-byte entry)
    pub(super) x_pos: u8,
}

/// CHR pages a segment has already spoken for before any pick is made.
/// `all` is every fixed entry; `chaser` is the subset that follows the player
/// across proximity groups (see `CHASER_IDS`).
#[derive(Clone, Copy)]
pub(super) struct SegmentPins {
    pub(super) all: (ChrSlot, ChrSlot),
    pub(super) chaser: (ChrSlot, ChrSlot),
}

/// Placement limits that hold for every entry in one segment, whatever pool
/// that entry draws from. Both are properties of the *room*, not the slot,
/// which is why they ride alongside the per-entry protections rather than
/// being expressed as `EntryRule`s.
#[derive(Clone, Copy)]
pub(super) struct SegmentLimits {
    /// The Big Bertha cap is already reached — no new bertha here.
    pub(super) cap_full: bool,
    /// No `$81` here: this room rewrites one into an unkillable object. See
    /// `enemy_protections::rewrites_hammer_bro`.
    pub(super) no_hammer_bro: bool,
}

impl SegmentPins {
    /// Nothing committed — the Big-?-block pass, which skips the CHR model.
    pub(super) fn none() -> Self {
        SegmentPins { all: (ChrSlot::Free, ChrSlot::Free), chaser: (ChrSlot::Free, ChrSlot::Free) }
    }
}

/// Whether an entry's obj_id is settled before the picks run: pinned by class
/// or protection, or a chaser wild injection just wrote here.
///
/// The injected case is not redundant with `is_pinned`: `should_precommit`
/// returns false for anything in a Wild pool, so an injected Boss Bass with
/// water Wild would otherwise be swapped straight back out — the exact
/// reshuffle that kept Bertha out of the injection pool to begin with.
pub(super) fn is_fixed(entry: &SegmentEntry, modes: &ClassModes, injected: &[usize]) -> bool {
    injected.contains(&entry.data_index)
        || is_pinned(entry.obj_id, ENEMY_DATA_START + entry.data_index, modes)
}

/// Accumulate the segment's fixed CHR pages. `skip` drops one entry from the
/// tally — the wild-injection decision uses it to ignore the enemy it is about
/// to replace, whose page is on its way out.
///
/// Single definition on purpose: the injection decision and the walker's own
/// seeding must agree about what "already pinned" means, and a second copy of
/// this loop is what let the old pre-walk injection pass drift.
pub(super) fn segment_pins(
    entries: &[SegmentEntry],
    modes: &ClassModes,
    injected: &[usize],
    skip: Option<usize>,
) -> SegmentPins {
    let mut pins =
        SegmentPins { all: (ChrSlot::Free, ChrSlot::Free), chaser: (ChrSlot::Free, ChrSlot::Free) };
    for entry in entries {
        if Some(entry.data_index) == skip || !is_fixed(entry, modes, injected) {
            continue;
        }
        commit_chr_page(entry.obj_id, &mut pins.all.0, &mut pins.all.1);
        if CHASER_IDS.contains(&entry.obj_id) {
            commit_chr_page(entry.obj_id, &mut pins.chaser.0, &mut pins.chaser.1);
        }
    }
    pins
}

/// Split entries into proximity groups based on X-position gaps.
/// Entries within `CHR_GROUP_GAP` tiles of their neighbors stay in the same group.
/// Returns groups of entry indices (sorted by X within each group).
pub(super) fn chr_groups(entries: &[SegmentEntry]) -> Vec<Vec<usize>> {
    if entries.is_empty() {
        return Vec::new();
    }
    let mut sorted: Vec<usize> = (0..entries.len()).collect();
    sorted.sort_by_key(|&i| entries[i].x_pos);

    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = vec![sorted[0]];
    for &idx in &sorted[1..] {
        let last = *current.last().unwrap();
        if entries[idx].x_pos.saturating_sub(entries[last].x_pos) > CHR_GROUP_GAP {
            groups.push(std::mem::take(&mut current));
        }
        current.push(idx);
    }
    groups.push(current);
    groups
}

/// HB Wild segment randomization with stompability constraints.
/// 1-enemy segments: pick from STOMPABLE_ENEMIES only.
/// 2-enemy segments: `HB_NONSTOMPABLE_ODDS` chance for the non-stompable path
/// (one from HB_NEEDS_SHELL_ENEMIES + one from SHELL_ENEMIES), otherwise both
/// stompable.
pub(super) fn randomize_hb_wild_segment<R: Rng>(
    data: &mut [u8],
    entries: &[SegmentEntry],
    hb_modes: &ClassModes,
    limits: SegmentLimits,
    rng: &mut R,
) {
    // A Hammer Bro is a placeholder the engine rewrites into something
    // unkillable in a treasure-box room that isn't a real bro battle, which
    // would strand the player (see `rewrites_hammer_bro`). Clearability is
    // otherwise a property of the pools themselves, not of the room.
    let stompable: Cow<[u8]> = if limits.no_hammer_bro {
        Cow::Owned(STOMPABLE_ENEMIES.iter().copied().filter(|&id| id != HAMMER_BRO_ID).collect())
    } else {
        Cow::Borrowed(STOMPABLE_ENEMIES)
    };

    let swappable: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| find_class_pool(e.obj_id, hb_modes).is_some())
        .map(|(idx, _)| idx)
        .collect();

    // Pre-commit CHR from non-swappable entries
    let mut slot4 = ChrSlot::Free;
    let mut slot5 = ChrSlot::Free;
    for (idx, entry) in entries.iter().enumerate() {
        if !swappable.contains(&idx) {
            commit_chr_page(entry.obj_id, &mut slot4, &mut slot5);
        }
    }

    if swappable.len() == 1 {
        if let Some(chosen) = pick_compatible(&stompable, slot4, slot5, rng) {
            swap_enemy(data, entries[swappable[0]].data_index, chosen);
        }
    } else if swappable.len() == 2 {
        // Roll whether this segment gets a non-stompable enemy
        let (num, den) = HB_NONSTOMPABLE_ODDS;
        if rng.random_range(..den) < num {
            // Pick non-stompable, then a shell partner
            if let Some(ns) = pick_compatible(HB_NEEDS_SHELL_ENEMIES, slot4, slot5, rng) {
                let mut s4 = slot4;
                let mut s5 = slot5;
                commit_chr_page(ns, &mut s4, &mut s5);
                if let Some(shell) = pick_compatible(SHELL_ENEMIES, s4, s5, rng) {
                    // Randomly assign which slot gets which
                    let (di0, di1) =
                        (entries[swappable[0]].data_index, entries[swappable[1]].data_index);
                    if rng.random_range(..2u32) == 0 {
                        swap_enemy(data, di0, ns);
                        swap_enemy(data, di1, shell);
                    } else {
                        swap_enemy(data, di0, shell);
                        swap_enemy(data, di1, ns);
                    }
                }
            }
        } else {
            // Both from stompable pool
            if let Some(first) = pick_compatible(&stompable, slot4, slot5, rng) {
                swap_enemy(data, entries[swappable[0]].data_index, first);
                let mut s4 = slot4;
                let mut s5 = slot5;
                commit_chr_page(first, &mut s4, &mut s5);
                if let Some(second) = pick_compatible(&stompable, s4, s5, rng) {
                    swap_enemy(data, entries[swappable[1]].data_index, second);
                }
            }
        }
    }
}
