//! Item keys — the dispenser gate, as a proof of concept.
//!
//! **This is not the feature.** It is the smallest thing that answers the one
//! question the whole design rests on: *is a level that needs its own power-up
//! genuinely unbeatable when that power-up stops being dispensed?*
//! `docs/mimaze_layer_design.md` chains three facts —
//!
//! 1. a level tile is a wall until it is beaten,
//! 2. a level requiring a power-up carries its own source of it
//!    (`vision.md`'s fourth charter point),
//! 3. so gating that source turns the level into a wall
//!
//! — and everything else in MiMaze is built on step 3. If it is false, or if
//! it is true but reads as a bug rather than a locked door, nothing downstream
//! is worth writing. So this ships the gate in its most degenerate form, with
//! no found-mask at all: **nothing is ever found**, so every power-up block in
//! the game pays a coin, forever.
//!
//! # It costs no ROM space, and no code
//!
//! `LATP_JumpTable` (PRG008, CPU `$B7D1`) dispatches a bumped block on its
//! content byte, and **entry 4 is already `LATP_Coin`**. So the degraded
//! behaviour the real feature has to reach with a routine is, for this POC,
//! reachable by pointing the power-up entries at a vector the table already
//! holds. Three words, six bytes, no allocation, no relocation, nothing for
//! `asm::check` to check — it writes data, not code.
//!
//! That is also why this is safe to throw away: it claims no free space and
//! leaves `LATP_Flower`/`LATP_Leaf`/`LATP_Star` sitting untouched where they
//! are, which is exactly where the real routine will be built (see
//! `item_keys_design.md`'s allocation section — those three bodies are the
//! 28 reclaimed bytes the feature is budgeted into).
//!
//! # What lands in the coin block's lap
//!
//! `LATP_Coin` runs `LATP_CoinCommon` and the 10-coin-block bookkeeping, so
//! the block really is a coin block for that bump rather than a power-up
//! block that emits nothing. Vanilla already proves the splice is safe:
//! `LATP_CoinStar` reaches `LATP_Coin` by `JMP` from a handler that has
//! already set `PUp_StarManFlash`, so arriving there with that flag dirty is
//! a path the engine takes today.
//!
//! # The three entries, and why not more
//!
//! | entry | handler | what it gated |
//! |---|---|---|
//! | 1 | `LATP_Flower` | mushroom (small) / fire flower (big) |
//! | 2 | `LATP_Leaf` | mushroom (small) / super leaf (big) |
//! | 3 | `LATP_Star` | starman |
//!
//! Those three cover three of the four known requirement levels: 6-5 (leaf),
//! 8-F (mushroom, via the small-player default in entries 1 and 2) and 7-7
//! (star). 7-F1 needs the Big [?] block as well, which is an *object* in
//! PRG005 rather than a jump-table entry, and is deliberately left out — one
//! bank at a time.
//!
//! Entry 0 (`LATP_None`) is left alone: it dispenses nothing already.

use crate::rom::Rom;

/// `LATP_JumpTable`, PRG008 CPU `$B7D1`. Twelve words, one per block content
/// type, indexed by `Y = type * 2`.
const LATP_JUMP_TABLE: usize = 0x117E1;

/// `LATP_Coin`, CPU `$B810` — the table's own entry 4.
const LATP_COIN_CPU: u16 = 0xB810;

/// The entries that dispense a power-up, with the vanilla vector each holds.
/// Checked before writing: if a word is not what vanilla put there, something
/// else has already claimed the table and silently overwriting it would be
/// the bug this pass exists to avoid.
const GATED: &[(usize, u16)] = &[
    (1, 0xB7EC), // LATP_Flower
    (2, 0xB7FA), // LATP_Leaf
    (3, 0xB808), // LATP_Star
];

/// File offset of jump-table entry `n`'s word.
const fn entry_offset(n: usize) -> usize {
    LATP_JUMP_TABLE + n * 2
}

/// Point every power-up block at `LATP_Coin`.
///
/// Panics if the table does not hold vanilla's vectors, which means either the
/// ROM is not the one this was written against or another pass got here first.
pub fn apply_poc(rom: &mut Rom) {
    rom.push_tag("item_keys/poc");
    for &(entry, vanilla) in GATED {
        let off = entry_offset(entry);
        let found = u16::from_le_bytes([rom.read_byte(off), rom.read_byte(off + 1)]);
        assert_eq!(
            found, vanilla,
            "LATP_JumpTable entry {entry} holds {found:#06X}, not vanilla's {vanilla:#06X}"
        );
        rom.write_range(off, &LATP_COIN_CPU.to_le_bytes());
    }
    rom.pop_tag();
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROM_PATH: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";

    fn vanilla() -> Option<Rom> {
        Rom::from_bytes(&std::fs::read(ROM_PATH).ok()?).ok()
    }

    fn word(rom: &Rom, entry: usize) -> u16 {
        let off = entry_offset(entry);
        u16::from_le_bytes([rom.read_byte(off), rom.read_byte(off + 1)])
    }

    /// The table is where this module says it is. Without this the whole
    /// module writes six bytes into the middle of something else.
    #[test]
    fn jump_table_is_where_we_think() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        assert_eq!(word(&rom, 0), 0xB7E9, "entry 0 should be LATP_None");
        for &(entry, vector) in GATED {
            assert_eq!(word(&rom, entry), vector, "entry {entry}");
        }
        assert_eq!(word(&rom, 4), LATP_COIN_CPU, "entry 4 should be LATP_Coin");
    }

    /// Every power-up entry points at the coin vector, and nothing else moved.
    #[test]
    fn poc_repoints_only_the_power_up_entries() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let before: Vec<u16> = (0..12).map(|n| word(&rom, n)).collect();
        apply_poc(&mut rom);

        for (n, &was) in before.iter().enumerate() {
            let expected = if GATED.iter().any(|&(e, _)| e == n) { LATP_COIN_CPU } else { was };
            assert_eq!(word(&rom, n), expected, "entry {n}");
        }
    }

    /// The bodies the real feature is budgeted into are still standing. The
    /// POC frees them by repointing, and must not also overwrite them — the
    /// 28 bytes at `LATP_Flower` are the feature's allocation.
    #[test]
    fn poc_leaves_the_reclaimed_bodies_intact() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let bodies = rom.read_range(0x117FC, 28).to_vec();
        apply_poc(&mut rom);
        assert_eq!(rom.read_range(0x117FC, 28), &bodies[..]);
    }
}
