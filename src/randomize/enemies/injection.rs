//! Wild-injection pass: seed Lakitu / Angry Sun / Boss Bass into a fraction
//! of level segments, keyed off authoritative level entry points.

use super::*;

/// Collect every `enemy_ptr` value (bytes 2-3 of every 9-byte level
/// header) from every region in [`LEVEL_DATA_REGIONS`]. This is the
/// authoritative set of file offsets where the SMB3 level loader actually
/// begins reading enemy data — the level/sub-area entry points. Used by
/// [`inject_at_entry_points`] so wild_injection writes only land where a
/// level will actually read them.
///
/// Returned values are unique and in first-seen order.
///
/// Exposed `pub` so integration tests (`tests/chr_stats.rs`) can use the
/// same authoritative set for distribution / visibility analysis.
pub fn enemy_entry_points(rom: &Rom) -> Vec<u16> {
    const LEVEL_HEADER_SIZE: usize = 9;
    let mut pts: Vec<u16> = Vec::new();
    let mut seen: std::collections::HashSet<u16> = std::collections::HashSet::new();
    for region in LEVEL_DATA_REGIONS {
        let len = region.end - region.start;
        let data = rom.read_range(region.start, len);
        let mut i = 0usize;
        while i + LEVEL_HEADER_SIZE < data.len() {
            // Header at data[i..i+9]; enemy_ptr is bytes 2-3 (little-endian).
            let ep = (data[i + 2] as u16) | ((data[i + 3] as u16) << 8);
            if seen.insert(ep) {
                pts.push(ep);
            }
            i += LEVEL_HEADER_SIZE;
            // Walk commands until the level's $FF terminator.
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

/// Wild-injection pass driven by *level entry points* rather than by
/// `$FF`-bounded walker segments. For every `enemy_ptr` reported by
/// [`enemy_entry_points`] that isn't an HB or protected segment, roll
/// against `WILD_INJECTION_CHANCE` and — on success — replace the first
/// entry the SMB3 level loader will actually read with a CHR-compatible
/// Lakitu / Angry Sun / Boss Bass.
///
/// This replaces the in-walker injection block: the walker historically
/// injected at `entries[0]` of every walker-segment, but most walker
/// segments don't start at a level entry point (the level enters
/// mid-segment). Driving the pass off `enemy_ptr` ensures injections are
/// visible in-game.
pub(super) fn inject_at_entry_points<R: Rng>(
    data: &mut [u8],
    entry_ptrs: &[u16],
    bounds: &[segment_writer::SegmentBounds],
    opts: &Options,
    rng: &mut R,
) {
    let normal_modes = ClassModes::from_options(opts);

    for &ep_u16 in entry_ptrs {
        let ep = ep_u16 as usize;
        if !(ENEMY_DATA_START..ENEMY_DATA_END).contains(&ep) {
            continue;
        }
        if is_injection_blocked(ep_u16) {
            continue;
        }

        let ep_local = ep - ENEMY_DATA_START;
        if ep_local >= data.len() {
            continue;
        }

        // SMB3 enemy data has two layout flavors at an entry point:
        //   (a) page byte (0x00 or 0x01) then 3-byte entries
        //   (b) entries-only (the entry_ptr lands directly on the first obj_id)
        // Real obj_ids never overlap 0x00/0x01, so the byte value is the
        // unambiguous discriminator the walker has always used.
        let first_entry_idx = if matches!(data[ep_local], 0x00 | 0x01) {
            ep_local + 1
        } else {
            ep_local
        };
        if first_entry_idx >= data.len() || data[first_entry_idx] == 0xFF {
            continue; // empty level — no enemy entries to inject into
        }

        // Gather entries from this entry point up to its $FF terminator.
        let mut entries: Vec<SegmentEntry> = Vec::new();
        let mut j = first_entry_idx;
        while j + 2 < data.len() && data[j] != 0xFF {
            entries.push(SegmentEntry {
                data_index: j,
                obj_id: data[j],
                x_pos: data[j + 1],
            });
            j += 3;
        }
        if entries.is_empty() {
            continue;
        }

        let roll: u8 = rng.random_range(..=255);
        if roll >= WILD_INJECTION_CHANCE {
            continue;
        }

        let entry = &entries[0];
        let fo = ENEMY_DATA_START + entry.data_index;
        // Skip if the walker pass would override or filter our pick: every
        // protection either keeps the vanilla enemy (SkipSwap), silently
        // replaces the injected enemy with a forced-pool member (e.g. 6-5's
        // shell at 0xC5EB must stay shell for brick-break progression), or
        // filters a Lakitu/Boss Bass back out (ExcludeHazards, e.g. 7F2's
        // Boom-Boom arena).
        let swappable = entry_protection_at(fo).is_none()
            && find_class_pool(entry.obj_id, &normal_modes).is_some();
        if !swappable {
            continue;
        }

        // Pre-commit pinned CHR pages from the WHOLE $FF-bounded segment
        // containing this ep — not just the ep's own run. Level enemy runs
        // nest inside segments: an outer level's run starts earlier and
        // reads straight through this ep, so the injected enemy (which
        // chases the player level-wide — every WILD_INJECTION_ID is a
        // chaser) is also on screen with entries *before* the ep when that
        // level is played.
        let Some(seg) = bounds.iter().find(|b| {
            let entries_start = b.file_offset + 1;
            let entries_end = entries_start + b.entry_count * 3;
            (entries_start..entries_end).contains(&first_entry_idx)
        }) else {
            continue; // ep not inside any walkable segment — don't inject
        };
        let mut s4 = ChrSlot::Free;
        let mut s5 = ChrSlot::Free;
        for k in 0..seg.entry_count {
            let off = seg.file_offset + 1 + k * 3;
            if off == entries[0].data_index {
                continue; // the slot being replaced — its enemy goes away
            }
            let fo = ENEMY_DATA_START + off;
            if is_pinned(data[off], fo, &normal_modes) {
                commit_chr_page(data[off], &mut s4, &mut s5);
            }
        }

        if let Some(chosen) = pick_compatible(WILD_INJECTION_IDS, s4, s5, rng) {
            let bertha_count: u8 = entries.iter()
                .filter(|e| BERTHA_IDS.contains(&e.obj_id))
                .count() as u8;
            let was_bertha = BERTHA_IDS.contains(&entries[0].obj_id);
            let chosen_is_bertha = BERTHA_IDS.contains(&chosen);
            let post_count = bertha_count
                .saturating_sub(was_bertha as u8)
                .saturating_add(chosen_is_bertha as u8);
            if !(chosen_is_bertha && post_count > MAX_BERTHA_PER_SEGMENT) {
                let di = entry.data_index;
                swap_enemy(data, di, chosen);
                // The Angry Sun idles in the background until its screen
                // counter hits the attack threshold; the Early Sun QoL patch
                // moves that threshold to screen 0, so a sun that spawns on any
                // later screen never fires (and a stuck sun blocks the level's
                // goal card). Injection inherits the replaced enemy's position,
                // which is usually deep in the level. Re-seed the sun at the
                // vanilla 2-Quicksand spawn (screen 0, Y=0x11) so it engages
                // immediately with Early Sun on and matches the one working
                // vanilla placement. The sun is already `entries[0]` (lowest X
                // in the sorted run), so moving it to screen 0 keeps the run
                // X-sorted for the writeback.
                if chosen == ANGRY_SUN_ID {
                    data[di + 1] = SUN_SPAWN_X; // X: screen 0
                    data[di + 2] = SUN_SPAWN_Y; // Y: sky row
                }
            }
        }
    }
}
