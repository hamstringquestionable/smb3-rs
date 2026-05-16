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
//! * **Composers** ([`bowser_castle`], [`podoboo_gauntlet`], [`hand_rooms`]):
//!   assemble a fresh entry list for one specific segment whose
//!   `enemy_ptr` is known. These supply [`SortMode::SortByX`] so the
//!   writer sorts before writing.
//!
//! * **In-place mutators** ([`enemies`]): walk the whole enemy data block
//!   and mutate individual obj_ids without changing X/Y or count. A
//!   walker-segment in the block-wide view often spans multiple logical
//!   levels (different `enemy_ptr`s pointing at different positions),
//!   each with its own X sequence — so a segment-wide X-sort is not just
//!   wasted work, it can move entries across logical-level boundaries.
//!   These supply [`SortMode::Preserve`] to keep vanilla byte order.
//!
//! Callers pass an entry list, a count, and a [`SortMode`]; the writer
//! validates count and writes back. It does NOT grow or shrink segments,
//! insert obj_ids the level loader can't handle, or coordinate writes
//! across segments.

use crate::rom::Rom;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentEntry {
    pub obj_id: u8,
    pub x: u8,
    pub y: u8,
}

/// How `write_segment` should treat the caller's entry order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortMode {
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

pub struct SegmentSpec<'a> {
    /// File offset of the segment's page/header byte. Entries start at
    /// `file_offset + 1`.
    pub file_offset: usize,
    /// Expected entry count (defends against accidental segment growth/shrink).
    pub original_count: usize,
    /// Proposed entries. Treated per `sort_mode`.
    pub entries: &'a [SegmentEntry],
    /// Optional caller-supplied name (e.g. `"3-2 sub-area 0"`) that gets
    /// embedded in error messages alongside the file offset. Useful when a
    /// single pass writes many segments — knowing which segment failed is
    /// hard from offset alone.
    pub label: Option<&'a str>,
    /// Whether the writer should sort entries by X (composers) or write
    /// them in caller-supplied order (in-place mutators).
    pub sort_mode: SortMode,
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

/// Validate `entries`, optionally sort by X per `spec.sort_mode`, and
/// write back to the segment. Errors are returned rather than panicking
/// so callers can decide whether a failure is recoverable.
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

/// Walk the enemy data block between `[start, end)` and return one
/// [`SegmentBounds`] per non-empty segment. Empty segments (0xFF
/// followed immediately by 0xFF, or a page byte with no entries) are
/// skipped — matching how the level loader walks the block.
///
/// `skip_ranges` are half-open byte ranges (same frame of reference as
/// `start`/`end`) that the walker jumps over entirely. Use this when an
/// earlier pass has rewritten certain segments to a form the per-level
/// loader handles fine but that would confuse the greedy block-wide
/// walker — e.g. `disable_autoscroll` inserts `$FF` mid-segment, which
/// without a skip range would create a "ghost" segment swallowing the
/// next real segment's page byte. Pass `&[]` for no skipping.
///
/// The caller passes a byte slice rather than a `Rom` so this function
/// can be used both against in-memory edit buffers (e.g. inside
/// `enemies.rs` which composes changes in a local `Vec<u8>` before
/// committing) and against ROM bytes.
pub fn walk_segments(
    data: &[u8],
    start: usize,
    end: usize,
    skip_ranges: &[core::ops::Range<usize>],
) -> Vec<SegmentBounds> {
    let in_skip = |i: usize| -> Option<usize> {
        skip_ranges.iter().find(|r| r.contains(&i)).map(|r| r.end)
    };
    let mut bounds = Vec::new();
    let mut i = start;
    while i < end {
        if let Some(skip_end) = in_skip(i) {
            i = skip_end;
            continue;
        }
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
            // Treat entering a skip range mid-segment as if we hit the
            // segment's terminator. The skip range starts on a boundary
            // a level loader would also treat as a stop (it covers
            // bytes a per-level loader would not read), so cutting the
            // segment there matches loader semantics.
            if in_skip(i).is_some() {
                break;
            }
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
    walk_segments(&rom.data[..end.min(rom.data.len())], start, end, &[])
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
        SegmentSpec {
            file_offset,
            original_count: count,
            entries,
            label: None,
            sort_mode: SortMode::SortByX,
        }
    }

    fn spec_preserve<'a>(file_offset: usize, count: usize, entries: &'a [SegmentEntry]) -> SegmentSpec<'a> {
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
        let bounds = walk_segments(&data, 0, data.len(), &[]);
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
        let bounds = walk_segments(&data, 0, data.len(), &[]);
        assert_eq!(bounds.len(), 1);
        assert_eq!(bounds[0], SegmentBounds { file_offset: 3, entry_count: 1 });
    }

    #[test]
    fn walk_segments_honors_skip_ranges() {
        // Mimics the disable_autoscroll post-patch layout: an "autoscroll"
        // segment was clobbered to [01 FF 00 10 FF], creating a ghost
        // segment that swallows the page byte + first entry of the next
        // real segment. With the spoiled range marked, the walker should
        // skip the clobbered bytes and find ONLY the real segment.
        let data = [
            0x01, 0xFF, 0x00, 0x10, 0xFF, // clobbered "autoscroll" — skip range covers this
            0x01, 0x25, 0x00, 0x80, 0xFF, // real segment with PIPEWAYCONTROLLER
        ];
        // Without skip ranges: ghost segment at index 2 swallows the real one
        let no_skip = walk_segments(&data, 0, data.len(), &[]);
        assert_ne!(
            no_skip.iter().map(|b| b.file_offset).collect::<Vec<_>>(),
            vec![5],
            "baseline: walker should mis-identify segment bounds without skip",
        );
        // With the spoiled segment skipped: exactly one segment at index 5
        // Reason: an explicit array literal of one Range is the most direct
        // expression of intent here; clippy's suggestion to collect a Vec
        // would mean something completely different.
        #[allow(clippy::single_range_in_vec_init)]
        let skip = [0..5];
        let with_skip = walk_segments(&data, 0, data.len(), &skip);
        assert_eq!(with_skip.len(), 1);
        assert_eq!(with_skip[0], SegmentBounds { file_offset: 5, entry_count: 1 });
    }
}
