//! Level-geometry fixes for the two spots where an unwanted Frog Suit strands
//! the player.
//!
//! [`super::fire_flower`] can hand out a Frog Suit from any in-level Fire
//! Flower, so a player who grabbed what looked like a power-up can arrive at an
//! obstacle wearing the one suit that cannot clear it. The Frog Suit's ground
//! movement is the problem: it cannot build a run, so a clearance the vanilla
//! level tuned for Big Mario stops being passable at all.
//!
//! Two vanilla spots do this, and both are fixed the same way: **decrement the
//! row nibble of one generator command** so the obstacle sits one tile higher.
//! The edits come from the community "SMB3 - Frog Patch" hack
//! (`patches/SMB3 - Frog Patch.ips`), which forces the Frog Suit game-wide and
//! carries seventeen records; only these two are about being stranded, so only
//! these two are taken.
//!
//! | level | what moves |
//! |---|---|
//! | 8F, screen 13 | the downward spike run before Boom-Boom, up one row |
//! | 7-5, screen 2 | a brick run, up one row — onto the identical run already there |
//!
//! # Why this is a byte splice and not a generator rewrite
//!
//! A level's object stream is a run of 3- and 4-byte generator commands whose
//! lengths depend on the tileset's variable-size dispatch table, so there is no
//! cheap way to address "the spike run on screen 13" symbolically. The offsets
//! are stable in vanilla, and [`EDITS`] carries the bytes each site must hold
//! before it is written — a site that does not match is skipped rather than
//! stamped over, so a hand-modified ROM (`--skip-rom-validation`) loses the fix
//! instead of having its geometry corrupted.
//!
//! # Ordering
//!
//! None required. The only other module that writes into these level-data
//! regions is [`super::powerups`], which round-trips each whole region through
//! `Rom::read_range` — the *working* buffer, not `rom.original` — so an edit
//! made either side of it survives. [`super::antechambers`] does not touch
//! object streams at all: it rewrites the entry area's header and its junction
//! spawn bytes, and 7-5's fix is in the interior (layout `$A5CD`), so it
//! travels with that interior wherever the shuffle routes it.

use crate::rom::Rom;

/// One geometry fix: where, what must be there, and what replaces it.
struct Edit {
    /// File offset of the first byte written.
    offset: usize,
    /// What vanilla holds here. The write is skipped unless it matches.
    vanilla: &'static [u8],
    /// The replacement, same length.
    patched: &'static [u8],
    /// Write-log tag, so `--write-log` attributes each byte to its own fix
    /// rather than to one lump. Doubles as the label in test failures.
    what: &'static str,
}

/// The two sites, both verified against the vanilla ROM.
///
/// **8F** (`0x2BAAA`) spans two adjacent commands, which is why its middle byte
/// is present but unchanged:
///
/// ```text
/// 0x2BAA8:  10 D1 E7 0E   scr=13 col=1 row=0   LoadLevel_SolidBrick (TS2 disp 13)
///                 ^^ size nibble 7 -> 6            the ceiling, one shorter
/// 0x2BAAC:  18 D1 DE      scr=13 col=1 row=8   LoadLevel_SpikeDown  (TS2 disp 12)
///           ^^ row 8 -> 7                          the spikes, one row higher
/// ```
///
/// Boom-Boom stands on screen 14; that spike run is the last obstacle before
/// the boss room, and the ceiling has to rise with it or the gap does not open.
///
/// **7-5** (`0x1E648`) is a single byte in the level's *interior* (layout
/// `$A5CD`, the sub-area the front-door pipe drops into):
///
/// ```text
/// 0x1E645:  37 25 10      scr=2 col=5 row=7    BRICK (TS1 disp 15)
/// 0x1E648:  38 25 10      scr=2 col=5 row=8    BRICK
///           ^^ row 8 -> 7
/// ```
///
/// Row 7 already holds an identical run at the same column, so raising the
/// row-8 one merges the two: the net effect is that the lower brick is gone.
const EDITS: [Edit; 2] = [
    Edit {
        offset: 0x2BAAA,
        vanilla: &[0xE7, 0x0E, 0x18],
        patched: &[0xE6, 0x0E, 0x17],
        what: "8f_spikes",
    },
    Edit { offset: 0x1E648, vanilla: &[0x38], patched: &[0x37], what: "7-5_brick" },
];

/// Apply both fixes. Gated by the caller on Random Fire Flower being enabled,
/// which is what turns "a Fire Flower" into "a suit the player did not pick".
///
/// A site whose bytes are not vanilla is left alone, so this is a no-op on a
/// ROM that already carries someone else's edit there.
pub fn apply(rom: &mut Rom) {
    for edit in &EDITS {
        if rom.read_range(edit.offset, edit.vanilla.len()) == edit.vanilla {
            rom.push_tag(edit.what);
            rom.write_range(edit.offset, edit.patched);
            rom.pop_tag();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_vanilla() -> Option<Rom> {
        let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&bytes).ok()
    }

    /// Every edit is the same length in and out, or `write_range` would spill
    /// into the next generator command.
    #[test]
    fn edits_are_length_preserving() {
        for edit in &EDITS {
            assert_eq!(
                edit.vanilla.len(),
                edit.patched.len(),
                "{}: replacement changes the command length",
                edit.what
            );
            assert_ne!(edit.vanilla, edit.patched, "{}: nothing to write", edit.what);
        }
    }

    /// **The guard that matters.** These offsets are raw positions inside level
    /// object streams; if level data ever moves, the bytes stop matching and
    /// [`apply`] silently does nothing. This turns that into a failure — on the
    /// machine the patch is written on, since it needs the ROM.
    #[test]
    fn every_site_still_holds_its_vanilla_bytes() {
        let Some(rom) = load_vanilla() else { return };
        for edit in &EDITS {
            assert_eq!(
                rom.read_range(edit.offset, edit.vanilla.len()),
                edit.vanilla,
                "0x{:05X} ({}) no longer holds the bytes this fix was measured against",
                edit.offset,
                edit.what
            );
        }
    }

    #[test]
    fn apply_writes_both_sites() {
        let Some(rom) = load_vanilla() else { return };
        let mut patched = rom.clone();
        apply(&mut patched);
        for edit in &EDITS {
            assert_eq!(
                patched.read_range(edit.offset, edit.patched.len()),
                edit.patched,
                "0x{:05X} ({}) was not written",
                edit.offset,
                edit.what
            );
        }
    }

    /// The row nibble is the whole point: each obstacle ends up exactly one
    /// tile higher, and nothing else in `byte0` moves.
    #[test]
    fn the_row_nibble_drops_by_one() {
        // 8F's spike command is the third byte of its edit; 7-5's is the first.
        for (edit, i) in [(&EDITS[0], 2), (&EDITS[1], 0)] {
            let (before, after) = (edit.vanilla[i], edit.patched[i]);
            assert_eq!(after & 0xF0, before & 0xF0, "{}: high nibble moved", edit.what);
            assert_eq!(after & 0x0F, (before & 0x0F) - 1, "{}: not up one row", edit.what);
        }
    }

    /// Applying twice is applying once — the second pass sees non-vanilla bytes
    /// and declines, rather than decrementing the row a second time.
    #[test]
    fn apply_is_idempotent() {
        let Some(rom) = load_vanilla() else { return };
        let mut once = rom.clone();
        apply(&mut once);
        let mut twice = once.clone();
        apply(&mut twice);
        assert_eq!(once.data, twice.data, "a second apply moved more bytes");
    }

    /// A site someone else already edited is left alone rather than stamped.
    #[test]
    fn a_modified_site_is_skipped() {
        let Some(rom) = load_vanilla() else { return };
        let mut patched = rom.clone();
        patched.write_byte(EDITS[1].offset, 0x5A);
        apply(&mut patched);
        assert_eq!(patched.read_byte(EDITS[1].offset), 0x5A, "an edited site was overwritten");
        // The other site is independent and still lands.
        assert_eq!(patched.read_range(EDITS[0].offset, 3), EDITS[0].patched);
    }
}
