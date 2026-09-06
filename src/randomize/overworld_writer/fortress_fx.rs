//! Step 4 — pair each lock with the fortress that opens it.
//!
//! This step used to *write* seven parallel ROM tables indexed by a global FX
//! slot. It now produces [`LockEntry`] values and nothing else; the tables are
//! gone and [`super::super::lock_keys`] owns every byte the console reads.
//! `docs/fx_table_redesign.md` has the why.
//!
//! What is left here is the one fact the builder knows and the console cannot
//! derive: **which fortress opens which lock**. Everything the old tables
//! cached about the target — its VRAM address, its `Map_Completions` column and
//! bit, the CHR quadrants, the replacement tile — is arithmetic over the target
//! cell, and the effect does that arithmetic itself now.

use super::*;
use crate::randomize::lock_keys::LockEntry;

/// Every `(fortress, lock)` pair this world places, appended to `out`.
///
/// Fortress assignments are ordered by section in `assign_pool`, so a lock's
/// `fort_section` indexes `wa.fortress` directly — and because no two locks in a
/// world share a section, the result is one lock per fortress. That bijection is
/// the charter's map-legibility rule (a lock breaking is the only feedback that
/// says which fort did it) and `lock_keys::assert_one_key_per_lock` holds it.
pub(super) fn collect_lock_entries(
    world_idx: usize,
    built: &BuiltWorld,
    wa: &WorldAssignments,
    out: &mut Vec<LockEntry>,
) {
    for lock in &built.locks {
        let Some(fort) = wa.fortress.get(lock.fort_section) else {
            continue;
        };
        out.push(LockEntry {
            key_world: world_idx,
            key_pos: fort.pos,
            target_world: world_idx,
            target_pos: lock.pos,
        });
    }
}

/// What the vanilla FX slots said, and why none of it had to be kept.
///
/// This was stage 0 of the rework (`docs/fx_table_redesign.md`): prove in Rust
/// that a slot's stored data is redundant with its target cell before writing
/// a line of 6502 that assumed it. The randomized half of that proof is gone
/// with the tables it measured — stage 1 writes no slots — but the vanilla half
/// still describes a ROM the randomizer *reads*: `overworld_pickup::open_fx_gaps`
/// opens vanilla's lock gaps from these same bytes before placement.
///
/// The two exceptions below are pinned as exact strings so a change to either
/// is a test failure rather than a surprise.
#[cfg(test)]
mod derivation {
    use crate::randomize::rom_data::{
        self, FX_MAP_COMP_IDX, FX_PATTERNS, FX_VADDR_H, FX_VADDR_L, MAP_COMPLETE_BITS,
        PRG012_FILE_BASE,
    };
    use crate::rom::Rom;

    /// `Map_Removable_Tiles` / `Map_RemoveTo_Tiles`, CPU `$A437`/`$A43F` in
    /// PRG012 (mapped at `$A000`), 8 parallel entries each. These are the
    /// engine's own answer to "what does this obstacle become", used by
    /// `Map_Reload_with_Completions` on every map load — and, since stage 1,
    /// mirrored into PRG010 so the effect can reach them too.
    const MAP_REMOVABLE_TILES: usize = PRG012_FILE_BASE + 0x437;
    const MAP_REMOVE_TO_TILES: usize = PRG012_FILE_BASE + 0x43F;
    const REMOVABLE_COUNT: usize = 8;

    /// The metatile quadrant tables are stored UL, LL, UR, LR — four 256-byte
    /// planes from [`PRG012_FILE_BASE`]. `FortressFX_Patterns` stored the same
    /// four bytes in *row* order (UL, UR, LL, LR), which is the order the
    /// effect queues them into `Graphics_Buffer`.
    const PATTERN_QUADRANT_ORDER: [usize; 4] = [0, 2, 1, 3];

    /// What the engine turns `tile` into when its obstacle is cleared, or
    /// `None` if the engine does not consider it removable.
    fn remove_to(rom: &Rom, tile: u8) -> Option<u8> {
        (0..REMOVABLE_COUNT)
            .find(|&i| rom.read_byte(MAP_REMOVABLE_TILES + i) == tile)
            .map(|i| rom.read_byte(MAP_REMOVE_TO_TILES + i))
    }

    /// The four CHR patterns that draw `tile`, read from the metatile quadrant
    /// tables in `FortressFX_Patterns` order.
    fn metatile_patterns(rom: &Rom, tile: u8) -> [u8; 4] {
        PATTERN_QUADRANT_ORDER.map(|q| rom.read_byte(PRG012_FILE_BASE + q * 256 + tile as usize))
    }

    /// The `(row, col)` a slot points at, decoded from
    /// `FortressFX_MapLocationRow`/`MapLocation`.
    fn target_cell(rom: &Rom, slot: usize) -> (usize, usize) {
        let loc_row = rom.read_byte(rom_data::FX_MAP_LOC_ROW + slot);
        let loc = rom.read_byte(rom_data::FX_MAP_LOC + slot);
        let row = (loc_row >> 4) as usize - 2;
        let col = (loc & 0x0F) as usize * 16 + (loc >> 4) as usize;
        (row, col)
    }

    /// Every stored byte of `slot` that does not equal the value derived from
    /// the slot's own target cell, as `(field, detail)` pairs.
    fn mismatches(rom: &Rom, slot: usize, world_idx: usize) -> Vec<(&'static str, String)> {
        let loc_row = rom.read_byte(rom_data::FX_MAP_LOC_ROW + slot);
        let (row, col) = target_cell(rom, slot);
        let col_in_screen = col % 16;

        let mut out = Vec::new();

        // The key itself: the engine ORed this low nibble into the map-data
        // write column at $C99B, so it had to be zero. Under position keying
        // the byte is ours and the nibble carries the completion row.
        if loc_row & 0x0F != 0 {
            out.push(("map_loc_row", format!("low nibble {:#04X} is not 0", loc_row & 0x0F)));
        }

        // VRAM address — no screen term; screens alias onto one nametable
        // window (docs/fx_table_redesign.md).
        let vram = 0x2880 + row * 64 + col_in_screen * 2;
        let (want_h, want_l) = ((vram >> 8) as u8, (vram & 0xFF) as u8);
        let (got_h, got_l) = (rom.read_byte(FX_VADDR_H + slot), rom.read_byte(FX_VADDR_L + slot));
        if (got_h, got_l) != (want_h, want_l) {
            out.push((
                "vaddr",
                format!("stored {got_h:02X}{got_l:02X}, derived {want_h:02X}{want_l:02X}"),
            ));
        }

        // Map_Completions index — the column is the map column and the bit is
        // Map_CompleteBit[row.min(7)], the table the engine already indexes in
        // Map_MarkLevelComplete. Rows 7 and 8 share bit $01 (#212).
        let (want_col, want_bit) = (col as u8, MAP_COMPLETE_BITS[row.min(7)]);
        let (got_col, got_bit) = (
            rom.read_byte(FX_MAP_COMP_IDX + slot * 2),
            rom.read_byte(FX_MAP_COMP_IDX + slot * 2 + 1),
        );
        if (got_col, got_bit) != (want_col, want_bit) {
            out.push((
                "map_comp_idx",
                format!(
                    "stored ({got_col:#04X}, {got_bit:#04X}), derived ({want_col:#04X}, {want_bit:#04X})"
                ),
            ));
        }

        // Replacement tile — the engine's removable-tile mapping applied to
        // the tile currently sitting at the target cell.
        let replace = rom.read_byte(rom_data::FX_MAP_TILE_REPLACE + slot);
        let current = rom.read_byte(rom_data::map_tile_offset(world_idx, row, col));
        match remove_to(rom, current) {
            Some(derived) if derived == replace => {}
            Some(derived) => out.push((
                "map_tile_replace",
                format!("cell holds {current:#04X}, stored {replace:#04X}, derived {derived:#04X}"),
            )),
            None => out.push((
                "map_tile_replace",
                format!("cell holds {current:#04X}, which is not in Map_Removable_Tiles"),
            )),
        }

        // Patterns — the metatile quadrants of the replacement tile. Checked
        // against the *stored* replacement so a tile mismatch above does not
        // also show up here as a second, derived failure.
        let want_pat = metatile_patterns(rom, replace);
        let got_pat: [u8; 4] = core::array::from_fn(|j| rom.read_byte(FX_PATTERNS + slot * 4 + j));
        if got_pat != want_pat {
            out.push((
                "patterns",
                format!("tile {replace:#04X}: stored {got_pat:02X?}, derived {want_pat:02X?}"),
            ));
        }

        out
    }

    fn load_rom() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    /// Vanilla's 17 slots, in the order `FortressFX_W1..W8` hands them out.
    fn vanilla_slots_by_world() -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for wi in 0..8 {
            let n = rom_data::FORTRESS_ENTRIES.iter().filter(|&&(w, _)| w == wi).count();
            for _ in 0..n {
                out.push((out.len(), wi));
            }
        }
        out
    }

    /// Vanilla derives cleanly except in two places.
    ///
    /// * **Slot `$0F` patterns.** W8's dark screen. Vanilla stores
    ///   `FF FF FF FF` — the disassembly's own comment says "Makes an all
    ///   black square in the dark" — instead of the quadrants of the tile it
    ///   writes into map RAM (`$46`). This is the one slot whose patterns
    ///   carry information the target cell does not. The rework does not
    ///   reproduce it and does not need to: the `World_8_Dark` gate in the
    ///   effect's own visibility check suppresses the VRAM write on that page
    ///   entirely and replays the arrival reveal instead.
    /// * **Slot `$08` replacement tile.** The removable table pairs each
    ///   obstacle with the path its terrain wants — `$56 -> $45` on the
    ///   ground, `$E4 -> $DA` in the sky — so a cloud lock reveals a cloud
    ///   path. W6 `(row 4, col 13)` is plain ground: it holds `$56`, its
    ///   corridor is `$42/$45/$47`, and `$DA` appears nowhere on that screen.
    ///   The slot nevertheless stores `$DA`, the sky answer, which is what the
    ///   *preceding* slot `$07` legitimately stores for the game's one real
    ///   sky lock (W5, cell `$E4`, cloud neighbours) — the value looks carried
    ///   down a row when the table was authored. It stayed invisible because
    ///   `$45` and `$DA` share CHR quadrants and the effect writes no
    ///   attribute byte, so the frame was correct either way, and the reload
    ///   mapped `$56` back to `$45` regardless. Deriving fixes it.
    const VANILLA_EXCEPTIONS: &[&str] = &[
        "slot 0x08 (map_tile_replace): cell holds 0x56, stored 0xDA, derived 0x45",
        "slot 0x0F (patterns): tile 0x46: stored [FF, FF, FF, FF], derived [FE, C0, FE, C0]",
    ];

    #[test]
    fn vanilla_fx_slot_data_is_derivable_from_its_target_cell() {
        let Some(rom) = load_rom() else { return };
        let slots = vanilla_slots_by_world();

        // The slot-to-world mapping is reconstructed from FORTRESS_ENTRIES;
        // check it against the ROM's own FortressFX_W1..W8 before trusting it.
        let from_rom: Vec<u8> =
            rom_data::read_world_fx_assignments(&rom).into_iter().flatten().collect();
        let derived: Vec<u8> = slots.iter().map(|&(slot, _)| slot as u8).collect();
        assert_eq!(from_rom, derived, "FortressFX_W1..W8 does not hand out slots 0..16 in order");

        let mut found = Vec::new();
        for &(slot, world_idx) in &slots {
            for (field, detail) in mismatches(&rom, slot, world_idx) {
                found.push(format!("slot {slot:#04X} ({field}): {detail}"));
            }
        }
        assert_eq!(
            found, VANILLA_EXCEPTIONS,
            "vanilla FX slots derive from their target cell except for the pinned exceptions"
        );
    }
}
