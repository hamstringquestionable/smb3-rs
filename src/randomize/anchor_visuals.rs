use crate::rom::Rom;

// Global Item ID for the Anchor — the visual we redirect every other item to.
// Derived patch values: `ANCHOR * 2` indexes the 14×2-byte hilite tile table,
// `ANCHOR * 4` indexes the 14×4-byte inventory tile table.
const ANCHOR: u8 = 0x0A;

// --- World-map inventory grid (PRG026 / Inventory_DrawItemsOrCards) ---
//
// Vanilla draws each non-empty slot's tile pattern from InvItem_Tile_Layout:
//   LDA $7D80,Y    ; load item ID
//   BEQ skip       ; empty slot? skip
//   ASL A          ; item * 2
//   ASL A          ; item * 4 (one row = 4 bytes)
//   TAY
//
// We keep LDA+BEQ so empty slots still skip, then replace ASL/ASL/TAY
// (`0A 0A A8`) with `LDY #<ANCHOR*4>; NOP`.
const INV_DRAW_ITEM_INDEX_OFFSET: usize = 0x3437D;
const INV_DRAW_ITEM_INDEX_PATCH: [u8; 3] = [0xA0, ANCHOR * 4, 0xEA];

// --- World-map inventory hilite (PRG026 / Inv_Display_Hilite) ---
//
// The cursor-highlighted slot draws from a separate 14×2-byte table
// `InvItem_Hilite_Layout`. The index is computed via `TXA; ASL A; TAX`
// (after `LDX Inventory_Items,Y`). Replacing `8A 0A AA` with
// `LDX #<ANCHOR*2>; NOP` forces X to the Anchor row.
const INV_HILITE_INDEX_OFFSET: usize = 0x348A9;
const INV_HILITE_INDEX_PATCH: [u8; 3] = [0xA2, ANCHOR * 2, 0xEA];

// --- World-map inventory hilite palette (PRG026 / InvItem_SetColor) ---
//
// `Inventory_DoHilites` re-uploads the BG palette slot of the highlighted
// inventory tiles via `InvItem_SetColor`, which reads `InvItem_Pal,X`. The
// vanilla per-item table colors each item distinctly, leaking the real item
// through palette. Replacing `LDA InvItem_Pal,X` (`BD 14 A5`) with
// `LDA #<pal>; NOP` forces the hilite color to the Anchor's palette entry.
const INV_HILITE_PAL_OFFSET: usize = 0x3453A;
// `InvItem_Pal[ANCHOR]` = $07 (per the table at file 0x34524).
const ANCHOR_HILITE_PAL: u8 = 0x07;
const INV_HILITE_PAL_PATCH: [u8; 3] = [0xA9, ANCHOR_HILITE_PAL, 0xEA];

// --- Toad House interior item reveal (PRG002 / ObjNorm_ToadHouseItem) ---
//
// Toad House chests use OBJ $35 (`OBJ_TOADHOUSEITEM`), a different handler
// from in-level treasure boxes. `ObjNorm_ToadHouseItem` reads
// `Objects_Frame,X` (the actual item ID) three times:
//
//   - file 0x05507 (`BC 69 06` = LDY Objects_Frame,X): drives the BG palette
//     via `ToadItem_PalPerItem,Y`.
//   - file 0x0556A (`BD 69 06` = LDA Objects_Frame,X): stores the item into
//     the player's inventory. **Not patched** — preserves the real reward.
//   - file 0x0558A (`BD 69 06` = LDA Objects_Frame,X): seeds the sprite tile
//     patterns + attribute via `ToadItem_PatternLeft-1,X`.
const TOAD_HOUSE_PAL_OFFSET: usize = 0x05507;
const TOAD_HOUSE_PAL_PATCH: [u8; 3] = [0xA0, ANCHOR, 0xEA];
const TOAD_HOUSE_TILE_OFFSET: usize = 0x0558A;
const TOAD_HOUSE_TILE_PATCH: [u8; 3] = [0xA9, ANCHOR, 0xEA];

// --- In-level treasure box reveal (PRG003 / ObjInit & ObjNorm_TreasureBox) ---
//
// Both handlers do `LDA Level_TreasureItem` (`AD 63 79`) to drive visuals:
//   - $A297 (file 0x62A7): seeds palette via ToadItem_PalPerItem,Y + Var5
//   - $A33A (file 0x634A): sets Objects_Frame,X (sprite frame) + indexes
//     TBoxItem_MirrorFlags
//
// A third read at $A321 (file 0x6331) drives Player_GetItem — left alone so
// the player still receives the actual item.
const TBOX_INIT_PALETTE_OFFSET: usize = 0x62A7;
const TBOX_NORM_FRAME_OFFSET: usize = 0x634A;
const TBOX_LDA_ANCHOR_PATCH: [u8; 3] = [0xA9, ANCHOR, 0xEA];

pub fn apply(rom: &mut Rom) {
    rom.write_range(INV_DRAW_ITEM_INDEX_OFFSET, &INV_DRAW_ITEM_INDEX_PATCH);
    rom.write_range(INV_HILITE_INDEX_OFFSET, &INV_HILITE_INDEX_PATCH);
    rom.write_range(INV_HILITE_PAL_OFFSET, &INV_HILITE_PAL_PATCH);
    rom.write_range(TBOX_INIT_PALETTE_OFFSET, &TBOX_LDA_ANCHOR_PATCH);
    rom.write_range(TBOX_NORM_FRAME_OFFSET, &TBOX_LDA_ANCHOR_PATCH);
    rom.write_range(TOAD_HOUSE_PAL_OFFSET, &TOAD_HOUSE_PAL_PATCH);
    rom.write_range(TOAD_HOUSE_TILE_OFFSET, &TOAD_HOUSE_TILE_PATCH);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::Rom;

    // Witness offsets — the production code intentionally does NOT touch
    // these, so the test seeds them with their vanilla bytes and asserts
    // they remain unchanged after `apply()`. If a patch ever drifts into
    // these instructions, the player's real reward delivery would break.
    const TBOX_GIVE_ITEM_OFFSET: usize = 0x6331;
    const TOAD_HOUSE_STORE_OFFSET: usize = 0x0556A;
    const VANILLA_TBOX_LDA: [u8; 3] = [0xAD, 0x63, 0x79];
    const VANILLA_TOAD_LDA: [u8; 3] = [0xBD, 0x69, 0x06];

    // The 7 bytes preceding the inventory-grid patch site — must survive
    // the patch so empty inventory slots still take the `BEQ` skip.
    const INV_DRAW_PROLOGUE_OFFSET: usize = 0x34376;
    const VANILLA_INV_DRAW_PROLOGUE: [u8; 7] = [0xA4, 0x0D, 0xB9, 0x80, 0x7D, 0xF0, 0x1E];

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        data[INV_DRAW_PROLOGUE_OFFSET..INV_DRAW_PROLOGUE_OFFSET + 7].copy_from_slice(&VANILLA_INV_DRAW_PROLOGUE);
        data[TBOX_GIVE_ITEM_OFFSET..TBOX_GIVE_ITEM_OFFSET + 3].copy_from_slice(&VANILLA_TBOX_LDA);
        data[TOAD_HOUSE_STORE_OFFSET..TOAD_HOUSE_STORE_OFFSET + 3].copy_from_slice(&VANILLA_TOAD_LDA);
        Rom::from_bytes_lax(&data, true).unwrap()
    }

    #[test]
    fn apply_patches_inventory_and_chest_visuals_only() {
        let mut rom = make_test_rom();
        apply(&mut rom);

        assert_eq!(rom.read_range(INV_DRAW_ITEM_INDEX_OFFSET, 3), &INV_DRAW_ITEM_INDEX_PATCH);
        assert_eq!(rom.read_range(INV_HILITE_INDEX_OFFSET, 3), &INV_HILITE_INDEX_PATCH);
        assert_eq!(rom.read_range(INV_HILITE_PAL_OFFSET, 3), &INV_HILITE_PAL_PATCH);
        assert_eq!(rom.read_range(TBOX_INIT_PALETTE_OFFSET, 3), &TBOX_LDA_ANCHOR_PATCH);
        assert_eq!(rom.read_range(TBOX_NORM_FRAME_OFFSET, 3), &TBOX_LDA_ANCHOR_PATCH);
        assert_eq!(rom.read_range(TOAD_HOUSE_PAL_OFFSET, 3), &TOAD_HOUSE_PAL_PATCH);
        assert_eq!(rom.read_range(TOAD_HOUSE_TILE_OFFSET, 3), &TOAD_HOUSE_TILE_PATCH);

        // Empty inventory slots must still take the BEQ skip after patching.
        assert_eq!(rom.read_range(INV_DRAW_PROLOGUE_OFFSET, 7), &VANILLA_INV_DRAW_PROLOGUE);

        // Reward-delivery paths must remain vanilla so the player still
        // receives the actual item.
        assert_eq!(rom.read_range(TBOX_GIVE_ITEM_OFFSET, 3), &VANILLA_TBOX_LDA);
        assert_eq!(rom.read_range(TOAD_HOUSE_STORE_OFFSET, 3), &VANILLA_TOAD_LDA);
    }
}
