//! Step 4 — write fortress FX tables and the screen-check patch.

use super::*;

/// `suppress` names lock cells in this world that must NOT get an FX slot.
///
/// The world maze uses it to take a lock's **local** key away. A cross-world
/// lock is opened by a fortress in another world through
/// [`foreign_locks`](crate::randomize::foreign_locks), and until this existed
/// the lock kept its original local fortress as well — two keys, the near one
/// always found first, and the mode's headline mechanic reduced to decoration.
/// Dropping the FX slot drops both halves of the local key at once: the crumble
/// animation and the `Map_Completions` bit write that persists it.
pub(super) fn write_fortress_fx(
    rom: &mut Rom,
    world_idx: usize,
    built: &BuiltWorld,
    wa: &WorldAssignments,
    data: &OverworldData,
    fx_slot: &mut usize,
    suppress: &HashSet<(usize, usize)>,
) {
    let pickup = data.pickup;
    let catalog = data.catalog;
    // Pair each lock with its fortress assignment (matched by section).
    // Fortress assignments are ordered by section in assign_pool, so
    // assignment index == fort_section for this world.
    let locked_forts: Vec<_> = built
        .locks
        .iter()
        .filter(|lock| !suppress.contains(&lock.pos))
        .filter_map(|lock| wa.fortress.get(lock.fort_section).map(|fa| (lock, fa)))
        .collect();

    // Write FX world table (up to 4 slots per world).
    let fx_base = rom_data::FX_WORLD_TABLE + world_idx * 4;
    for i in 0..4 {
        if i < locked_forts.len() {
            rom.write_byte(fx_base + i, (*fx_slot + i) as u8);
        } else {
            rom.write_byte(fx_base + i, 0x00);
        }
    }

    for (ordinal_0, (lock, fort_a)) in locked_forts.iter().enumerate() {
        let slot = *fx_slot;
        *fx_slot += 1;

        let ordinal = (ordinal_0 + 1) as u8;

        // Look up boomboom_y_offset from the assigned fortress pool entry.
        let ce = &catalog.entries[pickup.pool[fort_a.pool_idx].catalog_idx];
        let boomboom_y_offset = match &ce.kind {
            NodeKind::Fortress { boomboom_y_offset } => *boomboom_y_offset,
            _ => panic!("fortress assignment must reference a Fortress catalog entry"),
        };

        // Patch Boom-Boom Y-byte.
        let old_y = rom.read_byte(boomboom_y_offset);
        rom.write_byte(boomboom_y_offset, (ordinal << 4) | (old_y & 0x0F));

        // Lock position.
        let (ob_row, ob_col) = lock.pos;
        let col_in_screen = ob_col % 16;
        let screen = ob_col / 16;

        // FX pattern bytes.
        let patterns = overworld_helpers::fx_patterns_for(lock.replace_tile);

        // VRAM address.
        let vram = (0x2880 + ob_row * 64 + col_in_screen * 2) as u16;
        rom.write_byte(FX_VADDR_H + slot, (vram >> 8) as u8);
        rom.write_byte(FX_VADDR_L + slot, (vram & 0xFF) as u8);

        // Map location. The engine at $C99B does `ORA $C845,X` to fold this
        // byte into the map-data write offset, so the low nibble MUST be 0 —
        // anything in bits 0..3 corrupts the destination column and the
        // replacement tile lands in the wrong cell.
        let row_byte = ((ob_row + 2) as u8) << 4;
        rom.write_byte(rom_data::FX_MAP_LOC_ROW + slot, row_byte);
        rom.write_byte(rom_data::FX_MAP_LOC + slot, ((col_in_screen as u8) << 4) | (screen as u8));

        // Replacement tile.
        rom.write_byte(rom_data::FX_MAP_TILE_REPLACE + slot, lock.replace_tile);

        // Map_Completions persistence — encodes lock position.
        let comp_col = ob_col as u8;
        let comp_bit = MAP_COMPLETE_BITS[ob_row.min(7)];
        rom.write_byte(FX_MAP_COMP_IDX + slot * 2, comp_col);
        rom.write_byte(FX_MAP_COMP_IDX + slot * 2 + 1, comp_bit);

        // Pattern bytes.
        let pat_off = FX_PATTERNS + slot * 4;
        for (j, &b) in patterns.iter().enumerate() {
            rom.write_byte(pat_off + j, b);
        }
    }
}

pub(super) fn patch_fortress_fx_screen_check(rom: &mut Rom) {
    // --- Hook at $C8E6 ---
    const HOOK_OFFSET: usize = 0x148F6; // file offset of CPU $C8E6
    rom.write_byte(HOOK_OFFSET, 0x4C); // JMP
    rom.write_byte(HOOK_OFFSET + 1, 0x44); // lo($D544)
    rom.write_byte(HOOK_OFFSET + 2, 0xD5); // hi($D544)

    // --- Custom code at $D544 (file 0x15554) ---
    //
    // **Algorithm: compare lock's half-screen index to Mario's half-
    // screen index, not the scroll's screen index.** Cross-checked
    // against fcoughlin's SMB3 Randomizer (Fred): 21 Fred-generated
    // ROMs in /fred all carry these exact 80 bytes. Three in-house
    // attempts (beta.6/7/8) compared lock_screen to `$12` (scroll
    // page) and missed cases like same-screen-while-straddling and
    // mid-scroll transitions. Fred's insight is that **Mario's
    // position** (`$77` = map obj X hi, `$79` = map obj X lo, per
    // qol.rs:410) is the right reference — it's the *settled*
    // viewport target, not the in-flight scroll.
    //
    // Half-screen indexing (0..7) packs both screen number and
    // left/right half into one byte:
    //   lock_index   = 2 * lock_screen + (col >= 8 ? 1 : 0)    [→ $0A]
    //   mario_index  = 2 * $77 + (bit 7 of $79)                [computed inline]
    //
    // Same half-screen → animate. The PHA/PLA dance lets the patch
    // re-check after adjusting `$0A` by ±1 to cover the adjacent
    // half-screen that becomes visible during straddle. Whether to
    // adjust +1 or -1 depends on whether Mario is on the same side
    // as the scroll (`$79 EOR $FD` bit 7).
    //
    // The `(col<<4) EOR $FD` range check at +24..+32 filters out
    // cols 0 and 15 at certain scroll positions — those are edge
    // tiles where the lock-break animation would clip across screen
    // boundaries even when nominally "visible."
    //
    // One further gate suppresses the VRAM tile write (issue #131).
    // Vanilla queues that write into `Graphics_Buffer` at the *top* of
    // `MO_DoFortressFX` ($C8EA..$C94F), before it checks anything — so the
    // gate has to be decided here, ahead of the jump.
    //
    // A second gate used to sit alongside it, skipping the write when the
    // lock was already busted (`Map_Completions[col] & bit`, mirroring
    // vanilla's own test at $C964). It was measured redundant and removed:
    // with only the busted gate disabled the dark page still behaved
    // correctly, while disabling the darkness gate alone reproduced the
    // reveal. Vanilla reaches the same outcome with no gate at all — its
    // routine is correct on the dark page even once the hammer can break
    // locks — so the darkness gate compensates for something this
    // replacement loses between the $C8E6 hook and the buffer commit,
    // rather than adding behaviour vanilla lacks. Worth understanding
    // before this patch grows again. (Removing the busted gate saved 15
    // bytes: 97 → 82.)
    //
    // **Darkness active → poof but no write, then replay the reveal.**
    // `Map_W8DarknessFill` blacks out the nametable with tile $FF and
    // leaves map RAM alone; Mario carries a 3x3 metatile box of light that
    // `Map_W8DarknessUpdate` repaints from map RAM as he walks. A tile
    // written here would therefore stand lit against the black and stay
    // that way. The poof-only exit ($20=1 → JMP $C952) plays the sprites
    // at the lock's spot and updates map RAM + `Map_Completions`.
    //
    // But skipping the write alone leaves a *stale lock* whenever the lock
    // sits inside that box — the common case, since a lock is often right
    // beside the fortress that opens it. So the same path also resets
    // `World_8_Dark` to 1, which is what the map-load code sets on arrival
    // (PRG030 `INY; STY $0598`): `FX_World_8_Darkness` replays the arrival
    // reveal, repainting the light box from the now-updated map RAM.
    //
    // The replay needs no geometry: it fixes a tile *inside* the light
    // whose graphic is now stale. Mario is always standing still on the
    // fortress tile here — the map has just loaded and input is locked
    // through the effect — so the lit region is exactly the arrival box the
    // replay redraws.
    //
    // Reset timing takes care of itself: `FX_World_8_Darkness` is reached
    // only via `MapObjects_UpdateDrawEnter`, which is not on the
    // `MO_DoFortressFX` path (that ends at `WorldMap_UpdateAndDraw`, which
    // just draws the player sprite), so the replay runs once the effect
    // finishes — by which point map RAM has held the replacement tile since
    // the effect's first frame.
    //
    // `World_8_Dark` ($0598) is the engine's own darkness flag — the same
    // one `Map_W8DarknessFill` and `FX_World_8_Darkness` gate on — set on
    // map load from `World_Num == 7 && World_Map_XHi[player] == 2`. It is
    // preferred over deriving `world == 7 && lock_screen == 2` inline
    // because it is a real source of truth rather than a restatement of
    // one, and it needs no per-slot flag in the FX tables. (A prior
    // attempt at this feature, d608bf1, smuggled a poof-only flag into
    // bit 0 of `FX_MAP_LOC_ROW`; the engine ORs that nibble into the
    // map-data write column at $C99B, which corrupted the destination
    // tile and softlocked the lock — reverted in 13ac21e. No per-slot FX
    // table has a free bit; this reads engine state instead.) Note the
    // flag tracks *Mario's* page, not the lock's: it is set while the
    // player stands on the dark page, which is when darkness is on screen.
    //
    // **What the patch reads:**
    //   $0745    — resolved FX slot (engine stored it at $C8E3)
    //   $C7DF,Y  — FortressFX_MapCompIdx[slot*2] = (column, row bit)
    //   $7D00,X  — Map_Completions (Mario's copy, as vanilla's own check)
    //   $C856,Y  — FortressFX_MapLocation[slot] = (col<<4)|screen
    //   $77, $79 — Mario's map_obj X hi/lo (settled position)
    //   $FD      — Map_Scroll_X
    //   $0598    — World_8_Dark
    //   $0A      — temporary in zero page
    //
    // Exit:
    //   visible          → JMP $C8EA ($20=1, full animate)
    //   visible + dark   → JMP $C952 ($20=1, poof + map data, no VRAM,
    //                      World_8_Dark=1 to replay the reveal)
    //   invisible/busted → JMP $C952 ($20=6, data-only update)
    //
    // 102 bytes; fits the FS_FX_SCREEN_CHECK allocation in rom_data.rs.
    // debug_assert! locks the size.
    const CODE_OFFSET: usize = rom_data::FS_FX_SCREEN_CHECK;
    #[rustfmt::skip]
    let code: &[u8] = &[
        // ----- Edge-tile filter (skip cols 0/15 at certain scrolls) -----
        // Same as Fred's: (col<<4) EOR $FD, must be in [$10, $E8). Saves the
        // lock-break animation from clipping across screen boundaries on
        // edge tiles. The loc byte is stashed in X rather than re-read from
        // $C856,Y for the half-index below (TAX+TXA = 2 bytes vs a 3-byte
        // re-load).
        0xAC, 0x45, 0x07,    //  0: LDY $0745         ; Y = real FX slot
        0xB9, 0x56, 0xC8,    //  3: LDA $C856,Y       ; loc = (col<<4)|screen
        0xAA,                //  6: TAX               ; stash loc
        0x29, 0xF0,          //  7: AND #$F0          ; A = col<<4
        0x45, 0xFD,          //  9: EOR $FD           ; A ^= Map_Scroll_X
        0xC9, 0x10,          // 11: CMP #$10
        0x90, 0x39,          // 13: BCC +57 → skip
        0xC9, 0xE8,          // 15: CMP #$E8
        0xB0, 0x35,          // 17: BCS +53 → skip

        // ----- $0A = lock_half_index = 2*screen + (col>=8 ? 1 : 0) -----
        //
        // Fred's version of this block runs `LDA / ASL / LDA / AND #$03 /
        // ADC $C856,Y / AND #$0F` (16 bytes after the LDY) to compute the
        // same value via a more elaborate path. The shortcut here uses the
        // fact that for valid inputs (screen 0..3, col 0..15) the bits we
        // want are already present after a single ASL on the loc byte —
        // (loc<<1)&$06 is exactly `2*(screen&3)`, and the carry that ASL
        // dropped from bit 7 of loc is exactly `col>=8`. `ADC #$00` folds
        // them. Saves 6 bytes vs Fred. Equivalent for all in-use loc
        // values (verified by exhaustive enumeration of the 17 vanilla
        // slots and chr_stats's randomized layouts).
        0x8A,                // 19: TXA               ; loc back
        0x0A,                // 20: ASL A             ; A=(loc<<1)&$FF; C = col>=8
        0x29, 0x06,          // 21: AND #$06          ; A = (screen<<1)&$06 = 2*(screen&3)
        0x69, 0x00,          // 23: ADC #$00          ; A += C  → 2*screen + (col>=8)
        0x85, 0x0A,          // 25: STA $0A           ; lock_half_index (0..7)

        // ----- mario_half_index = 2*$77 + bit7($79) ; first compare -----
        // Cached in $0B (Temp_Var12) rather than on the stack: every exit
        // would otherwise need its own PLA discard, which is both bigger and
        // the reason "skip" used to have two entry points.
        0xA5, 0x79,          // 27: LDA $79           ; Mario X lo
        0x0A,                // 29: ASL A             ; C = bit 7 of $79
        0xA5, 0x77,          // 30: LDA $77           ; Mario X hi
        0x65, 0x77,          // 32: ADC $77           ; A = 2*$77 + C  (= mario_half_index)
        0x85, 0x0B,          // 34: STA $0B
        0xC5, 0x0A,          // 36: CMP $0A
        0xF0, 0x12,          // 38: BEQ +18 → animate ; same half-screen → visible

        // ----- adjacency: adjust $0A by ±1 per scroll/mario alignment -----
        // BMI path (B): $79 and $FD differ on bit 7 → INC $0A (+1)
        // BPL path (A): they agree → DEC twice + INC (net -1)
        0xA5, 0x79,          // 40: LDA $79
        0x45, 0xFD,          // 42: EOR $FD
        0x30, 0x04,          // 44: BMI +4 → path B
        0xC6, 0x0A,          // 46: DEC $0A           ; path A start
        0xC6, 0x0A,          // 48: DEC $0A
        0xE6, 0x0A,          // 50: INC $0A           ; path B target (fall-through for A)
        0xA5, 0x0B,          // 52: LDA $0B           ; mario_index
        0xC5, 0x0A,          // 54: CMP $0A
        0xD0, 0x0E,          // 56: BNE +14 → skip    ; neither half-screen → invisible

        // ----- animate: full FX, $20 = 1 -----
        0xA9, 0x01,          // 58: LDA #$01
        0x85, 0x20,          // 60: STA $20
        // Darkness active → poof + map data only, and restart the reveal so
        // a lock inside Mario's light gets repainted from the updated map
        // RAM. LDX (not LDA) so A stays #$01 for the STA below.
        0xAE, 0x98, 0x05,    // 62: LDX $0598         ; World_8_Dark
        0xF0, 0x0C,          // 65: BEQ +12 → full animate
        0x8D, 0x98, 0x05,    // 67: STA $0598         ; A=1 → replay arrival reveal
        0xD0, 0x04,          // 70: BNE +4 → common   ; A=1, always taken

        // ----- skip: data-only update, $20 = 6 -----
        0xA9, 0x06,          // 72: LDA #$06
        0x85, 0x20,          // 74: STA $20
        0x4C, 0x52, 0xC9,    // 76: JMP $C952         ; common
        0x4C, 0xEA, 0xC8,    // 79: JMP $C8EA         ; full animate
    ];
    debug_assert!(
        code.len() == 82,
        "FX screen-check patch must be 82 bytes (allocation is 112, 30 reserved free)"
    );
    for (i, &b) in code.iter().enumerate() {
        rom.write_byte(CODE_OFFSET + i, b);
    }
}

/// Stage 0 of the fortress-FX rework (`docs/fx_table_redesign.md`): prove in
/// Rust that a slot's stored data is redundant with its target cell.
///
/// The redesign's premise is that 153 of the 187 bytes of per-slot FX data are
/// cached arithmetic over `(world, row, col)` plus the engine's own
/// removable-tile mapping. That is a claim until something checks it, so this
/// module decodes each slot's target cell from its *own* position bytes and
/// re-derives every other byte from it, on vanilla and on randomized output.
///
/// It writes nothing. The value is the exception list: every slot that does
/// *not* derive is pinned below with the reason, so stage 1 knows exactly what
/// it changes rather than discovering it in a playtest.
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
    /// `Map_Reload_with_Completions` on every map load.
    const MAP_REMOVABLE_TILES: usize = PRG012_FILE_BASE + 0x437;
    const MAP_REMOVE_TO_TILES: usize = PRG012_FILE_BASE + 0x43F;
    const REMOVABLE_COUNT: usize = 8;

    /// The metatile quadrant tables are stored UL, LL, UR, LR — four 256-byte
    /// planes from [`PRG012_FILE_BASE`]. `FortressFX_Patterns` stores the same
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
    /// `FortressFX_MapLocationRow`/`MapLocation` — the key the redesign keeps.
    /// Which world it lives in is *not* recorded anywhere in the slot tables.
    fn target_cell(rom: &Rom, slot: usize) -> (usize, usize) {
        let loc_row = rom.read_byte(rom_data::FX_MAP_LOC_ROW + slot);
        let loc = rom.read_byte(rom_data::FX_MAP_LOC + slot);
        let row = (loc_row >> 4) as usize - 2;
        let col = (loc & 0x0F) as usize * 16 + (loc >> 4) as usize;
        (row, col)
    }

    /// Every stored byte of `slot` that does not equal the value derived from
    /// the slot's own target cell, as `(field, detail)` pairs.
    ///
    /// `world_idx` is needed only to read the map tile under the lock.
    fn mismatches(rom: &Rom, slot: usize, world_idx: usize) -> Vec<(&'static str, String)> {
        let loc_row = rom.read_byte(rom_data::FX_MAP_LOC_ROW + slot);
        let (row, col) = target_cell(rom, slot);
        let col_in_screen = col % 16;

        let mut out = Vec::new();

        // The key itself: the engine ORs this low nibble into the map-data
        // write column at $C99B, so it must be zero (see the writer above).
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

    /// Report [`mismatches`] for a run of slots, tagged with the slot number.
    fn scan(rom: &Rom, slots_by_world: &[(usize, usize)]) -> Vec<String> {
        let mut found = Vec::new();
        for &(slot, world_idx) in slots_by_world {
            for (field, detail) in mismatches(rom, slot, world_idx) {
                found.push(format!("slot {slot:#04X} ({field}): {detail}"));
            }
        }
        found
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

    /// Build `seeds` randomized ROMs and hand each to `visit`, along with the
    /// slots it wrote and the world each belongs to.
    ///
    /// Slot-to-world comes from the build rather than from `FortressFX_W1..W8`
    /// because that table *cannot* express it: `0x00` means both "slot 0" and
    /// "unused", so World 1's row reads `[0,0,0,0]` whether it holds one lock
    /// or none. Removing that ambiguity is one of the redesign's motivations.
    /// The writer hands out one running slot index across worlds in world
    /// order, which `test_fx_slots_valid` pins.
    fn for_each_randomized_rom(
        rom: &Rom,
        seeds: u64,
        mut visit: impl FnMut(&Rom, &[(usize, usize)]),
    ) {
        use crate::randomize::overworld_writer::{WriteFlags, write_overworld};
        use crate::randomize::{node_catalog, overworld_build, overworld_pickup};
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;

        for seed in 0..seeds {
            let catalog = node_catalog::NodeCatalog::build(rom, false);
            let pickup = overworld_pickup::pick_up(
                rom,
                &catalog,
                overworld_pickup::PickupFlags {
                    shuffle_spade_games: true,
                    shuffle_toad_houses: true,
                    ..Default::default()
                },
            );
            let data = overworld_build::OverworldData { pickup: &pickup, catalog: &catalog };
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let build = overworld_build::build(
                rom,
                &data,
                &mut rng,
                overworld_build::BuildFlags { shuffle_toad_houses: true, ..Default::default() },
            );

            let mut out = rom.clone();
            let _ = write_overworld(&mut out, &build, &data, &mut rng, WriteFlags::default());

            let mut slots_by_world = Vec::new();
            for (wi, world) in build.worlds.iter().enumerate() {
                for _ in 0..world.locks.len() {
                    slots_by_world.push((slots_by_world.len(), wi));
                }
            }
            visit(&out, &slots_by_world);
        }
    }

    /// Seeds for the randomized pass; `CENSUS_SEEDS` raises it.
    fn census_seeds() -> u64 {
        std::env::var("CENSUS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(8)
    }

    /// `path_for_gap_tile` is a hand-written copy of the engine's own
    /// `Map_Removable_Tiles` -> `Map_RemoveTo_Tiles` pairing. If the two ever
    /// disagreed, the builder would restore a different tile than the map
    /// reload does, so check the copy against the source.
    #[test]
    fn rust_gap_tile_mapping_agrees_with_the_engines_removable_tables() {
        let Some(rom) = load_rom() else { return };
        for gap in [0x54u8, 0x56, 0xE4, rom_data::WATER_GAP_TILE] {
            assert_eq!(
                rom_data::path_for_gap_tile(gap),
                remove_to(&rom, gap),
                "gap tile {gap:#04X}: path_for_gap_tile disagrees with Map_RemoveTo_Tiles"
            );
        }
    }

    /// Vanilla derives cleanly except in two places, both pinned here so a
    /// change to either is a test failure rather than a surprise.
    ///
    /// * **Slot `$0F` patterns.** W8's dark screen. Vanilla stores
    ///   `FF FF FF FF` — the disassembly's own comment says "Makes an all
    ///   black square in the dark" — instead of the quadrants of the tile it
    ///   writes into map RAM (`$46`). This is the one slot whose patterns
    ///   carry information the target cell does not. Our writer does not
    ///   reproduce it: `fx_patterns_for` has no darkness case, so a randomized
    ///   W8 lock on the dark screen already draws the ordinary path tile and
    ///   relies on the `World_8_Dark` gate in
    ///   [`patch_fortress_fx_screen_check`] instead.
    /// * **Slot `$08` replacement tile.** The removable table pairs each
    ///   obstacle with the path its terrain wants — `$56 -> $45` on the
    ///   ground, `$E4 -> $DA` in the sky — so a cloud lock reveals a cloud
    ///   path. W6 `(row 4, col 13)` is plain ground: it holds `$56`, its
    ///   corridor is `$42/$45/$47`, and `$DA` appears nowhere on that screen.
    ///   The slot nevertheless stores `$DA`, the sky answer, which is what the
    ///   *preceding* slot `$07` legitimately stores for the game's one real
    ///   sky lock (W5, cell `$E4`, cloud neighbours) — the value looks carried
    ///   down a row when the table was authored. It has stayed invisible
    ///   because `$45` and `$DA` share CHR quadrants and the effect writes no
    ///   attribute byte, so the frame is correct either way, and the reload
    ///   maps `$56` back to `$45` regardless. Deriving fixes it.
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

        let found = scan(&rom, &slots);
        assert_eq!(
            found, VANILLA_EXCEPTIONS,
            "vanilla FX slots derive from their target cell except for the pinned exceptions"
        );
    }

    /// The 68 bytes of `VAddrH/L` and `MapCompIdx` are pure arithmetic over the
    /// target cell, in vanilla and in every randomized ROM. Nothing the builder
    /// does perturbs them, so stage 1 can delete both tables outright.
    #[test]
    fn fx_position_bytes_are_always_derived_from_the_target_cell() {
        let Some(rom) = load_rom() else { return };
        const POSITION_FIELDS: [&str; 3] = ["map_loc_row", "vaddr", "map_comp_idx"];

        let mut offenders = Vec::new();
        let mut check = |rom: &Rom, slots: &[(usize, usize)], label: &str| {
            for &(slot, wi) in slots {
                for (field, detail) in mismatches(rom, slot, wi) {
                    if POSITION_FIELDS.contains(&field) {
                        offenders.push(format!("{label} slot {slot:#04X} ({field}): {detail}"));
                    }
                }
            }
        };
        check(&rom, &vanilla_slots_by_world(), "vanilla");
        for_each_randomized_rom(&rom, census_seeds(), |out, slots| check(out, slots, "randomized"));

        assert!(offenders.is_empty(), "position-derived FX bytes diverged: {offenders:#?}");
    }

    /// Randomized output does **not** derive its replacement tile, and stage 1
    /// has to decide what to do about it. This pins exactly how it diverges.
    ///
    /// The builder keeps the *original* path tile under a lock — a drawbridge,
    /// a sky path, a path variant — while `Map_Removable_Tiles` knows only the
    /// plain path of each orientation (`$54 -> $46`, `$56 -> $45`). So the
    /// stored replacement is a variant that the engine's table cannot name.
    ///
    /// The patterns give the game away: they are the *plain* tile's quadrants,
    /// not the stored variant's, because `fx_patterns_for` is a hand-written
    /// copy of exactly three plain tiles. Today's ROM therefore shows three
    /// different tiles at one cell over time — the plain graphic during the
    /// effect, the variant once map RAM is redrawn, and the plain tile again
    /// after the next map load turns `$54` back into `$46`. Deriving the write
    /// from `Map_RemoveTo_Tiles`, as the redesign does, collapses all three to
    /// the plain tile and loses the variant; extending the removable tables is
    /// the alternative (stage 3).
    #[test]
    fn randomized_fx_tile_and_patterns_diverge_only_by_path_variant() {
        let Some(rom) = load_rom() else { return };
        let seeds = census_seeds();

        let mut census: std::collections::BTreeMap<(u8, u8), usize> = Default::default();
        let (mut slots_seen, mut derived) = (0usize, 0usize);

        for_each_randomized_rom(&rom, seeds, |out, slots| {
            for &(slot, wi) in slots {
                slots_seen += 1;
                let (row, col) = target_cell(out, slot);
                let cell = out.read_byte(rom_data::map_tile_offset(wi, row, col));
                let stored = out.read_byte(rom_data::FX_MAP_TILE_REPLACE + slot);
                let plain = remove_to(out, cell).unwrap_or_else(|| {
                    panic!("slot {slot:#04X}: cell tile {cell:#04X} is not removable")
                });
                let patterns: [u8; 4] =
                    core::array::from_fn(|j| out.read_byte(FX_PATTERNS + slot * 4 + j));

                // Whatever the stored tile is, the patterns drawn are the
                // plain tile's. That is the fact that makes the 68-byte
                // pattern table deletable.
                assert_eq!(
                    patterns,
                    metatile_patterns(out, plain),
                    "slot {slot:#04X}: patterns must be the quadrants of {plain:#04X}"
                );

                if stored == plain {
                    derived += 1;
                    continue;
                }
                // The one licensed divergence: the stored tile is the path
                // variant this lock was placed over.
                assert_eq!(
                    rom_data::gap_tile_for(stored),
                    cell,
                    "slot {slot:#04X}: stored {stored:#04X} is not the path under gap {cell:#04X}"
                );
                *census.entry((cell, stored)).or_default() += 1;
            }
        });

        assert!(slots_seen > 0, "no FX slots were written across {seeds} seeds");
        let variants: usize = census.values().sum();
        println!(
            "fx tile derivation over {seeds} seeds: {slots_seen} slots, \
             {derived} derive from Map_RemoveTo_Tiles, {variants} keep a path variant"
        );
        for ((cell, stored), n) in &census {
            println!("  gap {cell:#04X} -> stored {stored:#04X}  x{n}");
        }
    }
}
