//! Retire the 2-player Vs Challenge, and free 339 bytes of PRG030 with it.
//!
//! # What it was, and why it goes
//!
//! Pressing A while standing on the other player started a minigame. Two things
//! made that worth removing rather than fixing:
//!
//! **Its trigger is unsound once the players can be in different worlds.** The
//! test compares three coordinate bytes — `World_Map_XHi`, `World_Map_Y`,
//! `World_Map_X` — and nothing else. In vanilla, equal coordinates meant
//! literally the same tile, because both players were always on the same map.
//! Under [`crate::randomize::maze::player_worlds`] two players in *different* worlds whose
//! coordinates coincide start a Vs battle out of nowhere. `World_Map_Y` has only
//! nine distinct values and there are sixteen columns to a screen, so the
//! coincidence is uncommon rather than rare — and likeliest on the tiles both
//! players stop on, which is why it read as a pipe bug.
//!
//! **And the collision path skips the tile-enterability test.** Look at where it
//! lands:
//!
//! ```text
//!         <three coordinate compares, each BNE PRG010_CEBF>
//!         LDA #$12
//!         STA <Map_Enter2PFlag
//! $CEA7:  LDA #$10
//!         STA Map_Operation        ; "begin enter level" — no tile test at all
//! ```
//!
//! The `Map_EnterSpecialTiles` scan and the `World_Map_Tile >=
//! Tile_AttrTable+4[page]` threshold both live in `PRG010_CEBF`, the *other*
//! path. Standing on the other player therefore made an otherwise-dead tile
//! "enterable", which is fine in vanilla — the thing being entered is the Vs
//! battlefield, not the tile's level — and is exactly the surprise a player
//! sees when a Vs fires on the beaten tile their partner is standing on.
//!
//! So this is not only a defect fix. It retires the one interaction on the world
//! map that bypassed the enterability rules every other tile obeys.
//!
//! # What happens instead
//!
//! [`DISABLE_COLLISION`] sends the A-press straight to `PRG010_CEBF`, so the
//! normal path runs and answers for every tile type:
//!
//! * a beaten level, which is not enterable, does **nothing** — no re-entry, no
//!   battle;
//! * something genuinely enterable under your partner — an unbeaten level, a
//!   pipe, a telepad — behaves exactly as it would if they were not there;
//! * the *inactive* player's A does nothing, because `PRG010_CEBF` re-tests
//!   `Pad_Input` (the live player) where the collision test had `OR`-ed both
//!   controllers together.
//!
//! No new behaviour is invented: this is the path that already ran whenever the
//! players were not stacked.
//!
//! # Why it is unconditional
//!
//! Not gated on the world maze, and that is forced rather than chosen: the point
//! is to free ROM, and a `FREE_SPACE_ALLOCATIONS` row has to be free in *every*
//! seed. Gating would leave the 339 bytes unusable for allocation, which is most
//! of the value. It costs one-player games nothing — the collision test is
//! behind an `AND #$80` on both controllers and a `Player_Lives,Y` check that
//! one-player mode never passes — and it removes a two-player interaction most
//! players never saw on purpose.
//!
//! # The space, and why all three splices exist
//!
//! One splice fixes the bug. The other two are what make the freed runs
//! *unreferenced by construction* rather than by argument, which is the standard
//! `CLAUDE.md` sets before anything may be written into a gap.
//!
//! | run | bytes | freed because |
//! |---|---|---|
//! | `$88F4..$8919` | 38 | [`DEAD_FLAG_READ`] makes the `BEQ` past it always taken |
//! | `$934C..$9478` | **301** | [`DEAD_VS_JUMP`] removes `Do_2PVsChallenge`'s only reference |
//!
//! Every internal label of the 301-byte block (`PRG030_939A`, `_93B1`, `_93E7`,
//! `_93F1`, `_93F4`, `_946C`) was checked against the whole disassembly and is
//! referenced only from inside it; its one outward branch is the closing
//! `JMP PRG030_8FB2`.
//!
//! **Nothing claims the space yet.** It is recorded in
//! `docs/smb3_rom_reference.md` and `CLAUDE.md` so the first feature that needs
//! it can add its own `FS_*` row, exactly the way `FS_FORTRESS_FX` works —
//! neither run is `$FF`, so `--free-space` cannot see either of them. Moving a
//! working allocation into them now would be risk for no present benefit.
//!
//! About 23 further bytes go dead in PRG010 (`$CE8A..$CEA6`, the compares and
//! the flag store the jump skips). Too small to be worth a row, noted so nobody
//! puzzles over them.
//!
//! The rest of the subsystem sits in banks under no pressure and is left alone:
//! PRG009 (`Vs_2PVsInit`/`Run`/`PauseHandler`), PRG014 (`Vs_Battlefields` and
//! its battlefield data), PRG027 (`PalSet_2PVs`). Its CHR pages are a separate
//! question — see the issue linked from the changelog.

use crate::rom::Rom;

use crate::randomize::rom_data::prg010_file_to_cpu;
#[cfg(test)]
use crate::randomize::rom_data::prg030_file_to_cpu;

// --- Sites ---------------------------------------------------------------

/// `LDA Player_Lives,Y` at the head of the collision test (PRG010, CPU
/// `$CE87`), three bytes — the whole instruction, and the first of the block.
///
/// Everything from here to `$CEA6` is the dead-partner check, the three
/// coordinate compares and the flag store; the jump that replaces it makes all
/// of that unreachable. `$CEA7` itself stays live: `PRG010_CEBF` branches to it
/// on a special-tile match, and [`crate::randomize::maze::world_persist`]'s telepad hook sits
/// there.
const COLLISION_TEST_OFFSET: usize = 0x14E97;

/// `PRG010_CEBF`, the path taken when the players are *not* stacked — the one
/// that tests whether the tile can be entered at all. Derived rather than
/// transcribed, because a hand-written `$CEBF` cannot drift back into agreement.
const NO_COLLISION_OFFSET: usize = 0x14ECF;
const NO_COLLISION_CPU: u16 = prg010_file_to_cpu(NO_COLLISION_OFFSET);

/// `LDA Map_Enter2PFlag` in `PRG030_84A0`'s level-entry dispatch (CPU `$88F0`),
/// two bytes. The flag is zero page `$1D` and has exactly two references in the
/// ROM: the store this module orphans, and this read.
///
/// **Worth knowing even though it is now moot:** nothing ever *cleared* it.
/// `$1D` is zeroed only by a cold boot, a death-style return (`Map_ReturnStatus`
/// non-zero, `PRG030_8FCA`) and the wand cutscene — never by an ordinary level
/// clear. Vanilla got away with it because a Vs always ends in a loss, which
/// always takes the death-style return. A flag nobody sets cannot go stale, so
/// removing the store settles that too.
const FLAG_READ_OFFSET: usize = 0x3C900;

/// `JMP Do_2PVsChallenge` at CPU `$8AE4`, three bytes — reached only when
/// `Level_Tileset == 18`, which only the run [`DEAD_FLAG_READ`] strands could
/// set. `NOP`ing it lands on `PRG030_8AE7`, which is where the `BNE` two
/// instructions earlier goes anyway.
const VS_DISPATCH_OFFSET: usize = 0x3CAF4;

/// First byte and length of the two runs this frees. Not claimed — see the
/// module docs — but named so the accounting is in the source and not only in
/// prose, and asserted against the disabling splices by
/// `the_freed_runs_sit_past_their_disabling_splices`. Test-only precisely
/// because nothing owns the space yet: the feature that claims it adds an
/// `FS_*` row and these two go away.
#[cfg(test)]
const FREED_SETUP: (usize, usize) = (0x3C904, 38);
#[cfg(test)]
const FREED_CHALLENGE: (usize, usize) = (0x3D35C, 301);

// --- The splices ---------------------------------------------------------

/// Skip the collision test entirely: `JMP PRG010_CEBF`.
const DISABLE_COLLISION: [u8; 3] = [0x4C, NO_COLLISION_CPU as u8, (NO_COLLISION_CPU >> 8) as u8];

/// `LDA #$00` in place of `LDA Map_Enter2PFlag`, so the `BEQ` after it is always
/// taken and the 2P Vs setup block is unreachable by construction.
const DEAD_FLAG_READ: [u8; 2] = [0xA9, 0x00];

/// Three `NOP`s in place of `JMP Do_2PVsChallenge`, removing the block's last
/// reference.
const DEAD_VS_JUMP: [u8; 3] = [0xEA, 0xEA, 0xEA];

// --- Application ---------------------------------------------------------

/// Retire the Vs Challenge. Unconditional — see the module docs.
///
/// No ordering requirement. None of the three sites is touched by any other
/// module: the nearest neighbour is `world_persist`'s telepad hook at `$CEA7`,
/// sixteen bytes past the end of the collision block this orphans.
pub(crate) fn apply(rom: &mut Rom) {
    rom.write_range(COLLISION_TEST_OFFSET, &DISABLE_COLLISION);
    rom.write_range(FLAG_READ_OFFSET, &DEAD_FLAG_READ);
    rom.write_range(VS_DISPATCH_OFFSET, &DEAD_VS_JUMP);
}

#[cfg(test)]
mod asm_checks {
    use super::*;
    use crate::randomize::rom_data::asm;

    /// `LDA Player_Lives,Y`.
    const COLLISION_VANILLA: [u8; 3] = [0xB9, 0x36, 0x07];
    /// `LDA $1D`.
    const FLAG_READ_VANILLA: [u8; 2] = [0xA5, 0x1D];
    /// `JMP $934C`.
    const VS_DISPATCH_VANILLA: [u8; 3] = [0x4C, 0x4C, 0x93];

    fn vanilla() -> Option<Rom> {
        let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&bytes).ok()
    }

    /// Each splice is exactly as wide as the whole instruction it replaces, and
    /// each one decodes. A splice one byte short would leave an orphan operand
    /// running as an opcode.
    #[test]
    fn the_splices_are_well_formed() {
        assert_eq!(DISABLE_COLLISION.len(), COLLISION_VANILLA.len());
        assert_eq!(DEAD_FLAG_READ.len(), FLAG_READ_VANILLA.len());
        assert_eq!(DEAD_VS_JUMP.len(), VS_DISPATCH_VANILLA.len());

        asm::check(&DISABLE_COLLISION).fragment().assert_ok();
        asm::check(&DEAD_FLAG_READ).fragment().assert_ok();
        // Not run through the checker: it refuses an all-`NOP` array, and
        // rightly — for every other patch here that would mean a routine that
        // fell out of an edit. This one is `NOP`s on purpose.
        assert!(DEAD_VS_JUMP.iter().all(|&b| b == 0xEA), "the dispatch splice must be NOPs");
    }

    /// The jump has to land on the path that tests whether the tile can be
    /// entered. Anywhere else and a beaten tile becomes enterable again, which
    /// is the surprise this removal exists to stop.
    #[test]
    fn the_jump_names_the_no_collision_path() {
        assert_eq!(
            u16::from_le_bytes([DISABLE_COLLISION[1], DISABLE_COLLISION[2]]),
            NO_COLLISION_CPU,
            "the splice must jump to PRG010_CEBF",
        );
        assert_eq!(NO_COLLISION_CPU, 0xCEBF, "PRG010_CEBF moved");
    }

    /// All three sites hold what this module thinks they hold, and the
    /// displaced `BEQ` really does reach `PRG010_CEBF` — a splice aimed at some
    /// other loop's compare would pass every other check here.
    #[test]
    fn the_sites_are_where_vanilla_put_them() {
        let Some(rom) = vanilla() else { return };
        assert_eq!(
            rom.data[COLLISION_TEST_OFFSET..COLLISION_TEST_OFFSET + 3],
            COLLISION_VANILLA,
            "the collision test no longer opens with LDA Player_Lives,Y",
        );
        assert_eq!(
            rom.data[FLAG_READ_OFFSET..FLAG_READ_OFFSET + 2],
            FLAG_READ_VANILLA,
            "the level-entry dispatch no longer opens with LDA Map_Enter2PFlag",
        );
        assert_eq!(
            rom.data[VS_DISPATCH_OFFSET..VS_DISPATCH_OFFSET + 3],
            VS_DISPATCH_VANILLA,
            "the tileset-18 dispatch no longer jumps to Do_2PVsChallenge",
        );

        // `BEQ PRG010_CEBF` sits two instructions into the block, at $CE8C.
        // Its operand is what pins CEBF independently of our own arithmetic.
        let beq = COLLISION_TEST_OFFSET + 5;
        assert_eq!(rom.data[beq], 0xF0, "the dead-partner early-out is not a BEQ");
        let target = (beq + 2) as isize + (rom.data[beq + 1] as i8) as isize;
        assert_eq!(
            target as usize, NO_COLLISION_OFFSET,
            "vanilla's own early-out does not reach the offset we jump to",
        );
    }

    /// The freed runs are where the survey said, and neither overlaps a site we
    /// splice — a run that swallowed its own disabling instruction would be
    /// claimed and then overwritten.
    #[test]
    fn the_freed_runs_sit_past_their_disabling_splices() {
        let (setup, setup_len) = FREED_SETUP;
        let (block, block_len) = FREED_CHALLENGE;
        assert_eq!(setup, FLAG_READ_OFFSET + 4, "the setup run must start after the BEQ");
        assert!(setup + setup_len <= VS_DISPATCH_OFFSET, "the setup run runs into the dispatch");
        assert!(block > VS_DISPATCH_OFFSET, "the challenge block must sit past its dispatch");
        assert_eq!(prg030_file_to_cpu(block), 0x934C, "Do_2PVsChallenge moved from $934C");
        assert_eq!(prg030_file_to_cpu(block + block_len), 0x9479, "the block's end moved");
    }
}
