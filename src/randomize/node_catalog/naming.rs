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

/// Number of kinds that get per-world ordinal suffixes (see `ordinal_slot`).
const ORDINAL_SLOTS: usize = 6;

/// Slot index + name abbreviation for kinds that get ordinal suffixes.
fn ordinal_slot(kind: &NodeKind) -> Option<(usize, &'static str)> {
    match kind {
        NodeKind::Fortress { .. } => Some((0, "F")),
        NodeKind::Pipe { .. } => Some((1, "Pi")),
        NodeKind::ToadHouse => Some((2, "TH")),
        NodeKind::BonusGame => Some((3, "BG")),
        NodeKind::HammerBro => Some((4, "HB")),
        NodeKind::MapObject => Some((5, "MO")),
        _ => None,
    }
}

/// "prefix" when the world has exactly one of the kind, "prefix<ord>" otherwise.
fn suffixed(prefix: &str, count: usize, ord: usize) -> String {
    if count == 1 {
        prefix.to_string()
    } else {
        format!("{prefix}{ord}")
    }
}

/// Assign human-readable names to all catalog entries.
///
/// Two-pass: first count per-world per-kind totals, then assign names
/// with ordinal suffixes when a world has multiples of the same kind.
pub(super) fn assign_names(entries: &mut [CatalogEntry]) {
    // Count per-world kind totals for ordinal suffixes
    let mut counts = [[0usize; 8]; ORDINAL_SLOTS];
    for e in entries.iter() {
        if e.world_idx == usize::MAX { continue; } // skip synthetic (beta) entries
        if let Some((slot, _)) = ordinal_slot(&e.kind) {
            counts[slot][e.world_idx] += 1;
        }
    }

    // Track ordinals per world per kind
    let mut ords = [[0usize; 8]; ORDINAL_SLOTS];

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

        if let Some((slot, abbr)) = ordinal_slot(&e.kind) {
            ords[slot][w] += 1;
            e.name = if matches!(e.kind, NodeKind::Pipe { .. }) {
                // Pipes are always numbered, even when a world has just one.
                format!("{w1}{abbr}{}", ords[slot][w])
            } else {
                suffixed(&format!("{w1}{abbr}"), counts[slot][w], ords[slot][w])
            };
            continue;
        }

        e.name = match &e.kind {
            NodeKind::Start => format!("{w1}S"),
            NodeKind::Bowser => "8B".to_string(),
            NodeKind::Airship => format!("{w1}A"),
            NodeKind::Level => {
                // Numbered levels: tile 0x03-0x0F → level number = tile - 2
                if e.tile >= 0x03 && e.tile <= 0x0F {
                    format!("{w1}-{}", e.tile - 2)
                } else {
                    // Fallback for levels on non-numbered tiles
                    format!("{w1}L[{}]", e.entry_idx)
                }
            }
            // Suffixed kinds are handled by `ordinal_slot` above.
            _ => unreachable!("ordinal kinds handled above"),
        };
    }
}
