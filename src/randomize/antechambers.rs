//! Antechamber shuffle: ten levels open with an entry area whose pipe
//! leads to the level's interior (4-3, 5-2, 5-3, 6-6, 6-9, 7-1, 7-4,
//! 7-5, 7-6, 7-7). This module shuffles which interior each entry
//! area's pipe drops into, so walking into one level's front door can
//! land you inside another's. The interior's own exit transition is
//! untouched — the player finishes through the donor level's vanilla
//! ending (4-3's interior carries its own goal; the others loop back to
//! their own entry area's end side), and map completion still credits
//! the tile they entered from.
//!
//! Two writes per reassignment (see "Junction Spawn Positions" in
//! docs/smb3_rom_reference.md):
//!
//! 1. The entry area's header alt pointers (bytes 0-3, byte 6 low
//!    nibble) are re-pointed at the donor's interior.
//! 2. Every junction command in the entry area gets its bytes 1-2
//!    replaced with the donor's front-door command bytes. Spawn
//!    coordinates for a pipe transition live in the *source* area's
//!    junction command, so the host pipes must carry the arrival
//!    position (and vertical-mode flag) the donor interior expects.
//!    Byte 0 stays: its low nibble is the spawn-slot index, which must
//!    keep matching each host pipe's screen position.
//!
//! The entry header's timer bits (byte 8 bits 6-7) also follow the
//! interior so the clock fits the level actually being played.

use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;

use crate::rom::Rom;

/// One antechamber-pattern level (pool emitted by
/// `tools/rom_map.py --antechamber`).
struct Antechamber {
    /// Vanilla level name, for panic messages.
    name: &'static str,
    /// File offset of the entry area's 9-byte layout header.
    header: usize,
    /// File offsets of the entry area's 3-byte junction commands. All of
    /// them receive the donor's spawn bytes when hosting; the FIRST is
    /// the canonical front-door command whose bytes serve as this
    /// level's donor data (matters for 5-2, whose second pipe is a
    /// mid-level re-entry at different coordinates).
    junctions: &'static [usize],
}

const ANTECHAMBERS: [Antechamber; 10] = [
    Antechamber { name: "4-3", header: 0x2701F, junctions: &[0x27073] },
    Antechamber { name: "5-2", header: 0x1A587, junctions: &[0x1A804, 0x1A807] },
    Antechamber { name: "5-3", header: 0x1EC26, junctions: &[0x1EC4A] },
    Antechamber { name: "6-6", header: 0x23941, junctions: &[0x23990] },
    Antechamber { name: "6-9", header: 0x23CFE, junctions: &[0x23D17] },
    Antechamber { name: "7-1", header: 0x1EA71, junctions: &[0x1EA94, 0x1EAA2] },
    Antechamber { name: "7-4", header: 0x1F392, junctions: &[0x1F3BC] },
    Antechamber { name: "7-5", header: 0x1FCF9, junctions: &[0x1FD3F] },
    Antechamber { name: "7-6", header: 0x1F342, junctions: &[0x1F38E] },
    Antechamber { name: "7-7", header: 0x1EAB8, junctions: &[0x1EAEB] },
];

/// Everything that must travel with an interior when it is reassigned
/// to another level's entry room.
struct Interior {
    /// Header bytes 0-3: alt_layout + alt_objects (little-endian pairs).
    alt_ptrs: [u8; 4],
    /// Header byte 6 low nibble: alt_tileset.
    tileset: u8,
    /// Junction command bytes 1-2: arrival position + exit animation.
    spawn: [u8; 2],
    /// Header byte 8 bits 6-7: timer setting.
    timer: u8,
}

/// Randomly permute which interior each antechamber level's entry pipe
/// leads to. Identity assignments are allowed (a level may keep its own
/// interior) and skip their writes entirely.
pub fn shuffle(rom: &mut Rom, rng: &mut ChaCha8Rng) {
    // Snapshot all vanilla interiors before any writes, so a permutation
    // never reads a value another assignment already overwrote.
    let interiors: Vec<Interior> = ANTECHAMBERS
        .iter()
        .map(|a| {
            let hdr = rom.read_range(a.header, 9);
            for &j in a.junctions {
                assert_eq!(
                    rom.read_byte(j) & 0xE0,
                    0xE0,
                    "antechambers: {}: no junction command at 0x{j:05X}",
                    a.name,
                );
            }
            let front_door = a.junctions[0];
            Interior {
                alt_ptrs: [hdr[0], hdr[1], hdr[2], hdr[3]],
                tileset: hdr[6] & 0x0F,
                spawn: [rom.read_byte(front_door + 1), rom.read_byte(front_door + 2)],
                timer: hdr[8] & 0xC0,
            }
        })
        .collect();

    let mut assignment: Vec<usize> = (0..ANTECHAMBERS.len()).collect();
    assignment.shuffle(rng);

    for (host_idx, &donor_idx) in assignment.iter().enumerate() {
        if donor_idx == host_idx {
            continue; // keeps its own interior — leave vanilla bytes alone
        }
        let host = &ANTECHAMBERS[host_idx];
        let donor = &interiors[donor_idx];

        rom.write_range(host.header, &donor.alt_ptrs);
        let b6 = rom.read_byte(host.header + 6);
        rom.write_byte(host.header + 6, (b6 & 0xF0) | donor.tileset);
        let b8 = rom.read_byte(host.header + 8);
        rom.write_byte(host.header + 8, (b8 & 0x3F) | donor.timer);
        for &j in host.junctions {
            rom.write_range(j + 1, &donor.spawn);
        }
    }
}

#[cfg(test)]
mod tests {
    use rand::SeedableRng;

    use super::*;

    /// Build a minimal ROM with distinct fake header + junction bytes at
    /// each antechamber location so shuffles are observable.
    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        for (i, a) in ANTECHAMBERS.iter().enumerate() {
            let n = i as u8;
            // Header: distinct alt pointers, byte 6 = scroll bits (upper
            // nibble) + tileset, byte 8 = timer (bits 6-7) + music.
            let hdr = [
                0x10 + n, 0xA0, 0x20 + n, 0xC0, // alt_layout / alt_objects
                0x0A,                            // screens
                0x00,                            // palettes
                0x20 | (n & 0x0F),               // scroll flag + tileset n
                0x00,                            // init action
                ((n & 3) << 6) | 0x05,           // timer + music 5
            ];
            data[a.header..a.header + 9].copy_from_slice(&hdr);
            // Junction commands: distinct slot nibble per command, spawn
            // bytes distinct per level AND per command (the second command
            // must be observably different from the front door).
            for (k, &j) in a.junctions.iter().enumerate() {
                let k = k as u8;
                data[j] = 0xE0 | ((n + k) & 0x0F);
                data[j + 1] = 0x40 + n + (k << 4);
                data[j + 2] = 0x80 + n + (k << 4);
            }
        }

        Rom::from_bytes_lax(&data, true).unwrap()
    }

    fn interior_tuple(rom: &Rom, a: &Antechamber) -> (Vec<u8>, u8, u8, u8, u8) {
        let hdr = rom.read_range(a.header, 9);
        (
            hdr[0..4].to_vec(),
            hdr[6] & 0x0F,
            hdr[8] & 0xC0,
            rom.read_byte(a.junctions[0] + 1),
            rom.read_byte(a.junctions[0] + 2),
        )
    }

    #[test]
    fn shuffle_is_a_permutation_of_interiors() {
        let mut rom = make_test_rom();
        let vanilla: Vec<_> = ANTECHAMBERS.iter().map(|a| interior_tuple(&rom, a)).collect();

        let mut rng = ChaCha8Rng::seed_from_u64(1);
        shuffle(&mut rom, &mut rng);

        let mut shuffled: Vec<_> =
            ANTECHAMBERS.iter().map(|a| interior_tuple(&rom, a)).collect();
        let mut expected = vanilla.clone();
        shuffled.sort();
        expected.sort();
        assert_eq!(shuffled, expected, "every interior appears exactly once");
    }

    #[test]
    fn host_local_bytes_are_preserved() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        shuffle(&mut rom, &mut rng);

        for (i, a) in ANTECHAMBERS.iter().enumerate() {
            let n = i as u8;
            // Junction byte 0 (slot index) must keep matching each host pipe.
            for (k, &j) in a.junctions.iter().enumerate() {
                assert_eq!(rom.read_byte(j), 0xE0 | ((n + k as u8) & 0x0F));
            }
            // Header byte 6 upper nibble (scroll flags) stays the host's.
            assert_eq!(rom.read_byte(a.header + 6) & 0xF0, 0x20);
            // Header byte 8 music bits stay the host's.
            assert_eq!(rom.read_byte(a.header + 8) & 0x3F, 0x05);
        }
    }

    #[test]
    fn reassigned_hosts_write_donor_bytes_to_every_pipe() {
        let mut rom = make_test_rom();
        let vanilla: Vec<_> =
            ANTECHAMBERS.iter().map(|a| interior_tuple(&rom, a)).collect();
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        shuffle(&mut rom, &mut rng);

        for (a, before) in ANTECHAMBERS.iter().zip(&vanilla) {
            let moved = interior_tuple(&rom, a) != *before;
            if !moved {
                continue;
            }
            // A reassigned host must carry the donor's spawn bytes on ALL
            // of its pipes, not just the front door.
            let front = [
                rom.read_byte(a.junctions[0] + 1),
                rom.read_byte(a.junctions[0] + 2),
            ];
            for &j in a.junctions {
                assert_eq!(
                    [rom.read_byte(j + 1), rom.read_byte(j + 2)],
                    front,
                    "{}: pipe at 0x{j:05X} out of sync",
                    a.name
                );
            }
        }
    }

    #[test]
    fn same_seed_same_result() {
        let mut rom_a = make_test_rom();
        let mut rom_b = make_test_rom();
        let mut rng_a = ChaCha8Rng::seed_from_u64(42);
        let mut rng_b = ChaCha8Rng::seed_from_u64(42);
        shuffle(&mut rom_a, &mut rng_a);
        shuffle(&mut rom_b, &mut rng_b);

        for a in &ANTECHAMBERS {
            assert_eq!(
                rom_a.read_range(a.header, 9),
                rom_b.read_range(a.header, 9)
            );
            for &j in a.junctions {
                assert_eq!(rom_a.read_range(j, 3), rom_b.read_range(j, 3));
            }
        }
    }

    /// Guard the hardcoded offsets against the real ROM: every entry must
    /// hold a junction command, and the header alt pointers must match the
    /// values traced by `tools/rom_map.py --antechamber`. Skipped when the
    /// ROM isn't present.
    #[test]
    fn vanilla_offsets_match_real_rom() {
        let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            return;
        };
        let rom = Rom::from_bytes(&bytes).unwrap();

        // (alt_layout, alt_objects, alt_tileset, junction slots,
        //  front-door bytes 1-2) — values from `rom_map.py --antechamber`.
        // Byte 1 packs vertical(7) | ystart(6-4) | exit_dir(3-0); byte 2
        // packs spawn column(7-4) | spawn screen(3-0).
        // Reason: one-off test fixture row; a named type would just move
        // the field legend away from the data.
        #[allow(clippy::type_complexity)]
        let expected: [(u16, u16, u8, &[u8], [u8; 2]); 10] = [
            (0xB6D5, 0xC863, 3, &[2], [0x52, 0x20]),    // 4-3
            (0xB481, 0xCE4B, 8, &[0, 4], [0x82, 0x20]), // 5-2 (vert shaft)
            (0xAC3E, 0xC29E, 1, &[0], [0x02, 0x67]),    // 5-3
            (0xACDC, 0xC64B, 3, &[0], [0x12, 0x20]),    // 6-6
            (0xA9D7, 0xC60E, 3, &[0], [0x02, 0x40]),    // 6-9
            (0xAB97, 0xCD93, 8, &[0, 1], [0xF8, 0x27]), // 7-1 (vert shaft)
            (0xADC4, 0xCDC2, 6, &[0], [0x53, 0x20]),    // 7-4
            (0xA5CD, 0xC171, 1, &[0], [0x52, 0x20]),    // 7-5
            (0xB600, 0xCE56, 8, &[1], [0xF8, 0x27]),    // 7-6 (vert shaft)
            (0xBD2F, 0xCD35, 4, &[0], [0x73, 0x20]),    // 7-7
        ];

        for (a, (lay, obj, ts, slots, spawn)) in ANTECHAMBERS.iter().zip(expected) {
            let hdr = rom.read_range(a.header, 9);
            assert_eq!(u16::from_le_bytes([hdr[0], hdr[1]]), lay, "{} alt_layout", a.name);
            assert_eq!(u16::from_le_bytes([hdr[2], hdr[3]]), obj, "{} alt_objects", a.name);
            assert_eq!(hdr[6] & 0x0F, ts, "{} alt_tileset", a.name);
            assert_eq!(a.junctions.len(), slots.len(), "{} junction count", a.name);
            for (&j, &slot) in a.junctions.iter().zip(slots) {
                assert_eq!(rom.read_byte(j) & 0xE0, 0xE0, "{} junction cmd", a.name);
                assert_eq!(rom.read_byte(j) & 0x0F, slot, "{} junction slot", a.name);
            }
            let front = a.junctions[0];
            assert_eq!(
                [rom.read_byte(front + 1), rom.read_byte(front + 2)],
                spawn,
                "{} front-door spawn bytes",
                a.name
            );
        }
    }
}
