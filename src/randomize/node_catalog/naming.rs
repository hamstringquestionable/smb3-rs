//! Human-readable naming: assigns names like "1-1", "3F2", "7-P1" to every
//! catalog entry, adding ordinal suffixes when a world has multiples of a kind.

use super::{CatalogEntry, NodeKind};

// ---------------------------------------------------------------------------
// Naming
// ---------------------------------------------------------------------------

/// Special name overrides for entries that sit on non-standard tiles.
const LEVEL_NAME_OVERRIDES: &[(usize, usize, &str)] = &[
    (1, 32, "2-QS"),     // quicksand
    (1, 42, "2-Pyr"),    // pyramid
    (4, 10, "5-SC"),     // spiral castle
    (6, 11, "7-P1"),     // piranha plant 1
    (6, 45, "7-P2"),     // piranha plant 2
    (7, 5, "8-Tank"),    // tank level
    (7, 7, "8-Navy"),    // battleship
    (7, 10, "8-Air"),    // air force
    (7, 14, "8-Hnd1"),   // hand trap 1
    (7, 15, "8-Hnd2"),   // hand trap 2
    (7, 16, "8-Hnd3"),   // hand trap 3
    (7, 36, "8-STnk"),   // super tank
];

/// Assign human-readable names to all catalog entries.
///
/// Two-pass: first count per-world per-kind totals, then assign names
/// with ordinal suffixes when a world has multiples of the same kind.
pub(super) fn assign_names(entries: &mut [CatalogEntry]) {
    // Count per-world kind totals for ordinal suffixes
    let mut fortress_counts: [usize; 8] = [0; 8];
    let mut toad_counts: [usize; 8] = [0; 8];
    let mut bonus_counts: [usize; 8] = [0; 8];
    let mut hammer_counts: [usize; 8] = [0; 8];
    let mut map_obj_counts: [usize; 8] = [0; 8];
    let mut pipe_counts: [usize; 8] = [0; 8];

    for e in entries.iter() {
        if e.world_idx == usize::MAX { continue; } // skip synthetic (beta) entries
        match e.kind {
            NodeKind::Fortress { .. } => fortress_counts[e.world_idx] += 1,
            NodeKind::ToadHouse => toad_counts[e.world_idx] += 1,
            NodeKind::BonusGame => bonus_counts[e.world_idx] += 1,
            NodeKind::HammerBro => hammer_counts[e.world_idx] += 1,
            NodeKind::MapObject => map_obj_counts[e.world_idx] += 1,
            NodeKind::Pipe { .. } => pipe_counts[e.world_idx] += 1,
            _ => {}
        }
    }

    // Track ordinals per world per kind
    let mut fortress_ord: [usize; 8] = [0; 8];
    let mut toad_ord: [usize; 8] = [0; 8];
    let mut bonus_ord: [usize; 8] = [0; 8];
    let mut hammer_ord: [usize; 8] = [0; 8];
    let mut map_obj_ord: [usize; 8] = [0; 8];
    let mut pipe_ord: [usize; 8] = [0; 8];

    for e in entries.iter_mut() {
        // Skip synthetic entries (betas) — they already have names.
        if e.world_idx == usize::MAX {
            continue;
        }
        let w = e.world_idx;
        let w1 = w + 1; // 1-indexed for display

        // Check override first
        if let Some(name) = LEVEL_NAME_OVERRIDES
            .iter()
            .find(|&&(wi, ei, _)| wi == w && ei == e.entry_idx)
            .map(|&(_, _, name)| name)
        {
            e.name = name.to_string();
            continue;
        }

        e.name = match &e.kind {
            NodeKind::Start => format!("{w1}S"),
            NodeKind::Bowser => "8B".to_string(),
            NodeKind::Airship => format!("{w1}A"),
            NodeKind::Fortress { .. } => {
                fortress_ord[w] += 1;
                if fortress_counts[w] == 1 {
                    format!("{w1}F")
                } else {
                    format!("{w1}F{}", fortress_ord[w])
                }
            }
            NodeKind::Pipe { .. } => {
                pipe_ord[w] += 1;
                format!("{w1}Pi{}", pipe_ord[w])
            }
            NodeKind::ToadHouse => {
                toad_ord[w] += 1;
                if toad_counts[w] == 1 {
                    format!("{w1}TH")
                } else {
                    format!("{w1}TH{}", toad_ord[w])
                }
            }
            NodeKind::BonusGame => {
                bonus_ord[w] += 1;
                if bonus_counts[w] == 1 {
                    format!("{w1}BG")
                } else {
                    format!("{w1}BG{}", bonus_ord[w])
                }
            }
            NodeKind::HammerBro => {
                hammer_ord[w] += 1;
                if hammer_counts[w] == 1 {
                    format!("{w1}HB")
                } else {
                    format!("{w1}HB{}", hammer_ord[w])
                }
            }
            NodeKind::MapObject => {
                map_obj_ord[w] += 1;
                if map_obj_counts[w] == 1 {
                    format!("{w1}MO")
                } else {
                    format!("{w1}MO{}", map_obj_ord[w])
                }
            }
            NodeKind::Level => {
                // Numbered levels: tile 0x03-0x0F → level number = tile - 2
                if e.tile >= 0x03 && e.tile <= 0x0F {
                    format!("{w1}-{}", e.tile - 2)
                } else {
                    // Fallback for levels on non-numbered tiles
                    format!("{w1}L[{}]", e.entry_idx)
                }
            }
        };
    }
}
