//! Safe writes to enemy data segments.
//!
//! Each enemy data segment is a packed stream of 3-byte entries
//! `[obj_id, x, y]`, preceded by a single page/header byte and terminated
//! by `0xFF`. The level loader walks entries sequentially as Mario's
//! screen advances. For a logical level read by one `enemy_ptr`, entries
//! are typically X-sorted so activation timing tracks screen progression.
//!
//! This module is the single throat for segment edits. Two use cases:
//!
//! * **Composers** ([`bowser_castle`](super::bowser_castle), [`podoboo_gauntlet`](super::podoboo_gauntlet), [`hand_rooms`](super::hand_rooms)):
//!   build the entry list for one specific segment whose `enemy_ptr` is
//!   known (the 5-F2 and 8-Bowser composers start from the room as the
//!   enemy pass left it and move only their own hazards). These supply
//!   [`SortMode::SortByX`] so the writer sorts before writing.
//!
//! * **In-place mutators** ([`enemies`](crate::randomize::enemies)): visit every object list the
//!   level index reaches and mutate individual obj_ids without changing
//!   count, or X/Y beyond a row nudge. Vanilla lists are not always
//!   X-sorted (5-F2's gauntlet, 6-6's sub-area spikes), and an in-place
//!   pass has no reason to reorder them. These supply
//!   [`SortMode::Preserve`] to keep vanilla byte order.
//!
//! Callers pass an entry list, a count, and a [`SortMode`]; the writer
//! validates count and writes back. It does NOT grow or shrink segments,
//! insert obj_ids the level loader can't handle, or coordinate writes
//! across segments.

use crate::rom::Rom;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SegmentEntry {
    pub(crate) obj_id: u8,
    pub(crate) x: u8,
    pub(crate) y: u8,
}

/// How `write_segment` should treat the caller's entry order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SortMode {
    /// Sort entries by ascending X before writing. Use when assembling a
    /// segment from scratch (composers): you know the segment is one
    /// logical level and want SMB3's expected X-sorted layout.
    SortByX,
    /// Write entries in the caller-supplied order, byte-for-byte. Use
    /// when mutating in place over a region that may bridge multiple
    /// logical levels (block-wide walker passes): preserving vanilla
    /// byte order avoids reordering entries across level boundaries that
    /// the walker can't see.
    Preserve,
}

pub(crate) struct SegmentSpec<'a> {
    /// File offset of the segment's page/header byte. Entries start at
    /// `file_offset + 1`.
    pub(crate) file_offset: usize,
    /// Expected entry count (defends against accidental segment growth/shrink).
    pub(crate) original_count: usize,
    /// Proposed entries. Treated per `sort_mode`.
    pub(crate) entries: &'a [SegmentEntry],
    /// Optional caller-supplied name (e.g. `"3-2 sub-area 0"`) that gets
    /// embedded in error messages alongside the file offset. Useful when a
    /// single pass writes many segments — knowing which segment failed is
    /// hard from offset alone.
    pub(crate) label: Option<&'a str>,
    /// Whether the writer should sort entries by X (composers) or write
    /// them in caller-supplied order (in-place mutators).
    pub(crate) sort_mode: SortMode,
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

/// Validate `entries`, optionally sort by X per `spec.sort_mode`, and
/// write back to the segment. Errors are returned rather than panicking
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

    let entries: Vec<SegmentEntry> = match spec.sort_mode {
        SortMode::SortByX => {
            // Stable sort: same-X entries preserve their caller-provided
            // order (some vanilla SMB3 segments stack enemies at the same
            // X column with different Y, and the tie-breaker order can
            // matter for activation).
            let mut s = spec.entries.to_vec();
            s.sort_by_key(|e| e.x);
            s
        }
        SortMode::Preserve => spec.entries.to_vec(),
    };

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
        SegmentSpec {
            file_offset,
            original_count: count,
            entries,
            label: None,
            sort_mode: SortMode::SortByX,
        }
    }

    fn spec_preserve<'a>(
        file_offset: usize,
        count: usize,
        entries: &'a [SegmentEntry],
    ) -> SegmentSpec<'a> {
        SegmentSpec {
            file_offset,
            original_count: count,
            entries,
            label: None,
            sort_mode: SortMode::Preserve,
        }
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
    fn preserve_mode_writes_entries_in_input_order() {
        // SortByX would reorder these by ascending x; Preserve must not.
        let mut rom = make_rom();
        let entries = [
            SegmentEntry { obj_id: 0x9E, x: 0x30, y: 0x14 },
            SegmentEntry { obj_id: 0x53, x: 0x10, y: 0x0F },
            SegmentEntry { obj_id: 0xA5, x: 0x20, y: 0x12 },
        ];
        write_segment(&mut rom, &spec_preserve(0x1000, 3, &entries)).unwrap();
        // Byte-for-byte equal to input order — x values stay non-monotonic.
        assert_eq!(rom.read_byte(0x1001), 0x9E);
        assert_eq!(rom.read_byte(0x1002), 0x30);
        assert_eq!(rom.read_byte(0x1004), 0x53);
        assert_eq!(rom.read_byte(0x1005), 0x10);
        assert_eq!(rom.read_byte(0x1007), 0xA5);
        assert_eq!(rom.read_byte(0x1008), 0x20);
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
            sort_mode: SortMode::SortByX,
        };
        let err = write_segment(&mut rom, &spec_labeled).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("test segment"), "expected label in error: {msg}");
        assert!(msg.contains("0x01000"), "expected offset in error: {msg}");
    }
}
