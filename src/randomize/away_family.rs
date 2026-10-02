//! **Away families: halve the fortresses a some-hints player has to try.**
//!
//! Under some-hints an away lock only says "my fortress is in another world",
//! and a maze seed has about ten such fortresses (measured: 61% of its 17). A
//! player stuck at one was left to guess among all of them. This splits them in
//! two: every other `Elsewhere` fortress wears a nub in its lower-right corner,
//! and so does the lock it opens. A nubbed lock is opened by a nubbed fortress,
//! a plain one by a plain one.
//!
//! The *fact* is the maze's — `maze::stamp_into` alternates
//! `LockHint::Elsewhere { marked }` across the away fortresses, consuming no
//! RNG. The bytes are split three ways:
//!
//! * the fortress tile, [`rom_data::TILE_FORTRESS_AWAY_MARKED`] (`$EC`), stamped
//!   by `overworld_writer::grid`, with its removable row in
//!   `lock_keys::REMOVABLE_PAIRS`;
//! * the lock, allocated by `lock_keys::LockTiles` with `marked` set, wearing
//!   [`MARK`] in the same corner;
//! * this module: `$EC`'s art and its crumble.
//!
//! Full hints never shows it (its world digit already names the fortress's
//! world, and owns the same corner), and neither does Off. World 8's own
//! fortresses (`$6A`) are a family of their own already and are untouched.
//!
//! **To remove the feature:** make `stamp_into` always emit `marked: false`.
//! No `$EC` is then stamped, [`apply`] writes nothing, and every lock asks for
//! what it did before. The remaining code is inert.

use super::rom_data::{self, Grid, PRG012_FILE_BASE, prg_bank_cpu_to_file};
use crate::rom::Rom;

/// The nub both halves of a marked pair wear in their lower-right quadrant:
/// the small round marker vanilla draws at path ends (`$44`, `$66`
/// `TILE_PATHANDNUB`, and seven more use it there). Borrowed, never redrawn, so
/// it costs no CHR.
pub(crate) const MARK: u8 = 0xCD;

/// The fortress whose art the marked one copies: the away fortress `$EB`, so
/// the two away families differ only by the nub.
const ART: u8 = rom_data::TILE_FORTRESS_AWAY;

/// The fortress-clear tile pick in PRG011, after its `PLA` at `$AA8C`:
/// `Map_CompleteTile` gets X = 8 (rubble `$60`) for `$67`/`$6A`, X = 9 (alt
/// rubble `$E3`) for `$EB`, with the crumble sound, and the quadrant/player
/// index for everything else.
const CRUMBLE_PICK_CPU: u16 = 0xAA8D;
const CRUMBLE_PICK: usize = prg_bank_cpu_to_file(11, CRUMBLE_PICK_CPU);

/// Vanilla's pick. The crumble-sound store is written out twice.
#[cfg(test)]
#[rustfmt::skip]
const CRUMBLE_PICK_VANILLA: [u8; 26] = [
    0xC9, 0x67,             // CMP #TILE_FORT
    0xF0, 0x04,             // BEQ fort
    0xC9, 0x6A,             // CMP #TILE_LARGEFORT
    0xD0, 0x07,             // BNE not_fort
    0xA9, 0x01,             // fort: LDA #SND_LEVELCRUMBLE
    0x8D, 0xF3, 0x04,       //       STA Sound_QLevel2
    0xA2, 0x08,             //       LDX #$08          ; -> rubble $60
    0xC9, 0xEB,             // not_fort: CMP #TILE_ALTFORT
    0xD0, 0x07,             //       BNE done
    0xA9, 0x01,             //       LDA #SND_LEVELCRUMBLE
    0x8D, 0xF3, 0x04,       //       STA Sound_QLevel2
    0xA2, 0x09,             //       LDX #$09          ; -> alt rubble $E3
];

/// The same pick with `$EC` added, in the same 26 bytes.
///
/// **One shared tail pays for the extra compare.** Vanilla writes "set X,
/// queue the crumble" out twice, once per fortress colour. Here all four
/// fortress tiles branch to one `fort` tail, which works out *which* rubble
/// from the tile itself: X has to be 8 for `$67`/`$6A` and 9 for `$EB`/`$EC`,
/// and that is exactly the tile's top bit (page 1 vs page 3). `ASL A` drops
/// that bit into carry; `LDA #$04` leaves carry alone; `ROL A` makes
/// `4 << 1 | carry` — 8 or 9. Five bytes, against the four (`LDX #` twice) plus
/// a skip the two-copy layout would need, and no branch lands mid-instruction.
/// `A` was about to be overwritten by `done`'s `LDA Map_CompleteTile,X` in
/// vanilla too.
#[rustfmt::skip]
const CRUMBLE_PICK_PATCHED: [u8; 26] = [
    0xC9, 0x67,             //  0: CMP #TILE_FORT
    0xF0, 0x0C,             //  2: BEQ fort          ; -> 16
    0xC9, 0x6A,             //  4: CMP #TILE_LARGEFORT
    0xF0, 0x08,             //  6: BEQ fort
    0xC9, 0xEB,             //  8: CMP #TILE_ALTFORT
    0xF0, 0x04,             // 10: BEQ fort
    0xC9, rom_data::TILE_FORTRESS_AWAY_MARKED,
                            // 12: CMP #$EC          ; the marked away fortress
    0xD0, 0x0A,             // 14: BNE done          ; -> 26, X untouched
    0x0A,                   // 16: fort: ASL A       ; carry = page 3?
    0xA9, 0x04,             // 17:       LDA #$04
    0x2A,                   // 19:       ROL A       ; 8, or 9 for page 3
    0xAA,                   // 20:       TAX         ; -> rubble $60 / alt rubble $E3
    0xA9, 0x01,             // 21:       LDA #SND_LEVELCRUMBLE
    0x8D, 0xF3, 0x04,       // 23:       STA Sound_QLevel2
                            // 26: done (vanilla $AAA7: LDA Map_CompleteTile,X)
];

/// Install `$EC`'s art and crumble — **only on a map that wears it**, so every
/// other mode's ROM is byte-for-byte what it was.
///
/// The art is `$EB`'s four quadrants with [`MARK`] in the lower-right; the
/// planes are stored UL, LL, UR, LR, so plane 3 is that corner.
///
/// The crumble splice is what makes a beaten `$EC` behave like `$EB` at the
/// moment of the clear: without it the tile falls through to the
/// quadrant/player index and briefly becomes a page-3 Mario/Luigi panel with no
/// sound, only turning to rubble on the next map load. The splice changes no
/// other tile's outcome — `the_pick_matches_vanilla_for_every_other_tile` runs
/// both versions over all 256.
pub(crate) fn apply(rom: &mut Rom, grids: &[Grid]) {
    let present = super::lock_keys::tiles_on_map(grids);
    if !present[rom_data::TILE_FORTRESS_AWAY_MARKED as usize] {
        return;
    }
    for plane in 0..4 {
        let pattern = if plane == 3 {
            MARK
        } else {
            rom.read_byte(PRG012_FILE_BASE + plane * 256 + ART as usize)
        };
        rom.write_byte(
            PRG012_FILE_BASE + plane * 256 + rom_data::TILE_FORTRESS_AWAY_MARKED as usize,
            pattern,
        );
    }
    rom.write_range(CRUMBLE_PICK, &CRUMBLE_PICK_PATCHED);
}

#[cfg(test)]
mod asm_checks {
    use mos6502::cpu::CPU;
    use mos6502::instruction::Ricoh2a03;
    use mos6502::memory::{Bus, Memory};

    use super::*;
    use crate::randomize::rom_data::asm;

    const ROM_PATH: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";
    const SOUND_QLEVEL2: u16 = 0x04F3;
    /// `PRG011_AAA7`, where both versions hand over to `LDA Map_CompleteTile,X`.
    const DONE: u16 = CRUMBLE_PICK_CPU + 26;

    fn vanilla() -> Option<Vec<u8>> {
        std::fs::read(ROM_PATH).ok()
    }

    /// The splice is whole instructions over whole instructions, and every
    /// branch inside it lands on one.
    #[test]
    fn the_crumble_pick_is_well_formed() {
        let Some(v) = vanilla() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        asm::check(&CRUMBLE_PICK_PATCHED)
            .fragment()
            .hook(&v, CRUMBLE_PICK, &CRUMBLE_PICK_PATCHED)
            .assert_ok();
    }

    /// The bytes replaced really are vanilla's pick, at the address claimed —
    /// and nothing earlier in the pipeline has touched them.
    #[test]
    fn the_site_is_vanillas_crumble_pick() {
        let Some(v) = vanilla() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        assert_eq!(&v[CRUMBLE_PICK..CRUMBLE_PICK + 26], &CRUMBLE_PICK_VANILLA);
        assert_eq!(v[CRUMBLE_PICK - 1], 0x68, "the pick follows the tile's PLA");
    }

    /// Run one version of the pick: `A` the tile, `X` the quadrant/player
    /// index it arrives with. Returns `(X, sound)` at `done`.
    fn run(code: &[u8; 26], tile: u8, x: u8) -> (u8, u8) {
        let mut mem = Memory::new();
        mem.set_bytes(CRUMBLE_PICK_CPU, code);
        let mut cpu = CPU::new(mem, Ricoh2a03);
        cpu.memory.set_byte(SOUND_QLEVEL2, 0);
        cpu.registers.program_counter = CRUMBLE_PICK_CPU;
        cpu.registers.accumulator = tile;
        cpu.registers.index_x = x;
        for _ in 0..code.len() {
            if cpu.registers.program_counter == DONE {
                return (cpu.registers.index_x, cpu.memory.get_byte(SOUND_QLEVEL2));
            }
            cpu.single_step();
        }
        panic!("tile {tile:#04X}: the pick never reached done");
    }

    /// **The rewrite is vanilla plus exactly one case.** Over every tile and
    /// every index the engine can arrive with, the two versions agree on X and
    /// on the crumble sound — except `$EC`, which now takes the alt fortress's
    /// outcome instead of falling through.
    #[test]
    fn the_pick_matches_vanilla_for_every_other_tile() {
        for tile in 0..=u8::MAX {
            for x in 0..8 {
                let new = run(&CRUMBLE_PICK_PATCHED, tile, x);
                if tile == rom_data::TILE_FORTRESS_AWAY_MARKED {
                    assert_eq!(new, (9, 1), "$EC must crumble into alt rubble");
                    assert_eq!(run(&CRUMBLE_PICK_VANILLA, tile, x), (x, 0));
                } else {
                    assert_eq!(new, run(&CRUMBLE_PICK_VANILLA, tile, x), "tile {tile:#04X} x={x}");
                }
            }
        }
        // And the cases vanilla already had, spelled out.
        assert_eq!(run(&CRUMBLE_PICK_PATCHED, 0x67, 6), (8, 1));
        assert_eq!(run(&CRUMBLE_PICK_PATCHED, 0x6A, 3), (8, 1));
        assert_eq!(run(&CRUMBLE_PICK_PATCHED, 0xEB, 7), (9, 1));
        assert_eq!(run(&CRUMBLE_PICK_PATCHED, 0x03, 1), (1, 0));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::randomize::overworld_build::{FortRef, LockHint, SlotKind};

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
        hints: crate::HintMode,
    ) -> Option<(Rom, crate::randomize::overworld_build::BuildResult)> {
        let options = crate::Options {
            world_maze: true,
            hints,
            palettes: false,
            palette_themed: false,
            ..Default::default()
        };
        crate::randomize_rom_with_overworld_capture(bytes, seed, &options, None).ok()
    }

    /// **A marked fortress opens a marked lock, and only some-hints marks.**
    ///
    /// Checked positionally on the written ROM against the maze's own pairing:
    /// every away fortress's cell wears `$EC` exactly when its hint is marked,
    /// and the lock it opens wears [`MARK`] in its corner exactly when the
    /// fortress does. A fortress under a World 8 army sprite has had its cell
    /// blanked by the writer, as every hint there is — skipped, not failed.
    #[test]
    fn a_marked_fortress_opens_a_marked_lock() {
        let Ok(bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        for plain in [0x54, 0x56, 0xE4, rom_data::WATER_GAP_TILE] {
            let rom = Rom::from_bytes(&bytes).unwrap();
            assert_ne!(lr(&rom, plain), MARK, "{plain:#04X}'s corner is already the nub");
        }

        let mut pairs = [0usize; 2];
        for seed in 0..seeds() {
            let Some((rom, build)) = build(&bytes, seed, crate::HintMode::Partial) else {
                continue;
            };
            let grids = rom_data::read_all_tile_grids(&rom);

            let mut hint: HashMap<FortRef, LockHint> = HashMap::new();
            for (wi, w) in build.worlds.iter().enumerate() {
                for s in w.slots.iter().filter(|s| s.kind == SlotKind::Fortress) {
                    hint.insert(FortRef { world: wi, section: s.section }, s.lock_hint);
                    let cell = grids[wi].get(s.pos.0, s.pos.1);
                    if !rom_data::is_fortress(cell) {
                        continue;
                    }
                    let marked = matches!(s.lock_hint, LockHint::Elsewhere { marked: true });
                    assert_eq!(
                        cell == rom_data::TILE_FORTRESS_AWAY_MARKED,
                        marked,
                        "seed {seed}: W{} {:?} is {cell:#04X} with hint {:?}",
                        wi + 1,
                        s.pos,
                        s.lock_hint
                    );
                }
            }
            for (wi, w) in build.worlds.iter().enumerate() {
                for lock in &w.locks {
                    let Some(LockHint::Elsewhere { marked }) = hint.get(&lock.fort).copied() else {
                        continue;
                    };
                    let tile = grids[wi].get(lock.pos.0, lock.pos.1);
                    assert_eq!(
                        lr(&rom, tile) == MARK,
                        marked,
                        "seed {seed}: the W{} lock at {:?} ({tile:#04X}) disagrees with its \
                         fortress, whose hint is marked={marked}",
                        wi + 1,
                        lock.pos
                    );
                    pairs[usize::from(marked)] += 1;
                }
            }
            if super::super::lock_keys::tiles_on_map(&grids)
                [rom_data::TILE_FORTRESS_AWAY_MARKED as usize]
            {
                assert_eq!(lr(&rom, rom_data::TILE_FORTRESS_AWAY_MARKED), MARK);
            }
        }
        assert!(
            pairs[0] > 0 && pairs[1] > 0,
            "sampled {} plain and {} marked away pairs; both families must occur",
            pairs[0],
            pairs[1]
        );
    }

    /// **Full and Off never mark,** so the fortress byte, the nub and the
    /// crumble splice are all absent and those ROMs are what they were.
    #[test]
    fn only_some_hints_marks() {
        let Ok(bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let vanilla = Rom::from_bytes(&bytes).unwrap();
        for hints in [crate::HintMode::Full, crate::HintMode::Off] {
            for seed in 0..seeds().min(4) {
                let Some((rom, _)) = build(&bytes, seed, hints) else { continue };
                for g in rom_data::read_all_tile_grids(&rom) {
                    for r in 0..g.rows() {
                        for c in 0..g.cols {
                            let t = g.get(r, c);
                            assert_ne!(t, rom_data::TILE_FORTRESS_AWAY_MARKED, "{hints:?} {seed}");
                            assert!(
                                !super::super::lock_keys::is_pool_tile(t) || lr(&rom, t) != MARK,
                                "{hints:?} seed {seed}: {t:#04X} wears the nub"
                            );
                        }
                    }
                }
                assert_eq!(
                    rom.read_range(CRUMBLE_PICK, 26),
                    vanilla.read_range(CRUMBLE_PICK, 26),
                    "{hints:?} seed {seed}: the crumble pick was spliced"
                );
            }
        }
    }
}
