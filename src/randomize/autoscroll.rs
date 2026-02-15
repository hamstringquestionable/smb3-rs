use crate::rom::Rom;

/// Enemy/object data region in the ROM.
const ENEMY_DATA_START: usize = 0x0BFD8;
const ENEMY_DATA_END: usize = 0x0E00D;

/// Object ID for the autoscroll controller.
const OBJ_AUTOSCROLL: u8 = 0xD3;

/// File offset of the autoscroll object in level 5-9 (parabeetle ride).
/// This level requires autoscroll to function — without it, there is no
/// ground and the player cannot progress.
const LEVEL_5_9_AUTOSCROLL_OFFSET: usize = 0x0CECE;

/// Airship level 9-byte header offsets (start of each header in ROM).
/// These levels use airship path autoscroll (Y = 0x00–0x14) and need their
/// header scroll flags changed so the camera follows the player after the
/// D3 autoscroll object is removed.
///
/// Header byte5 (offset+5) encodes: bit7=unused, bits6-5=X-start,
/// bits4-3=obj palette, bits2-0=BG palette.
/// Airships have byte5=0xEA; we change to 0x0A (clear top 3 bits = X-start 0).
///
/// Header byte4 (offset+4) encodes: bits7-5=Y-start, bit4=flag, bits3-0=end page.
/// Airships have byte4=0x8A; we change to 0xAA (set bit 5 = Y-start adjustment).
const AIRSHIP_HEADERS: &[(usize, &str)] = &[
    (0x2ECA9, "W1 airship"),
    (0x2EDC9, "W2 airship"),
    (0x2EEBD, "W3 airship"),
    (0x2F01B, "W4 airship"),
    (0x2F14C, "W5 airship"),
    (0x2F2C5, "W6 airship"),
    (0x2F49B, "W7 airship"),
];

/// Additional level headers where byte4 needs bit 5 set (0x8C -> 0xAC).
/// These are sub-areas in the Ice/Sky tileset region that also use
/// airship-style scroll paths.
const EXTRA_HEADERS_BYTE4: &[usize] = &[
    0x2315E, // Ice/Sky area
    0x23AFC, // Ice/Sky area
];

/// Level headers where byte6 needs vertical scroll mode cleared.
/// byte6 encodes: bit7=pipe transition, bits6-5=vert scroll mode,
/// bit4=scroll direction, bits3-0=transition course type.
/// 0xEA (vert_scroll=3) -> 0x0A (vert_scroll=0).
const EXTRA_HEADERS_BYTE6: &[usize] = &[
    0x2F628, // Ship sub-area
    0x2FC26, // Ship sub-area
];

/// Disable all autoscrollers except 5-9 (parabeetle ride).
///
/// Two-part fix:
/// 1. Scan enemy/object data for all D3 (autoscroll) objects and replace
///    them with 0x00 (NOP), except the 5-9 parabeetle autoscroll.
/// 2. Patch airship/ship level headers to change scroll mode flags so the
///    camera properly follows the player instead of following a preset path.
pub fn disable_autoscroll(rom: &mut Rom) {
    // Part 1: Remove all D3 autoscroll objects except 5-9
    let len = ENEMY_DATA_END - ENEMY_DATA_START;
    let mut data = rom.read_range(ENEMY_DATA_START, len).to_vec();

    let mut i = 0;
    while i < data.len() {
        // Skip 0xFF terminators
        if data[i] == 0xFF {
            i += 1;
            continue;
        }

        // First non-FF byte after a terminator is the page/flag byte
        i += 1;

        // Parse 3-byte entries until 0xFF or end of data
        while i + 2 < data.len() && data[i] != 0xFF {
            let file_offset = ENEMY_DATA_START + i;

            if data[i] == OBJ_AUTOSCROLL && file_offset != LEVEL_5_9_AUTOSCROLL_OFFSET {
                data[i] = 0x00;
            }

            i += 3;
        }
    }

    rom.write_range(ENEMY_DATA_START, &data);

    // Part 2: Patch airship level headers to disable scroll-path camera
    for &(header_offset, _name) in AIRSHIP_HEADERS {
        // byte4: 0x8A -> 0xAA (set bit 5 for Y-start adjustment)
        rom.write_byte(header_offset + 4, 0xAA);
        // byte5: 0xEA -> 0x0A (clear X-start bits, disabling scroll-path mode)
        rom.write_byte(header_offset + 5, 0x0A);
    }

    for &header_offset in EXTRA_HEADERS_BYTE4 {
        // Set bit 5 in byte4
        let b4 = rom.read_byte(header_offset + 4);
        rom.write_byte(header_offset + 4, b4 | 0x20);
    }

    for &header_offset in EXTRA_HEADERS_BYTE6 {
        // Clear vert scroll mode (bits 6-5) in byte6: 0xEA -> 0x0A
        rom.write_byte(header_offset + 6, 0x0A);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::Rom;

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        // iNES header
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        // Set up enemy data with several autoscroll entries
        let start = ENEMY_DATA_START;
        let seg = &[
            0xFF, // leading terminator
            0x01, // page flag
            0xD3, 0x00, 0x50, // Horizontal autoscroll (y=0x50) at start+2
            0x72, 0x0E, 0x19, // Goomba
            0xD3, 0x00, 0x0A, // Airship autoscroll (y=0x0A) at start+8
            0x41, 0xA8, 0x15, // End Level Card
            0xD3, 0x00, 0x53, // Horizontal autoscroll (y=0x53) at start+14
            0xFF, // terminator
        ];
        data[start..start + seg.len()].copy_from_slice(seg);

        // Place an autoscroll at the 5-9 protected offset
        let seg_start = LEVEL_5_9_AUTOSCROLL_OFFSET - 2;
        data[seg_start] = 0xFF;
        data[seg_start + 1] = 0x00; // page flag
        data[LEVEL_5_9_AUTOSCROLL_OFFSET] = OBJ_AUTOSCROLL;
        data[LEVEL_5_9_AUTOSCROLL_OFFSET + 1] = 0x00;
        data[LEVEL_5_9_AUTOSCROLL_OFFSET + 2] = 0x55; // y=0x55, horizontal
        data[LEVEL_5_9_AUTOSCROLL_OFFSET + 3] = 0xFF;

        // Set up airship level headers with expected original values
        for &(offset, _) in AIRSHIP_HEADERS {
            if offset + 9 <= data.len() {
                data[offset + 4] = 0x8A;
                data[offset + 5] = 0xEA;
            }
        }

        // Set up extra byte4 headers
        for &offset in EXTRA_HEADERS_BYTE4 {
            if offset + 9 <= data.len() {
                data[offset + 4] = 0x8C;
            }
        }

        // Set up extra byte6 headers
        for &offset in EXTRA_HEADERS_BYTE6 {
            if offset + 9 <= data.len() {
                data[offset + 6] = 0xEA;
            }
        }

        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_all_autoscrolls_removed() {
        let mut rom = make_test_rom();

        assert_eq!(rom.read_byte(ENEMY_DATA_START + 2), OBJ_AUTOSCROLL);
        assert_eq!(rom.read_byte(ENEMY_DATA_START + 8), OBJ_AUTOSCROLL);
        assert_eq!(rom.read_byte(ENEMY_DATA_START + 14), OBJ_AUTOSCROLL);

        disable_autoscroll(&mut rom);

        assert_eq!(rom.read_byte(ENEMY_DATA_START + 2), 0x00,
            "Horizontal autoscroll (y=0x50) should be removed");
        assert_eq!(rom.read_byte(ENEMY_DATA_START + 8), 0x00,
            "Airship autoscroll (y=0x0A) should be removed");
        assert_eq!(rom.read_byte(ENEMY_DATA_START + 14), 0x00,
            "Horizontal autoscroll (y=0x53) should be removed");
    }

    #[test]
    fn test_5_9_autoscroll_preserved() {
        let mut rom = make_test_rom();

        assert_eq!(rom.read_byte(LEVEL_5_9_AUTOSCROLL_OFFSET), OBJ_AUTOSCROLL);

        disable_autoscroll(&mut rom);

        assert_eq!(rom.read_byte(LEVEL_5_9_AUTOSCROLL_OFFSET), OBJ_AUTOSCROLL,
            "Level 5-9 autoscroll must be preserved");
    }

    #[test]
    fn test_non_autoscroll_objects_unchanged() {
        let mut rom = make_test_rom();

        let goomba = rom.read_byte(ENEMY_DATA_START + 5);
        let card = rom.read_byte(ENEMY_DATA_START + 11);

        disable_autoscroll(&mut rom);

        assert_eq!(rom.read_byte(ENEMY_DATA_START + 5), goomba,
            "Goomba should be unchanged");
        assert_eq!(rom.read_byte(ENEMY_DATA_START + 11), card,
            "End Level Card should be unchanged");
    }

    #[test]
    fn test_airship_headers_patched() {
        let mut rom = make_test_rom();

        disable_autoscroll(&mut rom);

        for &(offset, name) in AIRSHIP_HEADERS {
            assert_eq!(rom.read_byte(offset + 4), 0xAA,
                "{} byte4 should be 0xAA", name);
            assert_eq!(rom.read_byte(offset + 5), 0x0A,
                "{} byte5 should be 0x0A", name);
        }
    }

    #[test]
    fn test_extra_byte4_headers_patched() {
        let mut rom = make_test_rom();

        disable_autoscroll(&mut rom);

        for &offset in EXTRA_HEADERS_BYTE4 {
            assert_eq!(rom.read_byte(offset + 4), 0xAC,
                "Header at 0x{:05X} byte4 should be 0xAC", offset);
        }
    }

    #[test]
    fn test_extra_byte6_headers_patched() {
        let mut rom = make_test_rom();

        disable_autoscroll(&mut rom);

        for &offset in EXTRA_HEADERS_BYTE6 {
            assert_eq!(rom.read_byte(offset + 6), 0x0A,
                "Header at 0x{:05X} byte6 should be 0x0A", offset);
        }
    }
}
