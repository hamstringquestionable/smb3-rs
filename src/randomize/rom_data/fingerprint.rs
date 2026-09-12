//! A stable hash of the bytes that describe the overworld.
//!
//! [`tests/overworld_baseline.rs`] compares this against a committed constant,
//! so that a change to the map a player walks is caught the moment it happens
//! rather than a fortnight later.
//!
//! **Why not hash the whole ROM.** That is what the baseline used to do, and it
//! moved on a version bump, on three new king quotes, on a 22-byte title
//! routine changing address, on any always-on splice — none of which is an
//! overworld change. Twenty recaptures in a month, each one paid for with a
//! paragraph of manual attribution proving the diff landed somewhere harmless.
//! A guard that is usually red is a guard nobody reads, and it went stale twice
//! for exactly that reason (see `docs/overworld_baseline_log.md`).
//!
//! Whole-ROM identity is still worth having, but as an on-demand check with no
//! committed constant to rot — that is `tests/rom_identity.rs`.
//!
//! **What is in.** Only data the engine reads to draw and walk a map: terrain,
//! where each tile leads, the sprites standing on it, where pipes come out, and
//! which fortress opens which lock. Every region is named by a `rom_data`
//! constant, never by an offset spelled out here — `tools/offset_dups.py`
//! exists to keep it that way.
//!
//! **What moves it, and should.** A real topology change. And an RNG-stream
//! shift from a new draw anywhere upstream, because that genuinely re-deals
//! every seed's map — the difference is that saying so takes a line, not a
//! paragraph.
//!
//! **What is out.** 6502 code, including the routines that *act on* these
//! tables. A patch that rewrites the lock-breaking routine byte for byte
//! changes no map, and the shape of that routine is the `asm::check` tests'
//! business, not this one's.

use super::*;
use crate::rom::Rom;

/// FNV-1a. Rolled by hand rather than using `DefaultHasher`, whose output is
/// explicitly not guaranteed stable across Rust releases — a baseline that
/// changes when the toolchain updates is worse than none.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    fn eat(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn eat_byte(&mut self, b: u8) {
        self.eat(&[b]);
    }
}

/// Hash the overworld-describing bytes of a finished ROM.
///
/// Stable across toolchains and across every part of the ROM that is not the
/// overworld. See the module docs for what that covers and why.
pub fn overworld_fingerprint(rom: &Rom) -> u64 {
    let mut h = Fnv::new();

    // 1. Terrain. Grids are screen-major, `ROWS * 16` bytes per screen, so the
    //    whole block is exactly the tiles and nothing else.
    for info in &MAP_TILE_GRIDS {
        h.eat(rom.read_range(info.file_offset, info.screens * ROWS * 16));
    }

    // 2. Where each tile leads: the five parallel per-entry arrays (row/type,
    //    screen-column, and the object and layout pointer halves).
    for world in &WORLDS {
        h.eat(rom.read_range(world.rowtype_offset, world.entry_count * 5));
    }

    // 3. The sprites standing on the map. The master words first — a repointed
    //    sub-table is a change even if the bytes it now names happen to match —
    //    then every slot's position and type, then its reward.
    for master in [MAP_OBJ_YS_MASTER, MAP_OBJ_XHIS_MASTER, MAP_OBJ_XLOS_MASTER, MAP_OBJ_IDS_MASTER]
    {
        h.eat(rom.read_range(master, WORLDS.len() * 2));
        for world_idx in 0..WORLDS.len() {
            for slot in 0..MAP_OBJ_SLOTS {
                h.eat_byte(rom.read_byte(map_obj_slot_offset(rom, master, world_idx, slot)));
            }
        }
    }
    h.eat(rom.read_range(MAP_OBJ_REWARDS, WORLDS.len() * MAP_OBJ_SLOTS));

    // 4. Where a pipe puts the player down.
    for base in [PIPE_MAP_XHI, PIPE_MAP_X, PIPE_MAP_Y, PIPE_MAP_SCRL_XHI] {
        h.eat(rom.read_range(base, PIPE_DEST_LEN));
    }

    // 5. Which fortress opens which lock — position-keyed, so it describes the
    //    map rather than the routine that reads it.
    h.eat(rom.read_range(FS_LOCK_ENTRIES, crate::randomize::lock_keys::ENTRIES_RESERVED));

    h.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vanilla() -> Option<Rom> {
        let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Some(Rom::from_bytes(&bytes).unwrap())
    }

    /// A differential test, not a golden one: flip one byte and say whether the
    /// fingerprint was supposed to notice. This is the whole claim the harness
    /// rests on, and the old whole-ROM hash could not have passed the first
    /// half of it.
    #[test]
    fn it_watches_the_overworld_and_nothing_else() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        let base = overworld_fingerprint(&rom);

        // Bytes it must ignore. Each is a place a real change has forced a
        // whole-ROM recapture at least once — see
        // `docs/overworld_baseline_log.md`.
        for (off, what) in [
            (FS_SEED_HASH_DATA, "the title-screen seed icons"),
            (FS_SEED_STAMP, "the flag-key stamp"),
            (ENEMY_DATA_START, "an enemy entry"),
        ] {
            let mut m = rom.clone();
            m.write_byte(off, rom.read_byte(off) ^ 0xFF);
            assert_eq!(
                overworld_fingerprint(&m),
                base,
                "the fingerprint moved for {what} ({off:#07X}), which is not the overworld"
            );
        }

        // Bytes it must notice, one per region it claims to cover.
        let map_obj_id = map_obj_slot_offset(&rom, MAP_OBJ_IDS_MASTER, 0, 2);
        for (off, what) in [
            (MAP_TILE_GRIDS[0].file_offset, "a W1 map tile"),
            (WORLDS[0].rowtype_offset, "a W1 pointer table entry"),
            (map_obj_id, "a map-object sprite id"),
            (map_obj_reward_offset(0, 2), "a map-object reward"),
            (PIPE_MAP_Y, "a pipe destination"),
            (FS_LOCK_ENTRIES, "a lock's key"),
        ] {
            let mut m = rom.clone();
            m.write_byte(off, rom.read_byte(off) ^ 0xFF);
            assert_ne!(
                overworld_fingerprint(&m),
                base,
                "the fingerprint ignored {what} ({off:#07X}), which is the overworld"
            );
        }
    }

    /// Reading the fingerprint must not change the ROM — it is run on output
    /// the tests then go on to compare by other means.
    #[test]
    fn it_is_a_pure_read() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        let before = rom.clone();
        overworld_fingerprint(&rom);
        assert_eq!(rom.data, before.data, "the fingerprint wrote to the ROM");
    }
}
