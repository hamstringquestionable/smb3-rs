use std::fmt;

const INES_MAGIC: [u8; 4] = [0x4E, 0x45, 0x53, 0x1A]; // "NES\x1a"
const HEADER_SIZE: usize = 16;

// Expected values for SMB3 (USA Rev 1)
const EXPECTED_PRG_PAGES: u8 = 16; // 16 x 16KB = 256KB
const EXPECTED_CHR_PAGES: u8 = 16; // 16 x 8KB = 128KB
const PRG_PAGE_SIZE: usize = 16384; // 16KB
const CHR_PAGE_SIZE: usize = 8192; // 8KB

// CRC32 (IEEE) of the unheadered PRG+CHR payload (393,216 bytes).
// Header-insensitive so it matches regardless of iNES flag variations or
// whether the input came in headered or unheadered. The canonical no-intro
// CRCs of the headered files are SMB3 (USA) = 0x85A79D9C and
// SMB3 (USA) (Rev 1) = 0x0B742B33; we don't compute those because we already
// strip/synthesize the header before this check.
const PRG_REV0_PAYLOAD_CRC32: u32 = 0xA0B0_B742;
const PRG_REV1_PAYLOAD_CRC32: u32 = 0x2E63_01ED;

/// CRC-32/IEEE (zlib polynomial 0xEDB88320, reflected). Slow byte-by-byte loop,
/// fine for a single 384 KiB pass at load time.
fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

const SUPPORTED_ROM_HELP: &str =
    "This randomizer requires \"Super Mario Bros. 3 (USA) (Rev 1)\" — the US Rev 1 release \
     in iNES (.nes) format, with or without an iNES header. The original (Rev 0 / PRG0) release \
     is not supported because it has bugs (e.g. the World 7-1 card-graphics glitch) that the \
     randomizer's hooks rely on having been fixed. Pass --skip-rom-validation \
     (or check the skip-validation box in the web UI) to bypass this check at your own risk.";

#[derive(Debug)]
pub enum RomError {
    TooSmall(usize),
    BadMagic([u8; 4]),
    UnexpectedPrg { expected: u8, got: u8 },
    UnexpectedChr { expected: u8, got: u8 },
    SizeMismatch { expected: usize, got: usize },
    /// ROM payload CRC matches the older USA (Rev 0 / PRG0) release.
    WrongRevisionPrg0,
    /// ROM payload CRC matches neither Rev 0 nor Rev 1.
    UnknownRevision { payload_crc32: u32 },
}

impl fmt::Display for RomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RomError::TooSmall(size) => {
                write!(
                    f,
                    "This doesn't appear to be a valid NES ROM \
                     (file is only {size} bytes). {SUPPORTED_ROM_HELP}"
                )
            }
            RomError::BadMagic(_) => {
                write!(
                    f,
                    "This file is not a valid NES ROM \
                     (missing iNES header). {SUPPORTED_ROM_HELP}"
                )
            }
            RomError::UnexpectedPrg { got, .. } => {
                write!(
                    f,
                    "This ROM has {got} PRG pages, but SMB3 (USA) should have \
                     {EXPECTED_PRG_PAGES}. You may have a different game or region. \
                     {SUPPORTED_ROM_HELP}"
                )
            }
            RomError::UnexpectedChr { got, .. } => {
                write!(
                    f,
                    "This ROM has {got} CHR pages, but SMB3 (USA) should have \
                     {EXPECTED_CHR_PAGES}. You may have a different game or region. \
                     {SUPPORTED_ROM_HELP}"
                )
            }
            RomError::SizeMismatch { expected, got } => {
                write!(
                    f,
                    "ROM size is {got} bytes, expected {expected}. \
                     The file may be corrupt or a different version. {SUPPORTED_ROM_HELP}"
                )
            }
            RomError::WrongRevisionPrg0 => {
                write!(
                    f,
                    "This is \"Super Mario Bros. 3 (USA)\" (Rev 0 / PRG0), the original 1990 \
                     release. {SUPPORTED_ROM_HELP}"
                )
            }
            RomError::UnknownRevision { payload_crc32 } => {
                write!(
                    f,
                    "This ROM is not a recognized SMB3 (USA) dump \
                     (unheadered payload CRC32 0x{payload_crc32:08X}, \
                     expected 0x{PRG_REV1_PAYLOAD_CRC32:08X} for Rev 1). \
                     {SUPPORTED_ROM_HELP}"
                )
            }
        }
    }
}

/// A single recorded ROM write operation.
#[derive(Clone, Debug)]
pub struct WriteRecord {
    pub offset: usize,
    pub len: usize,
    pub old_bytes: Vec<u8>,
    pub new_bytes: Vec<u8>,
    pub tag: String,
}

/// Parsed iNES header info.
#[derive(Debug, Clone)]
pub struct Header {
    pub prg_pages: u8,
    pub chr_pages: u8,
    pub mapper: u8,
    pub mirroring_horizontal: bool,
}

/// A loaded NES ROM with original bytes preserved for diffing.
#[derive(Clone)]
pub struct Rom {
    pub original: Vec<u8>,
    pub data: Vec<u8>,
    pub header: Header,
    /// True when a synthetic iNES header was prepended (unheadered input ROM).
    pub header_synthesized: bool,
    tag_stack: Vec<&'static str>,
    write_log: Vec<WriteRecord>,
}

impl Rom {
    /// Parse and validate a ROM from raw bytes.
    /// Validates it matches the expected SMB3 (USA Rev 1) layout.
    /// Accepts both headered (iNES, 393,232 bytes) and unheadered (393,216 bytes) ROMs.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RomError> {
        Self::from_bytes_lax(bytes, false)
    }

    /// Like [`from_bytes`], but optionally skips the SMB3 (USA Rev 1) layout
    /// checks (iNES magic, PRG/CHR page counts, exact size).
    ///
    /// The minimum-size check (16 bytes for the iNES header) is always
    /// enforced — without it the parser would index out of bounds while
    /// reading flag bytes. When `skip_validation` is true, the unheadered
    /// auto-detection still runs (so 393,216-byte raw ROMs are accepted),
    /// but any other shape is taken as-is and `Header` is filled in from
    /// whatever the file claims.
    pub fn from_bytes_lax(bytes: &[u8], skip_validation: bool) -> Result<Self, RomError> {
        // Detect unheadered ROM: exact raw PRG+CHR size with no iNES magic.
        const UNHEADERED_SIZE: usize =
            EXPECTED_PRG_PAGES as usize * PRG_PAGE_SIZE + EXPECTED_CHR_PAGES as usize * CHR_PAGE_SIZE;

        let (bytes, header_synthesized) = if bytes.len() == UNHEADERED_SIZE {
            let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
            if magic == INES_MAGIC {
                if !skip_validation {
                    // Unlikely: exactly UNHEADERED_SIZE but starts with NES magic — treat as corrupt
                    return Err(RomError::SizeMismatch {
                        expected: HEADER_SIZE + UNHEADERED_SIZE,
                        got: bytes.len(),
                    });
                }
                // In lax mode, fall through and accept the bytes verbatim.
                (bytes.to_vec(), false)
            } else {
                // Synthesize a standard iNES header and prepend it.
                let mut headered = Vec::with_capacity(HEADER_SIZE + UNHEADERED_SIZE);
                headered.extend_from_slice(&INES_MAGIC);
                headered.push(EXPECTED_PRG_PAGES);
                headered.push(EXPECTED_CHR_PAGES);
                headered.push(0x40); // flags6: mapper 4 lower nibble, horizontal mirroring
                headered.push(0x00); // flags7
                headered.extend_from_slice(&[0u8; 8]); // flags8–15
                headered.extend_from_slice(bytes);
                (headered, true)
            }
        } else {
            (bytes.to_vec(), false)
        };

        let bytes = &bytes;

        if bytes.len() < HEADER_SIZE {
            return Err(RomError::TooSmall(bytes.len()));
        }

        let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
        if !skip_validation && magic != INES_MAGIC {
            return Err(RomError::BadMagic(magic));
        }

        let prg_pages = bytes[4];
        let chr_pages = bytes[5];
        let flags6 = bytes[6];
        let flags7 = bytes[7];

        if !skip_validation {
            if prg_pages != EXPECTED_PRG_PAGES {
                return Err(RomError::UnexpectedPrg {
                    expected: EXPECTED_PRG_PAGES,
                    got: prg_pages,
                });
            }

            if chr_pages != EXPECTED_CHR_PAGES {
                return Err(RomError::UnexpectedChr {
                    expected: EXPECTED_CHR_PAGES,
                    got: chr_pages,
                });
            }

            let expected_size = HEADER_SIZE
                + (prg_pages as usize * PRG_PAGE_SIZE)
                + (chr_pages as usize * CHR_PAGE_SIZE);
            if bytes.len() != expected_size {
                return Err(RomError::SizeMismatch {
                    expected: expected_size,
                    got: bytes.len(),
                });
            }

            // Revision check: CRC32 the unheadered payload. Header bytes are
            // skipped because dumpers/emulators tweak iNES flags freely; the
            // payload is the actual ROM content.
            let payload_crc = crc32_ieee(&bytes[HEADER_SIZE..]);
            match payload_crc {
                PRG_REV1_PAYLOAD_CRC32 => {}
                PRG_REV0_PAYLOAD_CRC32 => return Err(RomError::WrongRevisionPrg0),
                other => return Err(RomError::UnknownRevision { payload_crc32: other }),
            }
        }

        let mapper = (flags6 >> 4) | (flags7 & 0xF0);
        let mirroring_horizontal = (flags6 & 0x01) == 0;

        let header = Header {
            prg_pages,
            chr_pages,
            mapper,
            mirroring_horizontal,
        };

        Ok(Rom {
            original: bytes.to_vec(),
            data: bytes.to_vec(),
            header,
            header_synthesized,
            tag_stack: Vec::new(),
            write_log: Vec::new(),
        })
    }

    /// Returns the output ROM bytes, stripping the synthetic header if one was added.
    pub fn output_bytes(&self) -> &[u8] {
        if self.header_synthesized {
            &self.data[HEADER_SIZE..]
        } else {
            &self.data
        }
    }

    /// Returns the original ROM bytes, stripping the synthetic header if one was added.
    pub fn original_bytes(&self) -> &[u8] {
        if self.header_synthesized {
            &self.original[HEADER_SIZE..]
        } else {
            &self.original
        }
    }

    /// Apply an IPS patch to the working data, leaving `original` untouched.
    /// Used to layer visual patches before randomization so the final IPS diff
    /// (original → data) captures both visual and randomization changes.
    ///
    /// The patch is applied against the user-visible bytes (synthetic header
    /// stripped if present), matching how external IPS patches are authored.
    pub fn apply_ips_patch(&mut self, patch: &[u8]) -> Result<(), String> {
        let view = if self.header_synthesized {
            &self.data[HEADER_SIZE..]
        } else {
            &self.data[..]
        };
        let patched = crate::ips::apply_ips_patch(view, patch)?;
        if self.header_synthesized {
            self.data[HEADER_SIZE..].copy_from_slice(&patched);
        } else {
            self.data.copy_from_slice(&patched);
        }
        Ok(())
    }

    pub fn read_byte(&self, offset: usize) -> u8 {
        self.data[offset]
    }

    pub fn write_byte(&mut self, offset: usize, val: u8) {
        let old = self.data[offset];
        if old != val {
            let tag = self.current_tag();
            self.write_log.push(WriteRecord {
                offset,
                len: 1,
                old_bytes: vec![old],
                new_bytes: vec![val],
                tag,
            });
            self.data[offset] = val;
        }
    }

    pub fn read_range(&self, start: usize, len: usize) -> &[u8] {
        &self.data[start..start + len]
    }

    pub fn write_range(&mut self, start: usize, data: &[u8]) {
        let old = self.data[start..start + data.len()].to_vec();
        if old != data {
            let tag = self.current_tag();
            self.write_log.push(WriteRecord {
                offset: start,
                len: data.len(),
                old_bytes: old,
                new_bytes: data.to_vec(),
                tag,
            });
            self.data[start..start + data.len()].copy_from_slice(data);
        }
    }

    // --- Tag management ---

    /// Replace the tag stack with a single tag. Used by the orchestrator
    /// before each module call.
    pub fn set_tag(&mut self, tag: &'static str) {
        self.tag_stack.clear();
        self.tag_stack.push(tag);
    }

    /// Push a sub-tag onto the stack for hierarchical tagging within a module.
    pub fn push_tag(&mut self, tag: &'static str) {
        self.tag_stack.push(tag);
    }

    /// Pop the most recent sub-tag from the stack.
    pub fn pop_tag(&mut self) {
        self.tag_stack.pop();
    }

    fn current_tag(&self) -> String {
        if self.tag_stack.is_empty() {
            "untagged".to_string()
        } else {
            self.tag_stack.join("/")
        }
    }

    // --- Write log queries ---

    /// Returns the full ordered write log.
    pub fn write_log(&self) -> &[WriteRecord] {
        &self.write_log
    }

    /// Returns all write records overlapping the byte range `[start, end)`.
    pub fn writes_in_range(&self, start: usize, end: usize) -> Vec<&WriteRecord> {
        self.write_log
            .iter()
            .filter(|r| r.offset < end && r.offset + r.len > start)
            .collect()
    }

    /// Returns all write records whose tag starts with `prefix`.
    pub fn writes_by_tag(&self, prefix: &str) -> Vec<&WriteRecord> {
        self.write_log
            .iter()
            .filter(|r| r.tag.starts_with(prefix))
            .collect()
    }

    /// Returns all write records covering a specific byte offset.
    pub fn writes_at(&self, offset: usize) -> Vec<&WriteRecord> {
        self.write_log
            .iter()
            .filter(|r| offset >= r.offset && offset < r.offset + r.len)
            .collect()
    }

    /// Returns true if any write record overlaps the byte range `[start, end)`.
    pub fn has_writes_in_range(&self, start: usize, end: usize) -> bool {
        self.write_log
            .iter()
            .any(|r| r.offset < end && r.offset + r.len > start)
    }

    /// Find write collisions: offsets written by more than one top-level tag.
    /// Returns a list of (offset, tag1, tag2) for each collision.
    pub fn find_collisions(&self) -> Vec<(usize, String, String)> {
        use std::collections::HashMap;
        // Map each byte offset to the top-level tag that last wrote it.
        let mut owner: HashMap<usize, &str> = HashMap::new();
        let mut collisions: Vec<(usize, String, String)> = Vec::new();

        for rec in &self.write_log {
            let top_tag = rec.tag.split('/').next().unwrap_or(&rec.tag);
            for off in rec.offset..rec.offset + rec.len {
                if let Some(&prev) = owner.get(&off) && prev != top_tag {
                    collisions.push((off, prev.to_string(), top_tag.to_string()));
                }
                owner.insert(off, top_tag);
            }
        }

        collisions.sort_by_key(|(off, _, _)| *off);
        collisions.dedup();
        collisions
    }

    /// Format the write log as a human-readable string, grouped by tag.
    pub fn format_write_log(&self) -> String {
        use std::collections::BTreeMap;
        use std::fmt::Write;

        let mut by_tag: BTreeMap<&str, Vec<&WriteRecord>> = BTreeMap::new();
        for rec in &self.write_log {
            by_tag.entry(&rec.tag).or_default().push(rec);
        }

        let mut out = String::new();
        for (tag, records) in &by_tag {
            let total_bytes: usize = records.iter().map(|r| r.len).sum();
            let _ = writeln!(out, "[{tag}] {total_bytes} bytes, {} writes", records.len());
            for rec in records {
                if rec.len <= 4 {
                    let _ = writeln!(
                        out,
                        "  0x{:05X}  {} -> {}",
                        rec.offset,
                        rec.old_bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "),
                        rec.new_bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "),
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "  0x{:05X}..0x{:05X}  ({} bytes)",
                        rec.offset,
                        rec.offset + rec.len - 1,
                        rec.len,
                    );
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_valid_rom() -> Vec<u8> {
        let prg_size = EXPECTED_PRG_PAGES as usize * PRG_PAGE_SIZE;
        let chr_size = EXPECTED_CHR_PAGES as usize * CHR_PAGE_SIZE;
        let mut rom = vec![0u8; HEADER_SIZE + prg_size + chr_size];
        rom[0..4].copy_from_slice(&INES_MAGIC);
        rom[4] = EXPECTED_PRG_PAGES;
        rom[5] = EXPECTED_CHR_PAGES;
        rom[6] = 0x40; // mapper 4 lower nibble
        rom[7] = 0x00;
        rom
    }

    /// Build a Rom from synthetic bytes for read/write/log tests, bypassing
    /// the Rev 1 CRC check (the payload is all zeros and would otherwise be
    /// rejected as UnknownRevision).
    fn rom_from(data: &[u8]) -> Rom {
        Rom::from_bytes_lax(data, true).unwrap()
    }

    #[test]
    fn test_valid_header_parses() {
        // Lax mode: skip both layout and CRC checks, just verify header decoding.
        let data = make_valid_rom();
        let rom = Rom::from_bytes_lax(&data, true).unwrap();
        assert_eq!(rom.header.prg_pages, 16);
        assert_eq!(rom.header.chr_pages, 16);
        assert_eq!(rom.header.mapper, 4);
        assert!(rom.header.mirroring_horizontal);
    }

    #[test]
    fn test_unknown_revision_rejected() {
        // Valid iNES layout but all-zero payload → CRC doesn't match Rev 0 or Rev 1.
        let data = make_valid_rom();
        match Rom::from_bytes(&data) {
            Err(RomError::UnknownRevision { payload_crc32 }) => {
                assert_ne!(payload_crc32, PRG_REV1_PAYLOAD_CRC32);
                assert_ne!(payload_crc32, PRG_REV0_PAYLOAD_CRC32);
            }
            Err(other) => panic!("expected UnknownRevision, got {other:?}"),
            Ok(_) => panic!("expected UnknownRevision, got Ok"),
        }
    }

    #[test]
    fn test_skip_validation_bypasses_crc() {
        // All-zero payload would fail strict CRC but passes in lax mode.
        let data = make_valid_rom();
        Rom::from_bytes_lax(&data, true).expect("lax mode must accept any layout");
    }

    #[test]
    fn crc32_ieee_known_vectors() {
        // Standard CRC-32/IEEE check value from RFC 3309 et al.
        assert_eq!(crc32_ieee(b""), 0x0000_0000);
        assert_eq!(crc32_ieee(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn test_bad_magic() {
        let mut data = make_valid_rom();
        data[0] = 0x00;
        assert!(matches!(Rom::from_bytes(&data), Err(RomError::BadMagic(_))));
    }

    #[test]
    fn test_too_small() {
        let data = vec![0u8; 10];
        assert!(matches!(Rom::from_bytes(&data), Err(RomError::TooSmall(10))));
    }

    #[test]
    fn test_wrong_prg() {
        let mut data = make_valid_rom();
        data[4] = 8;
        assert!(matches!(
            Rom::from_bytes(&data),
            Err(RomError::UnexpectedPrg { .. })
        ));
    }

    #[test]
    fn test_size_mismatch() {
        let mut data = make_valid_rom();
        data.push(0xFF); // extra byte
        assert!(matches!(
            Rom::from_bytes(&data),
            Err(RomError::SizeMismatch { .. })
        ));
    }

    #[test]
    fn test_read_write() {
        let data = make_valid_rom();
        let mut rom = rom_from(&data);
        rom.write_byte(20, 0xAB);
        assert_eq!(rom.read_byte(20), 0xAB);
        assert_eq!(rom.original[20], 0x00); // original unchanged

        rom.write_range(100, &[1, 2, 3]);
        assert_eq!(rom.read_range(100, 3), &[1, 2, 3]);
    }

    #[test]
    fn write_byte_logs_with_tag() {
        let data = make_valid_rom();
        let mut rom = rom_from(&data);
        rom.set_tag("test_module");
        rom.write_byte(20, 0xAB);

        let log = rom.write_log();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].offset, 20);
        assert_eq!(log[0].len, 1);
        assert_eq!(log[0].old_bytes, vec![0x00]);
        assert_eq!(log[0].new_bytes, vec![0xAB]);
        assert_eq!(log[0].tag, "test_module");
    }

    #[test]
    fn write_range_logs_with_tag() {
        let data = make_valid_rom();
        let mut rom = rom_from(&data);
        rom.set_tag("palettes");
        rom.write_range(100, &[1, 2, 3]);

        let log = rom.write_log();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].offset, 100);
        assert_eq!(log[0].len, 3);
        assert_eq!(log[0].old_bytes, vec![0, 0, 0]);
        assert_eq!(log[0].new_bytes, vec![1, 2, 3]);
        assert_eq!(log[0].tag, "palettes");
    }

    #[test]
    fn noop_write_byte_not_logged() {
        let data = make_valid_rom();
        let mut rom = rom_from(&data);
        rom.set_tag("test");
        rom.write_byte(20, 0x00); // same as existing value
        assert!(rom.write_log().is_empty());
    }

    #[test]
    fn noop_write_range_not_logged() {
        let data = make_valid_rom();
        let mut rom = rom_from(&data);
        rom.set_tag("test");
        rom.write_range(20, &[0x00, 0x00, 0x00]); // same as existing
        assert!(rom.write_log().is_empty());
    }

    #[test]
    fn untagged_writes_get_untagged_label() {
        let data = make_valid_rom();
        let mut rom = rom_from(&data);
        rom.write_byte(20, 0xFF);
        assert_eq!(rom.write_log()[0].tag, "untagged");
    }

    #[test]
    fn hierarchical_tags() {
        let data = make_valid_rom();
        let mut rom = rom_from(&data);
        rom.set_tag("overworld");
        rom.push_tag("fortress");
        rom.push_tag("fx_table");
        rom.write_byte(20, 0xFF);
        assert_eq!(rom.write_log()[0].tag, "overworld/fortress/fx_table");

        rom.pop_tag();
        rom.write_byte(21, 0xFE);
        assert_eq!(rom.write_log()[1].tag, "overworld/fortress");
    }

    #[test]
    fn query_writes_in_range() {
        let data = make_valid_rom();
        let mut rom = rom_from(&data);
        rom.set_tag("a");
        rom.write_byte(100, 0x01);
        rom.set_tag("b");
        rom.write_range(200, &[1, 2, 3]);
        rom.set_tag("c");
        rom.write_byte(300, 0x02);

        let hits = rom.writes_in_range(150, 250);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].tag, "b");

        // Range overlapping the write_range at 200..203
        let hits = rom.writes_in_range(202, 400);
        assert_eq!(hits.len(), 2); // b (200..203) and c (300)
    }

    #[test]
    fn query_writes_by_tag() {
        let data = make_valid_rom();
        let mut rom = rom_from(&data);
        rom.set_tag("qol/drawbridges");
        rom.write_byte(20, 0x01);
        rom.set_tag("qol/w2_rock");
        rom.write_byte(21, 0x02);
        rom.set_tag("powerups");
        rom.write_byte(22, 0x03);

        let qol = rom.writes_by_tag("qol");
        assert_eq!(qol.len(), 2);
        let powerups = rom.writes_by_tag("powerups");
        assert_eq!(powerups.len(), 1);
    }

    #[test]
    fn query_writes_at() {
        let data = make_valid_rom();
        let mut rom = rom_from(&data);
        rom.set_tag("a");
        rom.write_range(100, &[1, 2, 3]);

        assert_eq!(rom.writes_at(99).len(), 0);
        assert_eq!(rom.writes_at(100).len(), 1);
        assert_eq!(rom.writes_at(102).len(), 1);
        assert_eq!(rom.writes_at(103).len(), 0);
    }

    #[test]
    fn query_has_writes_in_range() {
        let data = make_valid_rom();
        let mut rom = rom_from(&data);
        rom.set_tag("test");
        rom.write_byte(100, 0x01);

        assert!(rom.has_writes_in_range(100, 101));
        assert!(rom.has_writes_in_range(50, 101));
        assert!(!rom.has_writes_in_range(101, 200));
        assert!(!rom.has_writes_in_range(50, 100));
    }
}
