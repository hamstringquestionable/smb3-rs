//! One parsed segment and the walker's state for it: entries, CHR pins,
//! placement limits and proximity groups; plus the Hammer Bro Wild batch pick.

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

/// The walker's running state for one segment. A CHR group that is picked
/// again restores it whole, so a new field here is restored too.
#[derive(Clone, Copy)]
pub(super) struct SegmentState {
    /// Big Berthas in the segment, for `MAX_BERTHA_PER_SEGMENT`.
    pub(super) berthas: u8,
    /// Hazards this pass has added, for `HazardLimit::Sparse`. Starts at zero
    /// rather than counting what's already here: the rule is additive-only, so
    /// a segment Nintendo filled with nippers keeps them and still has its
    /// budget for one more. Monotonic — replacing an added hazard with a
    /// non-hazard doesn't refund it.
    pub(super) added_hazards: u8,
    /// Every CHR commitment anywhere in the segment: fixed entries up front,
    /// then every pick as it's made. A chaser must fit all of it.
    pub(super) all: Pages,
    /// The pages of the level-wide chasers in the segment (see `CHASER_IDS`).
    /// They seed every group's local slots, since a chaser follows the player
    /// into each group.
    pub(super) chaser: Pages,
}

impl SegmentState {
    /// Commit `id`'s page to the group's `local` slots and to the segment.
    pub(super) fn commit(&mut self, id: u8, local: &mut Pages) {
        local.commit(id);
        self.all.commit(id);
        if CHASER_IDS.contains(&id) {
            self.chaser.commit(id);
        }
    }
}

/// Placement limits that hold for every entry in one segment, whatever pool
/// that entry draws from. The first two are properties of the *room*, not the
/// slot, which is why they ride alongside the per-entry protections rather
/// than being expressed as `EntryRule`s. `hazard_cap_full` is the one
/// member that changes as the segment's picks are made: it rides here because `pick_replacement`'s `keep` is the
/// single place every pick funnels through, so one field there covers the
/// class pools, the wild pool and all four protection paths at once.
#[derive(Clone, Copy)]
pub(super) struct SegmentLimits {
    /// The Big Bertha cap is already reached — no new bertha here.
    pub(super) bertha_cap_full: bool,
    /// No `$81` here: this room rewrites one into an unkillable object. See
    /// `crate::randomize::enemies::protections::rewrites_hammer_bro`.
    pub(super) no_hammer_bro: bool,
    /// This segment's introduced-hazard budget is spent, so no pick may put a
    /// hazard where the vanilla enemy wasn't already one of the same category.
    /// Always true under `HazardLimit::All`, always false under
    /// `HazardLimit::Off`, and under `Sparse` it flips once the segment has
    /// gained [`MAX_ADDED_HAZARDS_PER_SEGMENT`] of them.
    pub(super) hazard_cap_full: bool,
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

/// The segment's fixed CHR pages, as `(all, chaser)` — see [`SegmentState`].
/// `skip` drops one entry from the
/// tally — the wild-injection decision uses it to ignore the enemy it is about
/// to replace, whose page is on its way out.
///
/// Single definition on purpose: the injection decision and the walker's own
/// seeding must agree about what "already pinned" means.
pub(super) fn segment_pins(
    entries: &[SegmentEntry],
    modes: &ClassModes,
    injected: &[usize],
    skip: Option<usize>,
) -> (Pages, Pages) {
    let (mut all, mut chaser) = (Pages::FREE, Pages::FREE);
    for entry in entries {
        if Some(entry.data_index) == skip || !is_fixed(entry, modes, injected) {
            continue;
        }
        all.commit(entry.obj_id);
        if CHASER_IDS.contains(&entry.obj_id) {
            chaser.commit(entry.obj_id);
        }
    }
    (all, chaser)
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

/// Write one HB Wild pick, raising the single enemy in the pool whose AI
/// ignores the floor. See [`HB_FLYER_RAISE_ROWS`] for why 7 rows.
///
/// `closed_room` gates it: the raise exists to keep a treasure-box room
/// clearable, and applying it to W8's open 7-7 layout would only leave its
/// enemies floating.
fn place_hb_pick(data: &mut [u8], id_index: usize, id: u8, closed_room: bool) {
    swap_enemy(data, id_index, id);
    if closed_room && id == FLYING_RED_PARATROOPA {
        // The y byte is the absolute tile row as (page << 4) | row, so a plain
        // subtraction moves it. Saturating is belt-and-braces: every bro room
        // spawns its enemies on row 20 or below.
        data[id_index + 2] = data[id_index + 2].saturating_sub(HB_FLYER_RAISE_ROWS);
    }
}

/// HB Wild segment randomization with stompability constraints.
/// 1-enemy segments: pick from STOMPABLE_ENEMIES only.
/// 2-enemy segments: `HB_NONSTOMPABLE_ODDS` chance for the non-stompable path
/// (one from HB_NEEDS_SHELL_ENEMIES + one from SHELL_ENEMIES), otherwise both
/// stompable. Segments with more swappable entries are left as they are.
pub(super) fn randomize_hb_wild_segment<R: Rng>(
    data: &mut [u8],
    entries: &[SegmentEntry],
    hb_modes: &ClassModes,
    limits: SegmentLimits,
    closed_room: bool,
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
            place_hb_pick(data, entries[swappable[0]].data_index, chosen, closed_room);
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
                        place_hb_pick(data, di0, ns, closed_room);
                        place_hb_pick(data, di1, shell, closed_room);
                    } else {
                        place_hb_pick(data, di0, shell, closed_room);
                        place_hb_pick(data, di1, ns, closed_room);
                    }
                }
            }
        } else {
            // Both from stompable pool
            if let Some(first) = pick_compatible(&stompable, slot4, slot5, rng) {
                place_hb_pick(data, entries[swappable[0]].data_index, first, closed_room);
                let mut s4 = slot4;
                let mut s5 = slot5;
                commit_chr_page(first, &mut s4, &mut s5);
                if let Some(second) = pick_compatible(&stompable, s4, s5, rng) {
                    place_hb_pick(data, entries[swappable[1]].data_index, second, closed_room);
                }
            }
        }
    }
}
