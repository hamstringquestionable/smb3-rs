//! Canoe "call the boat" rescue (always on).
//!
//! The canoe is a single shared map object. A player can leave it parked on a
//! far island (or, in 2P, sail it away from their partner), stranding whoever
//! needs it — the classic canoe softlock. The [`super::map_warp`] Start+Select
//! escape hatch only covers the 2P warp-to-partner case; this covers the rest,
//! including 1P and both-players-stranded.
//!
//! **What it does:** while standing on a dock (`TILE_DOCK = $4B`) the player
//! presses **A** and the canoe is summoned to the adjacent water tile, ready to
//! board. Boarding itself is unchanged — you still board by walking into the
//! canoe from the dock (`Map_CheckDoMove`, prg010 `$D2E1`); this just guarantees
//! the boat is sitting where you can reach it.
//!
//! **How it works.** Boarding matches the canoe's *active* position
//! (`Map_Object_ActY/ActX/ActXH`, `$0500/$050F/$051E`) against the tile one step
//! off the dock, and both the sprite draw and the on-screen visibility flag are
//! rebuilt every frame from RAM (`MapObjects_UpdateDrawEnter`). Visibility,
//! however, reads the *persistent* copy (`Map_Objects_Y/XLo/XHi`,
//! `$7EEB/$7EF9/$7F07`), which an idle canoe never syncs to the active copy. So
//! the summon writes **both** triplets: persistent so the boat reappears when it
//! was parked off-screen, active so it draws at the new spot and boards. No map
//! reload is needed — moving the six bytes is enough.
//!
//! The navigable-water range `$82..=$A9` is the canoe engine's own bound
//! (`TILE_WATER_INVT $82 <= tile < TILE_VERTPATHWLU $AA` in the in-canoe move
//! check), so any dock's water neighbor lands in it and no path/blank tile can.
//!
//! **Hook.** A pressed on a dock does nothing in vanilla (`$4B` is a page-1 tile,
//! below its `$67` gate threshold, and not a special-entry tile), so the summon
//! is spliced in as a pure side-effect: it replaces the displaced
//! `LDA World_Map_Tile / LDY #$1A` that sets up the special-enter-tile scan at
//! `$CEC5` (reached only when A was just pressed), does the summon when the tile
//! is a dock, then re-establishes `A = tile` / `Y = $1A` and returns so the
//! vanilla scan runs untouched. This composes with the `map_warp` hook at `$CE78`
//! (Start+Select fires there first; a plain A press falls through to here).

use crate::rom::Rom;
use crate::randomize::rom_data::FS_CANOE_SUMMON;

// Hook site: the `LDA World_Map_Tile / LDY #$1A` (bytes A5 E5 A0 1A) at CPU
// $CEC5, the setup for the special-enter-tile scan in MO_NormalMoveEnter.
const SCAN_SETUP_HOOK: usize = 0x14ED5;

// CPU address of FS_CANOE_SUMMON: $C000 + (0x15EB5 - 0x14010) = $DEA5. The
// routine is origin-locked to this address (self-referential JMP + table reads).
const CANOE_SUMMON_CPU: u16 = (0xC000 + FS_CANOE_SUMMON - 0x14010) as u16;

/// The summon routine (PRG010 free space, origin $DEA5). Assembled from
/// `tools/_canoe_summon.a65` with `xa`. On entry it is reached only when A was
/// just pressed; it exits with `A = World_Map_Tile` and `Y = $1A` on every path
/// so the displaced special-enter-tile scan continues exactly as in vanilla.
#[rustfmt::skip]
const CANOE_SUMMON_ROUTINE: [u8; 148] = [
    0xA5, 0xE5,             // LDA World_Map_Tile
    0xC9, 0x4B,             // CMP #$4B         (TILE_DOCK)
    0xD0, 0x0C,             // BNE csexit       (not a dock -> passthrough)
    0xA2, 0x0D,             // LDX #$0D         (scan map-object slots 13..0)
    // csfind:
    0xBD, 0x15, 0x7F,       // LDA Map_Objects_IDs,X ($7F15)
    0xC9, 0x10,             // CMP #$10         (MAPOBJ_CANOE)
    0xF0, 0x08,             // BEQ cshave
    0xCA,                   // DEX
    0x10, 0xF6,             // BPL csfind
    // csexit: (also: no canoe found -> here)
    0xA5, 0xE5,             // LDA World_Map_Tile   (restore A for the scan)
    0xA0, 0x1A,             // LDY #$1A             (restore scan start index)
    0x60,                   // RTS
    // cshave:
    0x86, 0x04,             // STX Temp_Var5    (canoe slot)
    0xA9, 0x00,             // LDA #$00
    0x85, 0x03,             // STA Temp_Var4    (direction = 0)
    // csloop:
    0xAE, 0x26, 0x07,       // LDX Player_Current ($0726)
    0xA4, 0x03,             // LDY Temp_Var4    (direction)
    0xB5, 0x75,             // LDA World_Map_Y,X ($75)
    0x18,                   // CLC
    0x79, 0x2D, 0xDF,       // ADC csyoff,Y     ($DF2D)
    0x85, 0x0E,             // STA Temp_Var15   (neighbor Y)
    0xB5, 0x79,             // LDA World_Map_X,X ($79)
    0x18,                   // CLC
    0x79, 0x31, 0xDF,       // ADC csxoff,Y     ($DF31)
    0x85, 0x0F,             // STA Temp_Var16   (neighbor X)
    0xB5, 0x77,             // LDA World_Map_XHi,X ($77)
    0x79, 0x35, 0xDF,       // ADC csxhioff,Y   ($DF35, +carry from X)
    0x85, 0x05,             // STA Temp_Var6    (neighbor XHi)
    0x0A,                   // ASL             (screen index * 2)
    0xAA,                   // TAX
    0xBD, 0x00, 0x80,       // LDA Tile_Mem_Addr,X ($8000)
    0x85, 0x63,             // STA Map_Tile_AddrL
    0xBD, 0x01, 0x80,       // LDA Tile_Mem_Addr+1,X
    0x85, 0x64,             // STA Map_Tile_AddrH
    0xE6, 0x64,             // INC Map_Tile_AddrH  (screen tiles at base + $100)
    0xA5, 0x0F,             // LDA Temp_Var16   (neighbor X)
    0x4A, 0x4A, 0x4A, 0x4A, // LSR x4           (>> 4 -> column within screen)
    0x85, 0x06,             // STA Temp_Var7
    0xA5, 0x0E,             // LDA Temp_Var15   (neighbor Y)
    0x29, 0xF0,             // AND #$F0
    0x05, 0x06,             // ORA Temp_Var7
    0xA8,                   // TAY
    0xB1, 0x63,             // LDA (Map_Tile_AddrL),Y  (tile at neighbor cell)
    0xC9, 0x82,             // CMP #$82         (canoe water is $82..$A9)
    0x90, 0x21,             // BCC csnext
    0xC9, 0xAA,             // CMP #$AA
    0xB0, 0x1D,             // BCS csnext
    // water found -> move the canoe here (write both position copies):
    0xA6, 0x04,             // LDX Temp_Var5    (canoe slot)
    0xA5, 0x0E,             // LDA Temp_Var15   (Y)
    0x9D, 0xEB, 0x7E,       // STA Map_Objects_Y,X   ($7EEB, persistent -> visibility)
    0x9D, 0x00, 0x05,       // STA Map_Object_ActY,X ($0500, active -> sprite/board)
    0xA5, 0x0F,             // LDA Temp_Var16   (X)
    0x9D, 0xF9, 0x7E,       // STA Map_Objects_XLo,X   ($7EF9)
    0x9D, 0x0F, 0x05,       // STA Map_Object_ActX,X   ($050F)
    0xA5, 0x05,             // LDA Temp_Var6    (XHi)
    0x9D, 0x07, 0x7F,       // STA Map_Objects_XHi,X   ($7F07)
    0x9D, 0x1E, 0x05,       // STA Map_Object_ActXH,X  ($051E)
    0x4C, 0xB7, 0xDE,       // JMP csexit ($DEB7)
    // csnext:
    0xE6, 0x03,             // INC Temp_Var4    (next direction)
    0xA5, 0x03,             // LDA Temp_Var4
    0xC9, 0x04,             // CMP #$04
    0xD0, 0x98,             // BNE csloop
    0x4C, 0xB7, 0xDE,       // JMP csexit ($DEB7)  (no water neighbor)
    // csyoff / csxoff / csxhioff — cardinal offsets, dir 0..3 = right,left,down,up:
    0x00, 0x00, 0x10, 0xF0, // csyoff   ($DF2D)
    0x10, 0xF0, 0x00, 0x00, // csxoff   ($DF31)
    0x00, 0xFF, 0x00, 0x00, // csxhioff ($DF35)
];

/// Install the "call the boat" summon: A on a dock warps the canoe alongside.
pub fn apply_canoe_summon(rom: &mut Rom) {
    rom.write_range(FS_CANOE_SUMMON, &CANOE_SUMMON_ROUTINE);

    // Replace `LDA World_Map_Tile / LDY #$1A` (4 bytes) with `JSR canoe_summon`
    // + NOP. The routine re-establishes A and Y before returning, so the vanilla
    // special-enter-tile scan that follows is unaffected.
    let [lo, hi] = CANOE_SUMMON_CPU.to_le_bytes();
    rom.write_range(SCAN_SETUP_HOOK, &[0x20, lo, hi, 0xEA]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::qol::test_support::make_test_rom;

    #[test]
    fn routine_is_origin_locked_to_dea5() {
        // The assembled bytes embed absolute self-references ($DEB7 csexit,
        // $DF2D/31/35 tables), so the free-space slot must map to CPU $DEA5.
        assert_eq!(CANOE_SUMMON_CPU, 0xDEA5);
    }

    #[test]
    fn hook_and_routine_written() {
        let mut rom = make_test_rom();
        apply_canoe_summon(&mut rom);

        let [lo, hi] = CANOE_SUMMON_CPU.to_le_bytes();
        assert_eq!(rom.read_range(SCAN_SETUP_HOOK, 4), &[0x20, lo, hi, 0xEA]);
        // Routine: LDA World_Map_Tile / CMP #$4B at the top, offset tables at the tail.
        assert_eq!(rom.read_range(FS_CANOE_SUMMON, 4), &[0xA5, 0xE5, 0xC9, 0x4B]);
        assert_eq!(
            rom.read_range(FS_CANOE_SUMMON + CANOE_SUMMON_ROUTINE.len() - 12, 12),
            &[0x00, 0x00, 0x10, 0xF0, 0x10, 0xF0, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00],
        );
    }
}
