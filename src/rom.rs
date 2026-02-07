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
}

impl Rom {
    /// Parse and validate a ROM from raw bytes.
    /// Validates it matches the expected SMB3 (USA Rev 1) layout.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RomError> {
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
        })
    }

    pub fn read_byte(&self, offset: usize) -> u8 {
        self.data[offset]
    }

    pub fn write_byte(&mut self, offset: usize, val: u8) {
        self.data[offset] = val;
    }

    pub fn read_range(&self, start: usize, len: usize) -> &[u8] {
        &self.data[start..start + len]
    }

    pub fn write_range(&mut self, start: usize, data: &[u8]) {
        self.data[start..start + data.len()].copy_from_slice(data);
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
}
