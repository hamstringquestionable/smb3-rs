//! Safe writes to enemy data segments.
//!
//! Each enemy data segment is a packed stream of 3-byte entries
//! `[obj_id, x, y]`, preceded by a single page/header byte and terminated
//! by `0xFF`. The level loader walks entries sequentially as Mario's
//! screen advances. For a logical level read by one `enemy_ptr`, entries
//! are typically X-sorted so activation timing tracks screen progression.
//!
//! This module is the single throat for composed segments: the
//! [`bowser_castle`](super::bowser_castle), [`podoboo_gauntlet`](super::podoboo_gauntlet),
//! [`hand_rooms`](super::hand_rooms) and [`piranha_rooms`](super::piranha_rooms) composers
//! build the entry list for one specific segment whose `enemy_ptr` is known
//! (the 5-F2 and 8-Bowser composers start from the room as the enemy pass
//! left it and move only their own hazards), and the writer sorts it by X
//! before writing. The enemy pass changes IDs in place and writes its buffer
//! back directly: vanilla lists are not always X-sorted (5-F2's gauntlet,
//! 6-6's sub-area spikes), and it has no reason to reorder them.
//!
//! Callers pass an entry list and a count; the writer validates the count,
//! sorts and writes back. It does NOT grow or shrink segments,
//! insert obj_ids the level loader can't handle, or coordinate writes
//! across segments.

use crate::rom::Rom;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SegmentEntry {
    pub(crate) obj_id: u8,
    pub(crate) x: u8,
    pub(crate) y: u8,
}

pub(crate) struct SegmentSpec<'a> {
    /// File offset of the segment's page/header byte. Entries start at
    /// `file_offset + 1`.
    pub(crate) file_offset: usize,
    /// Expected entry count (defends against accidental segment growth/shrink).
    pub(crate) original_count: usize,
    /// Proposed entries, in any order: they are sorted by X before writing.
    pub(crate) entries: &'a [SegmentEntry],
    /// Optional caller-supplied name (e.g. `"3-2 sub-area 0"`) that gets
    /// embedded in error messages alongside the file offset. Useful when a
    /// single pass writes many segments — knowing which segment failed is
    /// hard from offset alone.
    pub(crate) label: Option<&'a str>,
}

#[derive(Debug)]
pub(crate) enum WriteError {
    CountMismatch { offset: usize, label: Option<String>, expected: usize, got: usize },
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn name(offset: usize, label: &Option<String>) -> String {
            match label {
                Some(l) => format!("segment 0x{offset:05X} ({l})"),
                None => format!("segment 0x{offset:05X}"),
            }
        }
        match self {
            WriteError::CountMismatch { offset, label, expected, got } => write!(
                f,
                "{}: count mismatch (expected {expected}, got {got})",
                name(*offset, label)
            ),
        }
    }
}

/// Read all entries from an existing segment.
pub(crate) fn read_segment(rom: &Rom, file_offset: usize, count: usize) -> Vec<SegmentEntry> {
    let base = file_offset + 1;
    (0..count)
        .map(|i| {
            let off = base + i * 3;
            SegmentEntry {
                obj_id: rom.read_byte(off),
                x: rom.read_byte(off + 1),
                y: rom.read_byte(off + 2),
            }
        })
        .collect()
}

/// Validate `entries`, sort them by X, and write back to the segment. Errors are returned rather than panicking
/// so callers can decide whether a failure is recoverable.
pub(crate) fn write_segment(rom: &mut Rom, spec: &SegmentSpec) -> Result<(), WriteError> {
    let label_owned = || spec.label.map(|s| s.to_string());

    if spec.entries.len() != spec.original_count {
        return Err(WriteError::CountMismatch {
            offset: spec.file_offset,
            label: label_owned(),
            expected: spec.original_count,
            got: spec.entries.len(),
        });
    }

    // Stable sort: same-X entries preserve their caller-provided order (some
    // vanilla SMB3 segments stack enemies at the same X column with different
    // Y, and the tie-breaker order can matter for activation).
    let mut entries = spec.entries.to_vec();
    entries.sort_by_key(|e| e.x);

    let base = spec.file_offset + 1;
    for (i, entry) in entries.iter().enumerate() {
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
        bytes[0] = b'N';
        bytes[1] = b'E';
        bytes[2] = b'S';
        bytes[3] = 0x1A;
        bytes[4] = 16;
        bytes[5] = 16;
        bytes[6] = 0x40;
        bytes[7] = 0x10;
        Rom::from_bytes_lax(&bytes, true).unwrap()
    }

    fn spec<'a>(file_offset: usize, count: usize, entries: &'a [SegmentEntry]) -> SegmentSpec<'a> {
        SegmentSpec { file_offset, original_count: count, entries, label: None }
    }

    #[test]
    fn sorts_by_x_before_write() {
        let mut rom = make_rom();
        let entries = [
            SegmentEntry { obj_id: 0x9E, x: 0x30, y: 0x14 },
            SegmentEntry { obj_id: 0x53, x: 0x10, y: 0x0F },
            SegmentEntry { obj_id: 0x9E, x: 0x20, y: 0x12 },
        ];
        write_segment(&mut rom, &spec(0x1000, 3, &entries)).unwrap();
        // After write: 0x10 first, then 0x20, then 0x30.
        assert_eq!(rom.read_byte(0x1001), 0x53);
        assert_eq!(rom.read_byte(0x1002), 0x10);
        assert_eq!(rom.read_byte(0x1004), 0x9E);
        assert_eq!(rom.read_byte(0x1005), 0x20);
        assert_eq!(rom.read_byte(0x1007), 0x9E);
        assert_eq!(rom.read_byte(0x1008), 0x30);
    }

    #[test]
    fn rejects_count_mismatch() {
        let mut rom = make_rom();
        let entries = [SegmentEntry { obj_id: 0x9E, x: 0x10, y: 0x14 }];
        let err = write_segment(&mut rom, &spec(0x1000, 3, &entries)).unwrap_err();
        assert!(matches!(err, WriteError::CountMismatch { .. }));
    }

    #[test]
    fn allows_same_x_different_y() {
        // Vanilla SMB3 stacks enemies at the same X column with different Y
        // (e.g. cannon-fire emplacements). The writer must accept these.
        let mut rom = make_rom();
        let entries = [
            SegmentEntry { obj_id: 0x9E, x: 0x20, y: 0x14 },
            SegmentEntry { obj_id: 0x53, x: 0x20, y: 0x0F },
        ];
        write_segment(&mut rom, &spec(0x1000, 2, &entries)).unwrap();
        let back = read_segment(&rom, 0x1000, 2);
        // Stable sort preserves input order for ties.
        assert_eq!(back[0].obj_id, 0x9E);
        assert_eq!(back[1].obj_id, 0x53);
    }

    #[test]
    fn read_roundtrips() {
        let mut rom = make_rom();
        let entries = [
            SegmentEntry { obj_id: 0x3F, x: 0x04, y: 0x18 },
            SegmentEntry { obj_id: 0x75, x: 0x62, y: 0x16 },
        ];
        write_segment(&mut rom, &spec(0x2000, 2, &entries)).unwrap();
        let back = read_segment(&rom, 0x2000, 2);
        assert_eq!(back, entries.to_vec());
    }

    #[test]
    fn label_embedded_in_error_message() {
        // Trigger a CountMismatch (the remaining error variant) with a
        // labeled spec and verify the label appears in the formatted error.
        let mut rom = make_rom();
        let entries = [SegmentEntry { obj_id: 0x9E, x: 0x10, y: 0x14 }];
        let spec_labeled = SegmentSpec {
            file_offset: 0x1000,
            original_count: 3,
            entries: &entries,
            label: Some("test segment"),
        };
        let err = write_segment(&mut rom, &spec_labeled).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("test segment"), "expected label in error: {msg}");
        assert!(msg.contains("0x01000"), "expected offset in error: {msg}");
    }
}
