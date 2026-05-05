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

/// Validate that an IPS patch only modifies "visual" data.
///
/// Policy:
/// - **CHR writes** (entirely within `chr_range`) are accepted unconditionally —
///   that region is graphics tiles, full byte range.
/// - **PRG writes** (outside CHR) are accepted only when every byte being
///   written is `<= 0x3F`, the NES color value range. This admits palette
///   mods (the most common reason a "visual" patch touches PRG) without
///   opening the door to arbitrary code rewrites: useful 6502 opcodes like
///   JMP ($4C), JSR ($20 alone is ≤0x3F but its target bytes typically aren't),
///   RTS ($60), LDA-imm ($A9), and most branches (BNE $D0, BEQ $F0) live
///   above 0x3F.
/// - **Forbidden zones** in `forbidden` are always rejected, even when their
///   vanilla bytes happen to be `<= 0x3F`. The iNES header and the level-
///   layout pointer table at `0x377E0..0x37807` belong here — rewriting them
///   would change ROM identity or crash the game.
///
/// Note: IPS records can extend the file. Writes that end past `chr_range.1`
/// are treated as "outside CHR" and fall to the byte check, so a visual
/// patch can't silently append arbitrary trailing data.
pub fn validate_visual_only(
    patch: &[u8],
    chr_range: (usize, usize),
    forbidden: &[(usize, usize)],
) -> Result<(), String> {
    if patch.len() < 8 {
        return Err("Patch too small".to_string());
    }
    if &patch[0..5] != HEADER {
        return Err("Invalid IPS header".to_string());
    }

    let mut pos = 5;
    let mut bad: Vec<String> = Vec::new();

    loop {
        if pos + 3 > patch.len() {
            return Err("Unexpected end of patch".to_string());
        }
        if &patch[pos..pos + 3] == FOOTER {
            break;
        }

        let offset = ((patch[pos] as usize) << 16)
            | ((patch[pos + 1] as usize) << 8)
            | (patch[pos + 2] as usize);
        pos += 3;

        if pos + 2 > patch.len() {
            return Err("Unexpected end of patch reading size".to_string());
        }
        let size = ((patch[pos] as usize) << 8) | (patch[pos + 1] as usize);
        pos += 2;

        let (write_len, all_palette_bytes) = if size == 0 {
            // RLE record: 2-byte count + 1-byte value
            if pos + 3 > patch.len() {
                return Err("Unexpected end of patch reading RLE data".to_string());
            }
            let rle_count = ((patch[pos] as usize) << 8) | (patch[pos + 1] as usize);
            let rle_value = patch[pos + 2];
            pos += 3;
            (rle_count, rle_value <= 0x3F)
        } else {
            if pos + size > patch.len() {
                return Err("Unexpected end of patch reading payload".to_string());
            }
            let all = patch[pos..pos + size].iter().all(|&b| b <= 0x3F);
            pos += size;
            (size, all)
        };

        let end = offset + write_len;

        // Forbidden zones are absolute — overlap rejects regardless of bytes.
        if forbidden.iter().any(|&(s, e)| offset < e && end > s) {
            bad.push(format!("0x{offset:05X}-0x{end:05X} (forbidden zone)"));
            continue;
        }

        // CHR writes are always OK.
        if offset >= chr_range.0 && end <= chr_range.1 {
            continue;
        }

        // Outside CHR: must look like palette data (every byte <= 0x3F).
        if !all_palette_bytes {
            bad.push(format!("0x{offset:05X}-0x{end:05X} (PRG, contains non-palette bytes)"));
        }
    }

    if !bad.is_empty() {
        let shown: Vec<&str> = bad.iter().take(5).map(String::as_str).collect();
        let extra = if bad.len() > 5 {
            format!(" (+{} more)", bad.len() - 5)
        } else {
            String::new()
        };
        return Err(format!(
            "Visual patch rejected: {}{}",
            shown.join(", "),
            extra
        ));
    }

    Ok(())
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

    // SMB3 USA Rev 1 layout used by the visual-patch validator.
    const CHR: (usize, usize) = (0x40010, 0x60010);
    const FORBIDDEN: &[(usize, usize)] = &[
        (0x00000, 0x00010), // iNES header
        (0x377E0, 0x37808), // level layout pointer table
    ];
    const ROM_END: usize = 0x60010;

    #[test]
    fn validate_visual_accepts_chr_only_patch() {
        let original = vec![0u8; ROM_END];
        let mut modified = original.clone();
        modified[CHR.0 + 100] = 0xAA;
        modified[CHR.0 + 1024] = 0xBB; // arbitrary CHR bytes including >0x3F
        let patch = build_ips_patch(&original, &modified);
        validate_visual_only(&patch, CHR, FORBIDDEN).expect("CHR-only patch should validate");
    }

    #[test]
    fn validate_visual_accepts_palette_only_prg_patch() {
        let original = vec![0u8; ROM_END];
        let mut modified = original.clone();
        // Write a 4-byte palette quartet (all bytes <= 0x3F) at a PRG location.
        modified[0x10539] = 0x00;
        modified[0x1053A] = 0x16;
        modified[0x1053B] = 0x36;
        modified[0x1053C] = 0x0F;
        let patch = build_ips_patch(&original, &modified);
        validate_visual_only(&patch, CHR, FORBIDDEN).expect("palette-only PRG patch should validate");
    }

    #[test]
    fn validate_visual_rejects_code_like_prg_write() {
        let original = vec![0u8; ROM_END];
        let mut modified = original.clone();
        // 6502 'JMP $C000' = $4C $00 $C0 — has bytes > 0x3F.
        modified[0x10000] = 0x4C;
        modified[0x10001] = 0x00;
        modified[0x10002] = 0xC0;
        let patch = build_ips_patch(&original, &modified);
        let err = validate_visual_only(&patch, CHR, FORBIDDEN).unwrap_err();
        assert!(err.contains("non-palette"), "error should call out non-palette bytes: {err}");
    }

    #[test]
    fn validate_visual_rejects_header_write_even_when_bytes_are_low() {
        let original = vec![0u8; ROM_END];
        let mut modified = original.clone();
        // iNES header byte 4 = PRG bank count. Setting to 0x10 (16) is <=0x3F
        // but still must be rejected — it changes ROM identity.
        modified[4] = 0x10;
        let patch = build_ips_patch(&original, &modified);
        let err = validate_visual_only(&patch, CHR, FORBIDDEN).unwrap_err();
        assert!(err.contains("forbidden zone"), "error should call out forbidden zone: {err}");
    }

    #[test]
    fn validate_visual_rejects_pointer_table_write() {
        let original = vec![0u8; ROM_END];
        let mut modified = original.clone();
        // Write into the level-layout pointer table — these bytes <=0x3F in vanilla
        // but rewriting them crashes level loading.
        modified[0x377E0] = 0x00;
        modified[0x377E1] = 0x10;
        let patch = build_ips_patch(&original, &modified);
        let err = validate_visual_only(&patch, CHR, FORBIDDEN).unwrap_err();
        assert!(err.contains("forbidden zone"), "error should call out forbidden zone: {err}");
    }

    #[test]
    fn validate_visual_rejects_extension_past_rom_end() {
        // Hand-craft a patch that writes a non-palette byte at ROM_END (extends file).
        let mut patch = b"PATCH".to_vec();
        let off = ROM_END;
        patch.push(((off >> 16) & 0xFF) as u8);
        patch.push(((off >> 8) & 0xFF) as u8);
        patch.push((off & 0xFF) as u8);
        patch.push(0x00);
        patch.push(0x01);
        patch.push(0xCC);
        patch.extend_from_slice(b"EOF");
        assert!(validate_visual_only(&patch, CHR, FORBIDDEN).is_err());
    }

    #[test]
    fn validate_visual_rle_palette_into_prg_ok() {
        // RLE writing 16 bytes of value 0x16 (NES color) into PRG palette region.
        let mut patch = b"PATCH".to_vec();
        let off = 0x36C54;
        patch.push(((off >> 16) & 0xFF) as u8);
        patch.push(((off >> 8) & 0xFF) as u8);
        patch.push((off & 0xFF) as u8);
        patch.push(0x00); // size=0 → RLE
        patch.push(0x00);
        patch.push(0x00); // count high
        patch.push(0x10); // count low (16 bytes)
        patch.push(0x16); // value (palette byte)
        patch.extend_from_slice(b"EOF");
        validate_visual_only(&patch, CHR, FORBIDDEN).expect("palette RLE in PRG should validate");
    }

    #[test]
    fn validate_visual_rle_rejects_code_byte_into_prg() {
        // RLE filling PRG with 0xEA (NOP opcode) — not a palette byte.
        let mut patch = b"PATCH".to_vec();
        let off = 0x10000;
        patch.push(((off >> 16) & 0xFF) as u8);
        patch.push(((off >> 8) & 0xFF) as u8);
        patch.push((off & 0xFF) as u8);
        patch.push(0x00);
        patch.push(0x00);
        patch.push(0x00);
        patch.push(0x10);
        patch.push(0xEA); // NOP — not a palette byte
        patch.extend_from_slice(b"EOF");
        assert!(validate_visual_only(&patch, CHR, FORBIDDEN).is_err());
    }
}
