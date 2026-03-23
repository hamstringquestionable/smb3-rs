use std::fmt;

const INES_MAGIC: [u8; 4] = [0x4E, 0x45, 0x53, 0x1A]; // "NES\x1a"
const HEADER_SIZE: usize = 16;

// Expected values for SMB3 (USA Rev 1)
const EXPECTED_PRG_PAGES: u8 = 16; // 16 x 16KB = 256KB
const EXPECTED_CHR_PAGES: u8 = 16; // 16 x 8KB = 128KB
const PRG_PAGE_SIZE: usize = 16384; // 16KB
const CHR_PAGE_SIZE: usize = 8192; // 8KB

#[derive(Debug)]
pub enum RomError {
    TooSmall(usize),
    BadMagic([u8; 4]),
    UnexpectedPrg { expected: u8, got: u8 },
    UnexpectedChr { expected: u8, got: u8 },
    SizeMismatch { expected: usize, got: usize },
}

impl fmt::Display for RomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RomError::TooSmall(size) => {
                write!(f, "ROM too small: {size} bytes (need at least {HEADER_SIZE})")
            }
            RomError::BadMagic(got) => {
                write!(f, "Invalid iNES magic: got {got:02X?}, expected {INES_MAGIC:02X?}")
            }
            RomError::UnexpectedPrg { expected, got } => {
                write!(f, "Unexpected PRG page count: got {got}, expected {expected}")
            }
            RomError::UnexpectedChr { expected, got } => {
                write!(f, "Unexpected CHR page count: got {got}, expected {expected}")
            }
            RomError::SizeMismatch { expected, got } => {
                write!(f, "ROM size mismatch: got {got} bytes, expected {expected}")
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
        // Detect unheadered ROM: exact raw PRG+CHR size with no iNES magic.
        const UNHEADERED_SIZE: usize =
            EXPECTED_PRG_PAGES as usize * PRG_PAGE_SIZE + EXPECTED_CHR_PAGES as usize * CHR_PAGE_SIZE;

        let (bytes, header_synthesized) = if bytes.len() == UNHEADERED_SIZE {
            let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
            if magic == INES_MAGIC {
                // Unlikely: exactly UNHEADERED_SIZE but starts with NES magic — treat as corrupt
                return Err(RomError::SizeMismatch {
                    expected: HEADER_SIZE + UNHEADERED_SIZE,
                    got: bytes.len(),
                });
            }
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
        } else {
            (bytes.to_vec(), false)
        };

        let bytes = &bytes;

        if bytes.len() < HEADER_SIZE {
            return Err(RomError::TooSmall(bytes.len()));
        }

        let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
        if magic != INES_MAGIC {
            return Err(RomError::BadMagic(magic));
        }

        let prg_pages = bytes[4];
        let chr_pages = bytes[5];
        let flags6 = bytes[6];
        let flags7 = bytes[7];

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

        let expected_size =
            HEADER_SIZE + (prg_pages as usize * PRG_PAGE_SIZE) + (chr_pages as usize * CHR_PAGE_SIZE);
        if bytes.len() != expected_size {
            return Err(RomError::SizeMismatch {
                expected: expected_size,
                got: bytes.len(),
            });
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

    #[test]
    fn test_valid_rom() {
        let data = make_valid_rom();
        let rom = Rom::from_bytes(&data).unwrap();
        assert_eq!(rom.header.prg_pages, 16);
        assert_eq!(rom.header.chr_pages, 16);
        assert_eq!(rom.header.mapper, 4);
        assert!(rom.header.mirroring_horizontal);
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
        let mut rom = Rom::from_bytes(&data).unwrap();
        rom.write_byte(20, 0xAB);
        assert_eq!(rom.read_byte(20), 0xAB);
        assert_eq!(rom.original[20], 0x00); // original unchanged

        rom.write_range(100, &[1, 2, 3]);
        assert_eq!(rom.read_range(100, 3), &[1, 2, 3]);
    }

    #[test]
    fn write_byte_logs_with_tag() {
        let data = make_valid_rom();
        let mut rom = Rom::from_bytes(&data).unwrap();
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
        let mut rom = Rom::from_bytes(&data).unwrap();
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
        let mut rom = Rom::from_bytes(&data).unwrap();
        rom.set_tag("test");
        rom.write_byte(20, 0x00); // same as existing value
        assert!(rom.write_log().is_empty());
    }

    #[test]
    fn noop_write_range_not_logged() {
        let data = make_valid_rom();
        let mut rom = Rom::from_bytes(&data).unwrap();
        rom.set_tag("test");
        rom.write_range(20, &[0x00, 0x00, 0x00]); // same as existing
        assert!(rom.write_log().is_empty());
    }

    #[test]
    fn untagged_writes_get_untagged_label() {
        let data = make_valid_rom();
        let mut rom = Rom::from_bytes(&data).unwrap();
        rom.write_byte(20, 0xFF);
        assert_eq!(rom.write_log()[0].tag, "untagged");
    }

    #[test]
    fn hierarchical_tags() {
        let data = make_valid_rom();
        let mut rom = Rom::from_bytes(&data).unwrap();
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
        let mut rom = Rom::from_bytes(&data).unwrap();
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
        let mut rom = Rom::from_bytes(&data).unwrap();
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
        let mut rom = Rom::from_bytes(&data).unwrap();
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
        let mut rom = Rom::from_bytes(&data).unwrap();
        rom.set_tag("test");
        rom.write_byte(100, 0x01);

        assert!(rom.has_writes_in_range(100, 101));
        assert!(rom.has_writes_in_range(50, 101));
        assert!(!rom.has_writes_in_range(101, 200));
        assert!(!rom.has_writes_in_range(50, 100));
    }
}
