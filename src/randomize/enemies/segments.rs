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
    seg_file_offset: usize,
    rng: &mut R,
) {
    // The coin-ship reward room is enclosed and never scrolls, so Dry Bones
    // (0x3F) — which revives after every stomp and has no edge to wander off —
    // can never be cleared there. Drop it from the stompable pool for that one
    // segment; everywhere else it's a fine HB-wild pick.
    let stompable: Cow<[u8]> = if is_coinship_fight(seg_file_offset) {
        Cow::Owned(STOMPABLE_ENEMIES.iter().copied().filter(|&id| id != 0x3F).collect())
    } else {
        Cow::Borrowed(STOMPABLE_ENEMIES)
    };

    let swappable: Vec<usize> = entries.iter()
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
                    let (di0, di1) = (entries[swappable[0]].data_index, entries[swappable[1]].data_index);
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
