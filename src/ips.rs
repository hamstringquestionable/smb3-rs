const HEADER: &[u8] = b"PATCH";
const FOOTER: &[u8] = b"EOF";

// Max sizes for IPS format: 3-byte offset, 2-byte length
const MAX_OFFSET: usize = 0xFFFFFF;
const MAX_RECORD_LEN: usize = 0xFFFF;

// Minimum run length where RLE saves space over raw data.
// RLE record = 3 (offset) + 2 (zero size) + 2 (rle count) + 1 (value) = 8 bytes
// Raw record = 3 (offset) + 2 (size) + N (data) = 5 + N bytes
// RLE saves when N >= 4, but we use a higher threshold to avoid many tiny RLE records.
const MIN_RLE_RUN: usize = 8;

/// Build an IPS patch by diffing original and modified byte slices.
pub fn build_ips_patch(original: &[u8], modified: &[u8]) -> Vec<u8> {
    assert_eq!(original.len(), modified.len(), "ROM sizes must match for diffing");

    let mut patch = Vec::new();
    patch.extend_from_slice(HEADER);

    let len = original.len().min(MAX_OFFSET + MAX_RECORD_LEN);
    let mut i = 0;

    while i < len {
        // Skip identical bytes
        if original[i] == modified[i] {
            i += 1;
            continue;
        }

        // Found a diff region — find its extent
        let start = i;
        while i < len && original[i] != modified[i] {
            i += 1;
            // Allow small gaps of identical bytes (up to 3) to be absorbed into the record
            // to avoid fragmenting into many tiny records
            if i < len && original[i] == modified[i] {
                let gap_start = i;
                while i < len && original[i] == modified[i] && (i - gap_start) < 4 {
                    i += 1;
                }
                if i < len && original[i] != modified[i] {
                    // Absorb the gap
                    continue;
                } else {
                    // Gap was at the end of diffs, rewind
                    i = gap_start;
                    break;
                }
            }
        }

        let region = &modified[start..i];
        write_records(&mut patch, start, region);
    }

    patch.extend_from_slice(FOOTER);
    patch
}

/// Write one or more IPS records for a diff region (splitting if > MAX_RECORD_LEN).
fn write_records(patch: &mut Vec<u8>, start: usize, data: &[u8]) {
    let mut offset = start;
    let mut remaining = data;

    while !remaining.is_empty() {
        let chunk_len = remaining.len().min(MAX_RECORD_LEN);
        let chunk = &remaining[..chunk_len];

        // Check if this chunk is an RLE candidate (all same byte)
        if chunk_len >= MIN_RLE_RUN && chunk.iter().all(|&b| b == chunk[0]) {
            write_rle_record(patch, offset, chunk_len, chunk[0]);
        } else {
            write_raw_record(patch, offset, chunk);
        }

        offset += chunk_len;
        remaining = &remaining[chunk_len..];
    }
}

fn write_raw_record(patch: &mut Vec<u8>, offset: usize, data: &[u8]) {
    // 3-byte offset (big-endian)
    patch.push(((offset >> 16) & 0xFF) as u8);
    patch.push(((offset >> 8) & 0xFF) as u8);
    patch.push((offset & 0xFF) as u8);
    // 2-byte size (big-endian)
    let size = data.len();
    patch.push(((size >> 8) & 0xFF) as u8);
    patch.push((size & 0xFF) as u8);
    // payload
    patch.extend_from_slice(data);
}

fn write_rle_record(patch: &mut Vec<u8>, offset: usize, count: usize, value: u8) {
    // 3-byte offset (big-endian)
    patch.push(((offset >> 16) & 0xFF) as u8);
    patch.push(((offset >> 8) & 0xFF) as u8);
    patch.push((offset & 0xFF) as u8);
    // 2-byte size = 0 (signals RLE)
    patch.push(0x00);
    patch.push(0x00);
    // 2-byte RLE count (big-endian)
    patch.push(((count >> 8) & 0xFF) as u8);
    patch.push((count & 0xFF) as u8);
    // 1-byte value
    patch.push(value);
}

/// Apply an IPS patch to a ROM, returning the patched bytes.
pub fn apply_ips_patch(rom: &[u8], patch: &[u8]) -> Result<Vec<u8>, String> {
    if patch.len() < 8 {
        return Err("Patch too small".to_string());
    }
    if &patch[0..5] != HEADER {
        return Err("Invalid IPS header".to_string());
    }

    let mut output = rom.to_vec();
    let mut pos = 5; // skip "PATCH"

    loop {
        if pos + 3 > patch.len() {
            return Err("Unexpected end of patch".to_string());
        }

        // Check for EOF marker
        if &patch[pos..pos + 3] == FOOTER {
            break;
        }

        // Read 3-byte offset
        let offset = ((patch[pos] as usize) << 16)
            | ((patch[pos + 1] as usize) << 8)
            | (patch[pos + 2] as usize);
        pos += 3;

        if pos + 2 > patch.len() {
            return Err("Unexpected end of patch reading size".to_string());
        }

        // Read 2-byte size
        let size = ((patch[pos] as usize) << 8) | (patch[pos + 1] as usize);
        pos += 2;

        if size == 0 {
            // RLE record
            if pos + 3 > patch.len() {
                return Err("Unexpected end of patch reading RLE data".to_string());
            }
            let rle_count = ((patch[pos] as usize) << 8) | (patch[pos + 1] as usize);
            let rle_value = patch[pos + 2];
            pos += 3;

            // Extend output if needed
            let end = offset + rle_count;
            if end > output.len() {
                output.resize(end, 0);
            }
            for byte in output[offset..end].iter_mut() {
                *byte = rle_value;
            }
        } else {
            // Raw record
            if pos + size > patch.len() {
                return Err("Unexpected end of patch reading payload".to_string());
            }
            let end = offset + size;
            if end > output.len() {
                output.resize(end, 0);
            }
            output[offset..end].copy_from_slice(&patch[pos..pos + size]);
            pos += size;
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_diff() {
        let data = vec![0u8; 100];
        let patch = build_ips_patch(&data, &data);
        // Should just be PATCH + EOF = 8 bytes
        assert_eq!(patch, b"PATCHEOF");
    }

    #[test]
    fn test_single_byte_diff() {
        let original = vec![0u8; 100];
        let mut modified = original.clone();
        modified[50] = 0xFF;

        let patch = build_ips_patch(&original, &modified);
        let result = apply_ips_patch(&original, &patch).unwrap();
        assert_eq!(result, modified);
    }

    #[test]
    fn test_contiguous_diff() {
        let original = vec![0u8; 100];
        let mut modified = original.clone();
        modified[10] = 0x01;
        modified[11] = 0x02;
        modified[12] = 0x03;

        let patch = build_ips_patch(&original, &modified);
        let result = apply_ips_patch(&original, &patch).unwrap();
        assert_eq!(result, modified);
    }

    #[test]
    fn test_rle_diff() {
        let original = vec![0u8; 100];
        let mut modified = original.clone();
        // Write 20 identical bytes — should trigger RLE
        for i in 10..30 {
            modified[i] = 0xAA;
        }

        let patch = build_ips_patch(&original, &modified);
        let result = apply_ips_patch(&original, &patch).unwrap();
        assert_eq!(result, modified);

        // Verify the patch used RLE (should be smaller than raw)
        // PATCH(5) + offset(3) + size=0(2) + rle_count(2) + value(1) + EOF(3) = 16
        assert_eq!(patch.len(), 16);
    }

    #[test]
    fn test_multiple_regions() {
        let original = vec![0u8; 200];
        let mut modified = original.clone();
        modified[10] = 0x01;
        modified[100] = 0x02;
        modified[150] = 0x03;

        let patch = build_ips_patch(&original, &modified);
        let result = apply_ips_patch(&original, &patch).unwrap();
        assert_eq!(result, modified);
    }

    #[test]
    fn test_roundtrip_random_diffs() {
        let original: Vec<u8> = (0..1000).map(|i| (i % 256) as u8).collect();
        let mut modified = original.clone();
        modified[0] = 0xFF;
        modified[100] = 0xFE;
        modified[500] = 0xFD;
        modified[999] = 0xFC;

        let patch = build_ips_patch(&original, &modified);
        let result = apply_ips_patch(&original, &patch).unwrap();
        assert_eq!(result, modified);
    }

    #[test]
    fn test_invalid_patch() {
        let rom = vec![0u8; 100];
        assert!(apply_ips_patch(&rom, b"GARBAGE").is_err());
        assert!(apply_ips_patch(&rom, b"PAT").is_err());
    }
}
