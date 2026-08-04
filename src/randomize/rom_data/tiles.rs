//! Overworld tile classification — the single producer for "which bytes mean
//! what" on the world map.
//!
//! # Why this exists
//!
//! Before this module, "which bytes are locks" was written out independently in
//! four places (`overworld_pickup`'s FX-slot assertion, `qol::hammer_breaks`'s
//! breakable table, `overworld_helpers::gap_tile_for`, and `testrom`'s unlock
//! table). Each answered a slightly different question inline, so none was
//! obviously the authority and any of them could drift.
//!
//! The rule this module enforces: **no fact about the ROM has two producers.**
//! Predicates are named after the *question being asked*, not lumped into one
//! list — the sites genuinely differ (the hammer takes locks but only takes
//! water gaps when asked to), and a single flat list would paper over
//! distinctions that matter.
//!
//! # What does *not* belong here
//!
//! Mirrors of the engine's own lookup tables — `capacity::is_completion_unsafe`
//! reproduces `Map_Removable_Tiles` from the ROM, which must track the *game's*
//! bytes rather than our semantics. Those two lists legitimately differ and
//! merging them would couple concepts that should move independently. The right
//! treatment for a table mirror is a test asserting it still matches the bytes
//! at its ROM offset, not consolidation into these predicates.

use super::FORTRESS_TILES;

// ---------------------------------------------------------------------------
// Gap tiles (locks and water gaps)
// ---------------------------------------------------------------------------

/// Lock tiles. Three visual variants — vertical, horizontal, sky — that all
/// block a path identically; only the graphic differs.
pub(crate) const LOCK_TILES: [u8; 3] = [
    0x54, // vertical lock
    0x56, // horizontal lock
    0xE4, // sky lock
];

/// The water-gap tile. Distinct from a lock: it replaces a bridge rather than a
/// path, is restored to `0xB3` rather than a path tile, and the hammer only
/// breaks it when explicitly asked (see `qol::hammer_breaks`).
pub(crate) const WATER_GAP_TILE: u8 = 0x9D;

/// The bridge tile a water gap is cut from and restored to.
pub(crate) const BRIDGE_TILE: u8 = 0xB3;

/// A lock tile — opened by clearing its fortress, or by the hammer when
/// `hammer_breaks_locks` is on.
pub(crate) fn is_lock(tile: u8) -> bool {
    LOCK_TILES.contains(&tile)
}

/// The water-gap tile.
pub(crate) fn is_water_gap(tile: u8) -> bool {
    tile == WATER_GAP_TILE
}

/// A "gap tile" in the codebase's existing sense (`LockAssignment::gap_tile`,
/// the output domain of [`gap_tile_for`]): a tile written over a path to block
/// it, removed again by a fortress-clear FX. Locks and the water gap both
/// qualify.
///
/// Use this where a site treats all four identically — `overworld_pickup`'s
/// `test_no_fx_gaps_remain` is the current consumer. Use [`is_lock`] where the
/// water gap must be excluded, as `qol::hammer_breaks` does.
// Reason: only a test consumes this today, so the lib build sees it as dead.
// Kept because it names the codebase's existing umbrella concept (the output
// domain of `gap_tile_for`, stored as `LockAssignment::gap_tile`) and is the
// correct predicate for any future site treating locks and gaps alike —
// re-inlining that set is exactly the drift this module exists to stop.
#[allow(dead_code)]
pub(crate) fn is_gap_tile(tile: u8) -> bool {
    is_lock(tile) || is_water_gap(tile)
}

// ---------------------------------------------------------------------------
// Path <-> gap mapping
// ---------------------------------------------------------------------------

/// Vertical path tiles (plain + variants + drawbridge). Both the gap-tile and
/// FX-pattern lookups key on this, so a new vertical variant added here gets
/// the vertical lock and its FX pattern together.
///
/// `0xB1` (vertical drawbridge) is unreachable in practice — the drawbridge
/// QoL patch removes drawbridges from the game, so no lock is ever placed on
/// one. Kept for completeness of the classification; do not expect a change to
/// it to move any output. (Verified: dropping it left all 20 baseline seeds
/// byte-identical.)
pub(crate) fn is_vertical_path(tile: u8) -> bool {
    matches!(tile, 0x46 | 0xAA | 0xAB | 0xB0 | 0xB1 | 0xDB | 0xBA)
}

/// The gap tile that blocks a given path tile.
///
/// **Many-to-one**: every vertical path variant maps onto the single vertical
/// lock `0x54`, so this cannot be inverted without loss — see
/// [`path_for_gap_tile`]. Keep the two adjacent so that relationship stays
/// visible.
pub(crate) fn gap_tile_for(tile: u8) -> u8 {
    match tile {
        BRIDGE_TILE => WATER_GAP_TILE,         // bridge → water gap
        0xDA => 0xE4,                          // sky path → sky lock
        t if is_vertical_path(t) => 0x54,      // vertical path → vertical lock
        _ => 0x56,                             // horizontal path → horizontal lock
    }
}

/// The path tile to restore under a gap tile.
///
/// **Lossy inverse of [`gap_tile_for`].** Because that mapping is many-to-one,
/// this can only return the plain path tile of the right orientation, never the
/// specific variant (drawbridge, path variant) that was originally there. That
/// is fine for a test ROM walking over the tile, and wrong for anything that
/// must restore the map faithfully — such callers must keep the original tile
/// (as `LockAssignment::replace_tile` does) rather than reconstruct it here.
///
/// Returns `None` for a tile that is not a gap tile.
pub(crate) fn path_for_gap_tile(tile: u8) -> Option<u8> {
    match tile {
        0x54 => Some(0x46),           // vertical lock   → vertical path
        0x56 => Some(0x45),           // horizontal lock → horizontal path
        0xE4 => Some(0xDA),           // sky lock        → sky path
        WATER_GAP_TILE => Some(BRIDGE_TILE), // water gap → bridge
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Node tiles
// ---------------------------------------------------------------------------

/// Lowest numbered-level tile. Level number = `tile - 2`.
pub(crate) const NUMBERED_TILE_LO: u8 = 0x03;
/// Highest numbered-level tile.
pub(crate) const NUMBERED_TILE_HI: u8 = 0x0F;

/// A numbered action level (`0x03..=0x0F`, level number = `tile - 2`).
pub(crate) fn is_numbered_level(tile: u8) -> bool {
    (NUMBERED_TILE_LO..=NUMBERED_TILE_HI).contains(&tile)
}

/// A fortress tile (any of the three variants).
pub(crate) fn is_fortress(tile: u8) -> bool {
    FORTRESS_TILES.contains(&tile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_and_gap_membership() {
        for t in LOCK_TILES {
            assert!(is_lock(t), "{t:#04X} should be a lock");
            assert!(is_gap_tile(t));
            assert!(!is_water_gap(t));
        }
        assert!(is_water_gap(WATER_GAP_TILE));
        assert!(is_gap_tile(WATER_GAP_TILE));
        assert!(!is_lock(WATER_GAP_TILE), "the water gap is not a lock");

        // Path tiles and the bridge are not gap tiles.
        for t in [0x45, 0x46, 0xDA, BRIDGE_TILE, 0x00] {
            assert!(!is_gap_tile(t), "{t:#04X} should not be a gap tile");
        }
    }

    /// Every gap tile `gap_tile_for` can produce must be invertible, and the
    /// round trip must land back on a tile that maps to the same gap.
    #[test]
    fn gap_mapping_round_trips_by_orientation() {
        let paths = [0x45, 0x46, 0xDA, BRIDGE_TILE, 0xAA, 0xAB, 0xB0, 0xB1, 0xDB, 0xBA];
        for p in paths {
            let gap = gap_tile_for(p);
            assert!(is_gap_tile(gap), "{p:#04X} produced non-gap {gap:#04X}");

            let back = path_for_gap_tile(gap).expect("gap tile must invert");
            assert_eq!(
                gap_tile_for(back),
                gap,
                "{p:#04X} -> {gap:#04X} -> {back:#04X} left the orientation class"
            );
        }
    }

    /// The inverse is documented as lossy; pin that so nobody "fixes" it into a
    /// silent identity assumption.
    #[test]
    fn inverse_is_lossy_for_path_variants() {
        assert_eq!(gap_tile_for(0xB1), 0x54, "drawbridge is a vertical path");
        assert_eq!(path_for_gap_tile(0x54), Some(0x46), "inverse yields plain path");
        assert_ne!(path_for_gap_tile(gap_tile_for(0xB1)), Some(0xB1));
    }

    #[test]
    fn non_gap_tiles_do_not_invert() {
        for t in [0x45, 0x46, BRIDGE_TILE, 0x67, 0x00, 0xFF] {
            assert_eq!(path_for_gap_tile(t), None, "{t:#04X} is not a gap tile");
        }
    }

    #[test]
    fn node_tile_classification() {
        assert!(is_numbered_level(0x03) && is_numbered_level(0x0F));
        assert!(!is_numbered_level(0x02) && !is_numbered_level(0x10));
        for t in FORTRESS_TILES {
            assert!(is_fortress(t));
        }
        assert!(!is_fortress(0x45));
    }

    /// A lock is never also a node tile — the classifications must not overlap,
    /// or a site substituting one predicate for another silently changes sets.
    #[test]
    fn classifications_are_disjoint() {
        for t in 0u8..=0xFF {
            let n = [is_lock(t), is_water_gap(t), is_numbered_level(t), is_fortress(t)]
                .iter()
                .filter(|b| **b)
                .count();
            assert!(n <= 1, "{t:#04X} matches {n} classifications");
        }
    }
}
