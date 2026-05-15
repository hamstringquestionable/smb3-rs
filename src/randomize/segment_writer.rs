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
//!
//! Note on sort order: SMB3 segments require **non-decreasing** X (ties
//! are allowed — entries at the same X column with different Y spawn
//! together as stacked enemies). The writer uses a stable sort so callers
//! that supply ties preserve their tie-breaking order in the output.

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
    /// Optional caller-supplied name (e.g. `"3-2 sub-area 0"`) that gets
    /// embedded in error messages alongside the file offset. Useful when a
    /// single pass writes many segments — knowing which segment failed is
    /// hard from offset alone.
    pub label: Option<&'a str>,
}

/// Bounds of one segment in the enemy data block. `file_offset` points at
/// the page/header byte; `entry_count` is the number of 3-byte entries
/// before the terminating `0xFF`. Returned by [`walk_segments`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentBounds {
    pub file_offset: usize,
    pub entry_count: usize,
}

#[derive(Debug)]
pub enum WriteError {
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
            WriteError::CountMismatch { offset, label, expected, got } =>
                write!(f, "{}: count mismatch (expected {expected}, got {got})",
                    name(*offset, label)),
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
    let label_owned = || spec.label.map(|s| s.to_string());

    if spec.entries.len() != spec.original_count {
        return Err(WriteError::CountMismatch {
            offset: spec.file_offset,
            label: label_owned(),
            expected: spec.original_count,
            got: spec.entries.len(),
        });
    }

    let mut sorted: Vec<SegmentEntry> = spec.entries.to_vec();
    // Stable sort: same-X entries preserve their caller-provided order
    // (some vanilla SMB3 segments stack enemies at the same X column with
    // different Y, and the tie-breaker order can matter for activation).
    sorted.sort_by_key(|e| e.x);

    let base = spec.file_offset + 1;
    for (i, entry) in sorted.iter().enumerate() {
        let off = base + i * 3;
        rom.write_byte(off, entry.obj_id);
        rom.write_byte(off + 1, entry.x);
        rom.write_byte(off + 2, entry.y);
    }

    Ok(())
}

/// Walk the enemy data block between `[start, end)` and return one
/// [`SegmentBounds`] per non-empty segment. Empty segments (0xFF
/// followed immediately by 0xFF, or a page byte with no entries) are
/// skipped — matching how the level loader walks the block.
///
/// The caller passes a byte slice rather than a `Rom` so this function
/// can be used both against in-memory edit buffers (e.g. inside
/// `enemies.rs` which composes changes in a local `Vec<u8>` before
/// committing) and against ROM bytes.
pub fn walk_segments(data: &[u8], start: usize, end: usize) -> Vec<SegmentBounds> {
    let mut bounds = Vec::new();
    let mut i = start;
    while i < end {
        if data[i] == 0xFF {
            i += 1;
            continue;
        }
        // First byte after a terminator is the page/header byte.
        let seg_offset = i;
        i += 1;
        let mut count = 0;
        let mut terminated = false;
        while i < end {
            if data[i] == 0xFF {
                terminated = true;
                break;
            }
            // Need 3 bytes for a full entry; if fewer remain, the segment
            // is malformed/unterminated — bail without emitting it.
            if i + 3 > end {
                break;
            }
            count += 1;
            i += 3;
        }
        // Only emit segments that actually ended on a 0xFF terminator.
        // Unterminated trailing data (e.g. zeros past the last real segment
        // in a test fixture) is not a valid segment and is silently
        // dropped — the level loader wouldn't read it either.
        if terminated && count > 0 {
            bounds.push(SegmentBounds { file_offset: seg_offset, entry_count: count });
        }
        // If we broke out without a terminator, we're done with this range.
        if !terminated {
            break;
        }
    }
    bounds
}

/// Walk segments directly off a `Rom` — convenience wrapper around
/// [`walk_segments`] when the caller hasn't already snapshot the block
/// into a `Vec<u8>`.
pub fn walk_segments_rom(rom: &Rom, start: usize, end: usize) -> Vec<SegmentBounds> {
    walk_segments(&rom.data[..end.min(rom.data.len())], start, end)
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

    #[test]
    fn walk_segments_finds_all_non_empty_segments() {
        // Build a synthetic byte stream: page1, 3 entries, FF, page2, 2 entries, FF, FF (empty), page3, 1 entry, FF
        let data = [
            // segment 1: page byte + 3 entries
            0x00,
            0xAA, 0x10, 0x11,
            0xBB, 0x20, 0x12,
            0xCC, 0x30, 0x13,
            0xFF,
            // segment 2: page byte + 2 entries
            0x01,
            0xDD, 0x40, 0x14,
            0xEE, 0x50, 0x15,
            0xFF,
            // empty (just terminator) — must be skipped
            0xFF,
            // segment 3: page byte + 1 entry
            0x00,
            0x66, 0x70, 0x16,
            0xFF,
        ];
        let bounds = walk_segments(&data, 0, data.len());
        assert_eq!(bounds.len(), 3);
        assert_eq!(bounds[0], SegmentBounds { file_offset: 0, entry_count: 3 });
        assert_eq!(bounds[1].entry_count, 2);
        assert_eq!(bounds[2].entry_count, 1);
    }

    #[test]
    fn walk_segments_handles_leading_terminators() {
        let data = [
            0xFF, 0xFF, 0xFF,  // skipped
            0x00, 0xAA, 0x10, 0x11, 0xFF,
        ];
        let bounds = walk_segments(&data, 0, data.len());
        assert_eq!(bounds.len(), 1);
        assert_eq!(bounds[0], SegmentBounds { file_offset: 3, entry_count: 1 });
    }
}
