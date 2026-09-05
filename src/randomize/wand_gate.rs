//! The world maze's wand gate: a wall on World 8's bridge that stands until
//! the player holds K of the seven wands.
//!
//! # Why it exists
//!
//! In the maze the eight worlds are linked by telepads, and a pad chain can
//! drop the player beside Bowser's castle in the first few minutes. Measured,
//! some seeds finish in a single level. K = 3 puts the floor at 16 levels
//! while leaving the median run alone, so the gate is not polish — it is what
//! makes the mode a game. K is the difficulty dial, and K = 0 is a pure maze
//! (this module then writes nothing at all).
//!
//! # The gate is a wall, not a lock
//!
//! A lock that no fortress opens teaches the player the wrong rule about every
//! other lock on the map. So the gate is a *wall*, standing among World 8's
//! own masonry: [`WAND_GATE_TILE`] (`$D5`), a tile byte no world's grid uses. The engine has no per-tile "blocks movement" flag — a tile
//! blocks a direction by being absent from `Map_Object_Valid_Left/Right/Up/
//! Down` — so an unused byte walls all four directions for nothing. It is in
//! no other registry either, which is the point: it is not in
//! `Map_Removable_Tiles`, so the packed completion stencil does not grow by
//! World 8's 155 wall cells and no completion bit can open it; it is in
//! neither rock list, so `hammer_breaks_tiles` cannot break it.
//!
//! # How it opens, and why that needs no persistence
//!
//! `Map_Reload_with_Completions` rebuilds the whole map into `Tile_Mem` from
//! the ROM grid on every map load. The opener therefore does not have to
//! *record* anything: it re-derives the gate every time the map is drawn, by
//! stamping a bridge tile over the gate cell in `Tile_Mem` when the wand count
//! is high enough. The wand counter is the only state, and it lives in SRAM
//! ([`WAND_COUNT`]). Game over, Continue and re-entering World 8 are all
//! automatically correct because there is nothing to get out of sync.
//!
//! The alternative — adding `$D5` to `Map_Removable_Tiles` and setting a
//! completion bit — was measured and rejected: those tables are byte-adjacent
//! and bounded by an `LDX #7` immediate, so it needs both tables relocated,
//! three vanilla operands repointed, the packed stencil grown, and
//! `capacity::is_completion_unsafe` mirrored — and it would *still* need a
//! routine to set the bit in both halves. This is 29 bytes and touches no
//! vanilla table.
//!
//! # Where the two routines hook
//!
//! `Map_Reload_with_Completions` has exactly two callers, and the gate has to
//! run after both — the first is the full map init after a world transition,
//! the second is the return from a level, which redraws the map from ROM just
//! the same and would otherwise wall the gate back up.
//!
//! | site | CPU | vanilla | what we hook |
//! |---|---|---|---|
//! | full map init | `$85BE` | `JSR Fill_Tile_AttrTable_ByTileset` | the call *after* the reload |
//! | return from level | `$91AC` | `JSR Map_Reload_with_Completions` | the reload call itself |
//!
//! The init path's own `JSR Map_Reload_with_Completions` at `$85BB` is
//! [`super::completion_bits`]'s hook, so this module takes the instruction
//! after it instead of fighting over the same three bytes.
//!
//! The counter is bumped at `world_order::WORLD_INC_OFFSET`, vanilla's
//! `INC World_Num` on the airship-cleared path. Whatever three bytes are
//! there are captured and replayed inside the bump routine, so the hook is
//! correct whether it sits over vanilla's `INC World_Num` or over
//! `world_order`'s `JMP`. **That makes the ordering a one-way rule:
//! [`apply`] must run AFTER `world_order::randomize`**, or `world_order` will
//! overwrite the hook and the counter will never move.
//!
//! # Ordering the integration must honour
//!
//! 1. after `world_order::randomize` — it shares the airship hook site;
//! 2. after the overworld writer — [`apply`] stamps the gate tile into World
//!    8's map grid, and the writer rewrites that grid wholesale. It must also
//!    not be visible to the *builder*, which would see Bowser's castle walled
//!    off and fail its own reachability invariant.

use crate::rom::Rom;

use super::maze_state::WAND_COUNT;
use super::rom_data::{
    BRIDGE_TILE, FS_MAZE_WAND_COUNT, FS_MAZE_WAND_GATE, PRG012_FILE_BASE, W8_IDX, W8_WAND_GATE_POS,
    WAND_GATE_TILE, map_tile_offset, prg030_file_to_cpu,
};
use super::world_order::WORLD_INC_OFFSET;

// --- Addresses ----------------------------------------------------------

/// PRG012 is banked at `$A000` before either reload call and stays there for
/// the map's whole life, so a file offset in it is `$A000 + (file - 0x18010)`.
const fn prg012_cpu(file: usize) -> u16 {
    (0xA000 + (file - PRG012_FILE_BASE)) as u16
}

/// Where [`wand_gate_routine`]'s output is assembled to run. `$BE30`.
const WAND_GATE_CPU: u16 = prg012_cpu(FS_MAZE_WAND_GATE);

/// Where [`wand_bump_routine`]'s output is assembled to run. `$9F90`, PRG030,
/// which is always mapped — the airship transition runs there with an
/// arbitrary bank at `$A000`.
const WAND_BUMP_CPU: u16 = prg030_file_to_cpu(FS_MAZE_WAND_COUNT);

/// `Map_Reload_with_Completions` (PRG012), the routine that rebuilds
/// `Tile_Mem` from the ROM grid and replays the completion bitfield.
const MAP_RELOAD_CPU: u16 = 0xA45D;

/// `Fill_Tile_AttrTable_ByTileset` (PRG030, always mapped) — the eight-byte
/// attribute copy the init path runs immediately after the reload. The gate
/// displaces this call and replays it.
const FILL_ATTR_CPU: u16 = 0x951B;

/// `World_Num`, 0-based; World 8 is 7.
const WORLD_NUM: u16 = 0x0727;

/// The gate cell's address in `Tile_Mem`.
///
/// Derived rather than transcribed. `Map_Reload_with_Completions` lays each
/// map screen out at `Tile_Mem_Addr[screen] + $110`, sixteen bytes per row —
/// the copy loop moves 144 bytes (nine rows of sixteen) per screen and then
/// adds `$1B0`, and the completion pass re-derives the same cell as
/// `Tile_Mem_Addr[screen] + $100 + (row + 1) * $10 + col`. `Tile_Mem_Addr` is
/// `$6000 + screen * $1B0`.
///
/// So for World 8 (5,59) — screen 3, column 11 within it:
/// `$6000 + 3 * $1B0 + $110 + 5 * $10 + $0B = $667B`.
const fn tile_mem_addr(row: usize, col: usize) -> u16 {
    const TILE_MEM: usize = 0x6000;
    const SCREEN_STRIDE: usize = 0x1B0;
    const MAP_ROW0: usize = 0x110;
    (TILE_MEM + (col / 16) * SCREEN_STRIDE + MAP_ROW0 + row * 0x10 + (col % 16)) as u16
}

const GATE_TILE_MEM: u16 = tile_mem_addr(W8_WAND_GATE_POS.0, W8_WAND_GATE_POS.1);

// --- The gate opener ----------------------------------------------------

/// Two entry points and one body. 29 bytes; 128 reserved.
///
/// ```text
/// $BE30  20 5D A4   JSR Map_Reload_with_Completions   ; entry: return-from-level
/// $BE33  4C 39 BE   JMP  gate
/// $BE36  20 1B 95   JSR Fill_Tile_AttrTable_ByTileset ; entry: full map init
/// gate:                                               ; ...falls through
/// $BE39  AD 27 07   LDA  World_Num
/// $BE3C  C9 07      CMP  #7                           ; World 8?
/// $BE3E  D0 0C      BNE  done
/// $BE40  AD C9 7A   LDA  WAND_COUNT
/// $BE43  C9 kk      CMP  #K
/// $BE45  90 05      BCC  done                         ; fewer than K wands
/// $BE47  A9 B3      LDA  #BRIDGE_TILE
/// $BE49  8D 7B 66   STA  $667B                        ; the gate cell
/// $BE4C  60 done:   RTS
/// ```
///
/// The two entries exist because the two callers of the reload displace
/// different instructions; each replays its own and lands on the shared body.
/// Sequencing the return-from-level entry as `JSR reload` + `JMP gate` rather
/// than a fall-through costs three bytes and keeps the init entry from calling
/// the reload a second time.
fn wand_gate_routine(wands_required: u8) -> [u8; 29] {
    let gate = WAND_GATE_CPU + 9;
    #[rustfmt::skip]
    let code = [
        0x20, MAP_RELOAD_CPU as u8, (MAP_RELOAD_CPU >> 8) as u8, // JSR Map_Reload_with_Completions
        0x4C, gate as u8, (gate >> 8) as u8,                     // JMP gate
        0x20, FILL_ATTR_CPU as u8, (FILL_ATTR_CPU >> 8) as u8,   // JSR Fill_Tile_AttrTable_ByTileset
        0xAD, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,           // LDA World_Num
        0xC9, W8_IDX as u8,                                      // CMP #7
        0xD0, 0x0C,                                              // BNE done
        0xAD, WAND_COUNT as u8, (WAND_COUNT >> 8) as u8,         // LDA WAND_COUNT
        0xC9, wands_required,                                    // CMP #K
        0x90, 0x05,                                              // BCC done
        0xA9, BRIDGE_TILE,                                       // LDA #BRIDGE_TILE
        0x8D, GATE_TILE_MEM as u8, (GATE_TILE_MEM >> 8) as u8,   // STA gate cell
        0x60,                                                    // done: RTS
    ];
    code
}

/// `JSR Map_Reload_with_Completions` on the return-from-level path, CPU
/// `$91AC`. Not the init path's call at `$85BB` — that one is
/// [`super::completion_bits`]'.
const RELOAD_FROM_LEVEL_OFFSET: usize = 0x3D1BC;

/// `JSR Fill_Tile_AttrTable_ByTileset` in `PRG030_84A0`, CPU `$85BE` — the
/// instruction immediately after the init path's reload call.
const FILL_ATTR_CALL_OFFSET: usize = 0x3C5CE;

// --- The wand counter ---------------------------------------------------

/// Bump the wand count on an airship clear, saturating at seven. 16 bytes,
/// which is the whole gap: `0x3DFA0..0x3DFB0` is `$FF` and real code follows
/// immediately, so this routine cannot grow in place.
///
/// ```text
/// $9F90  AD C9 7A   LDA WAND_COUNT
/// $9F93  C9 07      CMP #7
/// $9F95  B0 03      BCS skip          ; already at seven
/// $9F97  EE C9 7A   INC WAND_COUNT
/// $9F9A  .. .. ..   skip: <the three displaced bytes>
/// $9F9D  4C 94 90   JMP  WORLD_INC + 3
/// ```
///
/// The saturate is not decoration. `TILE_AIRSHIP` is in neither
/// `Map_Removable_Tiles` nor `Map_Completable_Tiles`, so nothing ever marks an
/// airship — in the maze, where a world can be re-entered, the spine edge is
/// repeatable and a player can clear the same airship again.
///
/// `displaced` is whatever three bytes stood at [`WORLD_INC_OFFSET`], read
/// from the ROM rather than transcribed. Vanilla has `INC World_Num`
/// (`EE 27 07`) there and `world_order::randomize` replaces it with its own
/// `JMP` — both are exactly three bytes and both are whole instructions, so
/// replaying them and then jumping past the site is right either way. In the
/// `world_order` case the replayed `JMP` leaves and the trailing `JMP` is
/// unreachable; in the vanilla case it lands on the `JMP $84A0` that follows.
fn wand_bump_routine(displaced: [u8; 3]) -> [u8; 16] {
    let resume = (WORLD_INC_OFFSET - 0x3C010 + 0x8000 + 3) as u16;
    #[rustfmt::skip]
    let code = [
        0xAD, WAND_COUNT as u8, (WAND_COUNT >> 8) as u8, // LDA WAND_COUNT
        0xC9, MAX_WANDS,                                 // CMP #7
        0xB0, 0x03,                                      // BCS skip
        0xEE, WAND_COUNT as u8, (WAND_COUNT >> 8) as u8, // INC WAND_COUNT
        displaced[0], displaced[1], displaced[2],        // skip: <displaced>
        0x4C, resume as u8, (resume >> 8) as u8,         // JMP WORLD_INC + 3
    ];
    code
}

/// The seven wands, one per airship.
pub(crate) const MAX_WANDS: u8 = 7;

// --- The gate's graphics ------------------------------------------------

/// CHR page `$16`, local tiles 0-3 — the four 8x8 patterns the gate's metatile
/// points at, in quadrant order upper-left, lower-left, upper-right,
/// lower-right.
///
/// Repurposing them is safe, and the check was three-part:
///
/// * **no metatile names them.** All 1024 quadrant entries (four 256-byte
///   tables at `PRG012 + 0/256/512/768`) were scanned; `$80`-`$83` appear in
///   none, for any tile byte, used or unused.
/// * **no nametable stream names them.** Every `vaddr`-led `.byte` block in
///   the disassembly was scanned for `$80`-`$83` as tile data. Three hit:
///   `TitleScreen_LogoLine10` (title CHR `$78`/`$7A`), `Video_CourseClear` and
///   `Video_YouGotCard` — and both of those run with `PatTable_BankSel+1 =
///   $5E`, the level font bank, set at `prg002.asm:5493` and `:5645`. In the
///   *map's* font `$80` is not a letter: `W` is `$D8`, `M` is `$BA`, `I` is
///   `$FC`.
/// * **page `$16` is the map's alone.** `#$16` reaches `PatTable_BankSel+1` at
///   exactly two sites, `prg030.asm:607` and `:2906`, both map init. No
///   `Level_BG_Pages1/2`, `PT2_Anim` or `PlantInfest_PatTablePerACnt` entry is
///   `$16` or `$17`, and the map's own animation rotates R0 (tiles `$00`-`$7F`)
///   only, so this pair is fixed for the map's whole life.
///
/// Vanilla draws four thin elbow fragments here — art referenced by nothing.
/// `gate_chr_donors_are_vanilla` pins their bytes so a ROM or bank drift
/// fails loudly instead of scribbling on live graphics.
const GATE_CHR_TILES: [u8; 4] = [0x80, 0x81, 0x82, 0x83];

/// The pattern table splits at `$80`: tiles `$00`-`$7F` come from the bank in
/// `PatTable_BankSel` (which the map ANIMATES, rotating R0 through four page
/// pairs) and `$80`-`$FF` from `PatTable_BankSel+1`, fixed at `$16` for the
/// map's whole life. [`GATE_CHR_TILES`] is on the fixed side, which is why the
/// gate never flickers; the local index inside page `$16` is the tile minus
/// this.
const CHR_PAGE_16_FIRST_TILE: u8 = 0x80;

/// File offset of CHR page `$16`, local tile 0 — pattern-table tile `$80`.
const GATE_CHR_BASE: usize = 0x40010 + 0x16 * 0x400;

/// File offset of one pattern-table tile in page `$16`.
const fn gate_chr_offset(tile: u8) -> usize {
    GATE_CHR_BASE + (tile - CHR_PAGE_16_FIRST_TILE) as usize * 16
}

/// The skull, as four 8x8 patterns in quadrant order UL, LL, UR, LR.
///
/// ```text
///   ................       . colour 3, the dark ground the wall uses
///   ....########....       # colour 1, bone
///   ..############..       o colour 2, shading
///   .##############.       (space) colour 0, the universal black
///   .##  ######  ##.
///   .#    ####    #.
///   .#    ####    #.
///   .##  ######  ##.
///   .#####    #####.
///   .######  ######.
///   .######oo######.
///   ..############..
///   ..# # #  # # #..
///   ..#oooooooooo#..
///   ....########....
///   ................
/// ```
///
/// Colour comes from the tile byte's top two bits, so page 3 of World 8's map
/// palette: `$0F` black, `$35` bone, `$25` shadow, `$17` the masonry brown the
/// Dark Land wall is already made of.
#[rustfmt::skip]
const SKULL_CHR: [[u8; 16]; 4] = [
    // upper-left
    [0xFF, 0xFF, 0xFF, 0xFF, 0xE7, 0xC3, 0xC3, 0xE7,
     0xFF, 0xF0, 0xC0, 0x80, 0x80, 0x80, 0x80, 0x80],
    // lower-left
    [0xFC, 0xFE, 0xFE, 0xFF, 0xEA, 0xE0, 0xFF, 0xFF,
     0x80, 0x80, 0x81, 0xC0, 0xC0, 0xDF, 0xF0, 0xFF],
    // upper-right
    [0xFF, 0xFF, 0xFF, 0xFF, 0xE7, 0xC3, 0xC3, 0xE7,
     0xFF, 0x0F, 0x03, 0x01, 0x01, 0x01, 0x01, 0x01],
    // lower-right
    [0x3F, 0x7F, 0x7F, 0xFF, 0x57, 0x07, 0xFF, 0xFF,
     0x01, 0x01, 0x81, 0x03, 0x03, 0xFB, 0x0F, 0xFF],
];

// --- Application --------------------------------------------------------

/// Write the wand gate: its graphics, its tile, and the two routines that
/// raise and lift it.
///
/// `wands_required` is K. **K = 0 writes nothing** — a pure maze has no goal
/// gate, and a wall that is open from the first frame is worse than no wall.
/// Values above [`MAX_WANDS`] are clamped, since the counter saturates there
/// and a higher K would be a gate nothing can open.
///
/// See the module docs for the two ordering rules this call has to sit inside:
/// after `world_order::randomize`, and after the overworld writer.
pub(crate) fn apply(rom: &mut Rom, wands_required: u8) {
    if wands_required == 0 {
        return;
    }
    let k = wands_required.min(MAX_WANDS);

    // The skull, and the metatile that points at it. Quadrant tables are four
    // 256-byte planes at PRG012 + 0/256/512/768, in UL, LL, UR, LR order.
    for (chr, pattern) in GATE_CHR_TILES.iter().zip(SKULL_CHR.iter()) {
        rom.write_range(gate_chr_offset(*chr), pattern);
    }
    for (plane, &chr) in GATE_CHR_TILES.iter().enumerate() {
        let table = PRG012_FILE_BASE + plane * 256;
        rom.write_byte(table + WAND_GATE_TILE as usize, chr);
    }

    // The gate cell itself, over the finished map.
    let (row, col) = W8_WAND_GATE_POS;
    rom.write_byte(map_tile_offset(W8_IDX, row, col), WAND_GATE_TILE);

    // The opener, and its two hooks.
    rom.write_range(FS_MAZE_WAND_GATE, &wand_gate_routine(k));
    rom.write_range(RELOAD_FROM_LEVEL_OFFSET, &jmp_free(0x20, WAND_GATE_CPU));
    rom.write_range(FILL_ATTR_CALL_OFFSET, &jmp_free(0x20, WAND_GATE_CPU + 6));

    // The counter, and its hook. The three bytes at the airship site are
    // captured before they are overwritten — see [`wand_bump_routine`].
    let displaced: [u8; 3] = core::array::from_fn(|i| rom.read_byte(WORLD_INC_OFFSET + i));
    rom.write_range(FS_MAZE_WAND_COUNT, &wand_bump_routine(displaced));
    rom.write_range(WORLD_INC_OFFSET, &jmp_free(0x4C, WAND_BUMP_CPU));
}

/// A three-byte `JSR`/`JMP` to `target`.
const fn jmp_free(opcode: u8, target: u16) -> [u8; 3] {
    [opcode, target as u8, (target >> 8) as u8]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::rom_data::asm;

    const K: u8 = 3;

    fn vanilla() -> Option<Rom> {
        let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&bytes).ok()
    }

    // -- addresses and data ---------------------------------------------

    #[test]
    fn addresses_are_where_the_banks_put_them() {
        assert_eq!(WAND_GATE_CPU, 0xBE30);
        assert_eq!(WAND_BUMP_CPU, 0x9F90);
        // The derivation in `tile_mem_addr`, spelled out for World 8 (5,59).
        assert_eq!(GATE_TILE_MEM, 0x667B);
        assert_eq!(GATE_TILE_MEM, 0x6000 + 3 * 0x1B0 + 0x110 + 5 * 0x10 + 0x0B);
        // Screen 0 row 0 col 0 is the base of the map area, which is the other
        // end of the same formula.
        assert_eq!(tile_mem_addr(0, 0), 0x6110);
    }

    /// The gate cell is on the bridge row and is one of the stamped spans, so
    /// `apply_w8_bridges` leaves a bridge there for the gate to replace — and
    /// the same cell is what `free_bridge_spans` now withholds.
    #[test]
    fn the_gate_cell_is_a_stamped_bridge_span() {
        use crate::randomize::qol::{W8_BRIDGE_COLS, W8_BRIDGE_ROW};
        assert_eq!(W8_WAND_GATE_POS.0, W8_BRIDGE_ROW);
        assert!(W8_BRIDGE_COLS.contains(&W8_WAND_GATE_POS.1));
    }

    /// The gate tile must block every direction, and must not be something
    /// another system can open, move or stand on.
    #[test]
    fn the_gate_tile_is_in_no_registry() {
        use crate::randomize::rom_data::{
            BACKGROUND_TILES, VALID_BLANK_TILES, VALID_HORZ, VALID_VERT,
        };
        assert!(!VALID_HORZ.contains(&WAND_GATE_TILE), "the gate would be walkable sideways");
        assert!(!VALID_VERT.contains(&WAND_GATE_TILE), "the gate would be walkable vertically");
        assert!(!VALID_BLANK_TILES.contains(&WAND_GATE_TILE), "content could be placed on it");
        assert!(!BACKGROUND_TILES.contains(&WAND_GATE_TILE));

        let Some(rom) = vanilla() else { return };
        // Map_Removable_Tiles (8 bytes) — membership here would grow the packed
        // completion stencil and let a stray bit open the gate.
        for i in 0..8 {
            assert_ne!(rom.read_byte(0x18447 + i), WAND_GATE_TILE, "gate byte is removable");
        }
        // Map_Completable_Tiles (5 bytes).
        for i in 0..5 {
            assert_ne!(rom.read_byte(0x18457 + i), WAND_GATE_TILE, "gate byte is completable");
        }
        // ...and no world's vanilla grid uses it, so nothing else can be
        // standing on it when the gate goes down.
        for world in 0..8 {
            let grid = crate::randomize::rom_data::read_tile_grid(&rom, world);
            for r in 0..grid.rows() {
                for c in 0..grid.cols {
                    assert_ne!(grid.get(r, c), WAND_GATE_TILE, "W{} uses the gate byte", world + 1);
                }
            }
        }
    }

    /// The four CHR tiles the skull is drawn into are the fragments this
    /// module measured as unreferenced. If a ROM or a bank ever disagrees,
    /// stop — overwriting live map graphics is not recoverable at runtime.
    ///
    /// The vanilla contents of [`GATE_CHR_TILES`] — four thin elbow fragments that
    /// nothing draws. Asserted before they are overwritten.
    #[rustfmt::skip]
    const VANILLA_GATE_CHR: [[u8; 16]; 4] = [
        [0x00, 0x00, 0x00, 0x0F, 0x1F, 0x18, 0x18, 0x18,
         0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        [0x00, 0x00, 0x00, 0xF0, 0xF8, 0x18, 0x18, 0x18,
         0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        [0x18, 0x18, 0x18, 0x1F, 0x0F, 0x00, 0x00, 0x00,
         0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        [0x18, 0x18, 0x18, 0xF8, 0xF0, 0x00, 0x00, 0x00,
         0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    ];

    #[test]
    fn gate_chr_donors_are_vanilla() {
        let Some(rom) = vanilla() else { return };
        for (chr, expect) in GATE_CHR_TILES.iter().zip(VANILLA_GATE_CHR.iter()) {
            assert_eq!(
                rom.read_range(gate_chr_offset(*chr), 16),
                expect,
                "CHR tile ${chr:02X} is not the fragment the skull replaces"
            );
        }
    }

    /// No metatile quadrant, for any of the 256 tile bytes, names one of the
    /// donor CHR tiles — the first leg of the unreferenced argument, kept as a
    /// test because it is the leg a future metatile edit could break.
    #[test]
    fn no_metatile_points_at_the_skull_donors() {
        let Some(rom) = vanilla() else { return };
        for plane in 0..4 {
            for tile in 0..256 {
                let chr = rom.read_byte(PRG012_FILE_BASE + plane * 256 + tile);
                assert!(
                    !GATE_CHR_TILES.contains(&chr),
                    "tile ${tile:02X} quadrant {plane} already draws CHR ${chr:02X}"
                );
            }
        }
    }

    // -- what apply() writes ---------------------------------------------

    #[test]
    fn k_zero_writes_nothing() {
        let Some(rom) = vanilla() else { return };
        let mut patched = rom.clone();
        apply(&mut patched, 0);
        assert_eq!(patched.data, rom.data, "K = 0 must leave the ROM alone");
    }

    #[test]
    fn apply_stamps_the_gate_and_repoints_the_metatile() {
        let Some(mut rom) = vanilla() else { return };
        crate::randomize::qol::apply_w8_bridges(&mut rom);
        let (row, col) = W8_WAND_GATE_POS;
        assert_eq!(rom.read_byte(map_tile_offset(W8_IDX, row, col)), BRIDGE_TILE);

        apply(&mut rom, K);
        assert_eq!(rom.read_byte(map_tile_offset(W8_IDX, row, col)), WAND_GATE_TILE);
        for (plane, &chr) in GATE_CHR_TILES.iter().enumerate() {
            assert_eq!(
                rom.read_byte(PRG012_FILE_BASE + plane * 256 + WAND_GATE_TILE as usize),
                chr
            );
        }
        for (chr, pattern) in GATE_CHR_TILES.iter().zip(SKULL_CHR.iter()) {
            assert_eq!(rom.read_range(gate_chr_offset(*chr), 16), pattern);
        }
    }

    /// K above seven would be a gate the saturating counter can never open.
    #[test]
    fn k_is_clamped_to_the_wand_count() {
        let Some(mut rom) = vanilla() else { return };
        apply(&mut rom, 200);
        assert_eq!(rom.read_byte(FS_MAZE_WAND_GATE + 20), MAX_WANDS, "K must clamp to 7");
    }

    // -- structural checks ------------------------------------------------

    #[test]
    fn wand_gate_is_well_formed() {
        let Some(rom) = vanilla() else { return };
        let code = wand_gate_routine(K);
        // The return-from-level hook: `JSR gate` over `JSR reload`.
        asm::check(&code)
            .allocation(FS_MAZE_WAND_GATE)
            .origin(WAND_GATE_CPU)
            .hook(&rom.data, RELOAD_FROM_LEVEL_OFFSET, &jmp_free(0x20, WAND_GATE_CPU))
            .assert_ok();
        // The init hook enters six bytes in, which `.hook`'s origin check would
        // read as a hook that missed its routine, so it is verified directly:
        // three bytes over a three-byte `JSR`, aimed at the second entry.
        assert_eq!(
            rom.read_range(FILL_ATTR_CALL_OFFSET, 3),
            &jmp_free(0x20, FILL_ATTR_CPU),
            "the Fill_Tile_AttrTable call has moved"
        );
        assert_eq!(&code[6..9], &jmp_free(0x20, FILL_ATTR_CPU), "the displaced call is replayed");
    }

    /// The bump routine is checked in both worlds it can be written into:
    /// over vanilla's `INC World_Num`, and over `world_order`'s `JMP`.
    #[test]
    fn wand_bump_is_well_formed() {
        let Some(rom) = vanilla() else { return };
        let vanilla_displaced: [u8; 3] =
            core::array::from_fn(|i| rom.read_byte(WORLD_INC_OFFSET + i));
        assert_eq!(vanilla_displaced, [0xEE, 0x27, 0x07], "vanilla INC World_Num has moved");
        assert_eq!(
            rom.read_range(WORLD_INC_OFFSET + 3, 3),
            &[0x4C, 0xA0, 0x84],
            "the JMP $84A0 the bump routine resumes into has moved"
        );

        for displaced in [vanilla_displaced, [0x4C, 0x10, 0x9F]] {
            let code = wand_bump_routine(displaced);
            asm::check(&code)
                .allocation(FS_MAZE_WAND_COUNT)
                .origin(WAND_BUMP_CPU)
                .hook(&rom.data, WORLD_INC_OFFSET, &jmp_free(0x4C, WAND_BUMP_CPU))
                .assert_ok();
        }
    }

    /// `world_order` and this module share the airship hook site, and the
    /// order is one-way: `world_order` first. Applied that way the site holds
    /// our `JMP` and `world_order`'s own `JMP` is replayed inside the routine;
    /// applied the other way round `world_order` overwrites us and the counter
    /// is dead. This asserts the working order and documents the broken one.
    #[test]
    fn the_bump_hook_survives_world_order() {
        use rand::SeedableRng;
        let Some(rom) = vanilla() else { return };

        let mut correct = rom.clone();
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
        crate::randomize::world_order::randomize(&mut correct, &mut rng, 7);
        let world_order_jmp: [u8; 3] =
            core::array::from_fn(|i| correct.read_byte(WORLD_INC_OFFSET + i));
        apply(&mut correct, K);
        assert_eq!(
            correct.read_range(WORLD_INC_OFFSET, 3),
            &jmp_free(0x4C, WAND_BUMP_CPU),
            "the airship site must reach the bump routine"
        );
        assert_eq!(
            correct.read_range(FS_MAZE_WAND_COUNT + 10, 3),
            world_order_jmp,
            "world_order's jump must be replayed, or the world never changes"
        );

        let mut wrong = rom.clone();
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
        apply(&mut wrong, K);
        crate::randomize::world_order::randomize(&mut wrong, &mut rng, 7);
        assert_ne!(
            wrong.read_range(WORLD_INC_OFFSET, 3),
            &jmp_free(0x4C, WAND_BUMP_CPU),
            "this is the broken order the module doc forbids; if it ever stops \
             breaking, the ordering rule can be dropped"
        );
    }
}

/// Executing the two routines on an emulated 2A03. Neither calls out to the
/// engine for anything but a tail jump, so both can be run over their whole
/// input space rather than argued about.
#[cfg(test)]
mod execution {
    use mos6502::cpu::CPU;
    use mos6502::instruction::Ricoh2a03;
    use mos6502::memory::{Bus, Memory};

    use super::*;

    /// An address nothing is assembled at; the harness parks a return there
    /// and stops when the PC arrives.
    const SENTINEL: u16 = 0x4242;

    /// Stand-ins for the engine routines the gate tail-calls. Each is an
    /// `RTS` at the real address, so a call lands somewhere real and comes
    /// straight back — and a *missing* call shows up as an untouched marker
    /// rather than a crash.
    const RELOAD_MARK: u16 = 0x0300;
    const ATTR_MARK: u16 = 0x0301;

    fn cpu_with_gate(k: u8) -> CPU<Memory, Ricoh2a03> {
        let mut mem = Memory::new();
        mem.set_bytes(WAND_GATE_CPU, &super::wand_gate_routine(k));
        // `INC abs; RTS` at each engine entry point: the marker counts calls.
        mem.set_bytes(MAP_RELOAD_CPU, &[0xEE, RELOAD_MARK as u8, (RELOAD_MARK >> 8) as u8, 0x60]);
        mem.set_bytes(FILL_ATTR_CPU, &[0xEE, ATTR_MARK as u8, (ATTR_MARK >> 8) as u8, 0x60]);
        CPU::new(mem, Ricoh2a03)
    }

    fn call(cpu: &mut CPU<Memory, Ricoh2a03>, entry: u16) {
        let ret = SENTINEL.wrapping_sub(1);
        cpu.memory.set_byte(0x01FF, (ret >> 8) as u8);
        cpu.memory.set_byte(0x01FE, ret as u8);
        cpu.registers.stack_pointer = mos6502::registers::StackPointer(0xFD);
        cpu.registers.program_counter = entry;
        for _ in 0..1000 {
            if cpu.registers.program_counter == SENTINEL {
                return;
            }
            cpu.single_step();
        }
        panic!("the gate ran away");
    }

    /// Run one entry point for one `(world, wands)` pair and report what the
    /// gate cell holds afterwards, plus how many times each engine routine was
    /// reached.
    fn run_gate(entry_offset: u16, k: u8, world: u8, wands: u8) -> (u8, u8, u8) {
        let mut cpu = cpu_with_gate(k);
        cpu.memory.set_byte(WORLD_NUM, world);
        cpu.memory.set_byte(WAND_COUNT, wands);
        // Poison: a gate that is never opened leaves this visible.
        cpu.memory.set_byte(GATE_TILE_MEM, 0xAA);
        cpu.memory.set_byte(RELOAD_MARK, 0);
        cpu.memory.set_byte(ATTR_MARK, 0);
        call(&mut cpu, WAND_GATE_CPU + entry_offset);
        (
            cpu.memory.get_byte(GATE_TILE_MEM),
            cpu.memory.get_byte(RELOAD_MARK),
            cpu.memory.get_byte(ATTR_MARK),
        )
    }

    /// The whole input space of the gate: both entries, every world, every
    /// counter value a byte can hold. It opens exactly when the player is in
    /// World 8 with at least K wands, and never otherwise.
    #[test]
    fn the_gate_opens_only_in_world_8_at_k_wands() {
        for k in 1..=MAX_WANDS {
            for &(entry, expect_reload, expect_attr) in &[(0u16, 1u8, 0u8), (6, 0, 1)] {
                for world in 0..8u8 {
                    for wands in 0..=255u8 {
                        let (cell, reload, attr) = run_gate(entry, k, world, wands);
                        let open = world == 7 && wands >= k;
                        assert_eq!(
                            cell,
                            if open { BRIDGE_TILE } else { 0xAA },
                            "k={k} entry={entry} world={world} wands={wands}"
                        );
                        assert_eq!(reload, expect_reload, "wrong reload count, entry={entry}");
                        assert_eq!(attr, expect_attr, "wrong attr count, entry={entry}");
                    }
                }
            }
        }
    }

    /// The gate writes one byte and no other. `Tile_Mem` neighbours are a map
    /// row apart, so an off-by-a-row store would be invisible to the test
    /// above and very visible in game.
    #[test]
    fn the_gate_writes_exactly_one_byte() {
        let mut cpu = cpu_with_gate(1);
        cpu.memory.set_byte(WORLD_NUM, 7);
        cpu.memory.set_byte(WAND_COUNT, 7);
        for i in 0..0x800u16 {
            cpu.memory.set_byte(0x6000 + i, 0x5A);
        }
        call(&mut cpu, WAND_GATE_CPU + 6);
        for i in 0..0x800u16 {
            let want = if 0x6000 + i == GATE_TILE_MEM { BRIDGE_TILE } else { 0x5A };
            assert_eq!(cpu.memory.get_byte(0x6000 + i), want, "stray write at ${:04X}", 0x6000 + i);
        }
    }

    /// Mutation test for the gate: a wrong compare and a wrong store address
    /// must both be caught by the tests above. Without this, a check that
    /// passes proves only that it ran.
    #[test]
    fn a_broken_gate_fails_the_gate_tests() {
        let good = super::wand_gate_routine(3);
        assert!(!differs_from_spec(&good, 3), "the check is vacuous if the real routine fails it");

        // The world compare is at byte 13 (`CMP #7`).
        let mut wrong_world = good;
        wrong_world[13] = 6;
        assert!(differs_from_spec(&wrong_world, 3), "a wrong world compare must be caught");

        // The wand compare is at byte 20 (`CMP #K`).
        let mut wrong_k = good;
        wrong_k[20] = 1;
        assert!(differs_from_spec(&wrong_k, 3), "a wrong K must be caught");

        // The store address is at bytes 26-27.
        let mut wrong_addr = good;
        wrong_addr[26] = wrong_addr[26].wrapping_add(0x10); // one map row down
        assert!(differs_from_spec(&wrong_addr, 3), "a wrong Tile_Mem address must be caught");
    }

    /// Does `code` disagree with the gate's specification anywhere in the
    /// input space? Used only by the mutation test.
    fn differs_from_spec(code: &[u8; 29], k: u8) -> bool {
        for world in 0..8u8 {
            for wands in 0..=8u8 {
                let mut cpu = cpu_with_gate(k);
                cpu.memory.set_bytes(WAND_GATE_CPU, code);
                cpu.memory.set_byte(WORLD_NUM, world);
                cpu.memory.set_byte(WAND_COUNT, wands);
                for i in 0..0x100u16 {
                    cpu.memory.set_byte(0x6600 + i, 0xAA);
                }
                call(&mut cpu, WAND_GATE_CPU + 6);
                let open = world == 7 && wands >= k;
                let want = if open { BRIDGE_TILE } else { 0xAA };
                if cpu.memory.get_byte(GATE_TILE_MEM) != want {
                    return true;
                }
                for i in 0..0x100u16 {
                    if 0x6600 + i != GATE_TILE_MEM && cpu.memory.get_byte(0x6600 + i) != 0xAA {
                        return true;
                    }
                }
            }
        }
        false
    }

    // -- the counter -----------------------------------------------------

    /// Where the bump routine's tail jump lands: an `INC` marker plus an `RTS`
    /// standing in for the rest of the airship transition.
    const RESUME_MARK: u16 = 0x0302;

    fn run_bump(displaced: [u8; 3], start: u8) -> (u8, u8) {
        let resume = (WORLD_INC_OFFSET - 0x3C010 + 0x8000 + 3) as u16;
        let mut mem = Memory::new();
        mem.set_bytes(WAND_BUMP_CPU, &super::wand_bump_routine(displaced));
        mem.set_bytes(resume, &[0xEE, RESUME_MARK as u8, (RESUME_MARK >> 8) as u8, 0x60]);
        // Vanilla's displaced `INC World_Num` needs somewhere to land, and
        // world_order's displaced `JMP $9F10` needs a routine to reach.
        mem.set_bytes(0x9F10, &[0xEE, RESUME_MARK as u8, (RESUME_MARK >> 8) as u8, 0x60]);
        let mut cpu = CPU::new(mem, Ricoh2a03);
        cpu.memory.set_byte(WAND_COUNT, start);
        cpu.memory.set_byte(RESUME_MARK, 0);
        call(&mut cpu, WAND_BUMP_CPU);
        (cpu.memory.get_byte(WAND_COUNT), cpu.memory.get_byte(RESUME_MARK))
    }

    /// The counter increments, saturates at seven, and always reaches the code
    /// the hook displaced — over both shapes that code can take.
    #[test]
    fn the_counter_bumps_saturates_and_chains() {
        for displaced in [[0xEE, 0x27, 0x07], [0x4C, 0x10, 0x9F]] {
            for start in 0..=255u8 {
                let (after, resumed) = run_bump(displaced, start);
                let want = if start >= MAX_WANDS { start } else { start + 1 };
                assert_eq!(after, want, "start={start} displaced={displaced:02X?}");
                assert_eq!(resumed, 1, "the displaced code was never reached");
            }
        }
    }

    /// Mutation test for the counter: dropping the saturate, or losing the
    /// chain, must both fail the test above.
    #[test]
    fn a_broken_counter_fails_the_counter_tests() {
        let displaced = [0xEE, 0x27, 0x07];
        let good = super::wand_bump_routine(displaced);
        assert!(
            !bump_differs(&good, displaced),
            "the check is vacuous if the real routine fails it"
        );

        // Byte 4 is the `CMP #7` operand: raise it and the counter runs past
        // seven, which the saturate case catches.
        let mut no_cap = good;
        no_cap[4] = 0xFF;
        assert!(bump_differs(&no_cap, displaced), "a missing saturate must be caught");

        // Bytes 13-15 are the tail `JMP`: aim it elsewhere and nothing resumes.
        let mut no_chain = good;
        no_chain[13] = 0xEA; // NOP, so the routine falls into whatever follows
        assert!(bump_differs(&no_chain, displaced), "a lost chain must be caught");
    }

    fn bump_differs(code: &[u8; 16], displaced: [u8; 3]) -> bool {
        let resume = (WORLD_INC_OFFSET - 0x3C010 + 0x8000 + 3) as u16;
        for start in 0..=255u8 {
            let mut mem = Memory::new();
            mem.set_bytes(WAND_BUMP_CPU, code);
            let _ = displaced;
            mem.set_bytes(resume, &[0xEE, RESUME_MARK as u8, (RESUME_MARK >> 8) as u8, 0x60]);
            mem.set_bytes(0x9F10, &[0xEE, RESUME_MARK as u8, (RESUME_MARK >> 8) as u8, 0x60]);
            let mut cpu = CPU::new(mem, Ricoh2a03);
            cpu.memory.set_byte(WAND_COUNT, start);
            cpu.memory.set_byte(RESUME_MARK, 0);
            let ret = SENTINEL.wrapping_sub(1);
            cpu.memory.set_byte(0x01FF, (ret >> 8) as u8);
            cpu.memory.set_byte(0x01FE, ret as u8);
            cpu.registers.stack_pointer = mos6502::registers::StackPointer(0xFD);
            cpu.registers.program_counter = WAND_BUMP_CPU;
            let mut ran_away = true;
            for _ in 0..1000 {
                if cpu.registers.program_counter == SENTINEL {
                    ran_away = false;
                    break;
                }
                cpu.single_step();
            }
            let want = if start >= MAX_WANDS { start } else { start + 1 };
            if ran_away
                || cpu.memory.get_byte(WAND_COUNT) != want
                || cpu.memory.get_byte(RESUME_MARK) != 1
            {
                return true;
            }
        }
        false
    }
}

/// The claim the gate rests on: (5,59) is the *only* way to Bowser's castle,
/// on every map the builder produces — not just on the vanilla grid.
#[cfg(test)]
mod chokepoint {
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    use super::*;
    use crate::randomize::map_walker::walk_reachable;
    use crate::randomize::node_catalog::NodeCatalog;
    use crate::randomize::overworld_build::{BuildFlags, OverworldData, build, stamp_slots};
    use crate::randomize::overworld_helpers::find_target;
    use crate::randomize::overworld_pickup::{PickupFlags, pick_up};
    use crate::randomize::{qol, start_airship_swap};

    /// One seed's World 8, over the same three flag arms the overworld and
    /// maze censuses use — the gate has to hold on every map the builder can
    /// produce, not on one arm of it.
    fn built_w8(raw: &Rom, seed: u64, world_maze: bool) -> BuiltW8 {
        let (hammer_rocks, eights_wild) = match seed % 4 {
            2 => (true, false),
            3 => (false, true),
            _ => (false, false),
        };
        let mut rom = raw.clone();
        qol::fix_w3_drawbridges(&mut rom);
        qol::remove_rocks(&mut rom);
        if hammer_rocks {
            qol::make_hammer_rocks(&mut rom);
        }
        qol::apply_w1_shortcut(&mut rom, hammer_rocks);
        qol::apply_w8_bridges(&mut rom);
        if eights_wild {
            qol::apply_w8_canoe_and_paths(&mut rom);
        }

        let mut catalog = NodeCatalog::build(&rom, false);
        let mut swap_rng = ChaCha8Rng::seed_from_u64(seed);
        start_airship_swap::pick_swaps(&mut catalog, &mut swap_rng);
        let pickup = pick_up(
            &rom,
            &catalog,
            PickupFlags {
                shuffle_spade_games: true,
                shuffle_toad_houses: true,
                ..Default::default()
            },
        );
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let result = build(
            &rom,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            BuildFlags {
                shuffle_toad_houses: true,
                eights_are_wild: eights_wild,
                world_maze,
                ..Default::default()
            },
        );
        let w8 = result.worlds.iter().find(|w| w.world_idx == W8_IDX).unwrap();
        let mut grid = w8.grid.clone();
        stamp_slots(&mut grid, &w8.slots);
        BuiltW8 {
            grid,
            pipes: w8.pipe_pairs.clone(),
            span_cols: w8
                .locks
                .iter()
                .filter(|l| l.replace_tile == crate::randomize::rom_data::BRIDGE_TILE)
                .map(|l| l.pos.1)
                .collect(),
        }
    }

    type Pos = (usize, usize);

    /// What one seed's World 8 hands the tests below: the finished grid with
    /// content stamped and locks held OPEN, its pipe web, and which columns of
    /// the bridge approach ended up dealt as water gaps.
    struct BuiltW8 {
        grid: crate::randomize::rom_data::Grid,
        pipes: Vec<(Pos, Pos)>,
        span_cols: Vec<usize>,
    }

    /// With every lock held OPEN — the player's best case — the castle is
    /// reachable, and walling the single gate cell makes it unreachable. That
    /// second half is the whole argument for one cell being enough: if any
    /// seed could route around it, the gate would be decoration.
    #[test]
    fn the_gate_cell_is_the_only_approach() {
        let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let raw = Rom::from_bytes(&bytes).unwrap();
        let seeds: u64 =
            std::env::var("CENSUS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(40);

        for seed in 0..seeds {
            let w8 = built_w8(&raw, seed, true);
            let target = find_target(&w8.grid, W8_IDX).expect("W8 has a castle");

            let open = walk_reachable(&w8.grid, &w8.pipes, None, W8_IDX);
            assert!(open.contains(target), "seed {seed}: the castle is already unreachable");

            let mut walled = w8.grid.clone();
            walled.set(W8_WAND_GATE_POS.0, W8_WAND_GATE_POS.1, WAND_GATE_TILE);
            let gated = walk_reachable(&walled, &w8.pipes, None, W8_IDX);
            assert!(
                !gated.contains(target),
                "seed {seed}: the castle is reachable around the wand gate at {W8_WAND_GATE_POS:?}"
            );
        }
    }

    /// In maze mode the wand-gate cell must never carry a lock; in standard
    /// mode it must still be dealable, because that is what W8 has always
    /// done and the maze is not allowed to move it. Both halves matter: the
    /// first is the gate's correctness, the second is the baseline.
    #[test]
    fn the_gate_cell_is_reserved_only_in_maze_mode() {
        let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let raw = Rom::from_bytes(&bytes).unwrap();
        let seeds: u64 =
            std::env::var("CENSUS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(40);

        let mut standard_saw_it = false;
        for seed in 0..seeds {
            assert!(
                !built_w8(&raw, seed, true).span_cols.contains(&W8_WAND_GATE_POS.1),
                "seed {seed}: maze mode dealt a lock onto the wand-gate cell"
            );
            standard_saw_it |= built_w8(&raw, seed, false).span_cols.contains(&W8_WAND_GATE_POS.1);
        }
        assert!(
            standard_saw_it,
            "standard mode never dealt the gate span in {seeds} seeds \u{2014} the reservation              has leaked out of maze mode, or the deal has changed"
        );
    }

    /// The W8 bridge deal, both ways. Standard mode must reproduce the numbers
    /// `overworld_build::tests::w8_bridges_out_census` prints on `beta/next`;
    /// the maze arm is the new one, and what it answers is where the seed's
    /// gating span goes once the one nearest the goal is off the table.
    ///
    ///   CENSUS_SEEDS=2000 cargo test --release --lib wand_gate_bridge_deal \
    ///       -- --ignored --nocapture
    #[test]
    #[ignore]
    fn wand_gate_bridge_deal_census() {
        let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let raw = Rom::from_bytes(&bytes).unwrap();
        let seeds: u64 =
            std::env::var("CENSUS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(500);

        for (label, world_maze) in [("standard", false), ("maze", true)] {
            let mut count_hist = [0usize; 6];
            let mut col_hist = std::collections::BTreeMap::<usize, usize>::new();
            for seed in 0..seeds {
                let cols = built_w8(&raw, seed, world_maze).span_cols;
                count_hist[cols.len().min(5)] += 1;
                for c in cols {
                    *col_hist.entry(c).or_default() += 1;
                }
            }
            let pct = |n: usize| 100.0 * n as f64 / seeds as f64;
            println!("\nW8 bridges out over {seeds} seeds ({label}):");
            for (n, &count) in count_hist.iter().enumerate() {
                if count > 0 || n < 5 {
                    println!("  {n} out: {count:6} ({:6.2}%)", pct(count));
                }
            }
            println!("  which span:");
            for (col, count) in &col_hist {
                println!("    col {col}: {count}");
            }
        }
    }
}
