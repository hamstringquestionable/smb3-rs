//! **The away nub: "apart" by shape, not only by colour.**
//!
//! With map hints on, a fortress and its lock say whether they are together by
//! colour — tan here, the alternate colour apart — and on some world palettes
//! a colourblind player cannot tell the two apart (#325). So the away fortress
//! `$EB` also wears a nub in its lower-right corner, and so does every away
//! lock that has no world digit there (all of them on some-hints; World 8's on
//! full hints). The nub says nothing the colour does not; it says it twice.
//!
//! **The nub is drawn into `$EB` itself.** Until 2026-10-05 it was a separate
//! tile, `$EC`, which needed its own removable row and a rewrite of PRG011's
//! fortress-clear pick to crumble like `$EB`. Some and full hints now share one
//! colour scheme, so every hinted away fortress is the nubbed one and `$EB`
//! never has to mean anything else: vanilla's own crumble and rubble serve it
//! unchanged.
//!
//! **Only when hints are on.** `$EB` is also one of `FORTRESS_TILES`, the
//! cosmetic pick when the map says nothing, and there it keeps vanilla's art.
//! With hints on in the maze every fortress opens exactly one lock
//! (`maze::fill`), so every fortress carries a hint and the cosmetic pick never
//! runs.
//!
//! The lock half is `lock_keys::LockTiles`, which writes [`MARK`] into an
//! allocated lock's corner when its request is `marked`.

use crate::randomize::rom_data::{self, PRG012_FILE_BASE};
use crate::rom::Rom;

/// The nub the away fortress and its lock wear in their lower-right quadrant:
/// the small round marker vanilla draws at path ends (`$44`, `$66`
/// `TILE_PATHANDNUB`, and seven more use it there). Borrowed, never redrawn, so
/// it costs no CHR.
pub(crate) const MARK: u8 = 0xCD;

/// Put the nub on `$EB` — **only with hints on**, so every other mode's ROM is
/// byte-for-byte what it was.
///
/// One byte: the metatile planes are stored UL, LL, UR, LR, so plane 3 is the
/// lower-right corner.
pub(crate) fn apply(rom: &mut Rom, hints: crate::HintMode) {
    if hints.hints_at_all() {
        rom.write_byte(PRG012_FILE_BASE + 3 * 256 + rom_data::TILE_FORTRESS_AWAY as usize, MARK);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::randomize::overworld::build::{FortRef, LockHint, SlotKind};

    const ROM_PATH: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";

    fn seeds() -> u64 {
        std::env::var("CENSUS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(8)
    }

    fn lr(rom: &Rom, tile: u8) -> u8 {
        rom.read_byte(PRG012_FILE_BASE + 3 * 256 + tile as usize)
    }

    fn build(
        bytes: &[u8],
        seed: u64,
        world_maze: bool,
        hints: crate::HintMode,
    ) -> Option<(Rom, crate::randomize::overworld::build::BuildResult)> {
        let options = crate::Options {
            world_maze,
            hints,
            palettes: false,
            palette_themed: false,
            ..Default::default()
        };
        crate::randomize_rom_with_overworld_capture(bytes, seed, &options, None).ok()
    }

    /// **Every away fortress is the nubbed `$EB`, and its lock wears the nub
    /// unless a digit owns the corner** — on both hinted modes.
    ///
    /// Checked positionally on the written ROM against the maze's own pairing:
    /// a fortress's cell is `$EB` exactly when its hint is `Elsewhere`, and a
    /// lock wears [`MARK`] exactly when its fortress is in another world and
    /// it has no digit — every away lock on some-hints, only World 8's on
    /// full. That includes a World 8 lock opened by a beta `$6A` fortress,
    /// which keeps its own art. A fortress under a World 8 army sprite has had
    /// its cell blanked by the writer, as every hint there is — skipped, not
    /// failed.
    #[test]
    fn an_away_fortress_opens_a_marked_lock() {
        let Ok(bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        for plain in [0x54, 0x56, 0xE4, rom_data::WATER_GAP_TILE, rom_data::TILE_FORTRESS_AWAY] {
            let rom = Rom::from_bytes(&bytes).unwrap();
            assert_ne!(lr(&rom, plain), MARK, "{plain:#04X}'s corner is already the nub");
        }

        for hints in [crate::HintMode::Partial, crate::HintMode::Full] {
            let mut pairs = [0usize; 3];
            for seed in 0..seeds() {
                let Some((rom, build)) = build(&bytes, seed, true, hints) else { continue };
                assert_eq!(lr(&rom, rom_data::TILE_FORTRESS_AWAY), MARK, "{hints:?} seed {seed}");
                let grids = rom_data::read_all_tile_grids(&rom);

                let mut hint: HashMap<FortRef, LockHint> = HashMap::new();
                for (wi, w) in build.worlds.iter().enumerate() {
                    for s in w.slots.iter().filter(|s| s.kind == SlotKind::Fortress) {
                        hint.insert(FortRef { world: wi, section: s.section }, s.lock_hint);
                        let cell = grids[wi].get(s.pos.0, s.pos.1);
                        if !rom_data::is_fortress(cell) {
                            continue;
                        }
                        assert_eq!(
                            cell == rom_data::TILE_FORTRESS_AWAY,
                            s.lock_hint == LockHint::Elsewhere,
                            "{hints:?} seed {seed}: W{} {:?} is {cell:#04X} with hint {:?}",
                            wi + 1,
                            s.pos,
                            s.lock_hint
                        );
                    }
                }
                for (wi, w) in build.worlds.iter().enumerate() {
                    for lock in &w.locks {
                        let away = lock.fort.world != wi;
                        let digit_owns_corner =
                            hints == crate::HintMode::Full && wi != rom_data::W8_IDX;
                        let tile = grids[wi].get(lock.pos.0, lock.pos.1);
                        assert_eq!(
                            lr(&rom, tile) == MARK,
                            away && !digit_owns_corner,
                            "{hints:?} seed {seed}: the W{} lock at {:?} ({tile:#04X}) opened by \
                             W{}'s fortress (hint {:?})",
                            wi + 1,
                            lock.pos,
                            lock.fort.world + 1,
                            hint.get(&lock.fort)
                        );
                        match hint.get(&lock.fort) {
                            _ if !away => pairs[0] += 1,
                            Some(LockHint::World8) => pairs[2] += 1,
                            _ => pairs[1] += 1,
                        }
                    }
                }
            }
            assert!(
                pairs.iter().all(|&n| n > 0),
                "{hints:?}: sampled {} local, {} elsewhere and {} beta-fortress locks; each \
                 case must occur",
                pairs[0],
                pairs[1],
                pairs[2]
            );
        }
    }

    /// **Without hints nothing is nubbed**: hints off in the maze, and the
    /// default some-hints outside it (which the pipeline treats as off). `$EB`
    /// is a cosmetic fortress there and keeps vanilla's art.
    #[test]
    fn no_hints_no_nub() {
        let Ok(bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let vanilla = Rom::from_bytes(&bytes).unwrap();
        let unhinted = [(true, crate::HintMode::Off), (false, crate::HintMode::Partial)];
        for (maze, hints) in unhinted {
            for seed in 0..seeds().min(4) {
                let Some((rom, _)) = build(&bytes, seed, maze, hints) else { continue };
                assert_eq!(
                    lr(&rom, rom_data::TILE_FORTRESS_AWAY),
                    lr(&vanilla, rom_data::TILE_FORTRESS_AWAY),
                    "maze={maze} {hints:?} seed {seed}: $EB was nubbed"
                );
                for g in rom_data::read_all_tile_grids(&rom) {
                    for r in 0..g.rows() {
                        for c in 0..g.cols {
                            let t = g.get(r, c);
                            assert!(
                                !crate::randomize::overworld::lock_keys::is_pool_tile(t)
                                    || lr(&rom, t) != MARK,
                                "maze={maze} {hints:?} seed {seed}: {t:#04X} wears the nub"
                            );
                        }
                    }
                }
            }
        }
    }
}
