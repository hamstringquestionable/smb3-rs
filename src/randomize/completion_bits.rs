//! World-maze phase 1: which map cells own a completion bit, and where each
//! world's packed slice lives.
//!
//! # Why this exists
//!
//! `Map_Completions` is 128 bytes at `$7D00` — 64 columns of eight row-bits for
//! Mario, then the same again for the permanent-alteration mirror — and it
//! holds exactly *one* world. The world-maze needs eight worlds resident at
//! once, and 8 x 128 = 1024 bytes against the 384 bytes of free SRAM the
//! disassembly declares. Raw banking fits two worlds; a third has nowhere to
//! go. Packing is what makes a third world possible, not an optimisation.
//!
//! The saving is that almost none of those 512 bits can ever be set. A world's
//! grid is mostly path and scenery; only the cells
//! `Map_Reload_with_Completions` can act on — level panels, forts, toad houses,
//! locks, rocks, water gaps — ever carry a bit. Across seeds that is 21 to 103
//! cells per world (`completion_bit_census`), so one bit per *owning cell*
//! instead of one bit per (column, row) turns 128 bytes per world into ~6 to
//! ~13.
//!
//! # The contract
//!
//! A world's **column mask** is the whole model: one byte per map column, with
//! the same bit layout as `Map_Completions` itself, set wherever that cell owns
//! a bit. Packing is then "walk the mask, take the live bits in order"; the
//! mask is also what a 6502 twin has to reproduce, whether it derives the mask
//! by walking the ROM grid or reads one the randomizer emitted.
//!
//! Bit order within a plane is the engine's own: columns ascending, and within
//! a column `$80` down to `$01` — so a 6502 packer is a `ROL` accumulator over
//! the same loop the engine already runs in `PRG012_A4E5`.
//!
//! ## Row 7 and row 8 share bit `$01`
//!
//! `Map_Complete_Bits` has eight entries for nine map rows, and
//! `Map_MarkLevelComplete`'s `Map_CompleteY` table has seven Y coordinates for
//! the same nine rows, falling back to index 7 for anything it does not
//! recognise. Both rows 7 and 8 therefore write bit `$01`. Reading it back,
//! `PRG012_A55C` steps down to row 8 only when row 7's tile is not one it can
//! act on — so the bit means row 7 whenever row 7 is completable, and row 8
//! otherwise.
//!
//! The mask folds the pair into the one bit they share, which is faithful: one
//! bit is one bit however many tiles claim it. Keeping two pieces of *content*
//! off the pair is the builder's job, not this module's —
//! `WorldState::row78_barred` does it, over placed slots, locks and completable
//! terrain alike. `row78_collision_census` below watches that from this side.
//!
//! # Two planes, not one
//!
//! Mario's half and the mirror at `$7D40` are both stored. The mirror is not a
//! copy: three sites write a permanent alteration into it unconditionally so it
//! survives a game over, `Map_Reload_with_Completions` applies it as a second
//! pass over the same four screens, and the game-over merge at `PRG030_9314`
//! ANDs the two. Banking one and dropping the other is what made a World 1
//! fortress light a tile in World 2 (see [`super::world_persist`]). Each plane
//! is stored separately and byte-aligned per world: all eight Mario planes
//! first, then all eight mirror planes at a fixed offset.
//!
//! # What this module does not do
//!
//! It emits the tables; it does not write them to the ROM and it does not
//! replace the two-world swap at `$84CD`. That is the next step, and it needs
//! the 6502 twin this module is the reference for.

// Reason: every item here is the reference twin for the 6502 pack/unpack that
// replaces the two-world swap at `$84CD`, plus the tables that patch will read.
// The tests below exercise all of it; the ROM-writing consumer lands with the
// 6502 side.
#![allow(dead_code)]

use crate::rom::Rom;

use super::overworld_build::is_completion_unsafe;
use super::rom_data::{self, Grid, MAP_COMPLETE_BITS};

/// One `Map_Completions` half — 64 columns, one byte of row-bits each.
pub(crate) const HALF_LEN: usize = 64;

/// Which cells of one world's map own a completion bit, and where every
/// world's packed slice starts.
///
/// Built from the ROM's own tile grids, so it describes the map a given seed
/// actually produced rather than vanilla's layout.
#[derive(Clone, Debug)]
pub(crate) struct CompletionMap {
    /// Per world, one mask byte per map column, in `Map_Completions` bit order.
    masks: [Vec<u8>; 8],
    /// Byte offset of each world's plane within one plane region, plus a
    /// terminator: `bases[w + 1] - bases[w]` is world `w`'s plane length and
    /// `bases[8]` is the whole region's length.
    bases: [u8; 9],
}

impl CompletionMap {
    /// Read every world's grid and work out its owning cells.
    ///
    /// Reads the **ROM** grids, never `Tile_Mem`: the live copy mutates as you
    /// play — a cleared level becomes an M/L tile, a busted lock becomes path —
    /// so enumerating it would shift every bit after the first change.
    pub(crate) fn from_rom(rom: &Rom) -> Self {
        let masks: [Vec<u8>; 8] =
            std::array::from_fn(|w| world_mask(&rom_data::read_tile_grid(rom, w)));

        let mut bases = [0u8; 9];
        for w in 0..8 {
            let bytes = popcount(&masks[w]).div_ceil(8);
            bases[w + 1] = bases[w]
                .checked_add(bytes as u8)
                .expect("packed completion planes must fit a one-byte offset table");
        }

        Self { masks, bases }
    }

    /// World `w`'s column mask — one byte per map column.
    pub(crate) fn mask(&self, world: usize) -> &[u8] {
        &self.masks[world]
    }

    /// How many bits world `w` needs in one plane.
    pub(crate) fn bits(&self, world: usize) -> usize {
        popcount(&self.masks[world])
    }

    /// Byte length of world `w`'s slice of one plane.
    pub(crate) fn plane_bytes(&self, world: usize) -> usize {
        (self.bases[world + 1] - self.bases[world]) as usize
    }

    /// Byte offset of world `w`'s Mario plane from the start of the packed
    /// region. Its mirror plane sits [`Self::mirror_offset`] further on.
    pub(crate) fn base(&self, world: usize) -> usize {
        self.bases[world] as usize
    }

    /// Distance from a world's Mario plane to its mirror plane — the length of
    /// the whole Mario region, since the planes are stored one after the other.
    pub(crate) fn mirror_offset(&self) -> usize {
        self.bases[8] as usize
    }

    /// Total SRAM the packed region needs, both planes.
    pub(crate) fn total_bytes(&self) -> usize {
        2 * self.mirror_offset()
    }

    /// The nine-byte table a 6502 twin indexes by world.
    ///
    /// Nine and not eight so a world's length comes for free as
    /// `table[w + 1] - table[w]`, and `table[8]` doubles as the Mario-to-mirror
    /// distance. The offsets are a table and never a stride — World 6 runs to
    /// 103 cells against World 1's 21.
    pub(crate) fn base_table(&self) -> [u8; 9] {
        self.bases
    }

    /// Compress one `Map_Completions` half into world `w`'s plane.
    ///
    /// Bits the mask does not own are dropped, which is lossless by
    /// construction: the engine only ever sets a bit through a cell it acted
    /// on, and those are exactly the owned ones.
    pub(crate) fn pack(&self, world: usize, half: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; self.plane_bytes(world)];
        let mut k = 0;
        for (col, &m) in self.masks[world].iter().enumerate() {
            for bit in MAP_COMPLETE_BITS {
                if m & bit == 0 {
                    continue;
                }
                if half[col] & bit != 0 {
                    out[k / 8] |= 0x80 >> (k % 8);
                }
                k += 1;
            }
        }
        out
    }

    /// Expand world `w`'s plane back into a full `Map_Completions` half.
    pub(crate) fn unpack(&self, world: usize, plane: &[u8]) -> [u8; HALF_LEN] {
        let mut half = [0u8; HALF_LEN];
        let mut k = 0;
        for (col, &m) in self.masks[world].iter().enumerate() {
            for bit in MAP_COMPLETE_BITS {
                if m & bit == 0 {
                    continue;
                }
                if plane[k / 8] & (0x80 >> (k % 8)) != 0 {
                    half[col] |= bit;
                }
                k += 1;
            }
        }
        half
    }
}

/// One world's column mask.
///
/// Rows 0..=7 take their own bit; row 8 folds onto row 7's `$01`, which is the
/// engine's rule and not an approximation — see the module header.
fn world_mask(grid: &Grid) -> Vec<u8> {
    let mut mask = vec![0u8; grid.cols];
    for (col, m) in mask.iter_mut().enumerate() {
        for row in 0..grid.rows() {
            if is_completion_unsafe(grid.get(row, col)) {
                *m |= MAP_COMPLETE_BITS[row.min(7)];
            }
        }
    }
    mask
}

fn popcount(mask: &[u8]) -> usize {
    mask.iter().map(|b| b.count_ones() as usize).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROM_PATH: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";

    fn vanilla() -> Option<Rom> {
        let bytes = std::fs::read(ROM_PATH).ok()?;
        Some(Rom::from_bytes(&bytes).expect("vanilla ROM parses"))
    }

    fn seeds() -> u64 {
        std::env::var("CENSUS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(10)
    }

    /// A named option arm, the same shape `completion_bit_census` uses.
    type Arm = (&'static str, fn(&mut crate::Options));

    /// The option arms `completion_bit_census` uses — the ones that move the
    /// *cell count* rather than the routing.
    fn arms() -> [Arm; 3] {
        [
            ("defaults", |_| {}),
            ("rocks+8s-wild", |o| {
                o.more_hammer_rocks = crate::Tri::On;
                o.eights_are_wild = crate::Tri::On;
            }),
            ("all-promotions", |o| {
                o.more_hammer_rocks = crate::Tri::On;
                o.eights_are_wild = crate::Tri::On;
                o.shuffle_spade_games = true;
                o.shuffle_toad_houses = true;
                o.shuffle_hammer_bros = true;
                o.include_beta_stages = true;
            }),
        ]
    }

    /// Count of cells the engine's completion routine can act on — the
    /// predicate `completion_bit_census` measures the RAM budget with.
    fn completable_cells(grid: &Grid) -> usize {
        let mut n = 0;
        for row in 0..grid.rows() {
            for col in 0..grid.cols {
                if is_completion_unsafe(grid.get(row, col)) {
                    n += 1;
                }
            }
        }
        n
    }

    /// One bit per completable cell, and no bit without one.
    ///
    /// The mask folds row 8 onto row 7's `$01`, so it can only agree with a
    /// straight cell count while the builder's `is_row78_conflict` invariant
    /// holds. That makes this an assertion about two things at once, which is
    /// the point: a column carrying a completable cell in *both* row 7 and row
    /// 8 is a map the engine cannot represent, and it would show up here as a
    /// bit shortfall rather than as a mystery on a player's cartridge.
    ///
    /// ```sh
    /// CENSUS_SEEDS=200 cargo test --release --lib bit_per_completable_cell
    /// ```
    /// Where two map cells are fighting over one completion bit.
    ///
    /// Rows 7 and 8 share bit `$01` per column, so a column holding a cell the
    /// engine can act on in *both* rows has one bit for two jobs. Row 7 wins:
    /// `PRG012_A55C` only steps down to row 8 when row 7's tile is not one it
    /// recognises. So a level panel placed at row 8 under a completable row-7
    /// tile would be marked beaten on the tile above it and never on itself.
    ///
    /// `WorldState::row78_barred` is what prevents that, and
    /// `row78_completion_bit_is_never_double_claimed` asserts it on the written
    /// ROM. This is the same invariant seen from the storage side: the mask
    /// folds the pair onto one bit, so a shortfall against the plain cell count
    /// *is* a collision, found without knowing anything about placement.
    /// Cheap, and independent — if the two ever disagree, one of them is wrong.
    ///
    /// A count of zero is not the pass condition. `decor` collisions are two
    /// scenery tiles that merely score as completable — World 6's `$EA` ice
    /// band runs the length of both rows — and nothing ever sets their bit, so
    /// they cost nothing and the builder rightly leaves them alone. **`LIVE`
    /// collisions, with placed content on one side, are the failure**: 47 of
    /// them over 150 builds before the terrain claimant was handled (PR #212),
    /// zero after.
    ///
    /// ```sh
    /// CENSUS_SEEDS=50 cargo test --release --lib row78_collision_census \
    ///     -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn row78_collision_census() {
        let Ok(rom_bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };

        let mut collisions: Vec<(bool, String)> = Vec::new();
        let mut runs = 0usize;
        for (name, arm) in arms() {
            for seed in 0..seeds() {
                let mut options =
                    crate::Options { palettes: false, palette_themed: false, ..Default::default() };
                arm(&mut options);
                let Ok((rom, build)) =
                    crate::randomize_rom_with_overworld_capture(&rom_bytes, seed, &options, None)
                else {
                    continue;
                };
                runs += 1;
                let map = CompletionMap::from_rom(&rom);
                for w in 0..8 {
                    let grid = rom_data::read_tile_grid(&rom, w);
                    // The mask folds the pair onto one bit, so a shortfall
                    // against the cell count is exactly a collision.
                    if map.bits(w) == completable_cells(&grid) {
                        continue;
                    }
                    let placed: std::collections::HashSet<(usize, usize)> = build.worlds[w]
                        .slots
                        .iter()
                        .map(|s| s.pos)
                        .chain(build.worlds[w].locks.iter().map(|l| l.pos))
                        .collect();
                    for col in 0..grid.cols {
                        if !is_completion_unsafe(grid.get(7, col))
                            || !is_completion_unsafe(grid.get(8, col))
                        {
                            continue;
                        }
                        let live = placed.contains(&(7, col)) || placed.contains(&(8, col));
                        collisions.push((
                            live,
                            format!(
                                "{name} seed {seed} W{} col {col}: row7 {:#04X} row8 {:#04X}",
                                w + 1,
                                grid.get(7, col),
                                grid.get(8, col),
                            ),
                        ));
                    }
                }
            }
        }

        let live: Vec<&String> = collisions.iter().filter(|(l, _)| *l).map(|(_, t)| t).collect();
        eprintln!(
            "{} collisions over {runs} builds; {} with placed content",
            collisions.len(),
            live.len(),
        );
        for t in &live {
            eprintln!("  LIVE {t}");
        }
    }

    /// Vanilla's own fortress-FX table names a `(column, bit)` for every lock
    /// and drawbridge in the game. Each one must be a bit this model stores,
    /// or busting that lock would be forgotten on the way out of the world.
    ///
    /// This is the engine's answer rather than the builder's: the entries are
    /// hand-authored ROM data, and the slot-to-world mapping comes from
    /// `FORTRESS_ENTRIES` — the same derivation `world_persist` uses, and the
    /// one a hand-written list got wrong in the mega-map branch.
    #[test]
    fn vanilla_fx_bits_are_owned() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let map = CompletionMap::from_rom(&rom);

        for (slot, &(world, _)) in rom_data::FORTRESS_ENTRIES.iter().enumerate() {
            let col = rom.read_byte(rom_data::FX_MAP_COMP_IDX + slot * 2) as usize;
            let bit = rom.read_byte(rom_data::FX_MAP_COMP_IDX + slot * 2 + 1);
            let mask = map.mask(world);
            assert!(col < mask.len(), "FX slot {slot}: column {col} is off W{}'s map", world + 1);
            assert!(
                mask[col] & bit != 0,
                "FX slot {slot} (W{}, column {col}, bit {bit:#04X}) owns no completion bit",
                world + 1,
            );
        }
    }

    /// Round-tripping a `Map_Completions` half through a world's plane is the
    /// identity on the bits that world can hold, and drops the rest.
    ///
    /// Every bit is exercised in both states rather than sampled: with the mask
    /// as the alphabet, "all set" and "all clear" plus the alternating patterns
    /// cover every owned bit's two values and every unowned bit's ability to
    /// leak into a neighbour's slot.
    #[test]
    fn pack_round_trips_owned_bits() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let map = CompletionMap::from_rom(&rom);

        for w in 0..8 {
            let mask = map.mask(w).to_vec();
            // All-ones is the strongest input: every unowned bit is set too, so
            // anything that mistakes an unowned bit for an owned one shifts the
            // whole stream and shows up as a mismatch downstream.
            for fill in [0x00u8, 0xFF, 0xAA, 0x55] {
                let half = [fill; HALF_LEN];
                let plane = map.pack(w, &half);
                assert_eq!(plane.len(), map.plane_bytes(w), "W{}: plane length", w + 1);

                let back = map.unpack(w, &plane);
                for col in 0..HALF_LEN {
                    let owned = mask.get(col).copied().unwrap_or(0);
                    assert_eq!(
                        back[col],
                        half[col] & owned,
                        "W{} column {col}, fill {fill:#04X}: round trip",
                        w + 1,
                    );
                }
            }
        }
    }

    /// The packed region fits the free SRAM the design depends on, with the
    /// margin reported rather than assumed.
    ///
    /// `completion_bit_census` measures the same budget from the cell side;
    /// this measures it from the side that actually allocates, including the
    /// byte alignment each world's slice pays for being addressable on its own.
    #[test]
    fn packed_region_fits_free_sram() {
        let Ok(rom_bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };

        // The two largest anonymous `.ds` runs in the disassembly, each
        // referenced nowhere but its own declaration, and the pair the
        // `world_persist` POC already proved free at runtime.
        const LARGEST_RUNS: usize = 109 + 105;

        let mut worst = 0usize;
        let mut worst_at = (0u64, "", [0u8; 9]);
        // The stencil is ROM data, not SRAM, but its size decides whether the
        // console re-derives it or reads it — so it is measured here too. Raw
        // is one mask byte per column each world actually has; the alternative
        // is a per-world 8-byte "which columns are non-zero" bitmap plus one
        // byte per non-zero column.
        let mut stencil_raw = 0usize;
        let mut stencil_sparse = 0usize;
        for (name, arm) in arms() {
            for seed in 0..seeds() {
                let mut options =
                    crate::Options { palettes: false, palette_themed: false, ..Default::default() };
                arm(&mut options);
                let Ok((rom, _)) =
                    crate::randomize_rom_with_overworld_capture(&rom_bytes, seed, &options, None)
                else {
                    continue;
                };
                let map = CompletionMap::from_rom(&rom);
                if map.total_bytes() > worst {
                    worst = map.total_bytes();
                    worst_at = (seed, name, map.base_table());
                }
                let raw: usize = (0..8).map(|w| map.mask(w).len()).sum();
                // The presence bitmap is sized per world — World 1's 16
                // columns need two bytes, not World 8's eight.
                let sparse: usize = (0..8)
                    .map(|w| {
                        let m = map.mask(w);
                        m.len().div_ceil(8) + m.iter().filter(|b| **b != 0).count()
                    })
                    .sum();
                stencil_raw = stencil_raw.max(raw);
                stencil_sparse = stencil_sparse.max(sparse);
            }
        }

        let (seed, name, table) = worst_at;
        eprintln!("worst packed region: {worst} bytes (seed {seed}, arm {name})");
        eprintln!("  base table {table:?}  (both planes: 2 x {} bytes)", table[8]);
        eprintln!("  against {LARGEST_RUNS} bytes in the two proven-free SRAM runs");
        eprintln!("stencil, if emitted as ROM data: {stencil_raw} raw, {stencil_sparse} sparse");
        assert!(
            worst <= LARGEST_RUNS,
            "packed completion region needs {worst} bytes, more than the {LARGEST_RUNS} \
             the two proven-free SRAM runs hold",
        );
    }
}
