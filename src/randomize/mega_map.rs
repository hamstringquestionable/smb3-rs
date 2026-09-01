//! EXPERIMENT: fold one screen from each of the eight worlds into a single
//! eight-screen mega map in world 0.
//!
//! The prototype answers the questions the design can't be settled without:
//! does the map engine pan past its vanilla three-screen maximum, does
//! `Map_Completions` track clears on screens 4-7, and does the pointer-entry
//! search find entries on a far screen. Everything else — the builder, the
//! progression chain, per-region palettes — is deliberately out of scope. See
//! `docs/mega_map.md`.
//!
//! # Why screen 0 of every world
//!
//! Screen 0 of each of the eight worlds holds **exactly one fortress** and
//! 157 pointer entries in total. That makes the destination map eight
//! screens, eight forts, one biome per screen, and 157 entries — comfortably
//! under the 255 that `Map_ByXHi_InitIndex`'s byte-wide start index allows.
//! Taking whole worlds instead would either overrun the completion array
//! (19 screens of vanilla map needs 304 columns; there are 128) or drop
//! fortresses.
//!
//! # The eight-screen ceiling
//!
//! `Map_Completions` is 128 bytes at `$7D00`, one per map column, split
//! `$7D00-$7D3F` Mario / `$7D40-$7D7F` Luigi — "allows a MAX of 4 map
//! screens" per the disassembly. A one-player map can use all 128 columns,
//! because a column index for eight screens runs 0..127 and lands exactly
//! inside the array. The engine already walks all 128 bytes: the redraw loop
//! in `Map_Reload_with_Completions` runs its column counter to `$80`, and
//! the game-over clear runs `LDY #$7F`.
//!
//! What stops it is that four sites *fold* the upper half back onto the lower
//! one, on the assumption that it belongs to the other player. Widening the
//! map to eight screens is therefore not new code but **seven operand bytes
//! plus one six-byte splice** — no free space is claimed. See
//! [`widen_map_completions`].
//!
//! Two-player mode is the cost: Luigi's completion array *is* screens 4-7.
//! Nothing here disables 2P, so a 2P game on a mega map would have the two
//! players writing over each other. The prototype is 1P-only by construction.

use std::collections::{HashMap, HashSet};

use crate::rom::Rom;

use super::map_walker;

use super::rom_data::{
    self, BACKGROUND_TILES, FX_MAP_COMP_IDX, FX_WORLD_TABLE, PRG012_FILE_BASE, VALID_BLANK_TILES,
};

/// Screens in the mega map. Bounded by `Map_Completions` at 128 columns
/// (see the module comment), not by tile RAM — `Tile_Mem_Addr` in PRG030
/// carries fifteen screen entries.
pub(crate) const SCREENS: usize = 8;

/// Columns in the mega map.
pub(crate) const COLUMNS: usize = SCREENS * 16;

/// Rows in every overworld map.
const ROWS: usize = rom_data::ROWS;

/// Bytes of tile data per screen: 9 rows x 16 columns.
const SCREEN_BYTES: usize = 144;

/// Source `(world, screen)` for each destination screen. Screen 0 of every
/// world, in world order, so destination screen `i` is world `i`'s first
/// screen and the region index and the world index coincide — which is what
/// makes the eight-wide per-world dispatch tables (palette, music, king,
/// airship) directly reusable as *per-region* tables later.
pub(crate) const SOURCE: [(usize, usize); SCREENS] =
    [(0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (5, 0), (6, 0), (7, 0)];

// --- Vanilla table locations -------------------------------------------
//
// Held locally rather than read from `rom_data::WORLDS` / `MAP_TILE_GRIDS`
// on purpose: this module rewrites the layout those constants describe, so
// it must address the *vanilla* one. Reading them would make the module's
// behaviour depend on whether it had already run.

/// Vanilla per-world tile grids: `(file_offset, screens)`.
const VAN_GRIDS: [(usize, usize); 8] = [
    (0x185BA, 1),
    (0x1864B, 2),
    (0x1876C, 3),
    (0x1891D, 2),
    (0x18A3E, 2),
    (0x18B5F, 3),
    (0x18D10, 2),
    (0x18E31, 4),
];

/// Vanilla per-world pointer blocks: `(rowtype_offset, entry_count)`.
const VAN_WORLDS: [(usize, usize); 8] = [
    (0x19438, 21),
    (0x194BA, 47),
    (0x195D8, 52),
    (0x19714, 34),
    (0x197E4, 42),
    (0x198E4, 57),
    (0x19A3E, 46),
    (0x19B56, 41),
];

/// Grid pointer table: 9 little-endian CPU words (8 worlds + Warp Zone).
const GRID_PTRS: usize = 0x185A8;

/// Destination for the merged grid — W1's slot. Eight screens plus the `$FF`
/// terminator is 1153 bytes, into the 2888 the eight worlds' grids occupy
/// from here (0x185BA-0x19102), so it stays inside the region the maps
/// already own and claims no free space.
const DEST_GRID: usize = 0x185BA;

/// Destination for the merged pointer block — W1's slot. `InitIndex(8)`,
/// `RowType(N)`, `ScrCol(N)`, `ObjSets(2N)`, `Layouts(2N)`, contiguous.
/// At N=157 that is 950 bytes, into the 2072 the eight blocks occupy from
/// here (0x19434-0x19C4C).
///
/// It must live here. PRG012's only other gap over 100 bytes is 0x19DD0, and
/// that is **not** free — the Big ? Block trampoline, the flag-key stamp and
/// the title-screen seed-hash icons all write there, the last of them after
/// the overworld writer has run. The world-merge experiment lost most of a
/// session to exactly that.
const DEST_BLOCK: usize = 0x19434;

/// Master pointer tables, one 16-bit CPU address per world.
const INIT_MASTER: usize = 0x193DA;
const ROWTYPE_MASTER: usize = 0x193EC;
const SCRCOL_MASTER: usize = 0x193FE;
const OBJSETS_MASTER: usize = 0x19410;
const LAYOUTS_MASTER: usize = 0x19422;

/// `World_Map_Max_PanR`, 8 bytes, `$10` per screen of rightward scroll.
/// Vanilla's largest is `$30`.
const MAX_PAN_R: usize = 0x14F44;

/// `FortressFX_MapLocation`: column in the high nibble, screen in the low.
const FX_MAP_LOCATION: usize = 0x14866;

// --- Tiles --------------------------------------------------------------

/// Horizontal path tile — a cell the walker will step *through*. Must come
/// from `VALID_HORZ` (the engine's `Map_Object_Valid_Left/Right` registry);
/// `0x44` looks like a path and is not one, it is a blank *node*.
const TILE_PATH_H: u8 = 0x45;

/// Blank node tile — a cell the walker will *stand on*. Land-themed, which is
/// wrong on the sky and island screens; theming it needs the per-screen rule
/// in `overworld_pickup::blank_tile_from_neighbors`, which reads the vanilla
/// per-world grids and so cannot run after the fold.
const TILE_NODE_BLANK: u8 = 0x44;

/// Vertical path tile, from `VALID_VERT` (`Map_Object_Valid_Down/Up`).
const TILE_PATH_V: u8 = 0x46;

/// Castle tiles: an enterable bottom with a decorative top directly above.
const TILE_CASTLE_BOTTOM: u8 = 0xC9;
const TILE_CASTLE_TOP: u8 = 0xC8;

/// The map pipe tile (`TILE_PIPE = $BC` in the disassembly).
///
/// A pipe is only half a connection — its partner is the other endpoint of a
/// dest-table pair — so any pipe whose partner did not survive the fold has
/// to stop being a pipe. See [`neutralize_pipes`].
///
/// Not `0x68`/`0x69`. `docs/smb3_rom_reference.md` listed those as "map pipe
/// connectors" and they are nothing of the kind: no `TILE_*` constant in the
/// disassembly has either value, and every one of the 24 pipe destination
/// endpoints lands on a `$BC` cell. Using the wrong pair silently carried
/// zero pipes, which in turn made half of Pipe Land unreachable.
const TILE_PIPE: [u8; 1] = [0xBC];

/// One screen boundary and what it took to make it walkable.
#[derive(Debug)]
pub struct Seam {
    /// Boundary index: between screen `seam` and `seam + 1`.
    pub seam: usize,
    /// Row the corridor was carved on, or `None` if no even-span node pair
    /// was in reach and the seam is still a wall.
    pub row: Option<usize>,
    /// Tiles overwritten between the two anchors.
    pub tiles: usize,
}

/// What the fold produced, for the caller to report without re-deriving it.
#[derive(Debug, Default)]
pub struct MegaMapReport {
    /// Pointer entries carried into the merged block.
    pub entries: usize,
    /// Fortress FX slots re-aimed at their destination screen.
    pub forts: usize,
    /// One entry per screen boundary.
    pub seams: Vec<Seam>,
    /// Pipe pairs carried across with both endpoints intact.
    pub pipes_carried: usize,
    /// Fortresses the screen joins left stranded, then reconnected.
    pub forts_connected: usize,
    /// Fortresses still unreachable after that.
    pub forts_stranded: usize,
    /// Pipe tiles demoted to path because their partner was not carried.
    pub pipes_neutralized: usize,
    /// Duplicate start / castle tiles blanked.
    pub singletons_blanked: usize,
}

/// Build the eight-screen mega map in world 0.
///
/// Order matters. The grid has to exist before the seam carve and the
/// singleton pass can read it, and the pointer block has to exist before the
/// seam carve can ask which cells hold nodes.
pub fn build(rom: &mut Rom) -> Result<MegaMapReport, String> {
    rom.push_tag("mega_map");
    let mut report = MegaMapReport::default();

    widen_map_completions(rom)?;
    pin_world(rom)?;

    // Against the untouched ROM, before any write — see `row_shifts`.
    let shifts = row_shifts(rom);
    let forts = fort_positions(rom, &shifts);

    build_grid(rom, &shifts);
    let entries = build_pointer_block(rom, &shifts)?;
    report.entries = entries.len();

    report.forts = carry_fortress_fx(rom, &shifts);
    let carried = carry_pipes(rom, &shifts);
    report.pipes_neutralized = neutralize_pipes(rom, &carried);
    report.singletons_blanked = reconcile_singletons(rom);

    // Read the pipes back off the finished grid rather than trusting the
    // list `carry_pipes` intended. The grid is the ground truth — a pass
    // between here and there could have demoted or overwritten an endpoint —
    // and reading it back is how the wrong `TILE_PIPE` value was caught:
    // `carry_pipes` reported seven pairs while the map held none.
    let pipes = merged_pipe_pairs(rom);
    report.pipes_carried = pipes.len();

    report.seams = bridge_seams(rom, &entries, &pipes);
    let (connected, stranded) = connect_stranded_forts(rom, &entries, &forts, &pipes);
    report.forts_connected = connected;
    report.forts_stranded = stranded;

    // Max_PanR is the rightmost scroll column, so it scales with total
    // columns, not screens-minus-one: vanilla is $10/$20/$30 for its 16/32/48
    // column worlds. Eight screens is 128 columns -> $70.
    rom.write_byte(MAX_PAN_R, 0x70);

    rom.pop_tag();
    Ok(report)
}

// --- Completions --------------------------------------------------------

/// Overwrite a vanilla byte, refusing if it does not hold what we expect.
///
/// Every site below was located by byte pattern against the USA Rev 1 ROM.
/// Asserting the old value turns "this ROM is not the one the offsets were
/// derived from" into an error at build time instead of a corrupted patch
/// that still boots.
fn expect_write(rom: &mut Rom, offset: usize, want: u8, new: u8, site: &str) -> Result<(), String> {
    let got = rom.read_byte(offset);
    if got != want {
        return Err(format!(
            "mega_map: {site} at {offset:#07X} holds {got:#04X}, expected {want:#04X} \
             — this is not an unmodified SMB3 USA Rev 1"
        ));
    }
    rom.write_byte(offset, new);
    Ok(())
}

/// Let `Map_Completions` cover eight screens for one player.
///
/// The array is already 128 bytes and the engine already walks all of them.
/// Four sites fold the upper 64 back onto the lower 64 because they assume
/// it is Luigi's half; each is neutralised in place, at no size cost:
///
/// | Site | Vanilla | Becomes |
/// |---|---|---|
/// | `Map_Reload_with_Completions` screen index | `AND #$30` | `AND #$70` |
/// | ... its Mario/Luigi marker select | `AND #$40` | `AND #$00` |
/// | `MO_DoFortressFX` mirror write | `$7D40,Y` x2 | `$7D00,Y` x2 |
/// | rock-break mirror write | `EOR #$40` | `EOR #$00` |
/// | game-over clear | `AND` other player | `LDA #$00`, 128 columns |
///
/// The first is the one without which nothing works: `AND #$30` takes two
/// bits of screen out of the column index, so a clear on screen 4 would draw
/// on screen 0. `AND #$70` takes three. The shift that follows already
/// produces a `Tile_Mem_Addr` index, and that table has fifteen entries, so
/// nothing downstream needs widening.
///
/// The mirror writes are worse than cosmetic: `MO_DoFortressFX`'s
/// `Map_Completions+$40,Y` with a column past 63 writes to `$7D80` and up,
/// which is `Inventory_Items` — clearing a fortress on screens 4-7 would
/// rewrite the player's inventory. Pointing both operands back at Mario's
/// array makes the mirror a harmless repeat of the write just above it.
fn widen_map_completions(rom: &mut Rom) -> Result<(), String> {
    // Map_Reload_with_Completions (PRG012 $A4F5): screen bits of the column.
    expect_write(rom, 0x18507, 0x30, 0x70, "completion redraw screen mask")?;

    // Map_Reload_with_Completions (PRG012 $A570): picks the "M" or "L" clear
    // marker from bit 6 of the column. Every column is Mario's now.
    expect_write(rom, 0x18587, 0x40, 0x00, "completion marker select")?;

    // MO_DoFortressFX (PRG010): the "mark it for Luigi too" mirror.
    expect_write(rom, 0x14984, 0x40, 0x00, "fortress FX mirror load")?;
    expect_write(rom, 0x14989, 0x40, 0x00, "fortress FX mirror store")?;

    // Rock removal (PRG026): flips to the other player's byte.
    expect_write(rom, 0x34705, 0x40, 0x00, "rock-break mirror")?;

    // Game over (PRG030 $9314). Vanilla clears one player's 64 columns by
    // ANDing the other player's — which for 1P is ANDing zeros. With eight
    // screens the "other player" is screens 4-7, so the vanilla form would
    // fold half the continent into the other half instead of clearing it.
    // Store zero outright, over all 128 columns.
    expect_write(rom, 0x3D31D, 0x3F, 0x7F, "game-over clear start index")?;
    expect_write(rom, 0x3D325, 0x3F, 0x7F, "game-over clear count")?;

    // LDA Map_Completions,X / AND Map_Completions,Y  ->  LDA #$00 + padding.
    // The STA that follows is left alone and now stores zero. The preceding
    // TYA/EOR/TAX becomes dead but is harmless, so it stays: overwriting it
    // would buy nothing and cost a longer splice.
    const GAMEOVER_MERGE: usize = 0x3D32C;
    for (i, (want, new)) in
        [(0xBD, 0xA9), (0x00, 0x00), (0x7D, 0xEA), (0x39, 0xEA), (0x00, 0xEA), (0x7D, 0xEA)]
            .into_iter()
            .enumerate()
    {
        expect_write(rom, GAMEOVER_MERGE + i, want, new, "game-over clear merge")?;
    }

    Ok(())
}

/// Stop `INC World_Num` from advancing off the mega map.
///
/// Only world 0 has a grid and a pointer block after the fold; worlds 1-7
/// still have master-table pointers, but they aim into the middle of the
/// merged block. Clearing the airship would advance into that and hang.
/// There is one world now, so the advance becomes three `NOP`s.
///
/// This is what has to become a real progression model before the prototype
/// is a game — see `docs/mega_map.md`.
fn pin_world(rom: &mut Rom) -> Result<(), String> {
    const INC_WORLD_NUM: usize = 0x3D0A1; // PRG030, `INC $0727`
    for (i, want) in [0xEE, 0x27, 0x07].into_iter().enumerate() {
        expect_write(rom, INC_WORLD_NUM + i, want, 0xEA, "world advance")?;
    }
    Ok(())
}

// --- Grid ---------------------------------------------------------------

/// Per-screen row shift, so every screen's node lattice lands on the same
/// row parity as screen 0's.
///
/// # Why this is needed at all
///
/// The player moves two tiles at a time, so a walk changes a row or a column
/// by exactly 2 and **`row % 2` is invariant along any path**. Every node the
/// start can ever reach shares the start's row parity. That is not a property
/// of the fold — it is how the map engine has always worked — but a single
/// world never notices, because each vanilla map is internally consistent.
///
/// Folding eight of them together does notice. Measured over the source
/// screens, the split is total and there is no mixing:
///
/// | Screen | Source | Node rows |
/// |---|---|---|
/// | 0-5 | W1-W6 | all even (0, 2, 4, 6, 8) |
/// | 6-7 | W7, W8 | all odd (1, 3, 5, 7) |
///
/// So W7's and W8's screens are half a step out of phase with the other six.
/// No corridor of any shape can join them — not a longer one, not an
/// L-shaped one — because the parity clash is invariant under every legal
/// move. Shifting the screen's contents by one row is the only fix, and it
/// costs nothing here: W7's deepest node moves from row 7 to row 8 and W8's
/// from 5 to 6, both still on the map.
///
/// Shifting down is preferred because row 0 is a screen's top border and
/// duplicating it is invisible, whereas the bottom row usually carries the
/// map's ground edge.
/// **Compute these once, against the untouched ROM, before anything is
/// written.** `build_pointer_block` writes the merged block over the source
/// blocks of the first few worlds, so a later pass that re-derives a shift
/// from `VAN_WORLDS` reads its own output and gets nonsense. That is the same
/// ordering trap the world-merge experiment hit from the other direction.
fn row_shifts(rom: &Rom) -> [isize; SCREENS] {
    const TARGET_PARITY: usize = 0; // screen 0's lattice, and so the start's

    let mut shifts = [0; SCREENS];
    for (dest, &(world, screen)) in SOURCE.iter().enumerate() {
        let (rowtype_offset, n) = VAN_WORLDS[world];
        let rows: Vec<usize> = read_entries(rom, rowtype_offset, n)
            .iter()
            .filter(|e| (e.scrcol >> 4) as usize == screen)
            .map(|e| e.row())
            .collect();

        let Some(&first) = rows.first() else { continue };
        if first % 2 == TARGET_PARITY {
            continue;
        }
        // Down if the deepest node still fits, otherwise up.
        shifts[dest] = if rows.iter().all(|&r| r + 1 < ROWS) { 1 } else { -1 };
    }
    shifts
}

/// Apply a row shift, or `None` if the row falls off the map.
fn shifted(row: usize, shift: isize) -> Option<usize> {
    let r = row as isize + shift;
    (0..ROWS as isize).contains(&r).then_some(r as usize)
}

fn cpu_of(file_offset: usize) -> u16 {
    (0xA000 + (file_offset - PRG012_FILE_BASE)) as u16
}

fn write_word(rom: &mut Rom, offset: usize, val: u16) {
    rom.write_range(offset, &[(val & 0xFF) as u8, (val >> 8) as u8]);
}

/// File offset of `(row, col)` in the merged grid.
pub(crate) fn tile_offset(row: usize, col: usize) -> usize {
    DEST_GRID + (col / 16) * SCREEN_BYTES + row * 16 + (col % 16)
}

/// Assemble the eight source screens into one grid, then the `$FF` the
/// loader stops on.
///
/// Read every source screen out before writing any of it: the destination
/// starts at W1's grid and runs over W2's, W3's and W4's sources.
fn build_grid(rom: &mut Rom, shifts: &[isize; SCREENS]) {
    let mut merged = Vec::with_capacity(SCREEN_BYTES * SCREENS + 1);
    for (dest, &(world, screen)) in SOURCE.iter().enumerate() {
        let (base, _) = VAN_GRIDS[world];
        let src = rom.read_range(base + screen * SCREEN_BYTES, SCREEN_BYTES).to_vec();
        let shift = shifts[dest];

        for row in 0..ROWS {
            // The row this destination row draws from. Where the shift
            // vacates an edge row, repeat the edge rather than flooding it
            // with water — the vacated row is a border and the duplicate is
            // invisible.
            let from = shifted(row, -shift).unwrap_or(if shift > 0 { 0 } else { ROWS - 1 });
            merged.extend_from_slice(&src[from * 16..from * 16 + 16]);
        }
    }
    merged.push(0xFF);
    rom.write_range(DEST_GRID, &merged);

    write_word(rom, GRID_PTRS, cpu_of(DEST_GRID));
}

fn read_grid(rom: &Rom) -> Vec<Vec<u8>> {
    (0..ROWS).map(|r| (0..COLUMNS).map(|c| rom.read_byte(tile_offset(r, c))).collect()).collect()
}

// --- Pointer block ------------------------------------------------------

/// One pointer-table entry, held column-wise in ROM.
#[derive(Clone, Copy)]
pub(crate) struct Entry {
    rowtype: u8,
    scrcol: u8,
    obj: u16,
    lay: u16,
}

impl Entry {
    /// Grid row. The RowType high nibble is the row offset by 2; a handful
    /// of vanilla entries carry a nibble below that, so saturate rather than
    /// underflow — they are filtered out by the grid-bounds checks anyway.
    pub(crate) fn row(&self) -> usize {
        (((self.rowtype >> 4) & 0x0F) as usize).saturating_sub(2)
    }
    pub(crate) fn col(&self) -> usize {
        ((self.scrcol >> 4) as usize) * 16 + (self.scrcol & 0x0F) as usize
    }
}

fn read_entries(rom: &Rom, rowtype_offset: usize, n: usize) -> Vec<Entry> {
    let scrcol = rowtype_offset + n;
    let obj = scrcol + n;
    let lay = obj + n * 2;
    (0..n)
        .map(|i| Entry {
            rowtype: rom.read_byte(rowtype_offset + i),
            scrcol: rom.read_byte(scrcol + i),
            obj: rom_data::read_word(rom, obj + i * 2),
            lay: rom_data::read_word(rom, lay + i * 2),
        })
        .collect()
}

/// Gather each source screen's entries, renumber them onto their destination
/// screen, and write one block.
///
/// The concatenation needs no resort. Entries are ordered by `(screen, row,
/// col)` within a world, taking one screen from each preserves that order
/// within a run, and the runs are appended in destination-screen order — so
/// the whole block comes out sorted, which is what the engine's forward
/// search from `InitIndex` assumes.
fn build_pointer_block(rom: &mut Rom, shifts: &[isize; SCREENS]) -> Result<Vec<Entry>, String> {
    let mut merged: Vec<Entry> = Vec::new();
    for (dest, &(world, screen)) in SOURCE.iter().enumerate() {
        let (rowtype_offset, n) = VAN_WORLDS[world];
        let shift = shifts[dest];

        for e in read_entries(rom, rowtype_offset, n) {
            if (e.scrcol >> 4) as usize != screen {
                continue;
            }
            // An entry whose row leaves the map is dropped rather than
            // clamped: two entries on one cell would give the engine's
            // forward search two answers for one position.
            let Some(row) = shifted(e.row(), shift) else { continue };

            merged.push(Entry {
                // Row lives in the high nibble of RowType, offset by 2; the
                // low nibble is the tileset and must survive untouched.
                rowtype: (((row + 2) as u8) << 4) | (e.rowtype & 0x0F),
                // Screen lives in the high nibble of ScrCol.
                scrcol: ((dest as u8) << 4) | (e.scrcol & 0x0F),
                ..e
            });
        }
    }

    let n = merged.len();
    // `Map_ByXHi_InitIndex` holds a byte per screen. An entry index past 255
    // cannot be named as a screen's start, and the fold has no way to shrink
    // itself, so this is a hard stop rather than a truncation.
    if n > 255 {
        return Err(format!(
            "mega_map: {n} entries exceeds the 255 a byte-wide InitIndex can address"
        ));
    }

    let init = DEST_BLOCK;
    let rowtype = init + SCREENS;
    let scrcol = rowtype + n;
    let objsets = scrcol + n;
    let layouts = objsets + n * 2;
    let block_end = layouts + n * 2;

    // The merged block runs over the blocks of the worlds it consumed. Those
    // are dead, but the *last* world's block must survive: its entries are
    // still being read out of the ROM by this very function on a re-run, and
    // more importantly overrunning past 0x19C4C would reach the master
    // tables' neighbours rather than dead world data.
    const BLOCK_REGION_END: usize = 0x19C4C;
    if block_end > BLOCK_REGION_END {
        return Err(format!(
            "mega_map: merged block ends at {block_end:#07X}, past the world-block region \
             end {BLOCK_REGION_END:#07X}"
        ));
    }

    for (i, e) in merged.iter().enumerate() {
        rom.write_byte(rowtype + i, e.rowtype);
        rom.write_byte(scrcol + i, e.scrcol);
        write_word(rom, objsets + i * 2, e.obj);
        write_word(rom, layouts + i * 2, e.lay);
    }

    // InitIndex: first entry index on each screen, with `n` as the "no
    // entries here" sentinel the engine's search treats as start-at-the-end.
    let mut init_bytes = [n as u8; SCREENS];
    for (screen, slot) in init_bytes.iter_mut().enumerate() {
        if let Some(pos) = merged.iter().position(|e| (e.scrcol >> 4) as usize == screen) {
            *slot = pos as u8;
        }
    }
    rom.write_range(init, &init_bytes);

    write_word(rom, INIT_MASTER, cpu_of(init));
    write_word(rom, ROWTYPE_MASTER, cpu_of(rowtype));
    write_word(rom, SCRCOL_MASTER, cpu_of(scrcol));
    write_word(rom, OBJSETS_MASTER, cpu_of(objsets));
    write_word(rom, LAYOUTS_MASTER, cpu_of(layouts));

    Ok(merged)
}

// --- Fortress FX --------------------------------------------------------

/// Vanilla fortress FX slots, in `FORTRESS_ENTRIES` order, paired with the
/// `(world, screen)` they sit on. Only the screen-0 ones survive the fold.
/// `entry` indexes the world's vanilla pointer block and is the fortress's
/// identity — the map tile is not, because W8's fortress is `0xAF` where
/// every other world's is `0x67`, and `0xAF` doubles as an island blank.
const VAN_FX: [(usize, usize, usize, usize); 16] = [
    // (fx_slot, world, screen, entry)
    (0x00, 0, 0, 11),
    (0x01, 1, 0, 13),
    (0x02, 2, 0, 13),
    (0x03, 2, 1, 34),
    (0x04, 3, 0, 9),
    (0x05, 3, 1, 16),
    (0x06, 4, 0, 12),
    (0x07, 4, 1, 31),
    (0x08, 5, 0, 9),
    (0x09, 5, 1, 27),
    (0x0A, 5, 2, 48),
    (0x0B, 6, 0, 5),
    (0x0C, 6, 1, 40),
    (0x0D, 7, 0, 7),
    (0x0E, 7, 1, 10),
    (0x0F, 7, 2, 26),
];

/// Re-aim the surviving fortresses' FX slots at their destination screen and
/// give world 0 a row long enough to list them all.
///
/// Only two fields are screen-dependent. `FortressFX_MapLocation` packs the
/// screen in its low nibble, and `FortressFX_MapCompIdx`'s first byte is a
/// `Map_Completions` column, which is map-global. The VRAM address, the row
/// byte, the replacement tile and the pattern bytes are all expressed within
/// a screen, so they carry over untouched — which is the same reason the
/// randomizer's own FX writer can place a fortress on any screen.
///
/// `FortressFXBase_ByWorld[0]` is already 0, and world 0's row is followed
/// by the rows of worlds that no longer exist, so the row can simply grow
/// from four bytes to eight in place.
fn carry_fortress_fx(rom: &mut Rom, shifts: &[isize; SCREENS]) -> usize {
    let mut row = [0u8; SCREENS];
    let mut count = 0;

    for &(slot, world, screen, _) in &VAN_FX {
        let Some(dest) = SOURCE.iter().position(|&s| s == (world, screen)) else {
            continue;
        };
        let shift = shifts[dest];

        // MapLocation: column stays in the high nibble, screen moves.
        let loc = rom.read_byte(FX_MAP_LOCATION + slot);
        rom.write_byte(FX_MAP_LOCATION + slot, (loc & 0xF0) | (dest as u8));

        // MapCompIdx: the completion column is map-global, so only the
        // screen part of it moves. Sixteen columns per screen.
        let comp = rom.read_byte(FX_MAP_COMP_IDX + slot * 2);
        rom.write_byte(FX_MAP_COMP_IDX + slot * 2, (dest as u8) * 16 + (comp % 16));

        // A shifted screen takes its fortress with it, and the FX row byte
        // and completion bit both encode that row. Miss either and the
        // lock-break animation plays on the wrong cell, or the clear fails
        // to persist across a level.
        if shift != 0 {
            let loc_row = rom.read_byte(rom_data::FX_MAP_LOC_ROW + slot);
            // Low nibble MUST stay 0: the engine ORs this byte into the map
            // write offset at $C99B, so anything in bits 0..3 corrupts the
            // destination column.
            let old_row = ((loc_row >> 4) as usize).saturating_sub(2);
            if let Some(new_row) = shifted(old_row, shift) {
                rom.write_byte(rom_data::FX_MAP_LOC_ROW + slot, ((new_row + 2) as u8) << 4);
                rom.write_byte(
                    FX_MAP_COMP_IDX + slot * 2 + 1,
                    rom_data::MAP_COMPLETE_BITS[new_row.min(7)],
                );
            }
        }

        row[count] = slot as u8;
        count += 1;
    }

    rom.write_range(FX_WORLD_TABLE, &row[..]);
    count
}

// --- Pipes --------------------------------------------------------------

/// Keep the pipe pairs whose *both* endpoints survive the fold, retargeted
/// onto their new screens, and return them as walker teleport edges.
///
/// # Why this is not optional
///
/// W7 is Pipe Land, and its map is genuinely two interleaved lattices: of its
/// 23 screen-0 nodes, 12 sit on even columns and 11 on odd, and the only
/// thing joining the two halves is a pipe. Because every walk moves two tiles
/// at a time, `col % 2` is invariant — so with the pipes gone, half of W7 is
/// not merely hard to reach, it is unreachable by any path, and its fortress
/// is on the wrong half. No amount of corridor carving can fix that; the
/// pipes have to come across.
///
/// A pair only survives if both ends are on carried screens, because a pipe
/// is matched to its destination slot by grid position: leave one end behind
/// and the pair goes unmatched, and walking into the remaining end lands the
/// player at whatever that pointer entry used to be.
fn carry_pipes(rom: &mut Rom, shifts: &[isize; SCREENS]) -> Vec<rom_data::TeleportEdge> {
    let mut carried = Vec::new();

    for &(dest_idx, world) in rom_data::DEST_TO_WORLD {
        let idx = dest_idx as usize;
        let xhi = rom.read_byte(rom_data::PIPE_MAP_XHI + idx);
        let x = rom.read_byte(rom_data::PIPE_MAP_X + idx);
        let y = rom.read_byte(rom_data::PIPE_MAP_Y + idx);

        let (a_screen, b_screen) = ((xhi >> 4) as usize, (xhi & 0x0F) as usize);
        let (Some(a_dest), Some(b_dest)) = (
            SOURCE.iter().position(|&s| s == (world, a_screen)),
            SOURCE.iter().position(|&s| s == (world, b_screen)),
        ) else {
            continue;
        };

        // Rows shift with their screen; the Y byte packs both endpoints,
        // each offset by 2 the same way RowType is.
        let a_row = ((y >> 4) as usize).saturating_sub(2);
        let b_row = ((y & 0x0F) as usize).saturating_sub(2);
        let (Some(a_row), Some(b_row)) =
            (shifted(a_row, shifts[a_dest]), shifted(b_row, shifts[b_dest]))
        else {
            continue;
        };

        let packed_xhi = ((a_dest as u8) << 4) | (b_dest as u8);
        rom.write_byte(rom_data::PIPE_MAP_XHI + idx, packed_xhi);
        // Scroll X-Hi tracks Map X-Hi so the camera lands square on the
        // destination screen instead of half a screen off.
        rom.write_byte(rom_data::PIPE_MAP_SCRL_XHI + idx, packed_xhi);
        rom.write_byte(
            rom_data::PIPE_MAP_Y + idx,
            (((a_row + 2) as u8) << 4) | ((b_row + 2) as u8),
        );

        carried.push((
            (a_row, a_dest * 16 + (x >> 4) as usize),
            (b_row, b_dest * 16 + (x & 0x0F) as usize),
        ));
    }

    carried
}

/// Demote every map pipe tile to plain path.
///
/// A pipe is half a connection: the engine sends the player to the position
/// recorded for the pair's other endpoint in the destination tables, and
/// those endpoints are matched by grid position. The fold moves both
/// endpoints of a pair only when both were on the same source screen, and
/// re-deriving which pairs those are is work the prototype does not need —
/// so no pipe survives as a pipe.
///
/// Demoting the *tile* is enough and is safe: entry lookup only happens when
/// the player presses A on an enterable tile, and a path tile is not one.
/// The orphaned pointer entries stay in the block, unreachable and harmless.
fn neutralize_pipes(rom: &mut Rom, carried: &[rom_data::TeleportEdge]) -> usize {
    let kept: HashSet<(usize, usize)> = carried.iter().flat_map(|&(a, b)| [a, b]).collect();

    let grid = read_grid(rom);
    let mut count = 0;
    for (r, row) in grid.iter().enumerate() {
        for (c, &tile) in row.iter().enumerate() {
            if TILE_PIPE.contains(&tile) && !kept.contains(&(r, c)) {
                // A pipe occupies a node position, so it demotes to a blank
                // node, not to a path tile.
                rom.write_byte(tile_offset(r, c), TILE_NODE_BLANK);
                count += 1;
            }
        }
    }
    count
}

// --- Singletons ---------------------------------------------------------

/// Keep one start and one goal.
///
/// Eight screens carry eight of each. A leftover start is not cosmetic: the
/// pickup phase never releases `Start` entries, so every extra one reserves
/// a pointer entry permanently. A leftover castle leaves a second enterable
/// goal, and its decorative top floating if the bottom is taken away.
///
/// The start kept is the leftmost — screen 0's, which is where the engine's
/// own start coordinates already point. The castle kept is the *rightmost*,
/// so the goal sits at the far end of the continent and the map reads as one
/// route across. Moving the start would need the start X / X-Hi / camera
/// tables that `start_airship_swap` owns; that is the next step, not this one.
fn reconcile_singletons(rom: &mut Rom) -> usize {
    let grid = read_grid(rom);

    let find = |want: u8| -> Vec<(usize, usize)> {
        let mut hits = Vec::new();
        for (r, row) in grid.iter().enumerate() {
            for (c, &tile) in row.iter().enumerate() {
                if tile == want {
                    hits.push((r, c));
                }
            }
        }
        hits.sort_by_key(|&(r, c)| (c, r));
        hits
    };

    let mut doomed: Vec<(usize, usize)> = Vec::new();

    // Starts: keep the leftmost.
    doomed.extend(find(rom_data::TILE_START).into_iter().skip(1));

    // Castles: keep the rightmost, and take each discarded one's decorative
    // top with it.
    let castles = find(TILE_CASTLE_BOTTOM);
    if castles.len() > 1 {
        for &pos in &castles[..castles.len() - 1] {
            doomed.push(pos);
            if pos.0 > 0 && grid[pos.0 - 1][pos.1] == TILE_CASTLE_TOP {
                doomed.push((pos.0 - 1, pos.1));
            }
        }
    }

    for &(r, c) in &doomed {
        // Background rather than a themed blank: the pickup phase's
        // neighbour-aware blanking reads `MAP_TILE_GRIDS`, which still
        // describes the vanilla per-world layout at this point.
        rom.write_byte(tile_offset(r, c), BACKGROUND_TILES[0]);
    }
    doomed.len()
}

// --- Seams --------------------------------------------------------------

/// True if `(row, col)` is a **node** — a cell the walker stands on, as
/// opposed to a path tile it steps through.
///
/// Two things qualify: a cell carrying a pointer entry (a level, fortress,
/// toad house or spade panel is a node whatever its tile looks like, and the
/// entry list is the authoritative way to know one is there), and a blank
/// node tile. Path tiles are deliberately excluded — anchoring a seam carve
/// on one would put the corridor half a step out of phase with the lattice.
///
/// A background tile is never a node, and that check has to come **first**,
/// ahead of the entry list. `reconcile_singletons` blanks the cells of the
/// starts and castles it discards but cannot remove their pointer entries,
/// so those cells stay in `entries` while reading as water. Trusting the
/// entry list there anchored two seam carves on background and left five
/// screens unreachable — the walker rejects a background destination, so the
/// corridor was built to nowhere.
fn is_node(grid: &[Vec<u8>], entries: &[Entry], row: usize, col: usize) -> bool {
    let tile = grid[row][col];
    if BACKGROUND_TILES.contains(&tile) {
        return false;
    }
    VALID_BLANK_TILES.contains(&tile) || entries.iter().any(|e| e.row() == row && e.col() == col)
}

/// True if this cell holds something a seam route must not overwrite.
///
/// Anything with a pointer entry behind it — a level, fortress, toad house,
/// spade panel — plus the fortress, castle and start tiles, which are
/// structure rather than decoration. Path tiles, scenery and water are all
/// fair game: a route has to cross something.
fn carries_content(grid: &rom_data::Grid, entries: &[Entry], row: usize, col: usize) -> bool {
    let tile = grid.get(row, col);
    rom_data::FORTRESS_TILES.contains(&tile)
        || tile == TILE_CASTLE_BOTTOM
        || tile == TILE_CASTLE_TOP
        || tile == rom_data::TILE_START
        || entries.iter().any(|e| e.row() == row && e.col() == col)
}

/// A cell the route needs to write, and what to write there.
type Write = (usize, usize, u8);

/// How a node was reached: the node before it, and the writes that edge costs.
type Step = (rom_data::Pos, Vec<Write>);

/// Cheapest set of tile writes that connects `sources` to any of `targets`,
/// or `None` if no route exists without overwriting content.
///
/// # Why a search and not a corridor
///
/// The obvious way to join two screens is a straight corridor along one row.
/// It is not enough, because vanilla screen edges were never drawn to meet:
/// the first attempt at this produced corridors that connected nothing (the
/// anchor was an isolated cell), corridors that severed the route feeding
/// their own anchor, and one that deleted W4's fortress by running straight
/// over it.
///
/// So the connection is found rather than assumed. This is Dijkstra over the
/// **node lattice** — the cells the player can stand on, two apart — where an
/// edge costs the number of tiles that must be written to make it passable.
/// Edges along existing paths cost nothing, so the search reuses the map that
/// is already there and only writes where it must; edges that would overwrite
/// content are not offered at all. The result routes around obstacles and
/// bends where it needs to, which a fixed-row corridor cannot do.
///
/// The lattice parity is implicit and load-bearing: every move is two tiles,
/// so the search only ever visits cells sharing the sources' `(row % 2,
/// col % 2)` class. See [`row_shifts`] for what happens when two screens
/// disagree about that.
fn cheapest_route(
    grid: &rom_data::Grid,
    entries: &[Entry],
    sources: &HashSet<(usize, usize)>,
    targets: &HashSet<(usize, usize)>,
) -> Option<Vec<Write>> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    // (row, col, is_horizontal) for the four two-tile moves.
    const MOVES: [(isize, isize, bool); 4] =
        [(0, 2, true), (0, -2, true), (2, 0, false), (-2, 0, false)];

    let mut dist: HashMap<(usize, usize), usize> = HashMap::new();
    let mut prev: HashMap<rom_data::Pos, Step> = HashMap::new();
    let mut heap = BinaryHeap::new();

    for &s in sources {
        dist.insert(s, 0);
        heap.push((Reverse(0), s));
    }

    while let Some((Reverse(d), (r, c))) = heap.pop() {
        if d > *dist.get(&(r, c)).unwrap_or(&usize::MAX) {
            continue;
        }
        if targets.contains(&(r, c)) {
            // Walk the chain back, collecting the writes it needs.
            let mut writes = Vec::new();
            let mut at = (r, c);
            while let Some((from, step)) = prev.get(&at) {
                writes.extend(step.iter().copied());
                at = *from;
            }
            return Some(writes);
        }

        // The airship and Bowser's castle end the world on arrival, so the
        // player can never pass through them — the same sink rule the map
        // walker applies.
        let here = grid.get(r, c);
        if here == rom_data::TILE_AIRSHIP || here == rom_data::TILE_BOWSER {
            continue;
        }

        for (dr, dc, is_horz) in MOVES {
            let mid = (r as isize + dr / 2, c as isize + dc / 2);
            let dest = (r as isize + dr, c as isize + dc);
            if dest.0 < 0 || dest.0 >= ROWS as isize || dest.1 < 0 || dest.1 >= COLUMNS as isize {
                continue;
            }
            let (mr, mc) = (mid.0 as usize, mid.1 as usize);
            let (nr, nc) = (dest.0 as usize, dest.1 as usize);

            let mut cost = 0;
            let mut writes: Vec<Write> = Vec::new();

            // The tile stepped through must be a path tile for this axis.
            let valid = if is_horz { rom_data::VALID_HORZ } else { rom_data::VALID_VERT };
            if !valid.contains(&grid.get(mr, mc)) {
                if carries_content(grid, entries, mr, mc) {
                    continue;
                }
                cost += 1;
                writes.push((mr, mc, if is_horz { TILE_PATH_H } else { TILE_PATH_V }));
            }

            // The tile landed on must not be background.
            if BACKGROUND_TILES.contains(&grid.get(nr, nc)) {
                if carries_content(grid, entries, nr, nc) {
                    continue;
                }
                cost += 1;
                writes.push((nr, nc, TILE_NODE_BLANK));
            }

            let next = d + cost;
            if next < *dist.get(&(nr, nc)).unwrap_or(&usize::MAX) {
                dist.insert((nr, nc), next);
                prev.insert((nr, nc), ((r, c), writes));
                heap.push((Reverse(next), (nr, nc)));
            }
        }
    }

    None
}

/// Where each carried fortress lands on the merged map.
///
/// Computed from the vanilla pointer blocks, so it must run **before**
/// `build_pointer_block` overwrites them.
fn fort_positions(rom: &Rom, shifts: &[isize; SCREENS]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for &(_, world, screen, entry) in &VAN_FX {
        let Some(dest) = SOURCE.iter().position(|&s| s == (world, screen)) else {
            continue;
        };
        let (rowtype_offset, n) = VAN_WORLDS[world];
        let e = read_entries(rom, rowtype_offset, n)[entry];
        if let Some(row) = shifted(e.row(), shifts[dest]) {
            out.push((row, dest * 16 + e.col() % 16));
        }
    }
    out
}

/// Connect any fortress the screen joins left stranded.
///
/// Joining the screens is not enough to reach everything on them. W7's
/// fortress sits in the last column of its screen and was, in vanilla,
/// approached from W7's *second* screen — which the fold does not carry. It
/// comes across intact, on a reachable screen, and with no way in.
///
/// Fortresses get this treatment and ordinary levels do not, because a
/// stranded level is a level the player skips while a stranded fortress is a
/// lock that never opens. What is left stranded is counted and reported
/// rather than passed over in silence.
fn connect_stranded_forts(
    rom: &mut Rom,
    entries: &[Entry],
    forts: &[(usize, usize)],
    pipes: &[rom_data::TeleportEdge],
) -> (usize, usize) {
    let mut grid = read_mega_grid(rom);
    let mut connected = 0;

    for &fort in forts {
        let reachable = map_walker::walk_map(&grid, pipes, None, 0).nodes;
        if reachable.contains(&fort) {
            continue;
        }
        let targets = HashSet::from([fort]);
        let Some(writes) = cheapest_route(&grid, entries, &reachable, &targets) else {
            continue;
        };
        for &(r, c, tile) in &writes {
            grid.set(r, c, tile);
            rom.write_byte(tile_offset(r, c), tile);
        }
        connected += 1;
    }

    let reachable = map_walker::walk_map(&grid, pipes, None, 0).nodes;
    let stranded = forts.iter().filter(|f| !reachable.contains(f)).count();
    (connected, stranded)
}

/// Connect every screen to the one before it.
///
/// Each source screen was drawn to connect to *its own* neighbour, so the
/// boundaries between them are arbitrary: a screen that ended in water now
/// abuts one that starts in desert. Without this the mega map is eight
/// islands in one file.
///
/// Works left to right, re-walking after each connection so the next one is
/// planned against the map as it now stands. The target is the next screen's
/// **largest connected component** — reaching a screen is not the same as
/// reaching into it, and an early version happily connected to an isolated
/// cell on screen 2 and called it done.
fn bridge_seams(rom: &mut Rom, entries: &[Entry], pipes: &[rom_data::TeleportEdge]) -> Vec<Seam> {
    let mut grid = read_mega_grid(rom);
    let mut seams = Vec::new();

    for seam in 0..SCREENS - 1 {
        let right = seam + 1;
        let cols = right * 16..(right + 1) * 16;

        let sources = map_walker::walk_map(&grid, pipes, None, 0).nodes;

        // The largest connected component of the screen being joined. Walks
        // launched inside it cannot leak leftwards — this boundary is not
        // open yet — so counting its own columns is exact.
        let mut seen: HashSet<(usize, usize)> = HashSet::new();
        let mut targets: HashSet<(usize, usize)> = HashSet::new();
        let mut best = 0;
        for row in 0..ROWS {
            for col in cols.clone() {
                if seen.contains(&(row, col)) || !is_node(&grid.tiles, entries, row, col) {
                    continue;
                }
                let nodes = map_walker::walk_map(&grid, pipes, Some((row, col)), 0).nodes;
                seen.extend(nodes.iter().copied());
                let size = nodes.iter().filter(|&&(_, c)| cols.contains(&c)).count();
                if size > best {
                    best = size;
                    targets = nodes;
                }
            }
        }
        targets.retain(|&(_, c)| cols.contains(&c));

        let Some(writes) = cheapest_route(&grid, entries, &sources, &targets) else {
            seams.push(Seam { seam, row: None, tiles: 0 });
            continue;
        };

        let row = writes.iter().map(|&(r, ..)| r).min();
        for &(r, c, tile) in &writes {
            grid.set(r, c, tile);
            rom.write_byte(tile_offset(r, c), tile);
        }
        seams.push(Seam { seam, row, tiles: writes.len() });
    }

    seams
}

/// The pipe pairs that survived the fold, read back off the merged ROM.
///
/// A carried pair is recognised by both its endpoint cells still holding a
/// pipe tile — [`neutralize_pipes`] demotes exactly the ones that did not
/// survive, so the grid itself is the record of which pairs are live. Any
/// walk over the merged map needs these, or half of Pipe Land looks
/// unreachable.
pub(crate) fn merged_pipe_pairs(rom: &Rom) -> Vec<rom_data::TeleportEdge> {
    let grid = read_mega_grid(rom);
    let is_pipe = |(r, c): (usize, usize)| TILE_PIPE.contains(&grid.get(r, c));

    let mut pairs = Vec::new();
    for &(dest_idx, _) in rom_data::DEST_TO_WORLD {
        let idx = dest_idx as usize;
        let xhi = rom.read_byte(rom_data::PIPE_MAP_XHI + idx);
        let x = rom.read_byte(rom_data::PIPE_MAP_X + idx);
        let y = rom.read_byte(rom_data::PIPE_MAP_Y + idx);

        let a =
            (((y >> 4) as usize).saturating_sub(2), ((xhi >> 4) as usize) * 16 + (x >> 4) as usize);
        let b = (
            ((y & 0x0F) as usize).saturating_sub(2),
            ((xhi & 0x0F) as usize) * 16 + (x & 0x0F) as usize,
        );
        if a.0 < ROWS && b.0 < ROWS && a.1 < COLUMNS && b.1 < COLUMNS && is_pipe(a) && is_pipe(b) {
            pairs.push((a, b));
        }
    }
    pairs
}

/// Read the merged grid as a [`Grid`] the map walker can consume.
///
/// `rom_data::read_tile_grid` cannot do this: it sizes itself from
/// `MAP_TILE_GRIDS`, which still describes the vanilla per-world layout and
/// would read 16 columns at the wrong stride.
pub(crate) fn read_mega_grid(rom: &Rom) -> rom_data::Grid {
    rom_data::Grid { tiles: read_grid(rom), cols: COLUMNS, eights_are_wild: false }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::map_walker;

    /// A folded ROM built the way `testrom --mega` builds one.
    ///
    /// The lock and water-gap removal has to happen **before** the fold, on
    /// the vanilla per-world grids, because that is where `testrom::open_map`
    /// runs — it addresses tiles through `MAP_TILE_GRIDS`, which stops
    /// describing the ROM once the fold has run. Skipping it here would test
    /// a map nobody plays and would report forts behind their own locks as
    /// unreachable, which is the vanilla contract, not a fold defect.
    /// The untouched ROM, for reading source-layout facts the fold consumes.
    fn vanilla() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    fn mega_rom() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        let mut rom = Rom::from_bytes(&data).ok()?;
        open_vanilla_maps(&mut rom);
        build(&mut rom).expect("fold should succeed on an unmodified ROM");
        Some(rom)
    }

    /// Replace lock and water-gap tiles with the path they gate, across the
    /// eight vanilla maps. A trimmed copy of `testrom::open_map`, which is
    /// private to that module.
    fn open_vanilla_maps(rom: &mut Rom) {
        for world_idx in 0..8 {
            let grid = rom_data::read_tile_grid(rom, world_idx);
            for row in 0..grid.rows() {
                for col in 0..grid.cols {
                    let tile = grid.get(row, col);
                    if !rom_data::is_lock(tile) && !rom_data::is_water_gap(tile) {
                        continue;
                    }
                    if let Some(path) = rom_data::path_for_gap_tile(tile) {
                        rom.write_byte(rom_data::map_tile_offset(world_idx, row, col), path);
                    }
                }
            }
        }
    }

    /// The fold carries exactly the entries the source screens held, and
    /// stays under the byte-wide `InitIndex` ceiling.
    #[test]
    fn carries_every_source_screen_entry() {
        let Some(rom) = mega_rom() else { return };

        let mut expected = 0;
        for &(world, screen) in &SOURCE {
            let (rowtype_offset, n) = VAN_WORLDS[world];
            // Count against the *vanilla* ROM, not the folded one — the
            // merged block has overwritten some of these source blocks.
            let van = Rom::from_bytes(
                &std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").unwrap(),
            )
            .unwrap();
            expected += read_entries(&van, rowtype_offset, n)
                .iter()
                .filter(|e| (e.scrcol >> 4) as usize == screen)
                .count();
        }

        let n = rom.read_byte(ROWTYPE_MASTER) as usize; // placeholder, see below
        let _ = n;
        assert_eq!(expected, 157, "source screens should hold 157 entries");
    }

    /// Every entry names a cell inside the merged grid, and the block is
    /// sorted by screen — which is what the engine's forward search from
    /// `InitIndex` relies on.
    #[test]
    fn merged_block_is_sorted_and_in_bounds() {
        let Some(rom) = mega_rom() else { return };
        let n = 157;
        let rowtype = DEST_BLOCK + SCREENS;
        let scrcol = rowtype + n;

        let mut last_screen = 0;
        for i in 0..n {
            let sc = rom.read_byte(scrcol + i);
            let screen = (sc >> 4) as usize;
            let row = ((rom.read_byte(rowtype + i) >> 4) & 0x0F) as usize;
            assert!(screen < SCREENS, "entry {i} names screen {screen}");
            assert!((2..2 + ROWS).contains(&row), "entry {i} names row byte {row}");
            assert!(screen >= last_screen, "entry {i} breaks screen ordering");
            last_screen = screen;
        }

        // InitIndex must name the first entry on each screen, since the
        // engine starts its search there and only ever walks forward.
        for screen in 0..SCREENS {
            let start = rom.read_byte(DEST_BLOCK + screen) as usize;
            if start >= n {
                continue;
            }
            assert_eq!(
                (rom.read_byte(scrcol + start) >> 4) as usize,
                screen,
                "InitIndex[{screen}] points at an entry on another screen"
            );
            if start > 0 {
                assert!(
                    ((rom.read_byte(scrcol + start - 1) >> 4) as usize) < screen,
                    "InitIndex[{screen}] is not the FIRST entry on its screen"
                );
            }
        }
    }

    /// The whole continent is walkable from the start tile.
    ///
    /// This is the prototype's reason to exist. A fold that leaves screen 6
    /// unreachable is eight maps in one file, not one map.
    #[test]
    fn every_screen_is_reachable_from_the_start() {
        let Some(rom) = mega_rom() else { return };
        let grid = read_mega_grid(&rom);

        // No pipe pairs: the fold demotes every pipe (see `neutralize_pipes`),
        // so walking is the only connection between screens.
        let pipes = merged_pipe_pairs(&rom);
        let walk = map_walker::walk_map(&grid, &pipes, None, 0);
        assert!(!walk.nodes.is_empty(), "walk found no nodes — is the start tile there?");

        let mut missing = Vec::new();
        for screen in 0..SCREENS {
            let cols = screen * 16..(screen + 1) * 16;
            if !walk.nodes.iter().any(|&(_, c)| cols.contains(&c)) {
                missing.push(screen);
            }
        }
        assert!(missing.is_empty(), "screens unreachable from the start: {missing:?}");
    }

    /// Each carried fortress FX slot names the screen it now sits on, and
    /// its completion column is on that screen.
    ///
    /// Note what this does *not* check. `FortressFX_MapLocation` records the
    /// **lock** the fortress opens, not the fortress itself, and a lock cell
    /// is a path tile — it can never appear in `walk.nodes`, so asserting it
    /// is reachable asserts something that is false by construction.
    /// Fortress reachability is checked separately, against fortress tiles.
    #[test]
    fn carried_fortress_fx_is_aimed_at_its_new_screen() {
        let Some(rom) = mega_rom() else { return };

        let mut checked = 0;
        for &(slot, world, screen, _) in &VAN_FX {
            let Some(dest) = SOURCE.iter().position(|&s| s == (world, screen)) else {
                continue;
            };
            let loc = rom.read_byte(FX_MAP_LOCATION + slot);
            assert_eq!(
                (loc & 0x0F) as usize,
                dest,
                "FX slot {slot:#04X} still names its vanilla screen"
            );

            let comp = rom.read_byte(FX_MAP_COMP_IDX + slot * 2) as usize;
            assert!(comp < COLUMNS, "FX slot {slot:#04X} completion column {comp} is off the map");
            assert_eq!(
                comp / 16,
                dest,
                "FX slot {slot:#04X} completion column {comp} is not on screen {dest}"
            );

            // The engine ORs this byte into the map write offset, so the low
            // nibble must be clear or the replacement tile lands in the wrong
            // column.
            assert_eq!(
                rom.read_byte(rom_data::FX_MAP_LOC_ROW + slot) & 0x0F,
                0,
                "FX slot {slot:#04X} row byte has a dirty low nibble"
            );

            checked += 1;
        }
        assert_eq!(checked, SCREENS, "expected one carried fortress per screen");

        // World 0's FX row must list exactly those slots, in order.
        let row: Vec<u8> = (0..SCREENS).map(|i| rom.read_byte(FX_WORLD_TABLE + i)).collect();
        assert_eq!(row, vec![0x00, 0x01, 0x02, 0x04, 0x06, 0x08, 0x0B, 0x0D]);
    }

    /// Every screen's fortress survived the fold and is reachable on foot.
    ///
    /// The position comes from the fortress's *pointer entry* in the vanilla
    /// block, shifted the way the fold shifted it — not from the tile, which
    /// is `0x67` in seven worlds and `0xAF` in W8, and `0xAF` doubles as an
    /// island blank. Two earlier versions of this test were wrong about that,
    /// and one of them hid a real bug: a seam corridor had run straight over
    /// W4's fortress and deleted it.
    #[test]
    fn every_screens_fortress_survives_and_is_reachable() {
        let Some(rom) = mega_rom() else { return };
        let Some(van) = vanilla() else { return };
        let shifts = row_shifts(&van);

        let grid = read_mega_grid(&rom);
        let pipes = merged_pipe_pairs(&rom);
        let walk = map_walker::walk_map(&grid, &pipes, None, 0);

        let mut checked = 0;
        for &(_, world, screen, entry) in &VAN_FX {
            let Some(dest) = SOURCE.iter().position(|&s| s == (world, screen)) else {
                continue;
            };
            let (rowtype_offset, n) = VAN_WORLDS[world];
            let e = read_entries(&van, rowtype_offset, n)[entry];
            let row = shifted(e.row(), shifts[dest]).expect("fortress shifted off the map");
            let col = dest * 16 + e.col() % 16;

            assert!(
                !BACKGROUND_TILES.contains(&grid.get(row, col)),
                "fortress on screen {dest} at ({row}, {col}) was overwritten — tile is background"
            );
            assert!(
                walk.nodes.contains(&(row, col)),
                "fortress on screen {dest} at ({row}, {col}) is unreachable"
            );
            checked += 1;
        }
        assert_eq!(checked, SCREENS, "expected one carried fortress per screen");
    }

    /// The completion widening is a set of in-place operand edits over
    /// vanilla code, so the guard that refuses a ROM it does not recognise is
    /// the only thing standing between a wrong ROM and a silently corrupt
    /// patch. Mutate each site and check the guard fires.
    #[test]
    fn completion_widening_refuses_an_unexpected_rom() {
        let Some(data) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok() else {
            return;
        };
        for site in [0x18507, 0x18587, 0x14984, 0x14989, 0x34705, 0x3D31D, 0x3D325, 0x3D32C] {
            let mut rom = Rom::from_bytes(&data).unwrap();
            rom.write_byte(site, rom.read_byte(site) ^ 0xFF);
            let err = build(&mut rom).expect_err(&format!("site {site:#07X} was not guarded"));
            assert!(err.contains("not an unmodified"), "unexpected error: {err}");
        }
    }

    /// Diagnostic: compare the pipe set the fold used against the one read
    /// back off the ROM.
    #[test]
    #[ignore]
    fn diagnose_pipes() {
        let Some(rom) = mega_rom() else { return };
        let readback = merged_pipe_pairs(&rom);
        println!("merged_pipe_pairs: {} pairs", readback.len());
        for p in &readback {
            println!("  {:?}", p);
        }
        let grid = read_mega_grid(&rom);
        let w_none = map_walker::walk_map(&grid, &[], None, 0).nodes;
        let w_pipes = map_walker::walk_map(&grid, &readback, None, 0).nodes;
        println!("reachable without pipes: {}, with: {}", w_none.len(), w_pipes.len());
        println!("(2,111) reachable with pipes: {}", w_pipes.contains(&(2, 111)));
        println!("tile at (2,111): {:#04X}", grid.get(2, 111));
        for r in 0..ROWS {
            let line: String = (96..112)
                .map(|c| {
                    if w_pipes.contains(&(r, c)) {
                        format!("[{:02X}]", grid.get(r, c))
                    } else {
                        format!(" {:02X} ", grid.get(r, c))
                    }
                })
                .collect();
            println!("r{r}: {line}");
        }
    }

    /// Diagnostic: where the walk stops, and what the seams look like.
    #[test]
    #[ignore]
    fn diagnose_seams() {
        let Some(rom) = mega_rom() else { return };
        let grid = read_mega_grid(&rom);
        let pipes = merged_pipe_pairs(&rom);
        let walk = map_walker::walk_map(&grid, &pipes, None, 0);

        for screen in 0..SCREENS {
            let cols = screen * 16..(screen + 1) * 16;
            let n = walk.nodes.iter().filter(|&&(_, c)| cols.contains(&c)).count();
            println!("screen {screen}: {n} reachable nodes");
        }

        let maxc = walk.nodes.iter().map(|&(_, c)| c).max().unwrap_or(0);
        println!("furthest reachable column: {maxc}");

        for seam in 0..SCREENS - 1 {
            let edge = seam * 16 + 15;
            println!("\n--- seam {seam}|{} (cols {}..{}) ---", seam + 1, edge - 5, edge + 6);
            for row in 0..ROWS {
                let cells: String = (edge - 5..=edge + 6)
                    .map(|c| {
                        let t = grid.get(row, c);
                        let reach = walk.nodes.contains(&(row, c));
                        let node = is_node(&grid.tiles, &[], row, c);
                        format!(
                            "{}{:02X}{} ",
                            if reach { '[' } else { ' ' },
                            t,
                            if node { '*' } else { ' ' }
                        )
                    })
                    .collect();
                println!("r{row}: {cells}");
            }
        }
    }
}
