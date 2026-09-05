//! The world maze's battery-backed state, and the one place its addresses are
//! decided.
//!
//! Everything here lives in the `$7A73-$7ADF` run the disassembly declares
//! unused and the persistence POC **proved** free at runtime by parking a
//! completion bank in it. `completion_bits` owns the first part of that run
//! (the mask scratch, the arrival vars, `LIVE_WORLD`, the pack temporaries)
//! and stops at `$7AC0`. Everything from `$7AC1` up is the maze's, and it is
//! allocated here rather than beside whichever feature happened to need it
//! first — a second module picking its own address is exactly how two features
//! end up sharing a byte, and `sram_allocations_do_not_overlap` is the only
//! thing that would ever catch it.
//!
//! **None of this is initialised at boot.** The title screen's new-game signal
//! (`completion_bits`) is what clears it; a cold boot with dirty SRAM before
//! that runs is the same hole the packed planes have, closed the same way.

/// First byte of the maze's own SRAM, immediately after `completion_bits`'
/// last allocation at `$7AC0`.
pub(crate) const MAZE_STATE_START: u16 = 0x7AC1;

/// Last byte of the free run this all has to fit inside.
pub(crate) const MAZE_STATE_END: u16 = 0x7ADF;

/// Which worlds the player has stood on the start tile of, one byte per world.
///
/// A byte per world rather than a bitmask, and it is cheaper both ways:
/// marking is `LDY World_Num / LDA #$01 / STA VISITED,Y` (8 bytes, no mask
/// table and no shift), and the whistle's "next visited world" scan is
/// `LDA VISITED,Y / BNE`. A bitmask would save 7 bytes of SRAM and cost more
/// than that in ROM, which is the scarcer resource.
pub(crate) const VISITED_TABLE: u16 = MAZE_STATE_START;
pub(crate) const VISITED_TABLE_LEN: usize = 8;

/// Wands collected, 0-7. The wand gate on World 8's bridge compares against
/// this; it is the whole of the gate's state, which is why the gate needs no
/// completion bit and no persistence of its own — it is re-derived on every
/// map load.
pub(crate) const WAND_COUNT: u16 = VISITED_TABLE + VISITED_TABLE_LEN as u16;

/// First byte after everything allocated above — where the next allocation
/// starts.
pub(crate) const MAZE_STATE_NEXT: u16 = WAND_COUNT + 1;

/// The maze's own SRAM stays inside the run it was given, and behind
/// `completion_bits`' last byte.
///
/// Const assertions rather than a test, because there is no configuration in
/// which an overlapping allocation is worth building: the two numbers it
/// guards — `completion_bits`' last byte, and the end of the free run — live in
/// other places and would otherwise move silently.
const _: () = {
    assert!(MAZE_STATE_START > 0x7AC0, "maze SRAM must start after completion_bits' $7AC0");
    assert!(MAZE_STATE_NEXT <= MAZE_STATE_END + 1, "maze SRAM runs past the end of its free run");
    assert!(
        VISITED_TABLE + VISITED_TABLE_LEN as u16 <= WAND_COUNT,
        "the visited table overlaps the wand counter"
    );
};
