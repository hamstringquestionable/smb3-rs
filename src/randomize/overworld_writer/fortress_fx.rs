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
    // Fortress assignments are ordered by section in assign_pool, so
    // assignment index == fort_section for this world.
    let locked_forts: Vec<_> = built
        .locks
        .iter()
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
    debug_assert!(code.len() == 82, "FX screen-check patch must be 82 bytes (allocation is 112, 30 reserved free)");
    for (i, &b) in code.iter().enumerate() {
        rom.write_byte(CODE_OFFSET + i, b);
    }
}
