//! The canoe is gated on the anchor.
//!
//! Boats start one tile further out than vanilla parks them, and the only way
//! to bring one alongside is to stand on a dock and use an **Anchor** from the
//! inventory. The boat is the wall, the anchor is the key, and both are things
//! the player can see.
//!
//! # Why the anchor, and why it is never consumed
//!
//! Vanilla's anchor keeps an airship from moving. That effect is already dead
//! here — with the wand cutscene skipped the airship never leaves its castle
//! tile — which is why `items::write_mystery_anchor` repurposes the item at
//! all. This takes the same free slot for a job that needs one.
//!
//! The item is **not** consumed on use. That is the whole reason this feature
//! needs no state of its own: "can the player cross water" is exactly "is
//! there an anchor in the inventory", a fact the game already stores and
//! already carries between worlds. Nothing has to be remembered about which
//! boats have been freed, because freeing one is repeatable — walk back to any
//! dock, in any world, and use the anchor again.
//!
//! # The handler is small because the summon already exists
//!
//! [`qol::canoe_summon`](crate::randomize::qol) parks the boat on a water tile
//! beside the player, and its first act is to check that the player is
//! standing on `TILE_DOCK`. So the anchor's handler is that routine plus a
//! reason to deny: test the dock tile, `JSR` the summon, play vanilla's own
//! item-use poof, and flip back to the map.
//!
//! **The dock test reads the same byte the summon does** (`World_Map_Tile`,
//! `$E5`), which is what makes it trustworthy from inside the inventory. If
//! that variable were stale while the panel is open, the summon would be
//! equally wrong, and this would fail loudly — an anchor that always denies —
//! rather than quietly snapping a boat somewhere unintended.
//!
//! `JSR $DEA5` reaches across banks safely: the summon lives in PRG010 at
//! `$C000`, and PRG010 is still mapped with the inventory open. Vanilla's own
//! `Inv_UseItem_Hammer` calls `MapTile_Get_By_Offset` there for the same
//! reason.
//!
//! # It shares one word with mystery anchor
//!
//! `items::write_mystery_anchor` repoints the very same `Inv_UseItem` jump
//! table entry to make an anchor a random power-up. The two cannot both hold
//! it, so the caller picks: with the world maze on the anchor is the boat key,
//! and without it the mystery power-up stands. [`apply`] therefore accepts
//! either value at the vector, so it can be installed over a finished ROM as
//! well as a vanilla one — which is what `testrom` does.

use crate::randomize::rom_data::{
    FS_ANCHOR_USE, MAP_OBJ_IDS_MASTER, MAP_OBJ_XHIS_MASTER, MAP_OBJ_XLOS_MASTER,
    map_obj_slot_offset,
};
use crate::rom::Rom;

// --- The dispatch vector ---------------------------------------------------

/// `Inv_UseItem`'s per-item jump table (PRG026, CPU `$A540`). `DynJump` indexes
/// it with the raw Global Item ID, so the Anchor's word is entry `$0A`.
const USE_ITEM_TABLE: usize = 0x34550;
const ANCHOR_ITEM_ID: usize = 0x0A;

/// What the vector holds before anyone touches it: `Inv_UseItem_Anchor`.
const VANILLA_ANCHOR_HANDLER: u16 = 0xA682;
/// What `items::write_mystery_anchor` leaves there: `Inv_UseItem_Powerup`.
const MYSTERY_ANCHOR_HANDLER: u16 = 0xA5B6;

// --- The handler -----------------------------------------------------------

/// CPU address of [`FS_ANCHOR_USE`]: `$A000 + (0x3571D - 0x34010)`.
const ANCHOR_USE_CPU: u16 = 0xB70D;

/// `World_Map_Tile`, the tile the player is standing on.
const WORLD_MAP_TILE: u8 = 0xE5;
/// `TILE_DOCK` — the `$4B` the summon keys on.
const TILE_DOCK: u8 = 0x4B;
/// `qol::canoe_summon`'s routine (PRG010, origin-locked to `$DEA5`).
const CANOE_SUMMON_CPU: u16 = 0xDEA5;
/// `Sound_QLevel1` and the "item used" poof vanilla queues into it.
const SOUND_QLEVEL1: u16 = 0x04F2;
const SND_LEVELPOOF: u8 = 0x80;
/// `Inventory_ForceFlip` — closes the panel and returns to the map.
const INVENTORY_FORCE_FLIP: u16 = 0xA426;
/// `Inv_UseItem_Denial` — vanilla's "you cannot use that here" sound, and an
/// `RTS` that leaves the item in the inventory. Shared with the Hammer, which
/// is why the vanilla anchor handler's bytes are not reclaimed: `$A687` is
/// live code inside the run this would otherwise free.
const INV_USE_ITEM_DENIAL: u16 = 0xA687;

/// Snap the boat to the dock the player is standing on.
///
/// ```text
///   LDA World_Map_Tile
///   CMP #TILE_DOCK
///   BNE deny
///   JSR canoe_summon        ; parks the boat on adjacent water
///   LDA Sound_QLevel1 / ORA #SND_LEVELPOOF / STA Sound_QLevel1
///   JMP Inventory_ForceFlip ; back to the map — and no ShiftOver, so the
///                           ; anchor stays in the inventory
/// deny:
///   JMP Inv_UseItem_Denial
/// ```
///
/// Every vanilla item-use path ends `JSR Inv_UseItem_ShiftOver / JMP
/// Inventory_ForceFlip`, and the `ShiftOver` is what deletes the item. Leaving
/// it out is the entire "never consumed" behaviour — there is no flag for it.
#[rustfmt::skip]
const ANCHOR_USE: [u8; 23] = [
    0xA5, WORLD_MAP_TILE,                                       //  0: LDA World_Map_Tile
    0xC9, TILE_DOCK,                                            //  2: CMP #$4B
    0xD0, 0x0E,                                                 //  4: BNE deny
    0x20, CANOE_SUMMON_CPU as u8, (CANOE_SUMMON_CPU >> 8) as u8,//  6: JSR canoe_summon
    0xAD, SOUND_QLEVEL1 as u8, (SOUND_QLEVEL1 >> 8) as u8,      //  9: LDA Sound_QLevel1
    0x09, SND_LEVELPOOF,                                        // 12: ORA #SND_LEVELPOOF
    0x8D, SOUND_QLEVEL1 as u8, (SOUND_QLEVEL1 >> 8) as u8,      // 14: STA Sound_QLevel1
    0x4C, INVENTORY_FORCE_FLIP as u8, (INVENTORY_FORCE_FLIP >> 8) as u8, // 17: JMP flip
    0x4C, INV_USE_ITEM_DENIAL as u8, (INV_USE_ITEM_DENIAL >> 8) as u8,   // 20: deny
];

// --- The wall --------------------------------------------------------------

/// `MAPOBJ_CANOE`, the map-object id the summon scans for.
const MAPOBJ_CANOE: u8 = 0x10;

/// Push every canoe one tile further from its dock.
///
/// Boarding is walking into the boat from the dock, so a canoe parked on the
/// water cell *adjacent* to a dock is boardable on foot and the anchor gates
/// nothing. One tile further out it is visible and unreachable — which is the
/// point: seeing the thing you cannot reach is what sends a player looking for
/// an anchor, where an empty dock teaches nothing.
///
/// Both destinations are water and were checked: W3's boat sits at (6,21)
/// beside the dock at (6,20) with (6,22) also water (`$8D`), and under `8s are
/// Wild` W8's sits at (5,7) beside (5,6) with (5,8) water (`$8D`, written by
/// that option's own tile edits).
///
/// Scans for the id rather than naming slots, so it covers W3's vanilla slot
/// and the one `8s are Wild` adds without depending on which ran first —
/// **but it must run after both**, or a boat placed later keeps its vanilla
/// berth and that world's water is free to cross.
fn move_canoes_offshore(rom: &mut Rom) {
    for world in 0..8 {
        for slot in 0..9 {
            let id = rom.read_byte(map_obj_slot_offset(rom, MAP_OBJ_IDS_MASTER, world, slot));
            if id != MAPOBJ_CANOE {
                continue;
            }
            // One grid column is 16 units of XLo; past the last column of a
            // screen it carries into XHi. Done on the bytes rather than by
            // decoding to (row, col) and back, so the row is untouched.
            let xlo_off = map_obj_slot_offset(rom, MAP_OBJ_XLOS_MASTER, world, slot);
            let xhi_off = map_obj_slot_offset(rom, MAP_OBJ_XHIS_MASTER, world, slot);
            let xlo = rom.read_byte(xlo_off);
            if xlo >= 0xF0 {
                rom.write_byte(xlo_off, 0x00);
                rom.write_byte(xhi_off, rom.read_byte(xhi_off) + 1);
            } else {
                rom.write_byte(xlo_off, xlo + 0x10);
            }
        }
    }
}

// --- Installation ----------------------------------------------------------

/// Install the gate: the anchor becomes the boat key, and the boats move out
/// of reach.
///
/// Safe to run over a finished ROM as well as a vanilla one, which is what
/// `testrom` does — it takes the A-press summon back out itself rather than
/// leaving that to the caller, because a summon left installed opens the wall
/// for free and nothing downstream would notice.
pub fn apply(rom: &mut Rom) {
    rom.push_tag("canoe_gate");

    // The handler JSRs the summon, so the routine has to be there; the A-press
    // hook must not be, or the boat comes over for nothing.
    crate::randomize::qol::write_canoe_summon_routine(rom);
    crate::randomize::qol::remove_canoe_summon_hook(rom);

    let vector = USE_ITEM_TABLE + ANCHOR_ITEM_ID * 2;
    let found = u16::from_le_bytes([rom.read_byte(vector), rom.read_byte(vector + 1)]);
    assert!(
        found == VANILLA_ANCHOR_HANDLER || found == MYSTERY_ANCHOR_HANDLER,
        "Inv_UseItem entry {ANCHOR_ITEM_ID} holds {found:#06X}, which is neither vanilla's \
         {VANILLA_ANCHOR_HANDLER:#06X} nor mystery anchor's {MYSTERY_ANCHOR_HANDLER:#06X} — \
         something else has claimed the anchor"
    );
    rom.write_range(vector, &ANCHOR_USE_CPU.to_le_bytes());
    rom.write_range(FS_ANCHOR_USE, &ANCHOR_USE);

    move_canoes_offshore(rom);
    rom.pop_tag();
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROM_PATH: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";

    /// The map-object tables are read through their real pointer table, so
    /// these need the ROM rather than a synthetic one. Skips where it is
    /// absent, as every other ROM-backed test here does.
    fn vanilla() -> Option<Rom> {
        let bytes = std::fs::read(ROM_PATH).ok()?;
        Some(Rom::from_bytes(&bytes).expect("vanilla ROM parses"))
    }

    #[test]
    fn allocation_maps_to_the_address_the_routine_is_assembled_for() {
        // PRG026 is mapped at $A000; file 0x34010 is its first byte.
        assert_eq!(ANCHOR_USE_CPU, (0xA000 + FS_ANCHOR_USE - 0x34010) as u16);
    }

    #[test]
    fn vector_and_routine_written() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let vector = USE_ITEM_TABLE + ANCHOR_ITEM_ID * 2;
        assert_eq!(
            rom.read_range(vector, 2),
            &VANILLA_ANCHOR_HANDLER.to_le_bytes()[..],
            "the test ROM should start with vanilla's anchor handler"
        );

        apply(&mut rom);

        assert_eq!(rom.read_range(vector, 2), &ANCHOR_USE_CPU.to_le_bytes()[..]);
        assert_eq!(rom.read_range(FS_ANCHOR_USE, ANCHOR_USE.len()), &ANCHOR_USE[..]);
    }

    /// Installing over a finished ROM is the `testrom` path, and mystery
    /// anchor has already taken the vector there.
    #[test]
    fn accepts_the_mystery_anchor_vector() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let vector = USE_ITEM_TABLE + ANCHOR_ITEM_ID * 2;
        rom.write_range(vector, &MYSTERY_ANCHOR_HANDLER.to_le_bytes());

        apply(&mut rom);

        assert_eq!(rom.read_range(vector, 2), &ANCHOR_USE_CPU.to_le_bytes()[..]);
    }

    /// The boat has to end up somewhere the player cannot walk onto, and the
    /// row has to survive the move — a boat that changes row is a boat beside
    /// a different dock, or none.
    #[test]
    fn canoes_move_one_column_and_keep_their_row() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut before = Vec::new();
        for world in 0..8 {
            for slot in 0..9 {
                let id = rom.read_byte(map_obj_slot_offset(&rom, MAP_OBJ_IDS_MASTER, world, slot));
                if id == MAPOBJ_CANOE {
                    let xlo = map_obj_slot_offset(&rom, MAP_OBJ_XLOS_MASTER, world, slot);
                    let xhi = map_obj_slot_offset(&rom, MAP_OBJ_XHIS_MASTER, world, slot);
                    before.push((world, slot, rom.read_byte(xlo), rom.read_byte(xhi)));
                }
            }
        }
        assert!(!before.is_empty(), "vanilla has at least W3's canoe");

        apply(&mut rom);

        for (world, slot, xlo, xhi) in before {
            let after_lo =
                rom.read_byte(map_obj_slot_offset(&rom, MAP_OBJ_XLOS_MASTER, world, slot));
            let after_hi =
                rom.read_byte(map_obj_slot_offset(&rom, MAP_OBJ_XHIS_MASTER, world, slot));
            let moved = (u16::from(after_hi) << 8 | u16::from(after_lo))
                .wrapping_sub(u16::from(xhi) << 8 | u16::from(xlo));
            assert_eq!(moved, 0x10, "world {world} slot {slot} moved {moved:#06X}, not one column");
        }
    }
}

#[cfg(test)]
mod asm_checks {
    //! Decode the assembled routine and check the structural properties no
    //! assembler was around to enforce. See [`crate::randomize::rom_data::asm`].
    use super::*;
    use crate::randomize::rom_data::asm;

    #[test]
    fn anchor_use_is_well_formed() {
        asm::check(&ANCHOR_USE)
            // No `.allocation(FS_ANCHOR_USE)`: the row is deliberately absent
            // from the registry until this is wired to an option, so that
            // builder method would fail looking for it. The two properties it
            // would check are asserted below instead.
            // Runs once, from a menu, with no loop for the NMI to land inside —
            // but it still touches only `World_Map_Tile`, which it must read to
            // agree with the summon it calls.
            .zero_page(0x02, &[WORLD_MAP_TILE])
            .assert_ok();
    }

    /// What `.allocation()` would have checked: the routine fits the reserve
    /// [`FS_ANCHOR_USE`] documents, and it does not run off the end of PRG026
    /// into whatever is paged in next.
    #[test]
    fn anchor_use_fits_its_reserve_and_its_bank() {
        const RESERVED: usize = 40;
        const PRG026_END: usize = 0x34010 + 0x2000;
        assert!(
            ANCHOR_USE.len() <= RESERVED,
            "routine is {} bytes, past the {RESERVED} FS_ANCHOR_USE reserves",
            ANCHOR_USE.len()
        );
        const { assert!(FS_ANCHOR_USE + RESERVED <= PRG026_END, "the reservation leaves PRG026") };
    }
}
