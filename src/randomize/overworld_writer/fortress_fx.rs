//! Step 4 — write fortress FX tables and the screen-check patch.

use super::*;

pub(super) fn write_fortress_fx(
    rom: &mut Rom,
    world_idx: usize,
    built: &BuiltWorld,
    wa: &WorldAssignments,
    data: &OverworldData,
    fx_slot: &mut usize,
) {
    let pickup = data.pickup;
    let catalog = data.catalog;
    // Pair each lock with its fortress assignment (matched by section).
    let locked_forts: Vec<_> = built
        .locks
        .iter()
        .filter_map(|lock| {
            wa.fortress.iter().enumerate().find(|(fi, _)| {
                // Fortress assignments are ordered by section in assign_pool.
                // Assignment index fi == fort_section for this world.
                *fi == lock.fort_section
            }).map(|(_, fa)| (lock, fa))
        })
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
        rom.write_byte(
            rom_data::FX_MAP_LOC + slot,
            ((col_in_screen as u8) << 4) | (screen as u8),
        );

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
    rom.write_byte(HOOK_OFFSET, 0x4C);     // JMP
    rom.write_byte(HOOK_OFFSET + 1, 0x44); // lo($D544)
    rom.write_byte(HOOK_OFFSET + 2, 0xD5); // hi($D544)

    // --- Custom code at $D544 (file 0x15554), 80 bytes ---
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
    // **What the patch reads:**
    //   $0745    — resolved FX slot (engine stored it at $C8E3)
    //   $C856,Y  — FortressFX_MapLocation[slot] = (col<<4)|screen
    //   $77, $79 — Mario's map_obj X hi/lo (settled position)
    //   $FD      — Map_Scroll_X
    //   $0A      — temporary in zero page
    //
    // Exit:
    //   visible   → JMP $C8EA ($20=1, full animate)
    //   invisible → JMP $C952 ($20=6, data-only update)
    //
    // 80 bytes; matches the FS_FX_SCREEN_CHECK allocation in
    // rom_data.rs. debug_assert! locks the size.
    const CODE_OFFSET: usize = rom_data::FS_FX_SCREEN_CHECK;
    #[rustfmt::skip]
    let code: &[u8] = &[
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
        0xAC, 0x45, 0x07,    //  0: LDY $0745         ; Y = real FX slot
        0xB9, 0x56, 0xC8,    //  3: LDA $C856,Y       ; loc byte
        0x0A,                //  6: ASL A             ; A=(loc<<1)&$FF; C = col>=8
        0x29, 0x06,          //  7: AND #$06          ; A = (screen<<1)&$06 = 2*(screen&3)
        0x69, 0x00,          //  9: ADC #$00          ; A += C  → 2*screen + (col>=8)
        0x85, 0x0A,          // 11: STA $0A           ; lock_half_index (0..7)

        // ----- Edge-tile filter (skip cols 0/15 at certain scrolls) -----
        // Same as Fred's: (col<<4) EOR $FD, must be in [$10, $E8).
        // Saves the lock-break animation from clipping across screen
        // boundaries on edge tiles.
        0xB9, 0x56, 0xC8,    // 13: LDA $C856,Y       ; reload loc
        0x29, 0xF0,          // 16: AND #$F0          ; A = col<<4
        0x45, 0xFD,          // 18: EOR $FD           ; A ^= Map_Scroll_X
        0xC9, 0x10,          // 20: CMP #$10
        0x90, 0x23,          // 22: BCC +35 → skip
        0xC9, 0xE8,          // 24: CMP #$E8
        0xB0, 0x1F,          // 26: BCS +31 → skip

        // ----- mario_half_index = 2*$77 + bit7($79) ; first compare -----
        0xA5, 0x79,          // 28: LDA $79           ; Mario X lo
        0x0A,                // 30: ASL A             ; C = bit 7 of $79
        0xA5, 0x77,          // 31: LDA $77           ; Mario X hi
        0x65, 0x77,          // 33: ADC $77           ; A = 2*$77 + C  (= mario_half_index)
        0x48,                // 35: PHA               ; stash mario_index
        0xC5, 0x0A,          // 36: CMP $0A
        0xF0, 0x1A,          // 38: BEQ +26 → animate ; same half-screen → visible

        // ----- adjacency: adjust $0A by ±1 per scroll/mario alignment -----
        // BMI path (B): $79 and $FD differ on bit 7 → INC $0A (+1)
        // BPL path (A): they agree → DEC twice + INC (net -1)
        0xA5, 0x79,          // 40: LDA $79
        0x45, 0xFD,          // 42: EOR $FD
        0x30, 0x04,          // 44: BMI +4 → path B
        0xC6, 0x0A,          // 46: DEC $0A           ; path A start
        0xC6, 0x0A,          // 48: DEC $0A
        0xE6, 0x0A,          // 50: INC $0A           ; path B target (fall-through for A)
        0x68,                // 52: PLA               ; peek mario_index
        0x48,                // 53: PHA               ; re-push
        0xC5, 0x0A,          // 54: CMP $0A
        0xF0, 0x08,          // 56: BEQ +8 → animate

        // ----- skip: data-only update, $20 = 6 -----
        0x68,                // 58: PLA               ; discard stashed mario_index
        0xA9, 0x06,          // 59: LDA #$06
        0x85, 0x20,          // 61: STA $20
        0x4C, 0x52, 0xC9,    // 63: JMP $C952

        // ----- animate: full FX, $20 = 1 -----
        0x68,                // 66: PLA               ; discard stashed mario_index
        0xA9, 0x01,          // 67: LDA #$01
        0x85, 0x20,          // 69: STA $20
        0x4C, 0xEA, 0xC8,    // 71: JMP $C8EA
    ];
    debug_assert!(code.len() == 74, "FX screen-check patch must be 74 bytes (allocation is 80, 6 reserved free)");
    for (i, &b) in code.iter().enumerate() {
        rom.write_byte(CODE_OFFSET + i, b);
    }
}
