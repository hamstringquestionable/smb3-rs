//! World-maze: map objects a world has already lost stay lost.
//!
//! # The bug
//!
//! A wandering Hammer Bro was beaten in World 1, the player left for another
//! world and came back — and the Hammer Bro was standing there again.
//!
//! [`super::completion_bits`] persists `Map_Completions`, which covers every
//! cell the *map grid* can mark: cleared levels, busted locks, smashed rocks,
//! bridged gaps. Map objects are not map cells. They are nine parallel-array
//! slots that `Map_Init` (PRG011, `$A1D8`) **reloads from ROM on every world
//! entry**, and vanilla never cared because vanilla never returns you to a
//! world. The maze returns constantly, so every re-entry resurrects everything
//! the world had lost.
//!
//! # The slots
//!
//! Five parallel arrays, `MAPOBJ_TOTAL` = 14 entries each:
//!
//! | Array | Address | Length |
//! |---|---|---|
//! | `Map_Objects_Itm` | `$7956` | 13 |
//! | `Map_Objects_Y` | `$7EEB` | 14 |
//! | `Map_Objects_XLo` | `$7EF9` | 14 |
//! | `Map_Objects_XHi` | `$7F07` | 14 |
//! | `Map_Objects_IDs` | `$7F15` | 14 |
//!
//! plus `Map_Objects_Vis` (`$0587`, 15) and the `Map_Object_Act*` display
//! shadows, both re-derived every frame.
//!
//! **Only the ID needs to survive**, and only as one bit of it. `Map_Init`
//! copies Y / XLo / XHi / ID / Item for slots 8 down to 0 out of the world's
//! `Map_List_Object_*` tables — nine slots, `MAPOBJ_TOTALINIT` + 1 — so
//! position is restored from ROM and then re-randomised by the marching anyway.
//! What is destroyed and never recomputed is "is this one still here": the
//! defeat site writes `MAPOBJ_EMPTY` into the ID and nothing else, and
//! `MapObject_DrawSleepEnter`'s `DynJump` sends ID 0 straight to an `RTS`. One
//! bit per slot per world is therefore the whole of the state.
//!
//! Slots 9-13 are runtime-only: `Map_FindEmptyObjectSlot` can hand one out for
//! an N-Spade or a White Toad House, and `Map_Init` never reloads them. A bit
//! for one of those would mean nothing, so the mark routine bounds itself at
//! nine — and that bound is also what keeps it from writing past the store.
//!
//! # Where an object is beaten
//!
//! `MO_DoLevelClear` (PRG011). On the way back from a level it scans all 14
//! slots for one under the player's feet, runs the seven-tick poof, and then:
//!
//! ```text
//! $ABB5  A9 00        LDA #MAPOBJ_EMPTY
//! $ABB7  99 15 7F     STA Map_Objects_IDs,Y      <- displaced
//! $ABBA  85 20        STA Map_ClearLevelFXCnt
//! $ABBC  85 D7        STA Map_HideObj
//! ```
//!
//! That single `STA` is the one event in the engine that means "the player beat
//! a map object", so it is the hook: three bytes for three bytes, one whole
//! instruction, `Y` already holding the slot and `World_Num` still naming the
//! world the player is standing in. `$ABBE` — the shared exit the poof and
//! skid-back paths branch to — is left untouched.
//!
//! The airship (`MAPOBJ_AIRSHIP`) never reaches it: the engine branches away
//! two instructions earlier and increments `Map_Operation` instead. So a world
//! keeps its airship object however many times it is entered, which is what the
//! spine needs.
//!
//! # Where it is restored, and why not at the pack hook
//!
//! **The pack hook cannot see this state.** `completion_bits`' pack runs at
//! `$84CD`, and `PRG030_84A0` calls `Map_Init` at `$84AD` — thirty-two bytes
//! earlier. By the time the pack fires, the outgoing world's slots have already
//! been overwritten with the *destination* world's fresh objects. Packing there
//! would pack the wrong world, and a fix that looked right would fix nothing.
//!
//! So there is nothing to pack. The bit is written at the moment of defeat,
//! where the world is unambiguous, and the store is only ever `ORA`'d into —
//! monotone, which also means a bonus object occupying a slot the original
//! object was beaten out of cannot un-beat it.
//!
//! What is left is a **restore**, and it belongs exactly where `Map_Init` has
//! just refilled the slots. That is every entry through `$84A0`'s front door
//! and no other, which is precisely the set of entries that reach `$84CD` —
//! `PRG030_84D7`, the turn-end re-init the map loop jumps to on every level
//! entry and return, skips both. So the restore is chained onto the front of
//! [`super::completion_bits::WIPE_REPLACEMENT`], three bytes, reusing a hook
//! rather than inventing one.
//!
//! **It is deliberately *not* behind that routine's `World_Num != LIVE_WORLD`
//! test.** The pack asks "did the world change"; the restore asks "did
//! `Map_Init` just run", and those differ on a game over, on a whistle hop that
//! lands back where it started, and on the first map of a new game. All three
//! reload the objects and all three need the re-clear. Running it on a
//! same-world entry is free: clearing an already-empty slot is a no-op.
//!
//! # The world bit comes from the engine
//!
//! Both routines need `1 << world` and neither ships a table for it:
//! `Map_CompleteBit` (PRG011, `$BA2D`) is the engine's own eight-entry
//! `$80 $40 ... $01`, it sits in the bank both routines run in, and
//! `foreign_locks` already leans on the same table through the instruction it
//! displaces. Eight bytes of PRG011 saved, and the assignment is MSB-first
//! because the borrowed table is.

use crate::rom::Rom;

use super::maze_state::{MAP_OBJ_DEAD, MAP_OBJ_DEAD_LEN};
use super::rom_data::{FS_MAZE_OBJ_MARK, FS_MAZE_OBJ_RESTORE};

// --- Engine symbols ---------------------------------------------------------

/// `World_Num`, 0-based.
const WORLD_NUM: u16 = 0x0727;

/// `Map_Objects_IDs` — 14 slots, `$00` meaning "nothing here".
const MAP_OBJECTS_IDS: u16 = 0x7F15;

/// `Map_CompleteBit` (PRG011) — `$80 $40 $20 $10 $08 $04 $02 $01`, the engine's
/// own row-bit lookup, borrowed here as a world-bit lookup.
///
/// Verified against the ROM by `the_engine_still_has_the_bit_table_we_borrow`
/// rather than trusted from the disassembly's label.
const MAP_COMPLETE_BIT_CPU: u16 = 0xBA2D;
/// File offset of the same table, for that test.
#[cfg(test)]
const MAP_COMPLETE_BIT_FILE: usize = 0x17A3D;

/// PRG011 is mapped at `$A000` for the whole world map, so a file offset in it
/// is `$A000 + (file - 0x16010)` — the arithmetic `world_travel`,
/// `world_persist` and `foreign_locks` all use.
const fn prg011_cpu(file: usize) -> u16 {
    (0xA000 + (file - 0x16010)) as u16
}

const MARK_DEAD_CPU: u16 = prg011_cpu(FS_MAZE_OBJ_MARK);
pub(crate) const RESTORE_OBJECTS_CPU: u16 = prg011_cpu(FS_MAZE_OBJ_RESTORE);

// --- The hook site ----------------------------------------------------------

/// `STA Map_Objects_IDs,Y` in `MO_DoLevelClear` (PRG011, CPU `$ABB7`).
const MARK_HOOK_OFFSET: usize = 0x016BC7;
#[cfg(test)]
const MARK_HOOK_LEN: usize = 3;

/// Vanilla bytes there: `STA $7F15,Y`.
#[cfg(test)]
const MARK_HOOK_VANILLA: [u8; MARK_HOOK_LEN] = [0x99, 0x15, 0x7F];

/// `LDA #MAPOBJ_EMPTY`, the two bytes immediately before the hook.
///
/// [`MARK_DEAD`] restores `A` with an `LDA #$00` rather than juggling the
/// stack, so it owes the caller a value — the caller's next two instructions
/// are `STA Map_ClearLevelFXCnt` and `STA Map_HideObj`, both of which need the
/// zero. `the_hook_site_still_loads_zero` reads these bytes out of the ROM so
/// the debt is checked and not assumed.
#[cfg(test)]
const MARK_HOOK_PRELUDE: [u8; 2] = [0xA9, 0x00];

// --- The routines -----------------------------------------------------------

/// Empty the slot the way vanilla did, then remember that it happened.
///
/// 24 reserved, 22 used.
///
/// On entry `A` is `MAPOBJ_EMPTY` and `Y` is the slot; `X` is dead (the caller
/// reloads it) and `A` must come back zero. The store goes first so the arm
/// that skips the bookkeeping cannot skip the store, and so the routine is
/// vanilla's instruction with a tail rather than a replacement for it.
///
/// The `CPY #9` guard is the store's bound as much as it is a statement about
/// meaning: `Map_FindEmptyObjectSlot` scans upward with no ceiling of its own,
/// so a bonus object can sit at slot 9-13, and `STA MAP_OBJ_DEAD,Y` with `Y` up
/// there would write past the nine bytes into whatever the maze allocates next.
#[rustfmt::skip]
const MARK_DEAD: [u8; 22] = [
    0x99, MAP_OBJECTS_IDS as u8, (MAP_OBJECTS_IDS >> 8) as u8, //  0: STA Map_Objects_IDs,Y
    0xC0, MAP_OBJ_DEAD_LEN as u8,                             //  3: CPY #9
    0xB0, 0x0E,                                               //  5: BCS +14 -> done
    0xAE, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,            //  7: LDX World_Num
    0xBD, MAP_COMPLETE_BIT_CPU as u8,
          (MAP_COMPLETE_BIT_CPU >> 8) as u8,                  // 10: LDA Map_CompleteBit,X
    0x19, MAP_OBJ_DEAD as u8, (MAP_OBJ_DEAD >> 8) as u8,      // 13: ORA MAP_OBJ_DEAD,Y
    0x99, MAP_OBJ_DEAD as u8, (MAP_OBJ_DEAD >> 8) as u8,      // 16: STA MAP_OBJ_DEAD,Y
    0xA9, 0x00,                                               // 19: LDA #$00   ; A owed to caller
    0x60,                                                     // 21: RTS        ; done
];

/// Re-empty every slot this world has already lost.
///
/// 24 reserved, 22 used.
///
/// Called from the front of [`super::completion_bits::WIPE_REPLACEMENT`], so
/// `Map_Init` has just refilled all nine slots out of ROM and `World_Num` names
/// the world being drawn. `X` holds that world's bit for the whole loop, which
/// is why the store is indexed by slot rather than by world — see
/// [`MAP_OBJ_DEAD`].
///
/// Walks slot 8 down to 0 because `DEY`/`BPL` is two bytes cheaper than
/// counting up to a bound, and the store's nine bytes are the only bound there
/// is.
#[rustfmt::skip]
const RESTORE_OBJECTS: [u8; 22] = [
    0xAE, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,            //  0: LDX World_Num
    0xA0, (MAP_OBJ_DEAD_LEN - 1) as u8,                       //  3: LDY #8

    0xB9, MAP_OBJ_DEAD as u8, (MAP_OBJ_DEAD >> 8) as u8,      //  5: LDA MAP_OBJ_DEAD,Y  ; loop
    0x3D, MAP_COMPLETE_BIT_CPU as u8,
          (MAP_COMPLETE_BIT_CPU >> 8) as u8,                  //  8: AND Map_CompleteBit,X
    0xF0, 0x05,                                               // 11: BEQ +5 -> next
    0xA9, 0x00,                                               // 13: LDA #$00
    0x99, MAP_OBJECTS_IDS as u8, (MAP_OBJECTS_IDS >> 8) as u8, // 15: STA Map_Objects_IDs,Y

    0x88,                                                     // 18: DEY                 ; next
    0x10, 0xF0,                                               // 19: BPL -16 -> loop
    0x60,                                                     // 21: RTS
];

// --- Writer -----------------------------------------------------------------

/// Install both routines and the defeat hook.
///
/// The restore's *call* is not here: it is the first three bytes of
/// [`super::completion_bits::WIPE_REPLACEMENT`], because that is the routine
/// that owns the `$84CD` hook. `the_wipe_replacement_calls_the_restore` is what
/// keeps the two ends together.
pub(crate) fn apply(rom: &mut Rom) {
    rom.push_tag("map_objects");
    rom.write_range(FS_MAZE_OBJ_MARK, &MARK_DEAD);
    rom.write_range(FS_MAZE_OBJ_RESTORE, &RESTORE_OBJECTS);
    rom.write_range(MARK_HOOK_OFFSET, &[0x20, MARK_DEAD_CPU as u8, (MARK_DEAD_CPU >> 8) as u8]);
    rom.pop_tag();
}

#[cfg(test)]
mod tests {
    use super::*;

    use mos6502::cpu::CPU;
    use mos6502::instruction::Ricoh2a03;
    use mos6502::memory::{Bus, Memory};

    use crate::randomize::completion_bits;
    use crate::randomize::rom_data::{self, MAP_COMPLETE_BITS, asm};

    const ROM_PATH: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";

    fn vanilla() -> Option<Rom> {
        let bytes = std::fs::read(ROM_PATH).ok()?;
        Some(Rom::from_bytes(&bytes).expect("vanilla ROM parses"))
    }

    // --- Static checks ------------------------------------------------------

    #[test]
    fn routines_are_well_formed() {
        asm::check(&MARK_DEAD).allocation(FS_MAZE_OBJ_MARK).origin(MARK_DEAD_CPU).assert_ok();
        asm::check(&RESTORE_OBJECTS)
            .allocation(FS_MAZE_OBJ_RESTORE)
            .origin(RESTORE_OBJECTS_CPU)
            .assert_ok();
    }

    /// The defeat hook displaces one whole instruction, and the two bytes in
    /// front of it are still the `LDA #$00` the routine's exit relies on.
    #[test]
    fn the_hook_displaces_whole_instructions() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        assert_eq!(
            rom.read_range(MARK_HOOK_OFFSET, MARK_HOOK_LEN),
            MARK_HOOK_VANILLA,
            "MO_DoLevelClear's STA Map_Objects_IDs,Y has moved",
        );
        assert_eq!(
            rom.read_range(MARK_HOOK_OFFSET - MARK_HOOK_PRELUDE.len(), MARK_HOOK_PRELUDE.len()),
            MARK_HOOK_PRELUDE,
            "the site no longer loads MAPOBJ_EMPTY, so MARK_DEAD's LDA #$00 exit is wrong",
        );
        asm::check(&MARK_DEAD)
            .allocation(FS_MAZE_OBJ_MARK)
            .origin(MARK_DEAD_CPU)
            .hook(&MARK_HOOK_VANILLA, 0, &[0x20, MARK_DEAD_CPU as u8, (MARK_DEAD_CPU >> 8) as u8])
            .assert_ok();
    }

    /// The world bit is read out of the engine's table rather than a table of
    /// our own, so the table has to still be there — in vanilla, and in a
    /// finished world-maze ROM, which is the one that runs the code.
    #[test]
    fn the_engine_still_has_the_bit_table_we_borrow() {
        let Ok(rom_bytes) = std::fs::read(ROM_PATH) else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let van = Rom::from_bytes(&rom_bytes).expect("vanilla ROM parses");
        assert_eq!(
            prg011_cpu(MAP_COMPLETE_BIT_FILE),
            MAP_COMPLETE_BIT_CPU,
            "Map_CompleteBit's file offset and CPU address disagree",
        );
        assert_eq!(
            van.read_range(MAP_COMPLETE_BIT_FILE, 8),
            MAP_COMPLETE_BITS,
            "Map_CompleteBit is not where PRG011 says it is",
        );

        let mut options =
            crate::Options { palettes: false, palette_themed: false, ..Default::default() };
        options.world_maze = true;
        options.world_order = true;
        let Ok(patched) = crate::randomize_rom(&rom_bytes, 7, &options, None) else {
            eprintln!("SKIP: the maze build failed for this seed");
            return;
        };
        assert_eq!(
            patched.read_range(MAP_COMPLETE_BIT_FILE, 8),
            MAP_COMPLETE_BITS,
            "a world-maze run wrote over the bit table both routines index",
        );
    }

    /// The restore has to be *called*, and from the one hook that fires exactly
    /// when `Map_Init` has refilled the slots. It sits in front of
    /// `WIPE_REPLACEMENT`'s transition test on purpose — a game over, a whistle
    /// hop that lands where it started and a new game all reload the objects
    /// without changing `World_Num`.
    #[test]
    fn the_wipe_replacement_calls_the_restore() {
        assert_eq!(
            completion_bits::wipe_replacement_bytes()[..3],
            [0x20, RESTORE_OBJECTS_CPU as u8, (RESTORE_OBJECTS_CPU >> 8) as u8],
            "WIPE_REPLACEMENT must open with JSR RESTORE_OBJECTS, before its own compare",
        );
    }

    /// Neither routine touches zero page, so nothing here can be destroyed by
    /// the NMI mid-loop. Same rule, and same reason, as `completion_bits`'
    /// `no_routine_parks_state_in_unprotected_zero_page`.
    #[test]
    fn no_routine_touches_zero_page() {
        use mos6502::Variant;
        use mos6502::instruction::AddressingMode;

        for (name, code) in
            [("MARK_DEAD", &MARK_DEAD[..]), ("RESTORE_OBJECTS", &RESTORE_OBJECTS[..])]
        {
            let mut pc = 0usize;
            while pc < code.len() {
                let (instr, mode) =
                    Ricoh2a03::decode(code[pc]).unwrap_or_else(|| panic!("{name}: bad opcode"));
                assert!(
                    !matches!(
                        mode,
                        AddressingMode::ZeroPage
                            | AddressingMode::ZeroPageX
                            | AddressingMode::ZeroPageY
                            | AddressingMode::IndexedIndirectX
                            | AddressingMode::IndirectIndexedY
                    ),
                    "{name} byte {pc}: {instr:?} reaches zero page, which the NMI does not \
                     preserve above Temp_Var3",
                );
                pc += mode.extra_bytes() as usize + 1;
            }
        }
    }

    // --- The emulated 2A03 --------------------------------------------------

    /// Address the harness treats as "the routine returned".
    const SENTINEL: u16 = 0x0F00;

    fn call_routine(cpu: &mut CPU<Memory, Ricoh2a03>, entry: u16, what: &str) {
        let ret = SENTINEL.wrapping_sub(1);
        cpu.memory.set_byte(0x01FF, (ret >> 8) as u8);
        cpu.memory.set_byte(0x01FE, ret as u8);
        cpu.registers.stack_pointer = mos6502::registers::StackPointer(0xFD);
        cpu.registers.program_counter = entry;
        for _ in 0..100_000 {
            if cpu.registers.program_counter == SENTINEL {
                return;
            }
            cpu.single_step();
        }
        panic!("{what} ran away");
    }

    /// A CPU holding the two routines at their real origins over PRG011's real
    /// contents, so `Map_CompleteBit` is the engine's bytes and not a fixture.
    ///
    /// `mark` and `restore` are passed in rather than read from the consts so
    /// the mutation tests can plant a wrong byte and drive the same harness.
    fn cpu_with(rom: &Rom, mark: &[u8], restore: &[u8]) -> CPU<Memory, Ricoh2a03> {
        let mut mem = Memory::new();
        let prg011: Vec<u8> = (0..0x2000).map(|i| rom.read_byte(0x16010 + i)).collect();
        mem.set_bytes(0xA000, &prg011);
        mem.set_bytes(MARK_DEAD_CPU, mark);
        mem.set_bytes(RESTORE_OBJECTS_CPU, restore);
        CPU::new(mem, Ricoh2a03)
    }

    /// Load a world's nine slots the way `Map_Init` does — from the ROM's own
    /// per-world ID table, so the fixture is the map the engine would build.
    fn map_init(cpu: &mut CPU<Memory, Ricoh2a03>, rom: &Rom, world: usize) {
        cpu.memory.set_byte(WORLD_NUM, world as u8);
        for slot in 0..MAP_OBJ_DEAD_LEN {
            let id = rom.read_byte(rom_data::map_obj_slot_offset(
                rom,
                rom_data::MAP_OBJ_IDS_MASTER,
                world,
                slot,
            ));
            cpu.memory.set_byte(MAP_OBJECTS_IDS + slot as u16, id);
        }
        // The five runtime-only slots start empty, as they do after a reset.
        for slot in MAP_OBJ_DEAD_LEN..14 {
            cpu.memory.set_byte(MAP_OBJECTS_IDS + slot as u16, 0);
        }
    }

    fn ids(cpu: &mut CPU<Memory, Ricoh2a03>) -> Vec<u8> {
        (0..14u16).map(|i| cpu.memory.get_byte(MAP_OBJECTS_IDS + i)).collect()
    }

    /// Beat the object in `slot` the way `MO_DoLevelClear` does: `A` =
    /// `MAPOBJ_EMPTY`, `Y` = the slot, then the displaced instruction's
    /// replacement.
    fn beat(cpu: &mut CPU<Memory, Ricoh2a03>, slot: u8) {
        cpu.registers.accumulator = 0;
        cpu.registers.index_y = slot;
        call_routine(cpu, MARK_DEAD_CPU, "MARK_DEAD");
        assert_eq!(
            cpu.registers.accumulator, 0,
            "MARK_DEAD owes the caller A = MAPOBJ_EMPTY for the two stores that follow",
        );
    }

    /// The first slot of `world` that holds a real, beatable object — the
    /// airship is exempted by the engine, and slot 0 is the HELP bubble.
    fn a_beatable_slot(rom: &Rom, world: usize) -> Option<u8> {
        (2..MAP_OBJ_DEAD_LEN as u8).find(|&slot| {
            rom.read_byte(rom_data::map_obj_slot_offset(
                rom,
                rom_data::MAP_OBJ_IDS_MASTER,
                world,
                slot as usize,
            )) != 0
        })
    }

    /// **The playtest, on the emulated CPU.** Beat a Hammer Bro in World 1,
    /// leave for World 2, come back — and it is still gone.
    ///
    /// Driven through the real hooks, the way `completion_bits`'
    /// `a_beaten_level_survives_a_full_cycle` drives its own: a transition is
    /// `World_Num` changing and `Map_Init` reloading, and nothing else.
    #[test]
    fn a_beaten_map_object_survives_a_round_trip() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut cpu = cpu_with(&rom, &MARK_DEAD, &RESTORE_OBJECTS);
        for i in 0..MAP_OBJ_DEAD_LEN as u16 {
            cpu.memory.set_byte(MAP_OBJ_DEAD + i, 0);
        }

        // --- arrive in World 1 ---
        map_init(&mut cpu, &rom, 0);
        call_routine(&mut cpu, RESTORE_OBJECTS_CPU, "enter W1");
        let slot = a_beatable_slot(&rom, 0).expect("World 1 has a Hammer Bro");
        assert_ne!(cpu.memory.get_byte(MAP_OBJECTS_IDS + slot as u16), 0);

        // --- beat it ---
        beat(&mut cpu, slot);
        assert_eq!(cpu.memory.get_byte(MAP_OBJECTS_IDS + slot as u16), 0, "the poof still empties");

        // --- off to World 2, which must be untouched ---
        map_init(&mut cpu, &rom, 1);
        call_routine(&mut cpu, RESTORE_OBJECTS_CPU, "enter W2");
        let want: Vec<u8> = {
            let mut fresh = cpu_with(&rom, &MARK_DEAD, &RESTORE_OBJECTS);
            map_init(&mut fresh, &rom, 1);
            ids(&mut fresh)
        };
        assert_eq!(ids(&mut cpu), want, "World 2's objects were disturbed by World 1's loss");

        // --- and back ---
        map_init(&mut cpu, &rom, 0);
        call_routine(&mut cpu, RESTORE_OBJECTS_CPU, "re-enter W1");
        assert_eq!(
            cpu.memory.get_byte(MAP_OBJECTS_IDS + slot as u16),
            0,
            "World 1 slot {slot} came back to life",
        );
        // Everything else in World 1 is exactly as `Map_Init` left it.
        for other in 0..MAP_OBJ_DEAD_LEN {
            if other as u8 == slot {
                continue;
            }
            let id = rom.read_byte(rom_data::map_obj_slot_offset(
                &rom,
                rom_data::MAP_OBJ_IDS_MASTER,
                0,
                other,
            ));
            assert_eq!(
                cpu.memory.get_byte(MAP_OBJECTS_IDS + other as u16),
                id,
                "World 1 slot {other} was cleared and should not have been",
            );
        }
    }

    /// Every world, every slot, one at a time: the bit that comes back is the
    /// bit that went in, and no other world's slots move.
    ///
    /// This is the test that a wrong world bit fails — an index that ignored
    /// `World_Num` would clear the same slot in all eight worlds.
    #[test]
    fn each_world_keeps_its_own_losses() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        for world in 0..8usize {
            for slot in 0..MAP_OBJ_DEAD_LEN as u8 {
                let mut cpu = cpu_with(&rom, &MARK_DEAD, &RESTORE_OBJECTS);
                for i in 0..MAP_OBJ_DEAD_LEN as u16 {
                    cpu.memory.set_byte(MAP_OBJ_DEAD + i, 0);
                }
                map_init(&mut cpu, &rom, world);
                beat(&mut cpu, slot);

                for other in 0..8usize {
                    map_init(&mut cpu, &rom, other);
                    call_routine(&mut cpu, RESTORE_OBJECTS_CPU, "enter");
                    for s in 0..MAP_OBJ_DEAD_LEN {
                        let fresh = rom.read_byte(rom_data::map_obj_slot_offset(
                            &rom,
                            rom_data::MAP_OBJ_IDS_MASTER,
                            other,
                            s,
                        ));
                        let cleared = other == world && s as u8 == slot;
                        assert_eq!(
                            cpu.memory.get_byte(MAP_OBJECTS_IDS + s as u16),
                            if cleared { 0 } else { fresh },
                            "beat W{} slot {slot}, then entered W{}: slot {s} is wrong",
                            world + 1,
                            other + 1,
                        );
                    }
                }
            }
        }
    }

    /// A slot above the eight `Map_Init` reloads is still stored and still
    /// restored. Slot 8 is the one a byte-per-world layout would have had to
    /// drop, and the randomizer places Hammer Bros there
    /// (`eligible_hb_map_slots` runs to 9).
    #[test]
    fn the_ninth_slot_is_stored_too() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut cpu = cpu_with(&rom, &MARK_DEAD, &RESTORE_OBJECTS);
        for i in 0..MAP_OBJ_DEAD_LEN as u16 {
            cpu.memory.set_byte(MAP_OBJ_DEAD + i, 0);
        }
        cpu.memory.set_byte(WORLD_NUM, 5);
        // Pretend the builder put something at slot 8, as it may.
        cpu.memory.set_byte(MAP_OBJECTS_IDS + 8, 0x03);
        beat(&mut cpu, 8);
        assert_ne!(
            cpu.memory.get_byte(MAP_OBJ_DEAD + 8),
            0,
            "slot 8 was beaten and nothing was stored",
        );

        cpu.memory.set_byte(MAP_OBJECTS_IDS + 8, 0x03);
        call_routine(&mut cpu, RESTORE_OBJECTS_CPU, "re-enter");
        assert_eq!(cpu.memory.get_byte(MAP_OBJECTS_IDS + 8), 0, "slot 8 came back");
    }

    /// A runtime-only slot is out of the store's range, and must be left
    /// alone rather than written past the end of it.
    ///
    /// `Map_FindEmptyObjectSlot` scans upward with no ceiling, so an N-Spade
    /// can land at slot 9-13; `Map_Init` never reloads those, so remembering
    /// one would mean nothing and storing it would land in whatever the maze
    /// allocates after the nine bytes.
    #[test]
    fn a_runtime_only_slot_is_not_stored() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        for slot in MAP_OBJ_DEAD_LEN as u8..14 {
            let mut cpu = cpu_with(&rom, &MARK_DEAD, &RESTORE_OBJECTS);
            // Poison the store and the eight bytes after it: a write to either
            // is a failure, and the ones past the end are the dangerous half.
            for i in 0..(MAP_OBJ_DEAD_LEN as u16 + 8) {
                cpu.memory.set_byte(MAP_OBJ_DEAD + i, 0xAA);
            }
            cpu.memory.set_byte(WORLD_NUM, 3);
            cpu.memory.set_byte(MAP_OBJECTS_IDS + slot as u16, 0x09);
            beat(&mut cpu, slot);
            assert_eq!(
                cpu.memory.get_byte(MAP_OBJECTS_IDS + slot as u16),
                0,
                "slot {slot} must still be emptied — that is vanilla's instruction",
            );
            for i in 0..(MAP_OBJ_DEAD_LEN as u16 + 8) {
                assert_eq!(
                    cpu.memory.get_byte(MAP_OBJ_DEAD + i),
                    0xAA,
                    "beating runtime slot {slot} wrote byte {i} of the maze store",
                );
            }
        }
    }

    /// The bit is sticky. A White Toad House that lands in the slot a Hammer
    /// Bro was beaten out of must not un-beat the Hammer Bro on the next
    /// entry — which is exactly what a "pack the live IDs on the way out"
    /// design would have done.
    #[test]
    fn a_bonus_object_in_a_freed_slot_does_not_revive_it() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut cpu = cpu_with(&rom, &MARK_DEAD, &RESTORE_OBJECTS);
        for i in 0..MAP_OBJ_DEAD_LEN as u16 {
            cpu.memory.set_byte(MAP_OBJ_DEAD + i, 0);
        }
        map_init(&mut cpu, &rom, 0);
        let slot = a_beatable_slot(&rom, 0).expect("World 1 has a Hammer Bro");
        beat(&mut cpu, slot);
        // `Map_FindEmptyObjectSlot` hands the freed slot to a White Toad House.
        cpu.memory.set_byte(MAP_OBJECTS_IDS + slot as u16, 0x0A);

        map_init(&mut cpu, &rom, 0);
        call_routine(&mut cpu, RESTORE_OBJECTS_CPU, "re-enter W1");
        assert_eq!(
            cpu.memory.get_byte(MAP_OBJECTS_IDS + slot as u16),
            0,
            "the store forgot slot {slot} because something else stood in it",
        );
    }

    // --- Mutation tests -----------------------------------------------------
    //
    // Each plants one wrong byte and demands a named test above go red. A
    // guard nobody has watched fail is a guard nobody has tested.

    /// Drive `each_world_keeps_its_own_losses`' core claim over the given
    /// routines, returning whether it held.
    fn worlds_stay_separate(rom: &Rom, mark: &[u8], restore: &[u8]) -> bool {
        let mut cpu = cpu_with(rom, mark, restore);
        for i in 0..MAP_OBJ_DEAD_LEN as u16 {
            cpu.memory.set_byte(MAP_OBJ_DEAD + i, 0);
        }
        map_init(&mut cpu, rom, 0);
        let slot = a_beatable_slot(rom, 0).expect("World 1 has a Hammer Bro");
        cpu.registers.accumulator = 0;
        cpu.registers.index_y = slot;
        call_routine(&mut cpu, MARK_DEAD_CPU, "MARK_DEAD");

        // World 2 keeps everything, World 1 loses exactly that slot.
        map_init(&mut cpu, rom, 1);
        call_routine(&mut cpu, RESTORE_OBJECTS_CPU, "enter W2");
        let w2_intact = (0..MAP_OBJ_DEAD_LEN).all(|s| {
            cpu.memory.get_byte(MAP_OBJECTS_IDS + s as u16)
                == rom.read_byte(rom_data::map_obj_slot_offset(
                    rom,
                    rom_data::MAP_OBJ_IDS_MASTER,
                    1,
                    s,
                ))
        });
        map_init(&mut cpu, rom, 0);
        call_routine(&mut cpu, RESTORE_OBJECTS_CPU, "re-enter W1");
        let w1_lost = cpu.memory.get_byte(MAP_OBJECTS_IDS + slot as u16) == 0;
        w2_intact && w1_lost
    }

    /// **Wrong bit index.** Index the world-bit table with the *slot* instead
    /// of the world (`AND Map_CompleteBit,Y`) and every world starts sharing
    /// World 1's losses.
    #[test]
    fn mutation_a_slot_indexed_world_bit_is_caught() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        assert!(
            worlds_stay_separate(&rom, &MARK_DEAD, &RESTORE_OBJECTS),
            "the unmutated routines must pass, or the mutation proves nothing",
        );
        let mut bad = RESTORE_OBJECTS;
        bad[8] = 0x39; // AND abs,Y instead of AND abs,X
        assert!(
            !worlds_stay_separate(&rom, &MARK_DEAD, &bad),
            "a slot-indexed world bit went unnoticed",
        );
    }

    /// **Wrong loop bound.** Start the restore at slot 7 and the ninth slot is
    /// never re-cleared.
    #[test]
    fn mutation_a_short_restore_loop_is_caught() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let ninth_is_restored = |restore: &[u8]| {
            let mut cpu = cpu_with(&rom, &MARK_DEAD, restore);
            for i in 0..MAP_OBJ_DEAD_LEN as u16 {
                cpu.memory.set_byte(MAP_OBJ_DEAD + i, 0);
            }
            cpu.memory.set_byte(WORLD_NUM, 5);
            cpu.memory.set_byte(MAP_OBJECTS_IDS + 8, 0x03);
            cpu.registers.accumulator = 0;
            cpu.registers.index_y = 8;
            call_routine(&mut cpu, MARK_DEAD_CPU, "MARK_DEAD");
            cpu.memory.set_byte(MAP_OBJECTS_IDS + 8, 0x03);
            call_routine(&mut cpu, RESTORE_OBJECTS_CPU, "re-enter");
            cpu.memory.get_byte(MAP_OBJECTS_IDS + 8) == 0
        };
        assert!(ninth_is_restored(&RESTORE_OBJECTS), "the unmutated loop must reach slot 8");
        let mut bad = RESTORE_OBJECTS;
        bad[4] = 0x07; // LDY #7
        assert!(!ninth_is_restored(&bad), "a restore loop one slot short went unnoticed");
    }

    /// **Wrong mark bound.** Widen the guard to `CPY #14` and beating a
    /// runtime-only slot writes past the nine bytes of store.
    #[test]
    fn mutation_a_wide_mark_guard_is_caught() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let stays_in_bounds = |mark: &[u8]| {
            let mut cpu = cpu_with(&rom, mark, &RESTORE_OBJECTS);
            for i in 0..(MAP_OBJ_DEAD_LEN as u16 + 8) {
                cpu.memory.set_byte(MAP_OBJ_DEAD + i, 0xAA);
            }
            cpu.memory.set_byte(WORLD_NUM, 3);
            cpu.registers.accumulator = 0;
            cpu.registers.index_y = 11;
            call_routine(&mut cpu, MARK_DEAD_CPU, "MARK_DEAD");
            (0..(MAP_OBJ_DEAD_LEN as u16 + 8))
                .all(|i| cpu.memory.get_byte(MAP_OBJ_DEAD + i) == 0xAA)
        };
        assert!(stays_in_bounds(&MARK_DEAD), "the unmutated guard must hold");
        let mut bad = MARK_DEAD;
        bad[4] = 14; // CPY #MAPOBJ_TOTAL
        assert!(!stays_in_bounds(&bad), "a guard that admits the runtime slots went unnoticed");
    }
}
