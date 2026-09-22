//! Level-geometry fixes for the two spots where an unwanted Frog Suit strands
//! the player.
//!
//! [`super::fire_flower`] can hand out a Frog Suit from any in-level Fire
//! Flower, so a player who grabbed what looked like a power-up can arrive at an
//! obstacle wearing the one suit that cannot clear it. The Frog Suit cannot
//! crouch, so a gap the vanilla level expects Big Mario to duck under stops
//! being passable at all.
//!
//! | level | fix | source |
//! |---|---|---|
//! | 8F, screen 13 | only the *first* spike of the run before Boom-Boom goes up a row | MaCobra52 |
//! | 7-5, screen 2 | a brick run up one row, onto the identical run already there | "SMB3 - Frog Patch" |
//!
//! The 7-5 edit comes from `patches/SMB3 - Frog Patch.ips`, which forces the
//! Frog Suit game-wide and carries seventeen records; only the two about being
//! stranded were ever taken. Its 8F record raised the *whole* spike run, which
//! is more than the frog needs, so 8F uses MaCobra52's four-site edit instead —
//! see [`FIXES`].
//!
//! # Why this is a byte splice and not a generator rewrite
//!
//! A level's object stream is a run of 3- and 4-byte generator commands whose
//! lengths depend on the tileset's variable-size dispatch table, so there is no
//! cheap way to address "the spike run on screen 13" symbolically. The offsets
//! are stable in vanilla, and every [`Edit`] carries the bytes its site must
//! hold before it is written.
//!
//! **A fix lands whole or not at all.** 8F's four sites only make sense
//! together — the ceiling is shortened on the promise that another command puts
//! row 7 back — so if any one of them does not match (a hand-modified ROM under
//! `--skip-rom-validation`), none of them are written and the level stays
//! vanilla rather than half-edited.
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

/// One byte splice: where, what must be there, and what replaces it.
struct Edit {
    /// File offset of the first byte written.
    offset: usize,
    /// What vanilla holds here.
    vanilla: &'static [u8],
    /// The replacement, same length.
    patched: &'static [u8],
}

/// A set of edits that is applied together or not at all.
struct Fix {
    /// Write-log tag, so `--write-log` attributes each byte to its own fix
    /// rather than to one lump. Doubles as the label in test failures.
    what: &'static str,
    edits: &'static [Edit],
}

/// Both fixes, every site verified against the vanilla ROM.
///
/// # 8F — only the first spike raised
///
/// Screen 13 is a brick ceiling over a 15-wide run of downward spikes, leaving a
/// 1-tile tunnel over a right-moving conveyor. Big Mario ducks into it; the
/// frog cannot. Raising just the leftmost spike gives the frog a 2-tile notch
/// to get in, and leaves the rest of the tunnel as vanilla built it.
///
/// That needs three commands where vanilla has two (spikes in two different
/// rows), and 8F's stream is packed wall to wall, so two unrelated commands are
/// repurposed to pay for it. Every replacement is the same length as what it
/// replaces, so the stream stays aligned:
///
/// ```text
/// 0x2B93A  32 3A 10    -> 17 D1 D0     BRICK scr 3 (10,2)  -> SpikeDown scr 13 col 1 row 7, 1 wide
/// 0x2BA85  70 CD 3A 22 -> 17 D2 E0 0D  black backdrop      -> SolidBrick scr 13 row 7, cols 2-15
/// 0x2BAAA  E7          -> E6           ceiling rows 0-7    -> rows 0-6
/// 0x2BAAC  18 D1 DE    -> 18 D2 DD     spikes cols 1-15    -> cols 2-15, still row 8
///
///          col 0 1 2 3
///   row 6      . B B B
///   row 7      . v B B     <- first spike raised; row-7 brick restored beside it
///   row 8      . . v v     <- the rest of the run at vanilla height
///   row 9      . . . .
///   row 10     = = = =
/// ```
///
/// The new spike sits early in the stream (screen 3's slot) and survives
/// because nothing painted after it covers screen 13 row 7 col 1: the ceiling
/// now stops at row 6, the restored row-7 brick starts at col 2, and the one
/// command that did cover it — the black backdrop — is the one repurposed.
///
/// **What it costs.** The single breakable brick at screen 3 (row 2, col 10),
/// beside the 1-Up brick, is gone. And the backdrop over screen 12 col 13
/// through screen 14 — the tunnel approach and the Boom-Boom room — reverts
/// from black to the default fortress fill, whose top row is the dark diamond
/// tile: `$E5` is above TS2's `$E2` solidity threshold, so that row is a solid
/// ceiling where vanilla had open black. It matches the rest of the fortress.
///
/// # 7-5 — a brick up one row
///
/// A single byte in the level's *interior* (layout `$A5CD`, the sub-area the
/// front-door pipe drops into):
///
/// ```text
/// 0x1E645:  37 25 10      scr=2 col=5 row=7    BRICK (TS1 disp 15)
/// 0x1E648:  38 25 10      scr=2 col=5 row=8    BRICK
///           ^^ row 8 -> 7
/// ```
///
/// Row 7 already holds an identical run at the same column, so raising the
/// row-8 one merges the two: the net effect is that the lower brick is gone.
static FIXES: [Fix; 2] = [
    Fix {
        what: "8f_first_spike",
        edits: &[
            Edit { offset: 0x2B93A, vanilla: &[0x32, 0x3A, 0x10], patched: &[0x17, 0xD1, 0xD0] },
            Edit {
                offset: 0x2BA85,
                vanilla: &[0x70, 0xCD, 0x3A, 0x22],
                patched: &[0x17, 0xD2, 0xE0, 0x0D],
            },
            Edit { offset: 0x2BAAA, vanilla: &[0xE7], patched: &[0xE6] },
            Edit { offset: 0x2BAAC, vanilla: &[0x18, 0xD1, 0xDE], patched: &[0x18, 0xD2, 0xDD] },
        ],
    },
    Fix {
        what: "7-5_brick",
        edits: &[Edit { offset: 0x1E648, vanilla: &[0x38], patched: &[0x37] }],
    },
];

/// Apply both fixes. Gated by the caller on Random Fire Flower being enabled,
/// which is what turns "a Fire Flower" into "a suit the player did not pick".
///
/// A fix with any site that is not vanilla is left alone entirely, so this is
/// a no-op on a ROM that already carries someone else's edit there.
pub fn apply(rom: &mut Rom) {
    for fix in &FIXES {
        let intact =
            fix.edits.iter().all(|e| rom.read_range(e.offset, e.vanilla.len()) == e.vanilla);
        if !intact {
            continue;
        }
        rom.push_tag(fix.what);
        for edit in fix.edits {
            rom.write_range(edit.offset, edit.patched);
        }
        rom.pop_tag();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_vanilla() -> Option<Rom> {
        let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&bytes).ok()
    }

    fn all_edits() -> impl Iterator<Item = (&'static str, &'static Edit)> {
        FIXES.iter().flat_map(|f| f.edits.iter().map(move |e| (f.what, e)))
    }

    /// Every edit is the same length in and out, or `write_range` would spill
    /// into the next generator command.
    #[test]
    fn edits_are_length_preserving() {
        for (what, edit) in all_edits() {
            assert_eq!(
                edit.vanilla.len(),
                edit.patched.len(),
                "{what} 0x{:05X}: replacement changes the command length",
                edit.offset
            );
            assert_ne!(
                edit.vanilla, edit.patched,
                "{what} 0x{:05X}: nothing to write",
                edit.offset
            );
        }
    }

    /// No two sites overlap, inside a fix or across fixes.
    #[test]
    fn sites_do_not_overlap() {
        let mut spans: Vec<(usize, usize)> =
            all_edits().map(|(_, e)| (e.offset, e.offset + e.vanilla.len())).collect();
        spans.sort();
        for w in spans.windows(2) {
            assert!(w[0].1 <= w[1].0, "0x{:05X} overlaps 0x{:05X}", w[0].0, w[1].0);
        }
    }

    /// **The guard that matters.** These offsets are raw positions inside level
    /// object streams; if level data ever moves, the bytes stop matching and
    /// [`apply`] silently does nothing. This turns that into a failure — on the
    /// machine the patch is written on, since it needs the ROM.
    #[test]
    fn every_site_still_holds_its_vanilla_bytes() {
        let Some(rom) = load_vanilla() else { return };
        for (what, edit) in all_edits() {
            assert_eq!(
                rom.read_range(edit.offset, edit.vanilla.len()),
                edit.vanilla,
                "0x{:05X} ({what}) no longer holds the bytes this fix was measured against",
                edit.offset,
            );
        }
    }

    #[test]
    fn apply_writes_every_site() {
        let Some(rom) = load_vanilla() else { return };
        let mut patched = rom.clone();
        apply(&mut patched);
        for (what, edit) in all_edits() {
            assert_eq!(
                patched.read_range(edit.offset, edit.patched.len()),
                edit.patched,
                "0x{:05X} ({what}) was not written",
                edit.offset,
            );
        }
    }

    /// Decode 8F's patched commands and check they say what the doc says: one
    /// spike at row 7 col 1, a ceiling that stops at row 6, row 7 bricked back
    /// in from col 2, and the spike run at row 8 from col 2 — all on screen 13.
    #[test]
    fn the_8f_commands_raise_only_the_first_spike() {
        let e = FIXES[0].edits;
        // byte0 low nibble = row; byte1 = screen << 4 | col; byte2 low nibble =
        // width-1 for a spike run, height-1 for SolidBrick (whose extra byte is
        // width-1). byte0 bits 7-5 = 0 selects dispatch group 0 throughout.
        let pos = |b: &[u8]| (b[0] & 0x0F, b[1] >> 4, b[1] & 0x0F);

        let spike = e[0].patched;
        assert_eq!(pos(spike), (7, 13, 1), "raised spike position");
        assert_eq!(spike[2], 0xD0, "SpikeDown (var type D), 1 wide");

        let row7 = e[1].patched;
        assert_eq!(pos(row7), (7, 13, 2), "row-7 brick position");
        assert_eq!((row7[2], row7[3]), (0xE0, 0x0D), "SolidBrick 1 tall, cols 2-15");

        assert_eq!(e[2].patched[0] & 0x0F, 6, "ceiling now rows 0-6");

        let run = e[3].patched;
        assert_eq!(pos(run), (8, 13, 2), "spike run stays at row 8, starts col 2");
        assert_eq!(run[2], 0xDD, "SpikeDown, 14 wide: cols 2-15");
    }

    /// Applying twice is applying once — the second pass sees non-vanilla bytes
    /// and declines.
    #[test]
    fn apply_is_idempotent() {
        let Some(rom) = load_vanilla() else { return };
        let mut once = rom.clone();
        apply(&mut once);
        let mut twice = once.clone();
        apply(&mut twice);
        assert_eq!(once.data, twice.data, "a second apply moved more bytes");
    }

    /// One foreign byte in 8F keeps **all** of 8F vanilla — a half-applied 8F
    /// would shorten the ceiling without restoring row 7 — while 7-5, a
    /// separate fix, still lands.
    #[test]
    fn a_fix_with_a_modified_site_is_skipped_whole() {
        let Some(rom) = load_vanilla() else { return };
        let mut patched = rom.clone();
        let spoiled = FIXES[0].edits[3].offset + 1;
        patched.write_byte(spoiled, 0x5A);
        apply(&mut patched);

        for edit in &FIXES[0].edits[..3] {
            assert_eq!(
                patched.read_range(edit.offset, edit.vanilla.len()),
                edit.vanilla,
                "0x{:05X} was written although its fix had a foreign byte",
                edit.offset
            );
        }
        assert_eq!(patched.read_byte(spoiled), 0x5A, "the foreign byte was overwritten");
        let brick = &FIXES[1].edits[0];
        assert_eq!(patched.read_range(brick.offset, 1), brick.patched, "7-5 did not land");
    }
}
