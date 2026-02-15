use crate::rom::Rom;

/// Debug mode toggle byte. 0xCC = enabled, 0x35 = disabled.
/// When enabled, pressing Select cycles through powerup forms in-game.
const DEBUG_MODE_OFFSET: usize = 0x309D5;

/// Starting lives value byte (LDA #imm operand).
/// Both Mario and Luigi are initialized from this single byte.
const STARTING_LIVES_OFFSET: usize = 0x308E1;

/// Enable debug mode: press Select to cycle through powerup forms.
pub fn enable_debug_mode(rom: &mut Rom) {
    rom.write_byte(DEBUG_MODE_OFFSET, 0xCC);
}

/// Set starting lives for both Mario and Luigi (1–99).
pub fn set_starting_lives(rom: &mut Rom, lives: u8) {
    let clamped = lives.min(99).max(1);
    rom.write_byte(STARTING_LIVES_OFFSET, clamped);
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
        // Set original values
        data[DEBUG_MODE_OFFSET] = 0x35;
        data[STARTING_LIVES_OFFSET] = 0x04;
        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_debug_mode_enabled() {
        let mut rom = make_test_rom();
        assert_eq!(rom.read_byte(DEBUG_MODE_OFFSET), 0x35);
        enable_debug_mode(&mut rom);
        assert_eq!(rom.read_byte(DEBUG_MODE_OFFSET), 0xCC);
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
}
