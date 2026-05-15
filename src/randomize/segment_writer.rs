//! Safe writes to enemy data segments.
//!
//! Each enemy data segment is a packed stream of 3-byte entries
//! `[obj_id, x, y]`, preceded by a single page/header byte and terminated
//! by `0xFF`. The level loader walks entries sequentially as the screen
//! scrolls right, so entries within a segment **must be sorted by ascending
//! X**. Violating that breaks activation timing and can leave entries
//! unparsed.
//!
//! This module is the single throat for segment edits. A randomizer that
//! wants to change entries in a segment reads them (via [`read_segment`]),
//! produces a new entry list, and hands it to [`write_segment`]. The
//! writer sorts, validates count and X-collision invariants, then writes
//! back. Multiple randomizers operating on the same segment can produce
//! their proposed entries independently — the final caller composes the
//! lists and routes the result through this module.
//!
//! What this module does NOT do: grow or shrink segments (count is fixed
//! against the original), insert new obj_ids that the level loader can't
//! handle, or coordinate writes across segments.

use crate::rom::Rom;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentEntry {
    pub obj_id: u8,
    pub x: u8,
    pub y: u8,
}

pub struct SegmentSpec<'a> {
    /// File offset of the segment's page/header byte. Entries start at
    /// `file_offset + 1`.
    pub file_offset: usize,
    /// Expected entry count (defends against accidental segment growth/shrink).
    pub original_count: usize,
    /// Proposed entries, any order. The writer sorts by X.
    pub entries: &'a [SegmentEntry],
}

#[derive(Debug)]
pub enum WriteError {
    CountMismatch { offset: usize, expected: usize, got: usize },
    XCollision { offset: usize, x: u8 },
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::CountMismatch { offset, expected, got } =>
                write!(f, "segment 0x{offset:05X}: count mismatch (expected {expected}, got {got})"),
            WriteError::XCollision { offset, x } =>
                write!(f, "segment 0x{offset:05X}: X collision at 0x{x:02X}"),
        }
    }
}

/// Read all entries from an existing segment.
pub fn read_segment(rom: &Rom, file_offset: usize, count: usize) -> Vec<SegmentEntry> {
    let base = file_offset + 1;
    (0..count).map(|i| {
        let off = base + i * 3;
        SegmentEntry {
            obj_id: rom.read_byte(off),
            x: rom.read_byte(off + 1),
            y: rom.read_byte(off + 2),
        }
    }).collect()
}

/// Sort `entries` by X, validate, and write back to the segment. Errors
/// are returned rather than panicking so callers can decide whether a
/// failure is recoverable.
pub fn write_segment(rom: &mut Rom, spec: &SegmentSpec) -> Result<(), WriteError> {
    if spec.entries.len() != spec.original_count {
        return Err(WriteError::CountMismatch {
            offset: spec.file_offset,
            expected: spec.original_count,
            got: spec.entries.len(),
        });
    }

    let mut sorted: Vec<SegmentEntry> = spec.entries.to_vec();
    sorted.sort_by_key(|e| e.x);

    for window in sorted.windows(2) {
        if window[0].x == window[1].x {
            return Err(WriteError::XCollision {
                offset: spec.file_offset,
                x: window[0].x,
            });
        }
    }

    let base = spec.file_offset + 1;
    for (i, entry) in sorted.iter().enumerate() {
        let off = base + i * 3;
        rom.write_byte(off, entry.obj_id);
        rom.write_byte(off + 1, entry.x);
        rom.write_byte(off + 2, entry.y);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rom() -> Rom {
        let mut bytes = vec![0u8; 0x60010];
        bytes[0] = b'N'; bytes[1] = b'E'; bytes[2] = b'S'; bytes[3] = 0x1A;
        bytes[4] = 16; bytes[5] = 16; bytes[6] = 0x40; bytes[7] = 0x10;
        Rom::from_bytes_lax(&bytes, true).unwrap()
    }

    #[test]
    fn sorts_by_x_before_write() {
        let mut rom = make_rom();
        let entries = [
            SegmentEntry { obj_id: 0x9E, x: 0x30, y: 0x14 },
            SegmentEntry { obj_id: 0x53, x: 0x10, y: 0x0F },
            SegmentEntry { obj_id: 0x9E, x: 0x20, y: 0x12 },
        ];
        let spec = SegmentSpec { file_offset: 0x1000, original_count: 3, entries: &entries };
        write_segment(&mut rom, &spec).unwrap();
        // After write: 0x10 first, then 0x20, then 0x30.
        assert_eq!(rom.read_byte(0x1001), 0x53);  // first entry obj_id
        assert_eq!(rom.read_byte(0x1002), 0x10);  // first entry X
        assert_eq!(rom.read_byte(0x1004), 0x9E);  // second entry obj_id
        assert_eq!(rom.read_byte(0x1005), 0x20);
        assert_eq!(rom.read_byte(0x1007), 0x9E);
        assert_eq!(rom.read_byte(0x1008), 0x30);
    }

    #[test]
    fn rejects_count_mismatch() {
        let mut rom = make_rom();
        let entries = [SegmentEntry { obj_id: 0x9E, x: 0x10, y: 0x14 }];
        let spec = SegmentSpec { file_offset: 0x1000, original_count: 3, entries: &entries };
        let err = write_segment(&mut rom, &spec).unwrap_err();
        assert!(matches!(err, WriteError::CountMismatch { .. }));
    }

    #[test]
    fn rejects_x_collision() {
        let mut rom = make_rom();
        let entries = [
            SegmentEntry { obj_id: 0x9E, x: 0x20, y: 0x14 },
            SegmentEntry { obj_id: 0x53, x: 0x20, y: 0x0F },
        ];
        let spec = SegmentSpec { file_offset: 0x1000, original_count: 2, entries: &entries };
        let err = write_segment(&mut rom, &spec).unwrap_err();
        assert!(matches!(err, WriteError::XCollision { x: 0x20, .. }));
    }

    #[test]
    fn read_roundtrips() {
        let mut rom = make_rom();
        let entries = [
            SegmentEntry { obj_id: 0x3F, x: 0x04, y: 0x18 },
            SegmentEntry { obj_id: 0x75, x: 0x62, y: 0x16 },
        ];
        let spec = SegmentSpec { file_offset: 0x2000, original_count: 2, entries: &entries };
        write_segment(&mut rom, &spec).unwrap();
        let back = read_segment(&rom, 0x2000, 2);
        assert_eq!(back, entries.to_vec());
    }
}
