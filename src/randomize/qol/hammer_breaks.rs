//! Hammer item also breaks fortress locks / water-gap bridges.

use crate::rom::Rom;
use crate::randomize::rom_data::{self, FS_HAMMER_LOCKS, prg_bank_file_to_cpu};

// Make the hammer item also break fortress lock tiles and/or water-gap
// bridge locks on the overworld map.
//
// The vanilla hammer routine at PRG026 (file 0x346D5, CPU $A6C5) uses a
// 7-byte range check: `SEC; SBC #$51; CMP #$02; BCC .found` which only
// matches rock tiles $51–$52. We replace this with a JSR to a table-driven
// subroutine in PRG026 free space whose tables are built from the flags:
// the 2 rock tiles are always present, `locks` adds the 3 fortress lock
// tiles, and `bridges` adds the water gap lock (0x9D → 0xB3) — so the
// tables hold 2–6 entries.
//
// Patch site 1 — Range check (file 0x346D5, 7 bytes):
//   `SEC; SBC #$51; CMP #$02; BCC .found` →
//   `JSR HammerCheckTile; BCC .found; NOP; NOP`
//
// Patch site 2 — Replacement tile load (file 0x346E9, 3 bytes):
//   `LDA $A6B1,X` → `LDA $7EB6` (load from scratch RAM set by subroutine)
//
// New subroutine at FS_HAMMER_LOCKS (0x3557F, CPU $B56F), up to 50 bytes:
//   Table-driven check of breakable tiles, stores replacement tile in $7EB6,
//   saves/restores X via $7EB7, returns carry clear if breakable.

/// File offset of the 7-byte range check in the hammer routine ($A6C5).
const HAMMER_RANGE_CHECK: usize = 0x346D5;
/// File offset of `LDA $A6B1,X` (replacement tile load) at CPU $A6D8.
const HAMMER_REPLACE_LOAD: usize = 0x346E8;
/// CPU address of the subroutine in PRG026 ($A000 window): $B56F.
const HAMMER_LOCKS_SUB_CPU: u16 = prg_bank_file_to_cpu(26, FS_HAMMER_LOCKS);

pub fn hammer_breaks_tiles(rom: &mut Rom, locks: bool, bridges: bool) {
    // Build tables dynamically based on which flags are set.
    // Always include rocks (2 entries), then conditionally add locks (3) and bridge (1).
    let mut breakable: Vec<u8> = vec![0x51, 0x52]; // rocks
    let mut replace:   Vec<u8> = vec![0x45, 0x46];
    let mut tilefix:   Vec<u8> = vec![0x00, 0x01];

    // `replace` is derived from `path_for_gap_tile` rather than written out
    // again: it is the same lock→path mapping the FX restore uses, and a third
    // copy of it here could drift from the other two. `tilefix` stays explicit
    // — it is a hammer-specific orientation fixup, not tile classification.
    if locks {
        for &lock in &rom_data::LOCK_TILES {
            breakable.push(lock);
            replace.push(
                rom_data::path_for_gap_tile(lock).expect("a lock tile always inverts"),
            );
        }
        tilefix.extend_from_slice(&[0x01, 0x00, 0x00]);
    }
    if bridges {
        breakable.push(rom_data::WATER_GAP_TILE);
        replace.push(
            rom_data::path_for_gap_tile(rom_data::WATER_GAP_TILE)
                .expect("the water gap always inverts"),
        );
        tilefix.push(0x00);
    }

    let table_len = breakable.len();
    let ldx_imm = (table_len - 1) as u8;

    // Table CPU addresses start right after the 32-byte code block.
    let tbl_base = HAMMER_LOCKS_SUB_CPU + 32;
    let breakable_cpu = tbl_base;
    let replace_cpu = tbl_base + table_len as u16;
    let tilefix_cpu = tbl_base + (table_len * 2) as u16;

    // Patch site 1: JSR HammerCheckTile; BCC .found; NOP; NOP
    //
    // Original 7 bytes at 0x346D5 (CPU $A6C5):
    //   38        SEC
    //   E9 51     SBC #$51
    //   C9 02     CMP #$02
    //   90 07     BCC .found (+$07) → $A6D2 (file 0x346E2)
    //
    // .found ($A6D2) does: STX $01; LSR $01; PHA; TAX; LDA $A6B1,X
    // — the STX $01 needs the *original* X, so the subroutine preserves it.
    //
    // New BCC at $A6C8 (0x346D8) targeting $A6D2: offset = $A6D2 - $A6CA = 0x08.
    // Only 6 bytes — must preserve DEC $00 (C6 00) at 0x346DB so the outer
    // loop that checks 4 adjacent tiles still works on no-match fall-through.
    let lo = (HAMMER_LOCKS_SUB_CPU & 0xFF) as u8;
    let hi = (HAMMER_LOCKS_SUB_CPU >> 8) as u8;
    rom.write_range(HAMMER_RANGE_CHECK, &[
        0x20, lo, hi,   // JSR HammerCheckTile
        0x90, 0x08,     // BCC .found (targets $A6D2 / file 0x346E2)
        0xEA,           // NOP (1 byte padding)
    ]);

    // Patch site 2: LDA $7EB6 (absolute) instead of LDA $A6B1,X (indexed)
    // Original at 0x346E8 (CPU $A6D8): BD B1 A6 (LDA $A6B1,X)
    // New: AD B6 7E (LDA $7EB6)
    rom.write_range(HAMMER_REPLACE_LOAD, &[0xAD, 0xB6, 0x7E]);

    // Subroutine + tables at FS_HAMMER_LOCKS (CPU $B56F), up to 50 bytes.
    //
    // Saves/restores X via $7EB7 so the caller's STX $01 at .found sees the
    // original X register. Returns carry clear with A = tilefix_map (animation
    // index 0 or 1), $7EB6 = replacement tile.
    //
    // Code: 32 bytes, tables: 3 × table_len bytes.
    #[rustfmt::skip]
    let mut subroutine: Vec<u8> = vec![
        // HammerCheckTile:
        0x8E, 0xB7, 0x7E,                                      // STX $7EB7         ; save original X
        0xA2, ldx_imm,                                          // LDX #N            ; N entries (index N..0)
        // .loop:
        0xDD, breakable_cpu as u8, (breakable_cpu >> 8) as u8,  // CMP breakable,X
        0xF0, 0x08,                                             // BEQ .found (+8)
        0xCA,                                                   // DEX
        0x10, 0xF8,                                             // BPL .loop (-8)
        0xAE, 0xB7, 0x7E,                                      // LDX $7EB7         ; restore X (not found)
        0x38,                                                   // SEC
        0x60,                                                   // RTS
        // .found:
        0xBD, replace_cpu as u8, (replace_cpu >> 8) as u8,      // LDA replace,X
        0x8D, 0xB6, 0x7E,                                      // STA $7EB6         ; scratch RAM for replacement
        0xBD, tilefix_cpu as u8, (tilefix_cpu >> 8) as u8,      // LDA tilefix,X    ; tilefix_map (animation idx)
        0xAE, 0xB7, 0x7E,                                      // LDX $7EB7         ; restore original X
        0x18,                                                   // CLC               ; found
        0x60,                                                   // RTS
    ];
    subroutine.extend_from_slice(&breakable);
    subroutine.extend_from_slice(&replace);
    subroutine.extend_from_slice(&tilefix);
    rom.write_range(FS_HAMMER_LOCKS, &subroutine);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the generated tables byte-for-byte.
    ///
    /// `hammer_breaks_locks` is off by default, so the overworld baseline sweep
    /// never reaches this code — it would pass vacuously and prove nothing about
    /// deriving `replace` from `path_for_gap_tile`. The expected bytes below are
    /// the literals this function used before that change.
    fn tables(locks: bool, bridges: bool) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            return (vec![], vec![], vec![]);
        };
        let mut rom = crate::rom::Rom::from_bytes_lax(&bytes, true).unwrap();
        hammer_breaks_tiles(&mut rom, locks, bridges);

        let n = 2 + if locks { 3 } else { 0 } + usize::from(bridges);
        let base = FS_HAMMER_LOCKS + 32;
        let read = |i: usize| rom.read_range(base + i * n, n).to_vec();
        (read(0), read(1), read(2))
    }

    #[test]
    fn rocks_only_tables_are_unchanged() {
        let (breakable, replace, tilefix) = tables(false, false);
        if breakable.is_empty() {
            return; // no ROM available
        }
        assert_eq!(breakable, vec![0x51, 0x52]);
        assert_eq!(replace, vec![0x45, 0x46]);
        assert_eq!(tilefix, vec![0x00, 0x01]);
    }

    #[test]
    fn lock_tables_match_the_literals_they_replaced() {
        let (breakable, replace, tilefix) = tables(true, false);
        if breakable.is_empty() {
            return;
        }
        assert_eq!(breakable, vec![0x51, 0x52, 0x54, 0x56, 0xE4]);
        assert_eq!(replace, vec![0x45, 0x46, 0x46, 0x45, 0xDA]);
        assert_eq!(tilefix, vec![0x00, 0x01, 0x01, 0x00, 0x00]);
    }

    #[test]
    fn bridge_entry_appends_after_locks() {
        let (breakable, replace, tilefix) = tables(true, true);
        if breakable.is_empty() {
            return;
        }
        assert_eq!(breakable, vec![0x51, 0x52, 0x54, 0x56, 0xE4, 0x9D]);
        assert_eq!(replace, vec![0x45, 0x46, 0x46, 0x45, 0xDA, 0xB3]);
        assert_eq!(tilefix, vec![0x00, 0x01, 0x01, 0x00, 0x00, 0x00]);
    }

    /// Bridges without locks must not shift the lock entries in — the tables are
    /// parallel arrays indexed together, so order is load-bearing.
    #[test]
    fn bridges_without_locks() {
        let (breakable, replace, tilefix) = tables(false, true);
        if breakable.is_empty() {
            return;
        }
        assert_eq!(breakable, vec![0x51, 0x52, 0x9D]);
        assert_eq!(replace, vec![0x45, 0x46, 0xB3]);
        assert_eq!(tilefix, vec![0x00, 0x01, 0x00]);
    }
}
