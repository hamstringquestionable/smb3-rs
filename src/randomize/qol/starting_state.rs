//! Starting lives and inventory.

use crate::rom::Rom;
use crate::randomize::rom_data::FS_STARTING_ITEMS;

/// Starting lives value byte (LDA #imm operand).
/// Both Mario and Luigi are initialized from this single byte.
const STARTING_LIVES_OFFSET: usize = 0x308E1;

/// Base of the 8-byte lives init code: LDA #lives; STA $0736; STA $0737.
const LIVES_INIT_BASE: usize = 0x308E0;

/// Set starting lives for both Mario and Luigi (1–99).
pub fn set_starting_lives(rom: &mut Rom, lives: u8) {
    let clamped = lives.clamp(1, 99);
    rom.write_byte(STARTING_LIVES_OFFSET, clamped);
}

/// Write starting items into Mario's inventory via a trampoline in PRG031.
///
/// Replaces the 8-byte lives init at 0x308E0 with `JSR $E250`
/// (FS_STARTING_ITEMS) into a routine that sets lives, does the intro skip,
/// queues the seeded menu music, AND writes up to 3 items to inventory
/// ($7D80+).
///
/// Must run AFTER title_screen: both patch the lives-init region, and this
/// JSR overwrites title_screen's intro-skip hook at 0x308E2. The trampoline
/// replays the identical intro-skip + menu-music bytes (shared
/// `title_screen::intro_skip_music_bytes`), so behavior is preserved;
/// title_screen's FS_INTRO_SKIP routine is left in ROM unreferenced.
pub fn write_starting_items(rom: &mut Rom, seed: u64, lives: u8, items: &[u8]) {
    let lives = lives.clamp(1, 99);
    let cpu = crate::randomize::rom_data::prg031_file_to_cpu(FS_STARTING_ITEMS); // $E250
    // Build trampoline: lives init + intro skip + menu music + item writes + RTS
    let mut buf = Vec::with_capacity(33);
    buf.extend_from_slice(&[
        0xA9, lives,         // LDA #lives
        0x8D, 0x36, 0x07,    // STA $0736
        0x8D, 0x37, 0x07,    // STA $0737
    ]);
    buf.extend_from_slice(&crate::randomize::title_screen::intro_skip_music_bytes(seed));
    for (i, &item) in items.iter().take(3).enumerate() {
        buf.extend_from_slice(&[
            0xA9, item,                      // LDA #item
            0x8D, (0x80 + i as u8), 0x7D,    // STA $7D80+i
        ]);
    }
    buf.push(0x60); // RTS
    rom.write_range(FS_STARTING_ITEMS, &buf);

    // Patch lives init: JSR $E250 + NOP×5 (overwrites the title_screen
    // intro-skip hook at 0x308E2 — see the doc comment above).
    rom.write_range(LIVES_INIT_BASE, &[
        0x20, cpu as u8, (cpu >> 8) as u8,   // JSR $E250
        0xEA, 0xEA, 0xEA, 0xEA, 0xEA,       // NOP ×5
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::qol::test_support::make_test_rom;

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
