//! Water enemies can be stomped when Mario is on dry land.
//!
//! Stompability is `OA3_NOTSTOMPABLE`, bit 5 of each object's
//! `ObjectGroup_Attributes3` byte. The engine's stomp test (`prg000.asm`
//! `PRG000_D253`) checks `Player_InWater` *before* it reads that bit, so a
//! swimming Mario still can never stomp anything: clearing the bit only
//! changes what happens when Mario lands on one of these from dry land —
//! which Wild water enemies make common. Every ID here already has
//! `OA2_NOSHELLORSQUASH`, so a stomp kicks it off-screen like a jumping Cheep.
//!
//! Lava Lotus (`$67`) keeps the bit on purpose: it stays a hazard plant.

use crate::rom::Rom;

/// `OA3_NOTSTOMPABLE`.
const NOT_STOMPABLE: u8 = 0x20;

/// Swimming water enemies that vanilla marks unstompable.
const IDS: [u8; 7] = [
    0x48, // OBJ_TINYCHEEPCHEEP (baby blooper in our tables)
    0x61, // OBJ_BLOOPERWITHKIDS
    0x62, // OBJ_BLOOPER
    0x63, // OBJ_BIGBERTHABIRTHER
    0x6A, // OBJ_BLOOPERCHILDSHOOT
    0x77, // OBJ_GREENCHEEP
    0x88, // OBJ_ORANGECHEEP
];

/// File offset of `ObjectGroup_Attributes3` for `id`. Group N (36 IDs each)
/// lives in PRG00(N+1), and every group bank `.org`s the table at `$A120`.
fn attr3_offset(id: u8) -> usize {
    let group = id as usize / 0x24;
    0x10 + (group + 1) * 0x2000 + 0x120 + id as usize % 0x24
}

pub fn apply(rom: &mut Rom) {
    for id in IDS {
        let off = attr3_offset(id);
        let v = rom.read_byte(off);
        rom.write_byte(off, v & !NOT_STOMPABLE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Poison mushroom's hard-coded group-0 table address pins the formula.
    #[test]
    fn attr3_offset_matches_known_tables() {
        assert_eq!(attr3_offset(0x0A), 0x02130 + 0x0A);
        // PRG003 $A120 + ($48 - $48), PRG004 $A120 + ($88 - $6C)
        assert_eq!(attr3_offset(0x48), 0x06130);
        assert_eq!(attr3_offset(0x88), 0x08130 + 0x1C);
    }

    /// Every target starts unstompable, only the bit moves, and Lava Lotus
    /// keeps it. Skips where the ROM is absent.
    #[test]
    fn clears_only_the_stomp_bit() {
        let Some(mut rom) = crate::randomize::maze::key_sites::test_support::load_rom() else {
            return;
        };
        let before: Vec<u8> = IDS.iter().map(|&id| rom.read_byte(attr3_offset(id))).collect();
        let lotus = rom.read_byte(attr3_offset(0x67));
        apply(&mut rom);
        for (&id, &b) in IDS.iter().zip(&before) {
            assert_ne!(b & NOT_STOMPABLE, 0, "${id:02X} was already stompable");
            assert_eq!(rom.read_byte(attr3_offset(id)), b & !NOT_STOMPABLE);
        }
        assert_eq!(rom.read_byte(attr3_offset(0x67)), lotus);
    }
}
