use crate::rom::Rom;

/// Starting lives value byte (LDA #imm operand).
/// Both Mario and Luigi are initialized from this single byte.
const STARTING_LIVES_OFFSET: usize = 0x308E1;

// W3 drawbridge map tile offsets (2× $B2 horizontal, 2× $B1 vertical)
const W3_BRIDGE_H1: usize = 0x18777;
const W3_BRIDGE_H2: usize = 0x18779;
const W3_BRIDGE_V1: usize = 0x1880C;
const W3_BRIDGE_V2: usize = 0x188F3;

// Toggle code: LDA $07BB; EOR #$01; STA $07BB (8 bytes at 0x14A6B)
const W3_TOGGLE_OFFSET: usize = 0x14A6B;
const W3_TOGGLE_LEN: usize = 8;

// W2 rock blocking secret path (screen 1, row 0, col 5) — $51 → $45
const W2_SECRET_ROCK: usize = 0x186E0;

/// Set starting lives for both Mario and Luigi (1–99).
pub fn set_starting_lives(rom: &mut Rom, lives: u8) {
    let clamped = lives.min(99).max(1);
    rom.write_byte(STARTING_LIVES_OFFSET, clamped);
}

/// Remove the W2 rock blocking the secret path, replacing it with horizontal path.
pub fn remove_w2_rock(rom: &mut Rom) {
    rom.write_byte(W2_SECRET_ROCK, 0x45);
}

/// Replace W3 drawbridge tiles with normal path tiles and NOP the toggle code.
pub fn fix_w3_drawbridges(rom: &mut Rom) {
    // Replace horizontal drawbridge tiles ($B2) with horizontal path ($45)
    rom.write_byte(W3_BRIDGE_H1, 0x45);
    rom.write_byte(W3_BRIDGE_H2, 0x45);
    // Replace vertical drawbridge tiles ($B1) with vertical path ($46)
    rom.write_byte(W3_BRIDGE_V1, 0x46);
    rom.write_byte(W3_BRIDGE_V2, 0x46);
    // NOP out the toggle code (LDA $07BB; EOR #$01; STA $07BB)
    rom.write_range(W3_TOGGLE_OFFSET, &[0xEA; W3_TOGGLE_LEN]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::Rom;

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        data[STARTING_LIVES_OFFSET] = 0x04;
        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_starting_lives() {
        let mut rom = make_test_rom();
        assert_eq!(rom.read_byte(STARTING_LIVES_OFFSET), 0x04);
        set_starting_lives(&mut rom, 99);
        assert_eq!(rom.read_byte(STARTING_LIVES_OFFSET), 99);
    }

    #[test]
    fn test_starting_lives_clamped() {
        let mut rom = make_test_rom();
        set_starting_lives(&mut rom, 255);
        assert_eq!(rom.read_byte(STARTING_LIVES_OFFSET), 99);
        set_starting_lives(&mut rom, 0);
        assert_eq!(rom.read_byte(STARTING_LIVES_OFFSET), 1);
    }

    #[test]
    fn test_remove_w2_rock() {
        let mut rom = make_test_rom();
        rom.write_byte(W2_SECRET_ROCK, 0x51);
        remove_w2_rock(&mut rom);
        assert_eq!(rom.read_byte(W2_SECRET_ROCK), 0x45);
    }

    #[test]
    fn test_fix_w3_drawbridges() {
        let mut rom = make_test_rom();
        // Place original drawbridge tiles
        rom.write_byte(W3_BRIDGE_H1, 0xB2);
        rom.write_byte(W3_BRIDGE_H2, 0xB2);
        rom.write_byte(W3_BRIDGE_V1, 0xB1);
        rom.write_byte(W3_BRIDGE_V2, 0xB1);
        // Place original toggle code
        rom.write_range(W3_TOGGLE_OFFSET, &[0xAD, 0xBB, 0x07, 0x49, 0x01, 0x8D, 0xBB, 0x07]);

        fix_w3_drawbridges(&mut rom);

        assert_eq!(rom.read_byte(W3_BRIDGE_H1), 0x45);
        assert_eq!(rom.read_byte(W3_BRIDGE_H2), 0x45);
        assert_eq!(rom.read_byte(W3_BRIDGE_V1), 0x46);
        assert_eq!(rom.read_byte(W3_BRIDGE_V2), 0x46);
        assert_eq!(rom.read_range(W3_TOGGLE_OFFSET, W3_TOGGLE_LEN), &[0xEA; 8]);
    }
}
