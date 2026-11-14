//! **Sprite slots**: one ledger of a world's nine map-object slots, and the
//! only code that writes the map-object tables.
//!
//! See `docs/sprite_slots_design.md`. Every map object — HELP bubble, Hammer
//! Bros, piranha plants, World 8's military, canoes — lives in one of nine
//! slots per world, and the engine has opinions about some slot numbers. This
//! module holds those opinions ([`SpriteKind`]'s rules), answers "how much room
//! is left" in one place, and writes the result back once.
//!
//! **The ledger is a snapshot of the ROM's tables** taken when the build starts
//! ([`WorldSprites::read`]), edited in memory, and written back by
//! [`WorldSprites::write`] — which writes only the bytes that changed. That is
//! what keeps it byte-identical to the code it replaced: vanilla's empty slots
//! carry leftover position bytes, and a writer that zeroed every empty slot
//! would change them.

use crate::randomize::overworld::build::RESERVED_DYNAMIC_SLOTS;
use crate::randomize::rom_data::{
    MAP_OBJ_IDS_MASTER, MAP_OBJ_SLOTS, MAP_OBJ_XHIS_MASTER, MAP_OBJ_XLOS_MASTER, MAP_OBJ_YS_MASTER,
    W8_IDX, map_obj_reward_offset, map_obj_slot_offset,
};
use crate::rom::Rom;

/// `MAPOBJ_EMPTY`.
const ID_EMPTY: u8 = 0x00;

/// What occupies a slot, from its object id. Only the kinds a rule or a
/// claimant cares about are named; the rest are `Other` and simply occupy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpriteKind {
    Empty,
    /// `$01`, the HELP bubble: slot 0, the maze's wand indicator in W1-W7.
    Help,
    /// `$03`-`$06`: never slot 0 (the coin-ship revert skips it).
    HammerBro,
    /// `$07`, a piranha plant: stationary, any slot.
    Plant,
    /// `$0D`-`$0F`, World 8's military: stationary, any slot.
    Military,
    /// `$10`: never slot 0 (the engine's canoe search skips it).
    Canoe,
    Other,
}

impl SpriteKind {
    pub(crate) fn of(id: u8) -> SpriteKind {
        match id {
            ID_EMPTY => SpriteKind::Empty,
            0x01 => SpriteKind::Help,
            0x03..=0x06 => SpriteKind::HammerBro,
            0x07 => SpriteKind::Plant,
            0x0D..=0x0F => SpriteKind::Military,
            0x10 => SpriteKind::Canoe,
            _ => SpriteKind::Other,
        }
    }
}

/// One slot's five table bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SlotEntry {
    pub(crate) id: u8,
    y: u8,
    xhi: u8,
    xlo: u8,
    pub(crate) reward: u8,
}

impl SlotEntry {
    pub(crate) fn kind(&self) -> SpriteKind {
        SpriteKind::of(self.id)
    }

    /// The grid cell the sprite stands on, or `None` for a position no map
    /// cell has (a row above the grid — how vanilla parks World 8's HELP).
    pub(crate) fn grid_pos(&self) -> Option<(usize, usize)> {
        if self.y < 32 {
            return None;
        }
        Some((
            usize::from(self.y) / 16 - 2,
            usize::from(self.xhi) * 16 + usize::from(self.xlo) / 16,
        ))
    }

    fn set_grid_pos(&mut self, (row, col): (usize, usize)) {
        self.y = ((row + 2) * 16) as u8;
        self.xhi = (col / 16) as u8;
        self.xlo = ((col % 16) * 16) as u8;
    }
}

/// A world's nine slots: the snapshot it was read as, and the edits since.
#[derive(Clone, Debug)]
pub(crate) struct WorldSprites {
    world: usize,
    read_as: [SlotEntry; MAP_OBJ_SLOTS],
    pub(crate) entries: [SlotEntry; MAP_OBJ_SLOTS],
    /// Slot 1 is capacity: autoscroll removal retires the king scene, so the
    /// engine never puts the airship there ([`Self::open_airship_slot`]).
    airship_slot_free: bool,
    /// Slots held back from the Hammer Bros for the piranha plants
    /// ([`Self::reserve_for_plants`]).
    plant_reserve: usize,
}

impl WorldSprites {
    /// An empty ledger for a world not yet read. [`build`](super::build::build)
    /// reads every world before anything asks it a question.
    pub(crate) fn unread(world: usize) -> WorldSprites {
        let e = SlotEntry { id: ID_EMPTY, y: 0, xhi: 0, xlo: 0, reward: 0 };
        WorldSprites {
            world,
            read_as: [e; MAP_OBJ_SLOTS],
            entries: [e; MAP_OBJ_SLOTS],
            airship_slot_free: false,
            plant_reserve: 0,
        }
    }

    /// Make slot 1 usable in W1-W7 (World 8's always is). Only with autoscroll
    /// removal on: the engine's one writer of slot 1 is the king scene and the
    /// airship's own handler, and removal retires both.
    pub(crate) fn open_airship_slot(&mut self) {
        self.airship_slot_free = true;
    }

    /// World 8 only: move the sprite in `from` (the level tank) into slot 0,
    /// over the HELP bubble vanilla parks off the map there, and free `from`.
    /// Nothing reads World 8's slot 0 — no king, no wand, and the maze's
    /// slot-0 clear is keyed on `WANDS_TABLE[7]`, never set. Returns `false`,
    /// changing nothing, unless slot 0 is that HELP and `from` is military.
    ///
    /// **Playtested (2026-11-08):** the engine never hides slot 0 while a
    /// beaten object poofs (`MapObject_DrawSleepEnter`, `CPX #$00` — 0 is
    /// `Map_HideObj`'s "none"), so a beaten tank here stays drawn until the
    /// poof empties the slot. That is the only difference: it cannot be
    /// re-entered during the poof, dying in it behaves normally, and the
    /// cleared cell stays cleared.
    pub(crate) fn take_w8_help_slot(&mut self, from: usize) -> bool {
        let ok = self.world == W8_IDX
            && self.entries[0].kind() == SpriteKind::Help
            && self.entries[from].kind() == SpriteKind::Military;
        if ok {
            self.entries[0] = self.entries[from];
            self.clear_slot(from);
        }
        ok
    }

    /// Whether slot 1 is sprite capacity rather than the airship's.
    pub(crate) fn airship_slot_free(&self) -> bool {
        self.airship_slot_free
    }

    /// Hold `n` slots back from the Hammer Bros so the piranha plants always
    /// fit — Piranha Shuffle Off on generated maps, where both plant levels
    /// are pinned to World 7 (`overworld::piranha_home`).
    pub(crate) fn reserve_for_plants(&mut self, n: usize) {
        self.plant_reserve += n;
    }

    /// Snapshot world `world`'s slots from the ROM.
    pub(crate) fn read(rom: &Rom, world: usize) -> WorldSprites {
        let byte = |master: usize, slot: usize| {
            rom.read_byte(map_obj_slot_offset(rom, master, world, slot))
        };
        let entries: [SlotEntry; MAP_OBJ_SLOTS] = std::array::from_fn(|slot| SlotEntry {
            id: byte(MAP_OBJ_IDS_MASTER, slot),
            y: byte(MAP_OBJ_YS_MASTER, slot),
            xhi: byte(MAP_OBJ_XHIS_MASTER, slot),
            xlo: byte(MAP_OBJ_XLOS_MASTER, slot),
            reward: rom.read_byte(map_obj_reward_offset(world, slot)),
        });
        WorldSprites { read_as: entries, entries, ..WorldSprites::unread(world) }
    }

    /// Write back every byte that differs from the snapshot, and nothing else.
    pub(crate) fn write(&self, rom: &mut Rom) {
        for slot in 0..MAP_OBJ_SLOTS {
            let (was, now) = (self.read_as[slot], self.entries[slot]);
            let mut put = |master: usize, old: u8, new: u8| {
                if old != new {
                    rom.write_byte(map_obj_slot_offset(rom, master, self.world, slot), new);
                }
            };
            put(MAP_OBJ_IDS_MASTER, was.id, now.id);
            put(MAP_OBJ_YS_MASTER, was.y, now.y);
            put(MAP_OBJ_XHIS_MASTER, was.xhi, now.xhi);
            put(MAP_OBJ_XLOS_MASTER, was.xlo, now.xlo);
            if was.reward != now.reward {
                rom.write_byte(map_obj_reward_offset(self.world, slot), now.reward);
            }
        }
    }

    /// The first slot a randomizer-placed sprite may take: slot 0 is the HELP
    /// bubble, and slot 1 the airship's in W1-W7 unless
    /// [`Self::open_airship_slot`] freed it.
    fn first_usable(&self) -> usize {
        if self.world == W8_IDX || self.airship_slot_free { 1 } else { 2 }
    }

    /// Slots a Hammer Bro may take: from [`Self::first_usable`] up, empty or
    /// already a Hammer Bro (those are cleared before the bros are dealt).
    pub(crate) fn hb_eligible(&self) -> Vec<usize> {
        (self.first_usable()..MAP_OBJ_SLOTS)
            .filter(|&s| {
                matches!(self.entries[s].kind(), SpriteKind::Empty | SpriteKind::HammerBro)
            })
            .collect()
    }

    /// How many Hammer Bros the builder may deal this world: the eligible slots
    /// less the runtime-spawn reserve and any slots held for the plants.
    pub(crate) fn room(&self) -> usize {
        self.room_for_plants().saturating_sub(self.plant_reserve)
    }

    /// How many slots the plants may use, the reservation included: the
    /// eligible slots less the runtime-spawn reserve. The writer subtracts the
    /// Hammer Bros it dealt.
    pub(crate) fn room_for_plants(&self) -> usize {
        self.hb_eligible().len().saturating_sub(RESERVED_DYNAMIC_SLOTS)
    }

    /// The highest empty usable slot — where stationary sprites go, so the
    /// low slots stay for the Hammer Bros, which fill from the bottom.
    fn last_empty(&self) -> Option<usize> {
        (self.first_usable()..MAP_OBJ_SLOTS)
            .rev()
            .find(|&s| self.entries[s].kind() == SpriteKind::Empty)
    }

    /// Grid cells of every sprite of `kind`.
    pub(crate) fn positions(&self, kind: SpriteKind) -> Vec<(usize, usize)> {
        self.entries.iter().filter(|e| e.kind() == kind).filter_map(SlotEntry::grid_pos).collect()
    }

    /// Move the sprite in `slot` to `pos`, keeping its id and reward.
    pub(crate) fn move_to(&mut self, slot: usize, pos: (usize, usize)) {
        self.entries[slot].set_grid_pos(pos);
    }

    /// Empty every slot holding `kind`: id, position and reward zeroed.
    pub(crate) fn clear(&mut self, kind: SpriteKind) {
        for e in self.entries.iter_mut().filter(|e| e.kind() == kind) {
            *e = SlotEntry { id: ID_EMPTY, y: 0, xhi: 0, xlo: 0, reward: 0 };
        }
    }

    /// Put a stationary sprite in the highest empty slot. `None`, and nothing
    /// changed, when the world has no empty slot.
    pub(crate) fn place_high(&mut self, id: u8, pos: (usize, usize), reward: u8) -> Option<usize> {
        let slot = self.last_empty()?;
        self.set(slot, id, pos, reward);
        Some(slot)
    }

    /// Put `id` in `slot` at `pos` with `reward`.
    pub(crate) fn set(&mut self, slot: usize, id: u8, pos: (usize, usize), reward: u8) {
        let e = &mut self.entries[slot];
        e.id = id;
        e.reward = reward;
        e.set_grid_pos(pos);
    }

    /// Put `id` in `slot` at `pos`, keeping the slot's reward byte.
    pub(crate) fn put(&mut self, slot: usize, id: u8, pos: (usize, usize)) {
        self.entries[slot].id = id;
        self.entries[slot].set_grid_pos(pos);
    }

    /// Empty one slot: id, position and reward zeroed.
    pub(crate) fn clear_slot(&mut self, slot: usize) {
        self.entries[slot] = SlotEntry { id: ID_EMPTY, y: 0, xhi: 0, xlo: 0, reward: 0 };
    }

    /// Put the world's canoe at `pos`. The engine finds one canoe per map, so
    /// a slot already holding one is the canoe; a world with none (a generated
    /// World 7 lagoon) takes the highest empty slot. `false`, and nothing
    /// changed, when there is no slot to take.
    pub(crate) fn place_canoe(&mut self, pos: (usize, usize)) -> bool {
        let existing = (0..MAP_OBJ_SLOTS).find(|&s| self.entries[s].kind() == SpriteKind::Canoe);
        let Some(slot) = existing.or_else(|| self.last_empty()) else {
            return false;
        };
        self.set(slot, ID_CANOE, pos, 0);
        true
    }

    /// Read world `world`'s slots, let `edit` change them, and write back what
    /// changed. For the passes that run before the build has a ledger of its
    /// own — the map fixes, the map source, the plant clear — so they claim
    /// slots through the same rules and the same writer.
    pub(crate) fn edit<T>(
        rom: &mut Rom,
        world: usize,
        edit: impl FnOnce(&mut WorldSprites) -> T,
    ) -> T {
        let mut w = WorldSprites::read(rom, world);
        let out = edit(&mut w);
        w.write(rom);
        out
    }
}

/// `MAPOBJ_CANOE`.
pub(crate) const ID_CANOE: u8 = 0x10;

#[cfg(test)]
mod tests {
    use super::*;

    fn vanilla() -> Option<Rom> {
        let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&bytes).ok()
    }

    /// Reading a world and writing it straight back changes nothing — the
    /// property every byte-identical claim rests on.
    #[test]
    fn a_round_trip_writes_nothing() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut out = rom.clone();
        for wi in 0..8 {
            WorldSprites::read(&rom, wi).write(&mut out);
        }
        assert!(out.read_range(0, 0x60010) == rom.read_range(0, 0x60010));
    }

    /// An edit writes its own slot's bytes and leaves every other slot's —
    /// leftover position bytes in empty slots included — exactly as read.
    #[test]
    fn an_edit_writes_only_its_slot() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut w1 = WorldSprites::read(&rom, 0);
        let slot = w1.place_high(0x07, (4, 9), 0).expect("W1 has an empty slot");
        assert_eq!(w1.entries[slot].grid_pos(), Some((4, 9)));
        let mut out = rom.clone();
        w1.write(&mut out);
        let back = WorldSprites::read(&out, 0);
        for s in 0..MAP_OBJ_SLOTS {
            if s == slot {
                assert_eq!(back.entries[s].kind(), SpriteKind::Plant);
            } else {
                assert_eq!(
                    back.entries[s],
                    WorldSprites::read(&rom, 0).entries[s],
                    "slot {s} moved"
                );
            }
        }
    }

    /// The rules phase A keeps: Hammer Bros never take slot 0 or (W1-W7) slot
    /// 1, and stationary sprites fill from the top.
    #[test]
    fn hammer_bros_start_above_the_reserved_slots() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        for wi in 0..8 {
            let w = WorldSprites::read(&rom, wi);
            let first = if wi == W8_IDX { 1 } else { 2 };
            assert!(w.hb_eligible().iter().all(|&s| s >= first), "W{}", wi + 1);
            if let Some(top) = w.last_empty() {
                assert!(
                    w.hb_eligible()
                        .iter()
                        .all(|&s| s <= top || w.entries[s].kind() != SpriteKind::Empty)
                );
            }
        }
    }
}

#[cfg(test)]
mod phase_b {
    use super::*;

    fn vanilla() -> Option<Rom> {
        let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&bytes).ok()
    }

    /// Opening the airship slot makes slot 1 a Hammer Bro slot in W1-W7 and
    /// adds exactly one to the room; World 8 had it already.
    #[test]
    fn the_airship_slot_adds_one_slot_in_worlds_1_to_7() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        for wi in 0..8 {
            let closed = WorldSprites::read(&rom, wi);
            let mut open = closed.clone();
            open.open_airship_slot();
            let gain = usize::from(wi != W8_IDX);
            assert_eq!(open.room(), closed.room() + gain, "W{}", wi + 1);
            assert_eq!(open.hb_eligible().first(), Some(&1), "W{}: slot 1 not eligible", wi + 1);
        }
    }

    /// The plant reservation comes off the Hammer Bros' room and not the
    /// plants'.
    #[test]
    fn the_plant_reservation_is_the_plants() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut w7 = WorldSprites::read(&rom, 6);
        let (room, plants) = (w7.room(), w7.room_for_plants());
        w7.reserve_for_plants(2);
        assert_eq!(w7.room(), room - 2);
        assert_eq!(w7.room_for_plants(), plants);
    }

    /// World 8's level tank takes the parked HELP's slot 0 and frees its own;
    /// any other world, or a second call, changes nothing.
    #[test]
    fn the_w8_tank_takes_slot_0() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut w8 = WorldSprites::read(&rom, W8_IDX);
        let tank = w8.entries[2];
        assert!(w8.take_w8_help_slot(2));
        assert_eq!(w8.entries[0], tank);
        assert_eq!(w8.entries[2].kind(), SpriteKind::Empty);
        assert!(!w8.take_w8_help_slot(2), "a second call must not move anything");
        let mut w1 = WorldSprites::read(&rom, 0);
        assert!(!w1.take_w8_help_slot(2), "only World 8's HELP is free");
    }
}

#[cfg(test)]
mod ownership {
    /// **Only this module writes the map-object tables.** Every other file is
    /// scanned for a `write_byte`/`write_range` call naming a map-object table
    /// or offset helper; a writer that turns up here should claim its slot
    /// through [`super::WorldSprites`] instead, so the slot rules and the
    /// snapshot-diff write keep holding.
    ///
    /// A line scan, so it catches the direct form — the only one the code has
    /// ever used — and not an offset computed on one line and written on the
    /// next. That second form would also have to call `map_obj_slot_offset`
    /// or `map_obj_reward_offset`, which only readers do.
    #[test]
    fn only_sprite_slots_writes_the_map_object_tables() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("src is readable") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        walk(std::path::Path::new("src"), &mut files);
        assert!(files.len() > 100, "the scan found only {} files — wrong root?", files.len());
        let mut offenders = Vec::new();
        for path in files {
            if path.ends_with("overworld/sprite_slots.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("source is readable");
            for (n, line) in text.lines().enumerate() {
                let writes = line.contains("write_byte(") || line.contains("write_range(");
                let table = line.contains("map_obj_") || line.contains("MAP_OBJ_");
                if writes && table {
                    offenders.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "map-object tables written outside sprite_slots:\n{}",
            offenders.join("\n")
        );
    }
}
