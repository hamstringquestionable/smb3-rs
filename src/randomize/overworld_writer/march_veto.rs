//! Per-coordinate landing veto for wandering map objects (Hammer Bros et al).
//!
//! Piranha-plant and W8 army levels are map-object sprites over plain
//! path-node tiles. Beating one poofs the sprite (RAM only), leaving a node
//! tile that is a legal landing spot for the engine's wandering-object march
//! — in the vanilla blacklist regime AND under the opt-in
//! `limit_bro_movement` whitelist (which whitelists exactly the node tiles).
//! A Hammer Bro parking there re-enters the beaten level when touched.
//!
//! Fix: hook `Map_MarchValidateTravel` (PRG011 CPU $B3A3) at its second
//! `JSR Map_Object_March_PickTravel` call ($B3FD, the landing-zone pick) and
//! run a trampoline that rejects the hop when the candidate landing tile's
//! SRAM address matches a per-world registry of plant/army coordinates the
//! randomizer writes. HB encounter nodes are intentionally NOT vetoed —
//! replaying a Hammer Bro fight is acceptable.
//!
//! The trampoline also rejects landings on the hand-trap tile ($E6),
//! subsuming MaCobra52's former `bros_no_hands` patch (issue #14) with
//! stronger semantics: the bro re-picks its direction instead of stepping
//! onto the hand and marching off again. $E6 stays in the pass-through
//! whitelist (`VALID_HORZ`), so bros still walk THROUGH hands, matching
//! vanilla movement.
//!
//! Engine facts (verified in the southbird disassembly + ROM bytes):
//! - Each march hop is 2 tiles; `Map_Object_March_PickTravel` ($B43B) leaves
//!   `Map_Tile_Addr` ($63/$64) = `Tile_Mem_Addr[screen] + $F0` and
//!   `Temp_Var3` ($02) = (row-nibble << 4) | column-in-screen, so
//!   `($63) + $02` is the landing tile's absolute SRAM address — unique per
//!   (screen, row, col) within the loaded world. `World_Num` ($0727)
//!   disambiguates worlds.
//! - `Tile_Mem_Addr` word table lives in PRG030 (fixed bank) at CPU $8000 /
//!   file 0x3C010, base $6000, stride $1B0 per screen.
//! - A/X/Y/flags and Temp_Var15/16 ($0E/$0F) are dead at the hook point: the
//!   code right after the displaced JSR reloads all of them, and $0E/$0F are
//!   re-stored on every direction re-pick.
//! - Reject = `JMP $B3A3` (re-pick direction). The routine's give-up path
//!   does `PLA/PLA/RTS` to return to the caller's caller, so the trampoline
//!   must pop its own return address before the reject JMP to keep that
//!   stack contract exact. Worst case the object stays put (safe).
//! - The hook does not overlap `limit_bro_movement` (writes 0x17398-0x173AC
//!   and 0x17419-0x17420) and fires before either table-scan regime, so the
//!   two compose in both orders.

use super::*;
use super::rom_data::FS_MARCH_VETO;

/// Hook site: file offset of `JSR Map_Object_March_PickTravel` at CPU $B3FD
/// (the second, landing-zone call inside `Map_MarchValidateTravel`).
pub(super) const MARCH_VETO_HOOK: usize = 0x1740D;

/// The vanilla bytes at the hook site (`JSR $B43B`), displaced to become the
/// trampoline's first instruction.
pub(super) const DISPLACED_JSR: [u8; 3] = [0x20, 0x3B, 0xB4];

/// Hand-trap map tile. Landing on it is vetoed (bros still pass through).
const HAND_TRAP_TILE: u8 = 0xE6;

/// The direction re-pick loop inside `Map_MarchValidateTravel` (CPU) — the
/// vanilla landing-reject `JMP $B3A3` target. Jumping here retries with a new
/// direction WITHOUT resetting the 4-attempt give-up counter (Temp_Var2).
const RETRY_PICK_CPU: u16 = 0xB3A3;

/// Trampoline layout within the FS_MARCH_VETO block (all offsets in bytes).
const LOOP_OFF: usize = 30;
const ACCEPT_OFF: usize = 53;
const REJECT_OFF: usize = 54;
pub(super) const ROUTINE_LEN: usize = 59;
pub(super) const OFFSETS_LEN: usize = 8;
pub(super) const LIST_LEN: usize = 40; // 16 two-byte entries + 8 per-world terminators

/// SRAM address of the map tile at grid (row, col), as the engine computes it
/// in `Map_Object_March_PickTravel`: `Tile_Mem_Addr[col/16] + $F0 + Temp_Var3`
/// where the Y-coordinate byte is `(row + 2) * 16` (same convention as
/// `write_map_sprite_position`). The hi byte is always >= 0x61, so 0x00 is a
/// safe list terminator.
pub(super) fn veto_addr(row: usize, col: usize) -> u16 {
    (0x6000 + (col / 16) * 0x1B0 + 0xF0 + (row + 2) * 16 + (col % 16)) as u16
}

/// Write the march-veto hook, trampoline, and per-world coordinate registry.
///
/// `w8_positions` are the army sprite placements (world 7 implied);
/// `plant_positions` carry their own world index. Always applied — the
/// hand-trap veto replaces the always-on `bros_no_hands` patch even when no
/// plants are placed.
pub(super) fn write_march_veto(
    rom: &mut Rom,
    w8_positions: &[(usize, (usize, usize))],
    plant_positions: &[(usize, (usize, usize))],
) {
    // Per-world landing addresses, sorted + deduped for stable output.
    let mut per_world: [Vec<u16>; 8] = Default::default();
    for &(_, (row, col)) in w8_positions {
        per_world[7].push(veto_addr(row, col));
    }
    for &(wi, (row, col)) in plant_positions {
        per_world[wi].push(veto_addr(row, col));
    }
    for list in &mut per_world {
        list.sort_unstable();
        list.dedup();
    }

    // Registry: per-world byte offset into the list, then (hi, lo) pairs with
    // a 0x00-hi terminator per world.
    let mut offsets = [0u8; OFFSETS_LEN];
    let mut list: Vec<u8> = Vec::new();
    for (wi, addrs) in per_world.iter().enumerate() {
        offsets[wi] = list.len() as u8;
        for &addr in addrs {
            list.extend_from_slice(&[(addr >> 8) as u8, (addr & 0xFF) as u8]);
        }
        list.push(0x00);
    }
    assert!(
        list.len() <= LIST_LEN,
        "march veto list overflow: {} > {LIST_LEN} bytes",
        list.len()
    );
    list.resize(LIST_LEN, 0x00);

    // Assemble the trampoline. All absolute operands are derived from
    // FS_MARCH_VETO so they can't drift from where the bytes land (issue #14
    // convention).
    let base_cpu = rom_data::prg_bank_file_to_cpu(11, FS_MARCH_VETO);
    let loop_cpu = base_cpu + LOOP_OFF as u16;
    let offsets_cpu = base_cpu + ROUTINE_LEN as u16;
    let list_cpu = offsets_cpu + OFFSETS_LEN as u16;
    let lo = |a: u16| (a & 0xFF) as u8;
    let hi = |a: u16| (a >> 8) as u8;

    #[rustfmt::skip]
    let routine: [u8; ROUTINE_LEN] = [
        // +0: displaced PickTravel (landing zone)
        DISPLACED_JSR[0], DISPLACED_JSR[1], DISPLACED_JSR[2],
        0xA4, 0x02,                               // +3:  LDY Temp_Var3
        0xB1, 0x63,                               // +5:  LDA (Map_Tile_Addr),Y — landing tile byte
        0xC9, HAND_TRAP_TILE,                     // +7:  CMP #$E6
        0xF0, (REJECT_OFF - 11) as u8,            // +9:  BEQ Reject
        0xA5, 0x63,                               // +11: LDA Map_Tile_AddrL
        0x18,                                     // +13: CLC
        0x65, 0x02,                               // +14: ADC Temp_Var3
        0x85, 0x0E,                               // +16: STA Temp_Var15 = landing addr lo
        0xA5, 0x64,                               // +18: LDA Map_Tile_AddrH
        0x69, 0x00,                               // +20: ADC #$00
        0x85, 0x0F,                               // +22: STA Temp_Var16 = landing addr hi
        0xAE, 0x27, 0x07,                         // +24: LDX World_Num
        0xBC, lo(offsets_cpu), hi(offsets_cpu),   // +27: LDY VETO_OFFSETS,X
        // Loop (+30):
        0xB9, lo(list_cpu), hi(list_cpu),         // +30: LDA VETO_LIST,Y — entry hi; 0 = end
        0xF0, (ACCEPT_OFF - 35) as u8,            // +33: BEQ Accept
        0xC5, 0x0F,                               // +35: CMP Temp_Var16
        0xD0, 0x09,                               // +37: BNE Next
        0xB9, lo(list_cpu + 1), hi(list_cpu + 1), // +39: LDA VETO_LIST+1,Y — entry lo
        0xC5, 0x0E,                               // +42: CMP Temp_Var15
        0xD0, 0x02,                               // +44: BNE Next
        0xF0, (REJECT_OFF - 48) as u8,            // +46: BEQ Reject (always taken)
        // Next (+48):
        0xC8,                                     // +48: INY
        0xC8,                                     // +49: INY
        0x4C, lo(loop_cpu), hi(loop_cpu),         // +50: JMP Loop
        // Accept (+53): resume at $B400 (vanilla reloads A/X/Y right after)
        0x60,                                     // +53: RTS
        // Reject (+54): drop the trampoline frame, then the vanilla reject
        // path — keeps the give-up path's PLA/PLA/RTS stack contract exact.
        0x68,                                     // +54: PLA
        0x68,                                     // +55: PLA
        0x4C, lo(RETRY_PICK_CPU), hi(RETRY_PICK_CPU), // +56: JMP $B3A3
    ];

    debug_assert_eq!(
        rom.read_range(MARCH_VETO_HOOK, 3),
        DISPLACED_JSR,
        "march veto hook site changed — displaced instruction mismatch"
    );
    rom.write_range(FS_MARCH_VETO, &routine);
    rom.write_range(FS_MARCH_VETO + ROUTINE_LEN, &offsets);
    rom.write_range(FS_MARCH_VETO + ROUTINE_LEN + OFFSETS_LEN, &list);
    rom.write_range(MARCH_VETO_HOOK, &rom_data::jsr_into_bank(11, FS_MARCH_VETO));
}
