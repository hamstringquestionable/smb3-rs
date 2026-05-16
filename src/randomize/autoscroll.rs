use core::ops::Range;

use crate::rom::Rom;

/// File-offset ranges (half-open) of enemy-data segments that
/// `disable_autoscroll` rewrites into a form the level loader handles
/// correctly but that confuses a block-wide segment walker.
///
/// The "Segment terminator insertion" / "Enemy data structure rewrite"
/// patches below (`0x0CFE3`, `0x0D038`, `0x0D103`) overwrite an
/// autoscroll obj_id with `$FF` mid-segment. A per-level loader entering
/// that level at its obj_ptr sees the early `$FF`, treats the segment as
/// empty, and stops — exactly the intended autoscroll-disable behavior.
/// But a greedy walker that doesn't know obj_ptr boundaries will keep
/// parsing past the `$FF`, swallow orphaned bytes plus the page byte +
/// first entry of the *next* real segment, and emit a "ghost" segment
/// that scrambles adjacent real data when written back sorted.
///
/// Block-wide walkers (currently `enemies.rs`'s randomization pass) must
/// pass these ranges to [`segment_writer::walk_segments`] so the walker
/// jumps past the spoiled bytes and resumes parsing at the next real
/// segment.
///
/// Each range is conservative: it starts at the leading `$FF` (or page
/// byte) of the affected level's data and ends at the byte just before
/// the next real segment's page byte.
pub const SPOILED_SEGMENT_RANGES: &[Range<usize>] = &[
    0x0CFE2..0x0CFE7,
    0x0D037..0x0D042,
    0x0D102..0x0D107,
];

/// Disable all autoscrollers except 5-9 (parabeetle ride).
///
/// This applies the full set of patches derived from the reference
/// "Super_Mario_Bros_3_NoAutoscrolls(Except 5-9).ips" patch. The reference
/// patch does far more than simply removing D3 autoscroll objects — it:
///
/// 1. Removes D3 autoscroll controller objects from enemy data
/// 2. Rewrites airship enemy/object data with new configurations designed
///    for free-scroll play (repositioned enemies, new spawners, etc.)
/// 3. Redirects per-world level pointer table entries (ByRowType, ObjSets,
///    LevelLayouts) so airship levels load the new enemy/layout data
/// 4. Writes new level layout/tile generator data for the reworked levels
/// 5. Patches airship level headers (byte4 Y-start, byte5 X-start) so the
///    camera and player start position work correctly without scroll paths
/// 6. Patches additional fortress/ship sub-area headers
/// 7. Patches a PRG030 code byte to disable scroll-path camera logic
///
/// All patch data is applied as static byte writes to exact ROM offsets,
/// guaranteeing identical results to the reference IPS patch.
pub fn disable_autoscroll(rom: &mut Rom) {
    for &(offset, data) in PATCHES.iter() {
        rom.write_range(offset, data);
    }
}

/// Static patch table derived from the reference NoAutoscrolls IPS.
/// Each entry is (file_offset, &[u8]) applied in order.
///
/// Grouped by category for clarity:
///   - Enemy data: D3 removals + enemy/object rewrites (0x0BFD8–0x0E00D)
///   - Level pointer tables: per-world airship redirects (0x19000+)
///   - Level layout data: new tile generators (0x24000+)
///   - Level headers: airship + fortress/ship byte4/byte5 patches (0x2EC00+)
///   - Code patch: PRG030 scroll-path disable (0x3D7AD)
const PATCHES: &[(usize, &[u8])] = &[
    // =========================================================================
    // Enemy/object data region (0x0BFD8–0x0E00D)
    // D3 autoscroll removals + airship enemy data rewrites
    // =========================================================================

    // --- Non-airship autoscroll removals (horizontal scrollers, etc.) ---
    // These replace D3 object IDs with 0x00 (NOP) at specific offsets.
    (0x0CA74, &[0x00]),
    (0x0CB63, &[0x00]),
    (0x0CC44, &[0x00]),

    // Airship W1 area: enemy data rewrite (4 bytes)
    (0x0CC6C, &[0x69, 0x18, 0x36, 0x6C]),

    // More D3 removals
    (0x0CD28, &[0x00]),
    (0x0CDD3, &[0x00]),

    // Airship W2 area: enemy data rewrite (7 bytes)
    (0x0CDE7, &[0x13, 0x6A, 0x63, 0x12, 0x6A, 0x69, 0x16]),

    // 3-7 coin heaven: D3 removed so the sub-area is free-scroll. Vanilla uses
    // autoscroll as a penalty for grabbing the chest, but free-scroll is more
    // enjoyable when hand_rooms redirects a W8 Hand here as an alternate ending.
    (0x0CE9A, &[0x00]),

    // D3 removal
    (0x0CF51, &[0x00]),

    // Segment terminator insertion
    (0x0CFE3, &[0xFF]),

    // Enemy data structure rewrite (6 bytes)
    (0x0D038, &[0xFF, 0x00, 0x12, 0xFF, 0x01, 0xFF]),

    // Segment terminator
    (0x0D103, &[0xFF]),

    // D3 removal
    (0x0D6B7, &[0x00]),

    // Airship enemy data: cannon/fire objects repositioned (9 bytes)
    (0x0D6DB, &[
        0xBC, 0x5A, 0x0E, 0xCA, 0x5D, 0x0A, 0xC9, 0x5F, 0x10,
    ]),

    // Airship enemy data: more repositioned objects + terminator (18 bytes)
    (0x0D6EA, &[
        0xC8, 0x65, 0x10, 0xCB, 0x66, 0x0A, 0xC8, 0x68, 0x10,
        0xCB, 0x6A, 0x0A, 0xBC, 0x70, 0x0C, 0xFF, 0x00, 0x00,
    ]),

    // D3 removals
    (0x0D72D, &[0x00]),
    (0x0D768, &[0x00]),

    // Airship enemy data: repositioned objects (6 bytes)
    (0x0D789, &[0xC8, 0x3C, 0x10, 0xCA, 0x3F, 0x0A]),

    // D3 removal
    (0x0D7A9, &[0x00]),

    // Airship enemy data: page flag + header-like bytes (5 bytes)
    (0x0D7B3, &[0x14, 0x11, 0xAA, 0x15, 0x0F]),

    // Airship W6 area: large enemy data rewrite — new enemy set for
    // free-scroll play with repositioned cannons, fire jets, etc. (45 bytes)
    (0x0D7CA, &[
        0xBE, 0x3A, 0x0B, 0xB2, 0x3D, 0x0E, 0xAC, 0x3F, 0x11,
        0xB1, 0x49, 0x0A, 0xB2, 0x4A, 0x0E, 0xB1, 0x52, 0x11,
        0xB2, 0x55, 0x0C, 0xAC, 0x57, 0x0E, 0x9D, 0x58, 0x11,
        0x9D, 0x62, 0x11, 0xB1, 0x68, 0x10, 0xAC, 0x6A, 0x0D,
        0xB1, 0x6B, 0x0B, 0xAA, 0x77, 0x13, 0xFF, 0x00, 0x00,
    ]),

    // Airship W7 area: page flag + header-like bytes (5 bytes)
    (0x0D7FD, &[0x11, 0x11, 0xAA, 0x13, 0x0F]),

    // Airship W7 area: enemy data with cloud/event objects (18 bytes)
    (0x0D825, &[
        0x00, 0x00, 0x0C, 0xB8, 0x01, 0x03, 0xAE, 0x14, 0x08,
        0xAA, 0x15, 0x0A, 0x9D, 0x17, 0x07, 0x9D, 0x1E, 0x07,
    ]),

    // More repositioned objects (6 bytes)
    (0x0D849, &[0xAE, 0x5A, 0x0A, 0xBE, 0x5B, 0x0D]),

    // More repositioned objects (6 bytes)
    (0x0D858, &[0xAA, 0x8A, 0x0D, 0xBE, 0x8B, 0x09]),

    // D3 removal
    (0x0D878, &[0x00]),

    // Autoscroll type change: airship path -> horizontal (0x50)
    (0x0D8DF, &[0x50]),

    // D3 removals
    (0x0D92D, &[0x00]),
    (0x0D980, &[0x00]),
    (0x0DA15, &[0x00]),

    // =========================================================================
    // Level pointer table redirects (PRG012: 0x18010–0x1A00F)
    // Each world's airship entry gets its ByRowType, ObjSets, and
    // LevelLayouts pointers updated to reference the new data.
    // =========================================================================

    // --- World 1 airship pointer redirect ---
    (0x19449, &[0x8A]),                 // ByRowType
    (0x19484, &[0xEA, 0xD6]),           // ObjSets (enemy data CPU addr)
    (0x194AE, &[0xB7, 0xAD]),           // LevelLayouts (layout CPU addr)

    // --- World 2 airship pointer redirect ---
    (0x194DE, &[0x6A]),                 // ByRowType
    (0x19560, &[0x1C, 0xD7]),           // ObjSets
    (0x195BE, &[0xAB, 0xAE]),           // LevelLayouts

    // --- World 3 airship pointer redirect ---
    (0x19609, &[0x8A]),                 // ByRowType
    (0x196A2, &[0x57, 0xD7]),           // ObjSets
    (0x1970A, &[0x09, 0xB0]),           // LevelLayouts

    // --- World 4 airship pointer redirect ---
    (0x1971A, &[0x6A]),                 // ByRowType
    (0x19764, &[0x98, 0xD7]),           // ObjSets
    (0x197A8, &[0x3A, 0xB1]),           // LevelLayouts

    // --- World 5 airship pointer redirect ---
    (0x19807, &[0xAA]),                 // ByRowType
    (0x1987E, &[0xA6, 0xD6]),           // ObjSets
    (0x198D2, &[0x97, 0xAC]),           // LevelLayouts

    // --- World 6 airship pointer redirect ---
    (0x19919, &[0x6A]),                 // ByRowType
    (0x199C0, &[0xE5, 0xD7]),           // ObjSets
    (0x19A32, &[0xB3, 0xB2]),           // LevelLayouts

    // --- World 7 airship pointer redirect ---
    (0x19A69, &[0x9A]),                 // ByRowType
    (0x19AF0, &[0x14, 0xD8]),           // ObjSets
    (0x19B4C, &[0x89, 0xB4]),           // LevelLayouts

    // =========================================================================
    // Additional level header patches (fortress/ship sub-areas)
    // Set bit 5 of byte4 for Y-start adjustment (0x8C -> 0xAC)
    // =========================================================================
    (0x23162, &[0xAC]),
    (0x23B00, &[0xAC]),

    // =========================================================================
    // Level layout data rewrites (pipe/water level data region)
    // New tile generator data for reworked airship levels.
    // =========================================================================

    // Airship level tile generators — repeated metatile pattern (28 bytes)
    (0x24DE0, &[
        0x6A, 0x00, 0x8F, 0x6A, 0x10, 0x8F, 0x6A, 0x20, 0x8F,
        0x6A, 0x30, 0x8F, 0x6A, 0x40, 0x8F, 0x6A, 0x50, 0x8F,
        0x6A, 0x60, 0x8F, 0x6A, 0x70, 0x8F, 0x6A, 0x80, 0x8F,
        0x6A,
    ]),

    // Airship level tile generators — platform/geometry data (85 bytes)
    (0x24E6A, &[
        0x6C, 0x4D, 0x80, 0x6D, 0x46, 0x80, 0x6E, 0x49, 0x80,
        0x6E, 0x4F, 0x80, 0x6F, 0x41, 0x80, 0x6F, 0x4C, 0x80,
        0x70, 0x4E, 0x80, 0x71, 0x4A, 0x80, 0x72, 0x44, 0x80,
        0x75, 0x48, 0x80, 0x76, 0x4C, 0x80, 0x77, 0x4A, 0x80,
        0x77, 0x4E, 0x80, 0x78, 0x45, 0x80, 0x78, 0x4D, 0x80,
        0x79, 0x42, 0x80, 0x79, 0x48, 0x80, 0x6D, 0x51, 0x80,
        0x6D, 0x5A, 0x80, 0x6F, 0x51, 0x80, 0x6F, 0x56, 0x80,
        0x70, 0x53, 0x80, 0x73, 0x59, 0x80, 0x75, 0x50, 0x80,
        0x76, 0x53, 0x80, 0x77, 0x51, 0x80, 0x77, 0x56, 0x80,
        0x77, 0x5C, 0x80, 0x79,
    ]),

    // =========================================================================
    // Airship level header patches (ship level data: 0x2EC07–0x30005)
    // For each W1-W7 airship: byte4 (Y-start) -> 0xAA, byte5 (X-start) -> 0x0A
    // This positions Mario correctly and disables scroll-path camera mode.
    // =========================================================================
    (0x2ECAD, &[0xAA, 0x0A]),  // W1 airship header byte4+5
    (0x2EDCD, &[0xAA, 0x0A]),  // W2 airship header byte4+5
    (0x2EEC1, &[0xAA, 0x0A]),  // W3 airship header byte4+5
    (0x2F01F, &[0xAA, 0x0A]),  // W4 airship header byte4+5
    (0x2F150, &[0xAA, 0x0A]),  // W5 airship header byte4+5
    (0x2F2C9, &[0xAA, 0x0A]),  // W6 airship header byte4+5
    (0x2F49F, &[0xAA, 0x0A]),  // W7 airship header byte4+5

    // Extra ship sub-area byte5 patches (clear X-start bits)
    (0x2F62E, &[0x0A]),
    (0x2FC2C, &[0x0A]),

    // =========================================================================
    // PRG030 code patch: disable scroll-path camera logic
    // =========================================================================
    (0x3D7AD, &[0x80]),
];

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
        Rom::from_bytes_lax(&data, true).unwrap()
    }

    #[test]
    fn test_all_patches_applied() {
        let mut rom = make_test_rom();
        disable_autoscroll(&mut rom);

        // Verify every patch was applied correctly
        for &(offset, data) in PATCHES.iter() {
            let actual = rom.read_range(offset, data.len());
            assert_eq!(
                actual, data,
                "Patch at 0x{:05X} ({} bytes) was not applied correctly",
                offset,
                data.len()
            );
        }
    }

    #[test]
    fn test_5_9_autoscroll_preserved() {
        // Level 5-9 parabeetle ride autoscroll at 0x0CECE must NOT be touched.
        let mut rom = make_test_rom();
        let level_5_9_offset: usize = 0x0CECE;

        // Place a D3 autoscroll at the 5-9 offset
        rom.write_byte(level_5_9_offset, 0xD3);

        disable_autoscroll(&mut rom);

        // Verify 5-9 is not in any patch range
        for &(offset, data) in PATCHES.iter() {
            let end = offset + data.len();
            assert!(
                level_5_9_offset < offset || level_5_9_offset >= end,
                "Patch at 0x{:05X}-0x{:05X} overlaps 5-9 autoscroll at 0x{:05X}!",
                offset,
                end - 1,
                level_5_9_offset
            );
        }

        // The D3 should still be there
        assert_eq!(
            rom.read_byte(level_5_9_offset),
            0xD3,
            "Level 5-9 autoscroll must be preserved"
        );
    }

    #[test]
    fn test_airship_headers_patched() {
        let mut rom = make_test_rom();

        // Set up original airship header values
        let airship_offsets = [
            0x2ECAD, 0x2EDCD, 0x2EEC1, 0x2F01F, 0x2F150, 0x2F2C9, 0x2F49F,
        ];
        for &offset in &airship_offsets {
            rom.write_byte(offset, 0x8A);     // original byte4
            rom.write_byte(offset + 1, 0xEA); // original byte5
        }

        disable_autoscroll(&mut rom);

        for &offset in &airship_offsets {
            assert_eq!(
                rom.read_byte(offset),
                0xAA,
                "Airship byte4 at 0x{:05X} should be 0xAA",
                offset
            );
            assert_eq!(
                rom.read_byte(offset + 1),
                0x0A,
                "Airship byte5 at 0x{:05X} should be 0x0A",
                offset + 1
            );
        }
    }

    #[test]
    fn test_d3_removals_applied() {
        let mut rom = make_test_rom();

        // D3 removal offsets (single-byte 0x00 patches in enemy data range)
        let d3_offsets = [
            0x0CA74, 0x0CB63, 0x0CC44, 0x0CD28, 0x0CDD3, 0x0CF51,
            0x0D6B7, 0x0D72D, 0x0D768, 0x0D7A9, 0x0D878,
            0x0D92D, 0x0D980, 0x0DA15,
        ];

        // Place D3 at each offset
        for &offset in &d3_offsets {
            rom.write_byte(offset, 0xD3);
        }

        disable_autoscroll(&mut rom);

        for &offset in &d3_offsets {
            assert_eq!(
                rom.read_byte(offset),
                0x00,
                "D3 at 0x{:05X} should be removed (set to 0x00)",
                offset
            );
        }
    }

    #[test]
    fn test_extra_headers_patched() {
        let mut rom = make_test_rom();

        // Extra byte4 headers
        rom.write_byte(0x23162, 0x8C);
        rom.write_byte(0x23B00, 0x8C);

        // Extra byte5 headers
        rom.write_byte(0x2F62E, 0xEA);
        rom.write_byte(0x2FC2C, 0xEA);

        disable_autoscroll(&mut rom);

        assert_eq!(rom.read_byte(0x23162), 0xAC);
        assert_eq!(rom.read_byte(0x23B00), 0xAC);
        assert_eq!(rom.read_byte(0x2F62E), 0x0A);
        assert_eq!(rom.read_byte(0x2FC2C), 0x0A);
    }

    #[test]
    fn test_prg030_code_patch() {
        let mut rom = make_test_rom();
        rom.write_byte(0x3D7AD, 0x00);

        disable_autoscroll(&mut rom);

        assert_eq!(
            rom.read_byte(0x3D7AD),
            0x80,
            "PRG030 scroll-path disable patch not applied"
        );
    }

    #[test]
    fn test_level_pointer_redirects() {
        let mut rom = make_test_rom();
        disable_autoscroll(&mut rom);

        // Spot-check a few world pointer redirects

        // W1: ObjSets should be [0xEA, 0xD6]
        assert_eq!(rom.read_byte(0x19484), 0xEA);
        assert_eq!(rom.read_byte(0x19485), 0xD6);

        // W4: ObjSets should be [0x98, 0xD7]
        assert_eq!(rom.read_byte(0x19764), 0x98);
        assert_eq!(rom.read_byte(0x19765), 0xD7);

        // W7: ObjSets should be [0x14, 0xD8]
        assert_eq!(rom.read_byte(0x19AF0), 0x14);
        assert_eq!(rom.read_byte(0x19AF1), 0xD8);
    }

    #[test]
    fn test_level_layout_data_written() {
        let mut rom = make_test_rom();
        disable_autoscroll(&mut rom);

        // Verify the first few bytes of the tile generator data
        assert_eq!(rom.read_byte(0x24DE0), 0x6A);
        assert_eq!(rom.read_byte(0x24DE1), 0x00);
        assert_eq!(rom.read_byte(0x24DE2), 0x8F);

        // Verify the platform/geometry data
        assert_eq!(rom.read_byte(0x24E6A), 0x6C);
        assert_eq!(rom.read_byte(0x24E6B), 0x4D);
        assert_eq!(rom.read_byte(0x24E6C), 0x80);
    }

    #[test]
    fn test_no_overlapping_patches() {
        // Ensure no patches overlap each other (which would indicate a bug)
        let mut ranges: Vec<(usize, usize)> = PATCHES
            .iter()
            .map(|&(offset, data)| (offset, offset + data.len()))
            .collect();
        ranges.sort_by_key(|&(start, _)| start);

        for i in 1..ranges.len() {
            assert!(
                ranges[i].0 >= ranges[i - 1].1,
                "Patches overlap: 0x{:05X}-0x{:05X} and 0x{:05X}-0x{:05X}",
                ranges[i - 1].0,
                ranges[i - 1].1 - 1,
                ranges[i].0,
                ranges[i].1 - 1
            );
        }
    }

    #[test]
    fn test_deterministic() {
        let mut rom1 = make_test_rom();
        let mut rom2 = make_test_rom();

        disable_autoscroll(&mut rom1);
        disable_autoscroll(&mut rom2);

        // All patched regions should be identical
        for &(offset, data) in PATCHES.iter() {
            assert_eq!(
                rom1.read_range(offset, data.len()),
                rom2.read_range(offset, data.len()),
            );
        }
    }

    #[test]
    fn test_patch_count() {
        // 64 from the reference IPS + 1 local addition for the 3-7 coin
        // heaven (deliberately left intact by the reference).
        assert_eq!(PATCHES.len(), 65);
    }
}
