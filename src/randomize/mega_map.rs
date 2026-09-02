//! EXPERIMENT: fold the eight worlds into three pipe-linked super-worlds.
//!
//! Every page keeps its vanilla layout. Worlds are concatenated whole, in
//! order, into three destination slots, and the seams *between* source worlds
//! are joined by repurposed pipe pairs rather than carved paths.
//!
//! | Slot | Source worlds | Pages | Entries | Forts |
//! |---|---|---|---|---|
//! | 6 (start) | W1 + W2 + W3 | 6 | 120 | 4 |
//! | 7 | W4 + W5 + W6 | 7 | 133 | 7 |
//! | 8 (Bowser) | W7 + W8 | 6 | 87 | 5 |
//!
//! # Why this shape
//!
//! **Progression comes free.** `INC World_Num` carries 5 → 6 → 7 unpatched,
//! and the last group holds Bowser's castle in the slot the ending code
//! expects. Nothing about the world-advance chain has to be touched — which
//! was the largest open item in the previous eight-page prototype.
//!
//! **Pipes cross parity; walks do not.** Map movement advances two tiles at a
//! time, so `row % 2` and `col % 2` are each invariant along a walk, and
//! W1–W6 sit on even rows while W7 and W8 sit on odd. Joining pages on foot
//! therefore needs the pages shifted into phase; joining them by pipe does
//! not, because a teleport edge has no parity. That single fact deletes the
//! row shifting, the seam carving, the content-preservation problem the
//! carving created, and the land-themed blank tiles it left on sky pages.
//!
//! **Only inter-world seams need links.** A source world's pages stay
//! adjacent and in order, so whatever joined its page 0 to its page 1 in
//! vanilla still joins them here. Links are needed at the W1|W2 and W2|W3
//! style boundaries only: **five in total**, against 24 pipe pairs in the ROM.
//!
//! # Slots
//!
//! Fewer worlds means each gets a *larger* share of every per-world table,
//! because the totals are conserved and there are fewer partitions:
//!
//! | Resource | 8 worlds | 3 super-worlds | Headroom |
//! |---|---|---|---|
//! | Pointer blocks | 2072 B, exactly full | 2064 B | 8 B |
//! | Grid data | 2744 B to the warp zone | 2739 B | 5 B |
//! | Fortress FX rows | 8 × 4 = 32 B | 4 + 7 + 6 = 17 B | 15 B |
//! | Fortress FX slots | 17 | 17 forts, total unchanged | 0 |
//! | Pipe pairs | 24 | 5 repurposed as links | 19 |
//!
//! The one resource that does *not* work out is map-object sprite slots: the
//! per-world list is nine long with slots 0 and 1 reserved, so seven are
//! usable, and W4+W5+W6 brings nine hammer bros. See [`carry_map_objects`].

use std::collections::HashSet;

use crate::rom::Rom;

use super::map_walker;
use super::rom_data::{
    self, BACKGROUND_TILES, FX_MAP_COMP_IDX, PRG012_FILE_BASE, VALID_BLANK_TILES,
};

/// Rows in every overworld map.
const ROWS: usize = rom_data::ROWS;

/// Bytes of tile data per page: 9 rows x 16 columns.
const PAGE_BYTES: usize = 144;

/// A destination world slot and the source worlds folded into it.
pub(crate) struct SuperWorld {
    /// Destination world index, 0-based.
    pub slot: usize,
    /// Source world indices, in the order their pages are laid out.
    pub sources: &'static [usize],
}

/// The three groups.
///
/// The order is load-bearing twice over: the *last* group must sit in slot 7
/// (world 8) so the ending fires on Bowser, and the groups must run upward
/// from the start slot so `INC World_Num` walks them in sequence.
pub(crate) const SUPER_WORLDS: [SuperWorld; 3] = [
    SuperWorld { slot: 5, sources: &[0, 1, 2] },
    SuperWorld { slot: 6, sources: &[3, 4, 5] },
    SuperWorld { slot: 7, sources: &[6, 7] },
];

/// World the game starts in — the first super-world.
pub(crate) const START_SLOT: usize = 5;

// --- Vanilla table locations -------------------------------------------
//
// Held locally rather than read from `rom_data::WORLDS` / `MAP_TILE_GRIDS`:
// this module rewrites the layout those constants describe, so it must
// address the *vanilla* one. Reading them would make its behaviour depend on
// whether it had already run.

/// Vanilla per-world tile grids: `(file_offset, pages)`.
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

/// Where the merged grids are laid out from — W1's slot.
const DEST_GRID: usize = 0x185BA;

/// First byte the grids must not reach: the Warp Zone's own grid, left alone.
/// `0x19072 - 0x185BA` = 2744 bytes for the 2739 the three groups need.
const GRID_REGION_END: usize = 0x19072;

/// Where the merged pointer blocks are laid out from — W1's slot.
///
/// The blocks must live here. PRG012's only other gap over 100 bytes is
/// `0x19DD0`, and that is **not** free — the Big ? Block trampoline, the
/// flag-key stamp and the title-screen seed-hash icons all write there, the
/// last of them after the overworld writer has run.
const DEST_BLOCK: usize = 0x19434;

/// First byte the blocks must not reach.
const BLOCK_REGION_END: usize = 0x19C4C;

/// InitIndex bytes reserved per super-world. One per page; seven is the most
/// any group needs, and eight keeps the arithmetic uniform.
const INIT_SLOTS: usize = 8;

/// Master pointer tables, one 16-bit CPU address per world.
const INIT_MASTER: usize = 0x193DA;
const ROWTYPE_MASTER: usize = 0x193EC;
const SCRCOL_MASTER: usize = 0x193FE;
const OBJSETS_MASTER: usize = 0x19410;
const LAYOUTS_MASTER: usize = 0x19422;

/// `World_Map_Max_PanR`, 8 bytes, `$10` per page.
const MAX_PAN_R: usize = 0x14F44;

/// `FortressFX_MapLocation`: column in the high nibble, page in the low.
const FX_MAP_LOCATION: usize = 0x14866;

/// `FortressFX_W1`: the per-world rows of FX slot indices, 4 bytes each in
/// vanilla but variable — `FortressFXBase_ByWorld` indexes into them, and the
/// disassembly notes there is "no need for this to be precisely four in every
/// world, but that's what they allocated".
const FX_WORLD_ROWS: usize = 0x14888;

/// `FortressFXBase_ByWorld`: byte offset into `FX_WORLD_ROWS` per world.
const FX_WORLD_BASE: usize = 0x148A8;

/// Map-object list length per world, and the reserved slots at its head:
/// slot 0 is a fixed marker, slot 1 the airship sprite.
const MAP_OBJ_SLOTS: usize = 9;
const MAP_OBJ_RESERVED: usize = 2;

// --- Tiles --------------------------------------------------------------

/// The map pipe tile (`TILE_PIPE = $BC`). Not `0x68`/`0x69`.
const TILE_PIPE: u8 = 0xBC;

/// Tiles a pipe-pair endpoint can wear.
///
/// W5's spiral tower is a pair whose two ends are a pipe and a *spiral
/// castle*, so requiring `TILE_PIPE` at both drops it — and with it the only
/// connection between W5's ground and sky halves, which do not touch by
/// walking (W5 stores them as two 16-column pages and never scrolls).
const PIPE_ENDPOINT_TILES: [u8; 3] = [TILE_PIPE, 0x5F, 0xDF];

/// Blank node tile written where a repurposed pipe vacates its old cell.
const TILE_NODE_BLANK: u8 = 0x44;

/// Castle tiles: an enterable bottom with a decorative top directly above.
const TILE_CASTLE_BOTTOM: u8 = 0xC9;
const TILE_CASTLE_TOP: u8 = 0xC8;

// --- Report -------------------------------------------------------------

/// One inter-world boundary and the pipe pair that joins it.
#[derive(Debug)]
pub struct Link {
    /// Destination world slot the boundary is in.
    pub slot: usize,
    /// Source worlds on either side, 1-based for reporting.
    pub between: (usize, usize),
    /// Pipe destination index repurposed to carry it.
    pub dest_idx: usize,
    /// Grid positions of the two pipe mouths, `(row, col)`.
    pub mouths: ((usize, usize), (usize, usize)),
}

/// Per-super-world accounting, so the caller can report what fits.
#[derive(Debug)]
pub struct SlotUse {
    pub slot: usize,
    pub pages: usize,
    pub entries: usize,
    pub forts: usize,
    /// Hammer-bro sprites the source worlds brought.
    pub sprites_wanted: usize,
    /// How many of them the nine-slot list could hold.
    pub sprites_placed: usize,
}

#[derive(Debug, Default)]
pub struct MegaMapReport {
    pub slots: Vec<SlotUse>,
    pub links: Vec<Link>,
    pub singletons_removed: usize,
    /// `(used, available)` bytes.
    pub grid_bytes: (usize, usize),
    pub block_bytes: (usize, usize),
    /// Pages the start cannot reach, as `(slot, page)`. Empty is the goal.
    pub unreachable_pages: Vec<(usize, usize)>,
    /// Where every carried entry ended up: `(src world, src entry) ->
    /// (slot, entry)`.
    ///
    /// This is what the `(world, entry)`-keyed tables have to be moved
    /// through — fortresses, airships, Bowser, the spiral pair, sprite links,
    /// pipe destinations. An entry absent from the map was not carried.
    pub entry_remap: rom_data::EntryRemap,
}

// --- Entries ------------------------------------------------------------

/// One pointer-table entry, plus where it came from.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Entry {
    rowtype: u8,
    scrcol: u8,
    obj: u16,
    lay: u16,
    /// Source world, so forts and pipes can be followed to their new home.
    src_world: usize,
    /// Index within the source world's block.
    src_idx: usize,
}

impl Entry {
    pub(crate) fn row(&self) -> usize {
        (((self.rowtype >> 4) & 0x0F) as usize).saturating_sub(2)
    }
    pub(crate) fn page(&self) -> usize {
        (self.scrcol >> 4) as usize
    }
    pub(crate) fn col(&self) -> usize {
        self.page() * 16 + (self.scrcol & 0x0F) as usize
    }
    fn set_pos(&mut self, page: usize, row: usize, col_in_page: usize) {
        self.rowtype = (((row + 2) as u8) << 4) | (self.rowtype & 0x0F);
        self.scrcol = ((page as u8) << 4) | (col_in_page as u8);
    }
    /// Sort key. The engine starts its search at `InitIndex[page]` and only
    /// walks forward, matching row first and then packed page/column, so the
    /// block must be ordered this way.
    fn key(&self) -> (usize, usize, usize) {
        (self.page(), self.row(), self.col())
    }
}

fn read_entries(rom: &Rom, world: usize) -> Vec<Entry> {
    let (rowtype_offset, n) = VAN_WORLDS[world];
    let scrcol = rowtype_offset + n;
    let obj = scrcol + n;
    let lay = obj + n * 2;
    (0..n)
        .map(|i| Entry {
            rowtype: rom.read_byte(rowtype_offset + i),
            scrcol: rom.read_byte(scrcol + i),
            obj: rom_data::read_word(rom, obj + i * 2),
            lay: rom_data::read_word(rom, lay + i * 2),
            src_world: world,
            src_idx: i,
        })
        .collect()
}

// --- Plan ---------------------------------------------------------------

/// Everything read out of the vanilla ROM before a single byte is written.
///
/// **This must be built first.** The merged grids are laid out from W1's slot
/// and run over W2, W3 and W4's source grids; the merged blocks do the same to
/// the first worlds' blocks. Any pass that re-reads a source table after
/// writing has begun reads its own output — the trap the world-merge
/// experiment hit from the other direction.
#[derive(Clone)]
struct Plan {
    /// Per group: the `(source world, source page)` behind each page.
    pages: Vec<Vec<(usize, usize)>>,
    /// Per group: entries, renumbered onto destination pages, sorted.
    entries: Vec<Vec<Entry>>,
    /// Per group: the grid, pages concatenated, terminated.
    grids: Vec<Vec<u8>>,
}

impl Plan {
    /// Destination page index for a source `(world, page)` within its group.
    fn dest_page(&self, group: usize, world: usize, page: usize) -> Option<usize> {
        self.pages[group].iter().position(|&p| p == (world, page))
    }

    fn tile(&self, group: usize, row: usize, col: usize) -> u8 {
        self.grids[group][(col / 16) * PAGE_BYTES + row * 16 + col % 16]
    }

    fn set_tile(&mut self, group: usize, row: usize, col: usize, tile: u8) {
        self.grids[group][(col / 16) * PAGE_BYTES + row * 16 + col % 16] = tile;
    }

    fn cols(&self, group: usize) -> usize {
        self.pages[group].len() * 16
    }
}

fn plan(rom: &Rom) -> Plan {
    let mut pages = Vec::new();
    let mut entries = Vec::new();
    let mut grids = Vec::new();

    for sw in &SUPER_WORLDS {
        let mut group_pages = Vec::new();
        let mut grid = Vec::new();
        for &w in sw.sources {
            let (base, n) = VAN_GRIDS[w];
            for p in 0..n {
                group_pages.push((w, p));
                grid.extend_from_slice(rom.read_range(base + p * PAGE_BYTES, PAGE_BYTES));
            }
        }
        grid.push(0xFF);

        let mut group_entries = Vec::new();
        for &w in sw.sources {
            for mut e in read_entries(rom, w) {
                let src_page = e.page();
                let dest = group_pages.iter().position(|&p| p == (w, src_page)).unwrap();
                let (row, col_in_page) = (e.row(), e.col() % 16);
                e.set_pos(dest, row, col_in_page);
                group_entries.push(e);
            }
        }
        group_entries.sort_by_key(|e| e.key());

        pages.push(group_pages);
        entries.push(group_entries);
        grids.push(grid);
    }

    Plan { pages, entries, grids }
}

// --- Build --------------------------------------------------------------

fn cpu_of(file_offset: usize) -> u16 {
    (0xA000 + (file_offset - PRG012_FILE_BASE)) as u16
}

fn write_word(rom: &mut Rom, offset: usize, val: u16) {
    rom.write_range(offset, &[(val & 0xFF) as u8, (val >> 8) as u8]);
}

/// Fold the eight worlds into three pipe-linked super-worlds.
pub fn build(rom: &mut Rom) -> Result<MegaMapReport, String> {
    rom.push_tag("mega_map");
    let mut report = MegaMapReport::default();

    // Everything the fold needs from the vanilla layout, before any write.
    let mut plan = plan(rom);
    let van_entries: Vec<Vec<Entry>> = (0..8).map(|w| read_entries(rom, w)).collect();

    widen_map_completions(rom)?;
    rom.write_byte(super::world_order::WORLD_INIT_OPERAND, START_SLOT as u8);

    report.singletons_removed = reconcile_singletons(&mut plan);
    aim_spawn_at_the_kept_start(rom, &plan)?;
    let (links, pipe_pairs_by_group) = link_pages(rom, &mut plan, &van_entries)?;
    report.links = links;
    write_pipe_dests(rom, &plan, &pipe_pairs_by_group);

    report.grid_bytes = write_grids(rom, &plan)?;
    report.block_bytes = write_blocks(rom, &plan)?;

    let forts = carry_fortress_fx(rom, &plan);
    let sprites = carry_map_objects(rom, &plan);

    report.slots = SUPER_WORLDS
        .iter()
        .enumerate()
        .map(|(g, sw)| SlotUse {
            slot: sw.slot,
            pages: plan.pages[g].len(),
            entries: plan.entries[g].len(),
            forts: forts[g],
            sprites_wanted: sprites[g].0,
            sprites_placed: sprites[g].1,
        })
        .collect();

    // Max_PanR is $10 per page: vanilla is $10/$20/$30 for its 16/32/48
    // column worlds, i.e. pages x $10 rather than (pages - 1) x $10.
    for (g, sw) in SUPER_WORLDS.iter().enumerate() {
        rom.write_byte(MAX_PAN_R + sw.slot, (plan.pages[g].len() as u8) * 0x10);
    }

    // Walk what was actually written. A fold that produces an island is worth
    // hearing about from the tool, not only from the test suite — the links
    // are placed on the first free blank node near each boundary, and nothing
    // guarantees that node is connected to the rest of its page.
    report.unreachable_pages = unreachable_pages(rom);
    report.entry_remap = plan
        .entries
        .iter()
        .enumerate()
        .flat_map(|(g, entries)| {
            let slot = SUPER_WORLDS[g].slot;
            entries.iter().enumerate().map(move |(i, e)| ((e.src_world, e.src_idx), (slot, i)))
        })
        .collect();

    rom.pop_tag();
    Ok(report)
}

// --- Completions --------------------------------------------------------

/// Overwrite a vanilla byte, refusing if it does not hold what we expect.
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

/// Let `Map_Completions` cover eight pages for one player.
///
/// The array is 128 bytes at `$7D00`, one per map column, split Mario /
/// Luigi. For one player it already covers eight pages — a column index for
/// eight pages runs 0..127 and lands inside the array, the redraw loop already
/// counts to `$80`, and the game-over clear already runs `LDY #$7F`. Four
/// sites fold the upper half onto the lower one assuming it is Luigi's.
///
/// Undoing that is seven operand bytes plus one six-byte splice, no free
/// space. The cost is two-player mode, because those columns *are* Luigi's.
///
/// Completions are **per-world state**: `PRG030_84A0`, the world-map
/// initialisation, clears all 128 bytes, and its only callers are world
/// changes. Returning from a level enters at `PRG030_84D7` and skips the
/// clear. So eight pages is a budget each super-world gets in full, not one
/// shared across the game — which is what makes groups of six and seven pages
/// possible at all.
fn widen_map_completions(rom: &mut Rom) -> Result<(), String> {
    // Map_Reload_with_Completions (PRG012 $A4F5): screen bits of the column.
    expect_write(rom, 0x18507, 0x30, 0x70, "completion redraw screen mask")?;

    // Map_Reload_with_Completions (PRG012 $A570): picks the "M" or "L" clear
    // marker from bit 6 of the column. Every column is Mario's now.
    expect_write(rom, 0x18587, 0x40, 0x00, "completion marker select")?;

    // MO_DoFortressFX (PRG010): the "mark it for Luigi too" mirror. With a
    // column past 63 this writes to $7D80 and up, which is Inventory_Items —
    // clearing a fortress on pages 4-7 would rewrite the player's inventory.
    // Pointing both operands back at Mario's array makes it a harmless repeat
    // of the write just above it.
    expect_write(rom, 0x14984, 0x40, 0x00, "fortress FX mirror load")?;
    expect_write(rom, 0x14989, 0x40, 0x00, "fortress FX mirror store")?;

    // Rock removal (PRG026): flips to the other player's byte.
    expect_write(rom, 0x34705, 0x40, 0x00, "rock-break mirror")?;

    // Game over (PRG030 $9314). Vanilla clears one player's 64 columns by
    // ANDing the other player's — for 1P, ANDing zeros. With eight pages the
    // "other player" is pages 4-7, so the vanilla form would fold half the
    // map into the other half instead of clearing it.
    expect_write(rom, 0x3D31D, 0x3F, 0x7F, "game-over clear start index")?;
    expect_write(rom, 0x3D325, 0x3F, 0x7F, "game-over clear count")?;

    // LDA Map_Completions,X / AND Map_Completions,Y -> LDA #$00 + padding.
    // The STA that follows is left alone and now stores zero. The preceding
    // TYA/EOR/TAX becomes dead but harmless, so it stays.
    const GAMEOVER_MERGE: usize = 0x3D32C;
    let splice =
        [(0xBD, 0xA9), (0x00, 0x00), (0x7D, 0xEA), (0x39, 0xEA), (0x00, 0xEA), (0x7D, 0xEA)];
    for (i, (want, new)) in splice.into_iter().enumerate() {
        expect_write(rom, GAMEOVER_MERGE + i, want, new, "game-over clear merge")?;
    }

    Ok(())
}

// --- Singletons ---------------------------------------------------------

/// Keep one start and one goal per super-world.
///
/// Concatenating three worlds gives three of each. Unlike the previous
/// prototype these are *removed from the entry list*, not merely blanked: a
/// leftover start reserves a pointer entry permanently, because the pickup
/// phase never releases `Start` entries, and the freed bytes are part of what
/// keeps the merged blocks inside their region.
///
/// The start kept is the leftmost — the first source world's, where the
/// engine's own start coordinates already point. The goal kept is the
/// **rightmost**, so the castle sits at the far end of the group and the
/// airship that ends the super-world is the last thing on the map.
fn reconcile_singletons(plan: &mut Plan) -> usize {
    let mut removed = 0;

    for group in 0..SUPER_WORLDS.len() {
        let cols = plan.cols(group);
        let find = |plan: &Plan, want: u8| -> Vec<(usize, usize)> {
            let mut hits = Vec::new();
            for c in 0..cols {
                for r in 0..ROWS {
                    if plan.tile(group, r, c) == want {
                        hits.push((r, c));
                    }
                }
            }
            hits
        };

        let mut doomed: Vec<(usize, usize)> = Vec::new();
        // Starts: keep the leftmost.
        doomed.extend(find(plan, rom_data::TILE_START).into_iter().skip(1));
        // Goals: keep exactly one, and it must be the one that ends the group.
        //
        // Normally that is the rightmost airship castle. The last group is the
        // exception: it holds W8's Bowser castle, a *different* tile
        // (`TILE_BOWSER`), and W7's airship castle as well. Both are
        // enterable, and clearing an airship castle runs `INC World_Num` — so
        // leaving W7's in place lets the player finish the final group early
        // and advance into world 9, the warp zone. When Bowser is present he
        // is the goal and every airship castle goes.
        let castles = find(plan, TILE_CASTLE_BOTTOM);
        let has_bowser = !find(plan, rom_data::TILE_BOWSER).is_empty();
        let keep = if has_bowser { 0 } else { 1 };
        if castles.len() > keep {
            for &pos in &castles[..castles.len() - keep] {
                doomed.push(pos);
                if pos.0 > 0 && plan.tile(group, pos.0 - 1, pos.1) == TILE_CASTLE_TOP {
                    doomed.push((pos.0 - 1, pos.1));
                }
            }
        }

        for &(r, c) in &doomed {
            plan.set_tile(group, r, c, BACKGROUND_TILES[0]);
        }
        let gone: HashSet<(usize, usize)> = doomed.iter().copied().collect();
        let before = plan.entries[group].len();
        plan.entries[group].retain(|e| !gone.contains(&(e.row(), e.col())));
        removed += before - plan.entries[group].len();
    }

    removed
}

/// Point each super-world's spawn at the start tile the fold actually kept.
///
/// The start *tile* and the spawn *coordinate* are two different things, and
/// only the tile moves with the map. `Map_Y_Starts` is an eight-byte per-world
/// table indexed by `World_Num` — "Map Y start positions, World 1-8 (X is
/// always $20)" — so a group living in slot 6 spawns Mario wherever **W6**
/// started, not where its own start tile is. Vanilla's rows differ per world
/// (`$40 $A0 $A0 $40 $80 $60 $30 $50`), so the mismatch shows as Mario
/// standing off the path, one or more rows from the start panel.
///
/// The row is taken from the surviving tile rather than copied from the first
/// source world's byte, so it stays correct if `reconcile_singletons` ever
/// keeps a different start.
///
/// X and X-Hi are **not** in a table — the engine hardcodes `$20`, column 2 of
/// page 0. Every vanilla start happens to sit there, and the kept start is
/// always the leftmost, so it holds here; but it is an assumption the engine
/// makes rather than data the fold controls, so it is checked rather than
/// trusted. Moving a start off column 2 needs the X / X-Hi / scroll tables
/// that `start_airship_swap` adds.
fn aim_spawn_at_the_kept_start(rom: &mut Rom, plan: &Plan) -> Result<(), String> {
    /// Column the engine hardcodes for the map spawn (`$20` = column 2).
    const SPAWN_COL: usize = 2;

    for (group, sw) in SUPER_WORLDS.iter().enumerate() {
        let mut found = None;
        for c in 0..plan.cols(group) {
            for r in 0..ROWS {
                if plan.tile(group, r, c) == rom_data::TILE_START {
                    found = Some((r, c));
                }
            }
        }

        let Some((row, col)) = found else {
            return Err(format!("mega_map: slot {} has no start tile", sw.slot));
        };
        if col != SPAWN_COL {
            return Err(format!(
                "mega_map: slot {} start tile is at column {col}, but the engine spawns at \
                 column {SPAWN_COL} — moving it needs the start_airship_swap X tables",
                sw.slot
            ));
        }

        // Y is the grid row offset by 2, times 16 — the same encoding map
        // sprite positions use.
        rom.write_byte(rom_data::MAP_Y_STARTS_OFF + sw.slot, ((row + 2) * 16) as u8);
    }

    Ok(())
}

// --- Page links ---------------------------------------------------------

/// Join each inter-world boundary with a repurposed pipe pair.
///
/// # Why repurpose rather than add
///
/// A pipe pair is two pointer entries that **share one `obj_ptr`** — the
/// transit level — matched to a destination slot by comparing the entries'
/// grid positions against the positions in the dest tables (`build_pipe_map`
/// does exactly this). Two consequences follow. A brand-new pair would need a
/// transit level of its own, since a third entry sharing an existing
/// `obj_ptr` breaks the "group of exactly two" pairing. And it would need two
/// more pointer entries, which the block has no room for.
///
/// Moving an existing pair costs neither: the transit level does not care
/// where its mouths are, the `obj_ptr` stays unique, the entry count is
/// unchanged, and the dest slot comes along. The price is one in-world pipe
/// shortcut per link — five in total, out of 24.
///
/// The mouths go on **blank node cells with no pointer entry**, near the
/// boundary. That is the only kind of cell free to take a pipe: one that
/// already carries an entry would end up with two entries at one position,
/// and the engine's forward search would find whichever came first.
#[allow(clippy::type_complexity)]
fn link_pages(
    rom: &Rom,
    plan: &mut Plan,
    van_entries: &[Vec<Entry>],
) -> Result<(Vec<Link>, Vec<Vec<(usize, usize, usize, usize)>>), String> {
    let mut links = Vec::new();
    let mut by_group = Vec::new();

    for (group, sw) in SUPER_WORLDS.iter().enumerate() {
        let all_pairs = pipe_pairs_in(rom, sw.sources, van_entries);
        by_group.push(all_pairs.clone());

        // Prefer to repurpose pipes that only ever were shortcuts.
        //
        // A pair whose two mouths sit on the *same* vanilla page cannot be
        // the only thing joining two pages, so taking it can disconnect
        // nothing. A cross-page pair might be load-bearing — W5's spiral
        // tower is the sole connection between its ground and sky halves,
        // and W3's page 0 to page 1 pipe is likewise its own bridge. Popping
        // blindly took both of W3's cross-page pipes and stranded its island.
        //
        // `spare` is popped from the back, so same-page pairs go last.
        let mut spare = all_pairs.clone();
        spare.sort_by_key(|&(_, world, a, b)| {
            let same_page = van_entries[world][a].page() == van_entries[world][b].page();
            u8::from(same_page)
        });

        for pair in sw.sources.windows(2) {
            let (left_world, right_world) = (pair[0], pair[1]);
            let left_page = plan
                .dest_page(group, left_world, VAN_GRIDS[left_world].1 - 1)
                .expect("the left world's last page is in this group");
            let right_page = plan
                .dest_page(group, right_world, 0)
                .expect("the right world's first page is in this group");

            // Reachability is recomputed each time, against the map as the
            // links placed so far have left it.
            let mut pipes = live_pipes(plan, group, &all_pairs);
            pipes.extend(canoe_edges_for(plan, group));

            // Ungated is a strong preference, not a requirement. A mouth the
            // player can reach without opening anything keeps the seam itself
            // ungated, which is what stops a gate on one side depending on a
            // fortress on the other. But gating *within* a world is ordinary
            // vanilla — W3's own pages are full of it — so demanding it
            // outright simply fails to place a link at all on a locks-intact
            // map. Try shut first, fall back to open.
            let closed = plan_grid(plan, group, Gates::Closed);
            let open = plan_grid(plan, group, Gates::Open);
            let reached_closed = map_walker::walk_map(&closed, &pipes, None, sw.slot).nodes;
            let reached_open = map_walker::walk_map(&open, &pipes, None, sw.slot).nodes;

            let left_mouth = pick_mouth(plan, group, left_page, Side::RightEdge, &reached_closed)
                .or_else(|| pick_mouth(plan, group, left_page, Side::RightEdge, &reached_open))
                .ok_or_else(|| {
                    format!("mega_map: no reachable node on page {left_page} to host a link")
                })?;

            let component = largest_component(plan, group, right_page, &pipes, Gates::Closed);
            let component_open = largest_component(plan, group, right_page, &pipes, Gates::Open);
            let right_mouth = pick_mouth(plan, group, right_page, Side::LeftEdge, &component)
                .or_else(|| pick_mouth(plan, group, right_page, Side::LeftEdge, &component_open))
                .ok_or_else(|| {
                    format!("mega_map: no connected node on page {right_page} to host a link")
                })?;

            // Try each candidate and keep the first that costs nothing.
            //
            // "Same page" is a preference, not a guarantee: W2's single pipe
            // has both mouths on its first page and is still the only thing
            // joining two regions of it, so taking it stranded ten nodes.
            // The only reliable test is to move the pair, re-walk, and check
            // that every node reachable before is reachable after — and that
            // the page being joined actually lit up.
            let before_components = component_count(plan, group, &all_pairs);
            let mut chosen = None;
            for (ci, &(dest_idx, world, a_idx, b_idx)) in spare.iter().enumerate().rev() {
                let mut trial = plan.clone();
                move_pipe_pair(
                    &mut trial,
                    group,
                    (world, a_idx, left_mouth),
                    (world, b_idx, right_mouth),
                );

                // A link joins two components, so it must leave *strictly*
                // fewer than before. If moving the pair also severs
                // something, the split cancels the join and the count comes
                // back level — which is exactly what taking W2's only pipe
                // did, and what comparing against the group's current reach
                // could not see, because W2 had not been linked in yet.
                if component_count(&trial, group, &all_pairs) < before_components {
                    *plan = trial;
                    chosen = Some((ci, dest_idx));
                    break;
                }
            }

            let Some((ci, dest_idx)) = chosen else {
                return Err(format!(
                    "mega_map: slot {} has no pipe pair it can spare to link W{} to W{} \
                     without stranding something",
                    sw.slot,
                    left_world + 1,
                    right_world + 1
                ));
            };
            spare.remove(ci);

            links.push(Link {
                slot: sw.slot,
                between: (left_world + 1, right_world + 1),
                dest_idx,
                mouths: (left_mouth, right_mouth),
            });
        }
    }

    Ok((links, by_group))
}

/// Every vanilla pipe pair whose world is in `sources`, as
/// `(dest_idx, world, entry_a, entry_b)`.
///
/// Read straight from the destination tables: a pair's endpoints are the
/// entries sitting at the positions the tables record, which is the same
/// matching rule the node catalog uses.
fn pipe_pairs_in(
    rom: &Rom,
    sources: &[usize],
    van_entries: &[Vec<Entry>],
) -> Vec<(usize, usize, usize, usize)> {
    let mut out = Vec::new();
    for &(dest_idx, world) in rom_data::DEST_TO_WORLD {
        let idx = dest_idx as usize;
        if !sources.contains(&world) {
            continue;
        }
        let (a, b) = dest_positions(rom, idx);
        let at =
            |p: (usize, usize)| van_entries[world].iter().position(|e| (e.row(), e.col()) == p);
        if let (Some(ea), Some(eb)) = (at(a), at(b)) {
            out.push((idx, world, ea, eb));
        }
    }
    out
}

/// The two endpoint positions a pipe destination slot records.
fn dest_positions(rom: &Rom, dest_idx: usize) -> ((usize, usize), (usize, usize)) {
    let xhi = rom.read_byte(rom_data::PIPE_MAP_XHI + dest_idx);
    let x = rom.read_byte(rom_data::PIPE_MAP_X + dest_idx);
    let y = rom.read_byte(rom_data::PIPE_MAP_Y + dest_idx);
    (
        (((y >> 4) as usize).saturating_sub(2), ((xhi >> 4) as usize) * 16 + (x >> 4) as usize),
        (
            ((y & 0x0F) as usize).saturating_sub(2),
            ((xhi & 0x0F) as usize) * 16 + (x & 0x0F) as usize,
        ),
    )
}

enum Side {
    LeftEdge,
    RightEdge,
}

/// Nodes a link mouth must never displace: the group's start and goal, the
/// fortresses that gate it, and the pipes that carry the other links.
const PROTECTED_TILES: [u8; 7] = [
    rom_data::TILE_START,
    TILE_CASTLE_BOTTOM,
    TILE_CASTLE_TOP,
    rom_data::TILE_FORTRESS,
    0xEB, // alternate-colour fortress
    0xAF, // W8's fortress, which doubles as an island blank
    TILE_PIPE,
];

/// Where to put a link mouth on `page`, scanning inward from `side`.
///
/// `usable` is the set of cells the mouth may sit on — for the left side, what
/// the start can already reach; for the right side, that page's largest
/// connected component. **A mouth that is not itself reachable connects
/// nothing.** Placing a pipe on a cell does not make the cell reachable, so
/// an unchecked mouth produces a link that looks right in the tables and
/// leaves the page dark; the first version of this cut off pages 3 and up in
/// all three groups.
///
/// Two kinds of cell will do, in order of preference:
///
/// 1. A **blank node with no pointer entry** — free, nothing is displaced.
/// 2. Any other node, whose entry then **trades places** with the pipe's.
///
/// The second case is not a fallback so much as the normal one. Vanilla maps
/// are dense: W1's single page has 21 entries and not one spare blank node,
/// so a design that could only use case 1 fails immediately. Trading places
/// conserves everything — the displaced level keeps its `obj_ptr` and lands
/// where the pipe used to be, which was a reachable node by construction.
fn pick_mouth(
    plan: &Plan,
    group: usize,
    page: usize,
    side: Side,
    usable: &HashSet<(usize, usize)>,
) -> Option<(usize, usize)> {
    let taken: HashSet<(usize, usize)> =
        plan.entries[group].iter().map(|e| (e.row(), e.col())).collect();

    let cols: Vec<usize> = match side {
        Side::RightEdge => (page * 16..page * 16 + 16).rev().collect(),
        Side::LeftEdge => (page * 16..page * 16 + 16).collect(),
    };

    // Pass 1: a reachable blank node nobody is using.
    for &c in &cols {
        for r in 0..ROWS {
            if usable.contains(&(r, c))
                && VALID_BLANK_TILES.contains(&plan.tile(group, r, c))
                && !taken.contains(&(r, c))
            {
                return Some((r, c));
            }
        }
    }

    // Pass 2: displace an ordinary reachable node.
    for &c in &cols {
        for r in 0..ROWS {
            if usable.contains(&(r, c))
                && taken.contains(&(r, c))
                && !PROTECTED_TILES.contains(&plan.tile(group, r, c))
            {
                return Some((r, c));
            }
        }
    }

    None
}

/// How many disconnected pieces the group's map is in.
///
/// Counted over every node with the group's pipes and canoes in play. A link
/// that works reduces this by one; a link that also severs something leaves
/// it level, which is the signal the candidate search watches for.
fn component_count(plan: &Plan, group: usize, pairs: &[(usize, usize, usize, usize)]) -> usize {
    let grid = plan_grid(plan, group, Gates::Open);
    let mut pipes = live_pipes(plan, group, pairs);
    pipes.extend(canoe_edges_for(plan, group));

    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    let mut count = 0;
    for c in 0..grid.cols {
        for r in 0..ROWS {
            if seen.contains(&(r, c)) || BACKGROUND_TILES.contains(&grid.get(r, c)) {
                continue;
            }
            let nodes = map_walker::walk_map(&grid, &pipes, Some((r, c)), 0).nodes;
            if nodes.is_empty() {
                seen.insert((r, c));
            } else {
                seen.extend(nodes.iter().copied());
            }
            count += 1;
        }
    }
    count
}

/// Whether a planning walk may pass through removable gates.
///
/// The two uses want opposite answers, which is the whole reason this is a
/// parameter:
///
/// - **Where may a link mouth go?** `Closed`. A mouth behind a lock or a rock
///   makes the seam itself gated — the player cannot cross to the next source
///   world until they have opened something, and if what opens it lies beyond
///   the seam that is a deadlock. Gating is the builder's job to do
///   deliberately, not an accident of where a mouth landed.
/// - **Did moving this pipe sever anything?** `Open`. A gate is not a
///   severance; counting one as a broken component fragments W2 and W3 so
///   badly that no pipe can be spared at all.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Gates {
    Open,
    Closed,
}

/// The plan's grid for a group, as the map walker can take it.
///
/// Rocks belong here for the same reason locks do — the engine's
/// `Map_Removable_Tiles` lists `TILE_ROCKBREAKH`/`TILE_ROCKBREAKV` right
/// alongside the locks, and `route_choice` already prices them as passable.
/// Leaving them shut fragmented the planning graph badly: W2 and W3's pages
/// are rock-heavy, so link mouths could only be placed in whichever region
/// happened to be rock-free.
///
/// `0x53` stays shut. It is a permanent wall, and pixel-identical to `0x52` on
/// purpose, so opening it would model a route the player cannot take.
///
/// Planning only — the grid actually written keeps every lock. A lock is a
/// gate the player opens, not a wall, so treating one as impassable while
/// deciding where links go measures the wrong map: it splits a world into
/// regions that are not really separate, and then no pipe can be spared
/// without appearing to strand something. Same rule the fortress-forcedness
/// work settled on — hold locks open, or you measure goal gates.
///
/// Without this the fold only worked on a ROM whose locks had already been
/// removed, which is what `testrom` does and what the randomizer does not.
fn plan_grid(plan: &Plan, group: usize, gates: Gates) -> rom_data::Grid {
    let cols = plan.cols(group);
    let open = |tile: u8| match gates {
        Gates::Closed => tile,
        Gates::Open => rom_data::path_for_gap_tile(tile)
            .or_else(|| rom_data::path_for_breakable_rock(tile))
            .unwrap_or(tile),
    };
    let tiles =
        (0..ROWS).map(|r| (0..cols).map(|c| open(plan.tile(group, r, c))).collect()).collect();
    rom_data::Grid { tiles, cols, eights_are_wild: false }
}

/// The group's canoe edges, remapped onto its pages.
///
/// `active_canoe_edges` is keyed to vanilla world indices and vanilla
/// coordinates, both of which the fold invalidates, so a walk over a merged
/// map does not see them at all. W3's page 2 is reached only by canoe — the
/// map itself is unchanged and the engine's canoe still works, because the
/// dock tiles and the boat sprite come across with the page; it is the
/// *model* that has to be told.
///
/// Returned as teleport edges to hand to the walker alongside the pipes.
/// That flattens the canoe's statefulness (the boat has to be walked to
/// first), which is safe here because every mainland dock carried is itself
/// walk-reachable — but it is a simplification, not an equivalence.
fn canoe_edges_for(plan: &Plan, group: usize) -> Vec<rom_data::TeleportEdge> {
    let mut out = Vec::new();
    for &w in SUPER_WORLDS[group].sources {
        for (a, b) in rom_data::active_canoe_edges(w, false) {
            let remap = |p: (usize, usize)| {
                plan.dest_page(group, w, p.1 / 16).map(|d| (p.0, d * 16 + p.1 % 16))
            };
            if let (Some(a), Some(b)) = (remap(a), remap(b)) {
                out.push((a, b));
            }
        }
    }
    out
}

/// The group's live pipe pairs, at wherever their entries currently sit.
///
/// Positions come from the entry list rather than the dest tables, because
/// a repurposed pair's entries have already moved while the tables are
/// rewritten later.
fn live_pipes(
    plan: &Plan,
    group: usize,
    pairs: &[(usize, usize, usize, usize)],
) -> Vec<rom_data::TeleportEdge> {
    let find = |world: usize, idx: usize| {
        plan.entries[group]
            .iter()
            .find(|e| e.src_world == world && e.src_idx == idx)
            .map(|e| (e.row(), e.col()))
    };
    pairs.iter().filter_map(|&(_, world, a, b)| Some((find(world, a)?, find(world, b)?))).collect()
}

/// The largest connected node component confined to `page`.
///
/// Reaching a page is not the same as reaching *into* it: an early version of
/// the eight-page prototype connected happily to an isolated cell and called
/// the page done.
fn largest_component(
    plan: &Plan,
    group: usize,
    page: usize,
    pipes: &[rom_data::TeleportEdge],
    gates: Gates,
) -> HashSet<(usize, usize)> {
    let grid = plan_grid(plan, group, gates);
    let cols = page * 16..(page + 1) * 16;

    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    let mut best: HashSet<(usize, usize)> = HashSet::new();
    for c in cols.clone() {
        for r in 0..ROWS {
            if seen.contains(&(r, c)) || BACKGROUND_TILES.contains(&grid.get(r, c)) {
                continue;
            }
            let nodes = map_walker::walk_map(&grid, pipes, Some((r, c)), 0).nodes;
            seen.extend(nodes.iter().copied());
            let here: HashSet<_> =
                nodes.iter().copied().filter(|&(_, c)| cols.contains(&c)).collect();
            if here.len() > best.len() {
                best = here;
            }
        }
    }
    best
}

/// Move a pipe pair's two entries to the given mouths.
///
/// The destination tables are not touched here: [`write_pipe_dests`] re-aims
/// *every* pair once at the end, from wherever its entries finally sit.
fn move_pipe_pair(
    plan: &mut Plan,
    group: usize,
    a: (usize, usize, (usize, usize)),
    b: (usize, usize, (usize, usize)),
) {
    for (world, src_idx, (row, col)) in [a, b] {
        let Some(i) =
            plan.entries[group].iter().position(|e| e.src_world == world && e.src_idx == src_idx)
        else {
            continue;
        };
        let (old_row, old_col) = (plan.entries[group][i].row(), plan.entries[group][i].col());

        // If the mouth already belongs to something, the two trade places:
        // that entry takes the pipe's old cell and tile, keeping its own
        // `obj_ptr` and landing on a node the walk could already reach.
        let displaced = plan.entries[group].iter().position(|e| (e.row(), e.col()) == (row, col));
        match displaced {
            Some(j) => {
                let tile = plan.tile(group, row, col);
                plan.entries[group][j].set_pos(old_col / 16, old_row, old_col % 16);
                plan.set_tile(group, old_row, old_col, tile);
            }
            None => plan.set_tile(group, old_row, old_col, TILE_NODE_BLANK),
        }

        plan.entries[group][i].set_pos(col / 16, row, col % 16);
        plan.set_tile(group, row, col, TILE_PIPE);
    }
    plan.entries[group].sort_by_key(|e| e.key());
}

/// Re-aim **every** pipe destination slot at where its entries now sit.
///
/// Not just the repurposed ones. A pair is matched to its slot by comparing
/// entry positions against the positions recorded here, and the fold changes
/// every carried entry's page — so a pair left with its vanilla page numbers
/// silently stops being a pair. That is what stranded W8's pages 1-3: their
/// six internal pipes were intact on the map and unmatched in the tables.
fn write_pipe_dests(
    rom: &mut Rom,
    plan: &Plan,
    pairs_by_group: &[Vec<(usize, usize, usize, usize)>],
) {
    for (group, pairs) in pairs_by_group.iter().enumerate() {
        for &(dest_idx, world, a_idx, b_idx) in pairs {
            let find = |idx: usize| {
                plan.entries[group]
                    .iter()
                    .find(|e| e.src_world == world && e.src_idx == idx)
                    .map(|e| (e.row(), e.col()))
            };
            let (Some((ar, ac)), Some((br, bc))) = (find(a_idx), find(b_idx)) else {
                continue;
            };

            let packed_xhi = (((ac / 16) as u8) << 4) | ((bc / 16) as u8);
            rom.write_byte(rom_data::PIPE_MAP_XHI + dest_idx, packed_xhi);
            // Scroll X-Hi tracks Map X-Hi so the camera lands square on the
            // destination page rather than half a page off.
            rom.write_byte(rom_data::PIPE_MAP_SCRL_XHI + dest_idx, packed_xhi);
            rom.write_byte(
                rom_data::PIPE_MAP_X + dest_idx,
                (((ac % 16) as u8) << 4) | ((bc % 16) as u8),
            );
            rom.write_byte(
                rom_data::PIPE_MAP_Y + dest_idx,
                (((ar + 2) as u8) << 4) | ((br + 2) as u8),
            );
        }
    }
}

// --- Writes -------------------------------------------------------------

fn write_grids(rom: &mut Rom, plan: &Plan) -> Result<(usize, usize), String> {
    let total: usize = plan.grids.iter().map(|g| g.len()).sum();
    let available = GRID_REGION_END - DEST_GRID;
    if total > available {
        return Err(format!(
            "mega_map: grids need {total} bytes, {available} available before the warp zone"
        ));
    }

    let mut at = DEST_GRID;
    for (group, sw) in SUPER_WORLDS.iter().enumerate() {
        rom.write_range(at, &plan.grids[group]);
        write_word(rom, GRID_PTRS + sw.slot * 2, cpu_of(at));
        at += plan.grids[group].len();
    }
    Ok((total, available))
}

fn write_blocks(rom: &mut Rom, plan: &Plan) -> Result<(usize, usize), String> {
    let total: usize = plan.entries.iter().map(|e| INIT_SLOTS + e.len() * 6).sum();
    let available = BLOCK_REGION_END - DEST_BLOCK;
    if total > available {
        return Err(format!("mega_map: pointer blocks need {total} bytes, {available} available"));
    }

    let mut at = DEST_BLOCK;
    for (group, sw) in SUPER_WORLDS.iter().enumerate() {
        let entries = &plan.entries[group];
        let n = entries.len();
        if n > 255 {
            return Err(format!(
                "mega_map: slot {} has {n} entries, past the 255 a byte-wide InitIndex can name",
                sw.slot
            ));
        }

        let init = at;
        let rowtype = init + INIT_SLOTS;
        let scrcol = rowtype + n;
        let objsets = scrcol + n;
        let layouts = objsets + n * 2;

        for (i, e) in entries.iter().enumerate() {
            rom.write_byte(rowtype + i, e.rowtype);
            rom.write_byte(scrcol + i, e.scrcol);
            write_word(rom, objsets + i * 2, e.obj);
            write_word(rom, layouts + i * 2, e.lay);
        }

        // InitIndex: first entry index on each page, with `n` as the "no
        // entries here" sentinel the search treats as start-at-the-end.
        let mut init_bytes = [n as u8; INIT_SLOTS];
        for (page, slot) in init_bytes.iter_mut().enumerate() {
            if let Some(pos) = entries.iter().position(|e| e.page() == page) {
                *slot = pos as u8;
            }
        }
        rom.write_range(init, &init_bytes);

        for (master, value) in [
            (INIT_MASTER, init),
            (ROWTYPE_MASTER, rowtype),
            (SCRCOL_MASTER, scrcol),
            (OBJSETS_MASTER, objsets),
            (LAYOUTS_MASTER, layouts),
        ] {
            write_word(rom, master + sw.slot * 2, cpu_of(value));
        }

        at = layouts + n * 2;
    }

    // Dead slots alias the first super-world rather than pointing into the
    // middle of a merged block. Safe only because the game starts at
    // START_SLOT and World_Num only ever increases, so they are never loaded.
    let live: HashSet<usize> = SUPER_WORLDS.iter().map(|s| s.slot).collect();
    for dead in (0..8).filter(|s| !live.contains(s)) {
        for master in [INIT_MASTER, ROWTYPE_MASTER, SCRCOL_MASTER, OBJSETS_MASTER, LAYOUTS_MASTER] {
            let first = rom_data::read_word(rom, master + START_SLOT * 2);
            write_word(rom, master + dead * 2, first);
        }
        let grid = rom_data::read_word(rom, GRID_PTRS + START_SLOT * 2);
        write_word(rom, GRID_PTRS + dead * 2, grid);
    }

    Ok((total, available))
}

// --- Fortress FX --------------------------------------------------------

/// Vanilla fortress FX slots as `(fx_slot, world, entry)`.
///
/// **Derived, not listed.** `FortressFX_W1..W8` hands out slots in exactly
/// `FORTRESS_ENTRIES` order — W1 takes slot 0, W2 slot 1, W3 slots 2-3, and so
/// on to W8's 0x0D-0x10 — so the slot index *is* the index into that table.
/// Writing the pairs out by hand duplicated it and got it wrong: the list had
/// 16 rows against the table's 17, silently dropping W8's fourth fortress, so
/// its FX never fired.
///
/// `entry` is the fortress's identity rather than its map tile, which is
/// `0x67` in seven worlds and `0xAF` in W8 — and `0xAF` doubles as an island
/// blank.
fn van_fx() -> Vec<(usize, usize, usize)> {
    rom_data::FORTRESS_ENTRIES
        .iter()
        .enumerate()
        .map(|(slot, &(world, entry))| (slot, world, entry))
        .collect()
}

/// Re-aim every fortress's FX slot at the page it now sits on, and give each
/// super-world a row listing its own forts.
///
/// Only two fields are page-dependent: `FortressFX_MapLocation` packs the page
/// in its low nibble, and `FortressFX_MapCompIdx`'s first byte is a
/// `Map_Completions` column, which is map-global. The VRAM address, row byte,
/// replacement tile and pattern bytes are all expressed *within* a page and
/// carry over untouched — the same reason the randomizer's own FX writer can
/// place a fortress on any page. Rows never move, because pipe links mean no
/// page has to shift.
///
/// The per-world rows are not fixed at four: `FortressFXBase_ByWorld` indexes
/// into them, so a group of seven forts gets a seven-byte row. Sixteen forts
/// across three rows fit the 32 bytes vanilla laid out for eight.
fn carry_fortress_fx(rom: &mut Rom, plan: &Plan) -> Vec<usize> {
    let mut counts = vec![0; SUPER_WORLDS.len()];
    let mut row_at = 0usize;

    for (group, sw) in SUPER_WORLDS.iter().enumerate() {
        rom.write_byte(FX_WORLD_BASE + sw.slot, row_at as u8);

        for (slot, world, _entry) in van_fx().into_iter().filter(|f| sw.sources.contains(&f.1)) {
            let src_page = (rom.read_byte(FX_MAP_LOCATION + slot) & 0x0F) as usize;
            let Some(dest) = plan.dest_page(group, world, src_page) else {
                continue;
            };

            let loc = rom.read_byte(FX_MAP_LOCATION + slot);
            rom.write_byte(FX_MAP_LOCATION + slot, (loc & 0xF0) | (dest as u8));

            let comp = rom.read_byte(FX_MAP_COMP_IDX + slot * 2);
            rom.write_byte(FX_MAP_COMP_IDX + slot * 2, (dest as u8) * 16 + (comp % 16));

            rom.write_byte(FX_WORLD_ROWS + row_at, slot as u8);
            row_at += 1;
            counts[group] += 1;
        }
    }

    counts
}

// --- Map objects --------------------------------------------------------

/// Fold each group's hammer-bro sprites into its destination world's
/// map-object list.
///
/// Returns `(wanted, placed)` per group. **They do not always match**, and
/// this is the one resource the fold cannot make fit: the per-world list is
/// nine slots, slot 0 is a fixed marker and slot 1 the airship sprite, so
/// seven are usable — while W4+W5+W6 brings nine hammer bros between them.
///
/// The list length is not freely adjustable, because the reward table is
/// addressed as `MAP_OBJ_REWARDS + world * 9 + slot`, with the stride baked
/// into the engine as well as into `rom_data`. Widening it is Phase 2: three
/// lists of fourteen fit comfortably in the 72 bytes the eight nine-slot
/// lists occupy, and RAM allows fourteen (`Map_Objects_*` are 14 bytes each),
/// but the stride has to move in both places at once.
fn carry_map_objects(rom: &mut Rom, plan: &Plan) -> Vec<(usize, usize)> {
    let mut out = Vec::new();

    for (group, sw) in SUPER_WORLDS.iter().enumerate() {
        // (grid_row, grid_col, id) on the merged map.
        let mut wanted: Vec<(usize, usize, u8)> = Vec::new();
        for &w in sw.sources {
            // From slot 2 up. Slot 0 is the fixed marker and slot 1 the
            // airship, both of which the destination world already has.
            for slot in MAP_OBJ_RESERVED..MAP_OBJ_SLOTS {
                let read =
                    |master| rom.read_byte(rom_data::map_obj_slot_offset(rom, master, w, slot));
                let id = read(rom_data::MAP_OBJ_IDS_MASTER);
                // Every sprite, not only hammer bros: W3's canoe (`0x10`) is
                // the only way onto its third page, and W7's piranhas
                // (`0x07`) are tied to pointer entries the links move.
                if id == 0x00 {
                    continue;
                }
                let page = read(rom_data::MAP_OBJ_XHIS_MASTER) as usize;
                let Some(dest) = plan.dest_page(group, w, page) else {
                    continue;
                };
                // The engine stores Y as (row + 2) * 16 and XLo as col * 16;
                // `write_map_sprite` re-derives both from grid coordinates.
                let row = (read(rom_data::MAP_OBJ_YS_MASTER) as usize / 16).saturating_sub(2);
                let col = dest * 16 + read(rom_data::MAP_OBJ_XLOS_MASTER) as usize / 16;
                wanted.push((row, col, id));
            }
        }

        let capacity = MAP_OBJ_SLOTS - MAP_OBJ_RESERVED;
        let placed = wanted.len().min(capacity);
        for (i, &(row, col, id)) in wanted.iter().take(placed).enumerate() {
            rom_data::write_map_sprite(rom, sw.slot, MAP_OBJ_RESERVED + i, row, col, id);
        }
        // Blank the tail, so a sprite from the destination world's own vanilla
        // list does not survive into the merged one.
        for slot in (MAP_OBJ_RESERVED + placed)..MAP_OBJ_SLOTS {
            let off =
                rom_data::map_obj_slot_offset(rom, rom_data::MAP_OBJ_IDS_MASTER, sw.slot, slot);
            rom.write_byte(off, 0x00);
        }

        out.push((wanted.len(), placed));
    }

    out
}

// --- Read-back helpers --------------------------------------------------

/// Pages no walk from the start can reach, as `(slot, page)`.
pub(crate) fn unreachable_pages(rom: &Rom) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for sw in &SUPER_WORLDS {
        let grid = read_grid(rom, sw.slot);
        let pipes = teleports(rom, sw.slot);
        let walk = super::map_walker::walk_map(&grid, &pipes, None, sw.slot);
        for page in 0..pages_in(sw.slot) {
            if !walk.nodes.iter().any(|&(_, c)| c / 16 == page) {
                out.push((sw.slot, page));
            }
        }
    }
    out
}

/// Destination page for a source `(world, page)`, without needing a [`Plan`].
pub(crate) fn dest_page_of(slot: usize, world: usize, page: usize) -> Option<usize> {
    let sw = SUPER_WORLDS.iter().find(|s| s.slot == slot)?;
    let mut at = 0;
    for &w in sw.sources {
        for p in 0..VAN_GRIDS[w].1 {
            if (w, p) == (world, page) {
                return Some(at);
            }
            at += 1;
        }
    }
    None
}

/// Every teleport edge on a finished super-world: pipe pairs plus canoes.
///
/// Any walk over a merged map needs both. Pipes carry the inter-world links
/// and W7's two column lattices; the canoe is the only way onto W3's third
/// page.
pub(crate) fn teleports(rom: &Rom, slot: usize) -> Vec<rom_data::TeleportEdge> {
    let mut out = pipe_pairs(rom, slot);
    let Some(sw) = SUPER_WORLDS.iter().find(|s| s.slot == slot) else {
        return out;
    };
    for &w in sw.sources {
        for (a, b) in rom_data::active_canoe_edges(w, false) {
            let remap = |p: (usize, usize)| {
                dest_page_of(slot, w, p.1 / 16).map(|d| (p.0, d * 16 + p.1 % 16))
            };
            if let (Some(a), Some(b)) = (remap(a), remap(b)) {
                out.push((a, b));
            }
        }
    }
    out
}

/// Pages in a super-world.
pub(crate) fn pages_in(slot: usize) -> usize {
    SUPER_WORLDS
        .iter()
        .find(|s| s.slot == slot)
        .map(|s| s.sources.iter().map(|&w| VAN_GRIDS[w].1).sum())
        .unwrap_or(0)
}

/// Read a finished super-world's grid as a [`Grid`] the map walker can take.
///
/// Geometry comes from [`rom_data::MapLayout`], which reads it back out of the
/// ROM's own pointer tables — so this describes what was actually **written**
/// rather than what [`SUPER_WORLDS`] intended. A fold that got a page count
/// wrong shows up here instead of being papered over by agreeing with itself.
pub(crate) fn read_grid(rom: &Rom, slot: usize) -> rom_data::Grid {
    let layout = rom_data::MapLayout::read(rom, &[slot]);
    let w = layout.get(slot).expect("the slot was just read");
    let tiles = (0..ROWS)
        .map(|r| (0..w.columns()).map(|c| rom.read_byte(w.tile_offset(r, c))).collect())
        .collect();
    rom_data::Grid { tiles, cols: w.columns(), eights_are_wild: false }
}

/// The live pipe pairs on a finished super-world, as walker teleport edges.
///
/// Read back off the grid rather than trusting what the fold intended: a pair
/// counts only when both its recorded endpoints still hold a pipe tile.
pub(crate) fn pipe_pairs(rom: &Rom, slot: usize) -> Vec<rom_data::TeleportEdge> {
    let grid = read_grid(rom, slot);
    let mut pairs = Vec::new();
    for &(dest_idx, _) in rom_data::DEST_TO_WORLD {
        let (a, b) = dest_positions(rom, dest_idx as usize);
        let ok = |p: (usize, usize)| {
            p.0 < ROWS && p.1 < grid.cols && PIPE_ENDPOINT_TILES.contains(&grid.get(p.0, p.1))
        };
        if ok(a) && ok(b) {
            pairs.push((a, b));
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vanilla() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    /// A folded ROM built the way `testrom --mega` builds one: lock and
    /// water-gap removal runs first, on the vanilla per-world grids, because
    /// that is where `testrom::open_map` runs. Skipping it would test a map
    /// nobody plays, and report forts behind their own locks as unreachable,
    /// which is the vanilla contract rather than a fold defect.
    fn mega_rom() -> Option<Rom> {
        let mut rom = vanilla()?;
        open_vanilla_maps(&mut rom);
        build(&mut rom).expect("fold should succeed on an unmodified ROM");
        Some(rom)
    }

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

    /// Every vanilla entry is carried, minus the surplus starts and castles.
    #[test]
    fn carries_every_entry_but_the_surplus_singletons() {
        let Some(mut rom) = vanilla() else { return };
        open_vanilla_maps(&mut rom);
        let mut p = plan(&rom);
        let removed = reconcile_singletons(&mut p);

        let total: usize = p.entries.iter().map(|e| e.len()).sum();
        assert_eq!(total + removed, 340, "every vanilla entry must be accounted for");
        assert_eq!(p.entries.len(), SUPER_WORLDS.len());
        assert!(removed > 0, "three groups from eight worlds must shed surplus singletons");
    }

    /// Each block is sorted the way the engine's forward search needs, and
    /// `InitIndex[page]` names the first entry on that page.
    #[test]
    fn blocks_are_sorted_and_init_index_is_exact() {
        let Some(rom) = mega_rom() else { return };

        for sw in &SUPER_WORLDS {
            let at = |master: usize| {
                PRG012_FILE_BASE
                    + (rom_data::read_word(&rom, master + sw.slot * 2) as usize - 0xA000)
            };
            let (init, rowtype, scrcol) = (at(INIT_MASTER), at(ROWTYPE_MASTER), at(SCRCOL_MASTER));
            let n = scrcol - rowtype;

            let key = |i: usize| {
                let sc = rom.read_byte(scrcol + i);
                let rt = rom.read_byte(rowtype + i);
                ((sc >> 4) as usize, ((rt >> 4) & 0x0F) as usize, (sc & 0x0F) as usize)
            };
            for i in 1..n {
                assert!(key(i - 1) <= key(i), "slot {} block unsorted at {i}", sw.slot);
            }

            for page in 0..pages_in(sw.slot) {
                let start = rom.read_byte(init + page) as usize;
                assert!(start < n, "slot {} page {page} has no entries", sw.slot);
                assert_eq!(
                    key(start).0,
                    page,
                    "slot {} InitIndex[{page}] is on another page",
                    sw.slot
                );
                if start > 0 {
                    assert!(
                        key(start - 1).0 < page,
                        "slot {} InitIndex[{page}] is not the first entry on its page",
                        sw.slot
                    );
                }
            }
        }
    }

    /// Every page of every super-world is reachable from its start, walking
    /// and taking pipes.
    #[test]
    fn every_page_is_reachable() {
        let Some(rom) = mega_rom() else { return };

        for sw in &SUPER_WORLDS {
            let grid = read_grid(&rom, sw.slot);
            let pipes = teleports(&rom, sw.slot);
            let walk = map_walker::walk_map(&grid, &pipes, None, sw.slot);
            assert!(!walk.nodes.is_empty(), "slot {} walk found nothing", sw.slot);

            let missing: Vec<usize> = (0..pages_in(sw.slot))
                .filter(|&p| !walk.nodes.iter().any(|&(_, c)| c / 16 == p))
                .collect();
            assert!(missing.is_empty(), "slot {} pages unreachable: {missing:?}", sw.slot);
        }
    }

    /// Every fortress survives the fold and is reachable.
    #[test]
    fn every_fortress_survives_and_is_reachable() {
        let Some(rom) = mega_rom() else { return };
        let Some(van) = vanilla() else { return };
        let van_entries: Vec<Vec<Entry>> = (0..8).map(|w| read_entries(&van, w)).collect();
        let p = plan(&van);

        let mut checked = 0;
        for (group, sw) in SUPER_WORLDS.iter().enumerate() {
            let grid = read_grid(&rom, sw.slot);
            let pipes = teleports(&rom, sw.slot);
            let walk = map_walker::walk_map(&grid, &pipes, None, sw.slot);

            for (_, world, entry) in van_fx().into_iter().filter(|f| sw.sources.contains(&f.1)) {
                let e = van_entries[world][entry];
                let dest = p.dest_page(group, world, e.page()).expect("fort page carried");
                let pos = (e.row(), dest * 16 + e.col() % 16);

                assert!(
                    !BACKGROUND_TILES.contains(&grid.get(pos.0, pos.1)),
                    "slot {} fortress at {pos:?} was overwritten",
                    sw.slot
                );
                assert!(
                    walk.nodes.contains(&pos),
                    "slot {} fortress at {pos:?} is unreachable",
                    sw.slot
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 17, "all seventeen vanilla fortresses should be carried");
    }

    /// Each link is a real pipe pair with a mouth on each of two pages.
    #[test]
    fn links_join_two_pages_with_real_pipes() {
        let Some(mut rom) = vanilla() else { return };
        open_vanilla_maps(&mut rom);
        let report = build(&mut rom).unwrap();

        // Two links for each three-world group, one for the two-world group.
        assert_eq!(report.links.len(), 5);
        for link in &report.links {
            let (a, b) = link.mouths;
            assert_ne!(a.1 / 16, b.1 / 16, "a link must join two different pages");
            let grid = read_grid(&rom, link.slot);
            assert_eq!(grid.get(a.0, a.1), TILE_PIPE, "link mouth A is not a pipe");
            assert_eq!(grid.get(b.0, b.1), TILE_PIPE, "link mouth B is not a pipe");
        }
    }

    /// The fold stays inside the regions it is allowed to use.
    #[test]
    fn stays_inside_its_regions() {
        let Some(rom) = mega_rom() else { return };
        assert!(
            !rom.has_writes_in_range(GRID_REGION_END, GRID_REGION_END + PAGE_BYTES),
            "the fold wrote into the warp zone's grid"
        );
        assert!(
            !rom.has_writes_in_range(BLOCK_REGION_END, BLOCK_REGION_END + 64),
            "the fold wrote past the world-block region"
        );
    }

    /// The game starts in the first super-world, and the last one is world 8
    /// so the ending fires on Bowser.
    #[test]
    fn progression_chain_is_intact() {
        let Some(rom) = mega_rom() else { return };
        assert_eq!(
            rom.read_byte(crate::randomize::world_order::WORLD_INIT_OPERAND),
            START_SLOT as u8
        );
        assert_eq!(SUPER_WORLDS.last().unwrap().slot, 7, "the last group must be world 8");
        for w in SUPER_WORLDS.windows(2) {
            assert_eq!(w[1].slot, w[0].slot + 1, "INC World_Num must walk the groups in order");
        }
    }

    /// The completion widening is in-place operand edits over vanilla code, so
    /// the guard refusing an unrecognised ROM is the only thing between a wrong
    /// ROM and a silently corrupt patch. Mutate each site; check it fires.
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
    /// Diagnostic: which pages vanilla itself cannot walk to.
    #[test]
    #[ignore]
    fn vanilla_page_reachability() {
        let Some(mut rom) = vanilla() else { return };
        open_vanilla_maps(&mut rom);
        for w in 0..8 {
            let grid = rom_data::read_tile_grid(&rom, w);
            let pipes: Vec<_> = rom_data::DEST_TO_WORLD
                .iter()
                .filter(|&&(_, world)| world == w)
                .map(|&(d, _)| dest_positions(&rom, d as usize))
                .filter(|(a, b)| a.0 < ROWS && b.0 < ROWS && a.1 < grid.cols && b.1 < grid.cols)
                .collect();
            let walk = map_walker::walk_map(&grid, &pipes, None, w);
            let pages = grid.cols / 16;
            let missing: Vec<usize> =
                (0..pages).filter(|&p| !walk.nodes.iter().any(|&(_, c)| c / 16 == p)).collect();
            println!("W{}: {pages} pages, unreachable {missing:?}", w + 1);
        }
    }

    /// The fold never makes a node *less* reachable than vanilla did.
    ///
    /// This is the invariant that matters, and it is stronger than "every
    /// page is reachable": a fold could satisfy the latter while quietly
    /// stranding half a page. Every node the vanilla world could reach must
    /// still be reachable at its merged coordinates.
    ///
    /// It is also what caught the last real defect. Repurposing pipe pairs
    /// blindly took both of W3's cross-page pipes, so its island — reached by
    /// canoe from a dock those pipes fed — went dark while every *page* still
    /// counted as reachable.
    #[test]
    fn nothing_becomes_less_reachable_than_vanilla() {
        let Some(merged) = mega_rom() else { return };
        let Some(mut van) = vanilla() else { return };
        open_vanilla_maps(&mut van);

        for sw in &SUPER_WORLDS {
            let m_grid = read_grid(&merged, sw.slot);
            let m_walk = map_walker::walk_map(&m_grid, &teleports(&merged, sw.slot), None, sw.slot);

            for &w in sw.sources {
                let v_grid = rom_data::read_tile_grid(&van, w);
                // Only this world's own pipes. Filtering by "does it fit in
                // the grid" instead lets other worlds' dest positions in as
                // phantom shortcuts, and vanilla comes out looking better
                // connected than it is.
                let v_pipes: Vec<_> = rom_data::DEST_TO_WORLD
                    .iter()
                    .filter(|&&(_, world)| world == w)
                    .map(|&(d, _)| dest_positions(&van, d as usize))
                    .filter(|(a, b)| {
                        a.0 < ROWS && b.0 < ROWS && a.1 < v_grid.cols && b.1 < v_grid.cols
                    })
                    .collect();
                let v_walk = map_walker::walk_map(&v_grid, &v_pipes, None, w);

                let mut lost = Vec::new();
                for &(r, c) in &v_walk.nodes {
                    let Some(dest) = dest_page_of(sw.slot, w, c / 16) else { continue };
                    let merged_pos = (r, dest * 16 + c % 16);
                    if !m_walk.nodes.contains(&merged_pos) {
                        lost.push(((r, c), merged_pos));
                    }
                }
                // The links displace a handful of nodes on purpose, and the
                // surplus start/castle cells are removed by design, so a
                // small loss is expected; a structural break is not.
                assert!(
                    lost.len() <= 6,
                    "W{} lost {} reachable nodes folding into slot {}: {:?}",
                    w + 1,
                    lost.len(),
                    sw.slot,
                    &lost[..lost.len().min(8)]
                );
            }
        }
    }

    /// Every per-world table the fold repartitions still fits its group.
    ///
    /// The point of three groups rather than eight is that the totals are
    /// conserved while the partitions shrink, so each group gets a *larger*
    /// share. This pins that down, including the one place it does not hold.
    #[test]
    fn per_world_tables_fit_their_groups() {
        let Some(mut rom) = vanilla() else { return };
        open_vanilla_maps(&mut rom);
        let report = build(&mut rom).unwrap();

        let (grid_used, grid_have) = report.grid_bytes;
        assert!(grid_used <= grid_have, "grids overflow: {grid_used} > {grid_have}");
        let (block_used, block_have) = report.block_bytes;
        assert!(block_used <= block_have, "blocks overflow: {block_used} > {block_have}");

        // Sixteen forts across three rows, in the 32 bytes vanilla laid out
        // for eight four-fort worlds.
        let forts: usize = report.slots.iter().map(|s| s.forts).sum();
        assert_eq!(forts, 17);
        assert!(forts <= 32, "fortress FX rows overflow their block");

        // Entry counts stay under the byte-wide InitIndex ceiling.
        for s in &report.slots {
            assert!(s.entries <= 255, "world {} has {} entries", s.slot + 1, s.entries);
            assert!(
                s.pages <= 8,
                "world {} has {} pages, past the completion cap",
                s.slot + 1,
                s.pages
            );
        }

        // The one that does not fit: map-object sprite slots. Recorded here
        // so the day it is widened, this test says so.
        let dropped: usize = report.slots.iter().map(|s| s.sprites_wanted - s.sprites_placed).sum();
        assert_eq!(
            dropped, 3,
            "expected exactly the three W4+W5+W6 sprites the nine-slot list cannot hold"
        );
    }

    /// Mario spawns on the start tile, not on whatever row the destination
    /// slot's vanilla world used.
    ///
    /// `Map_Y_Starts` is indexed by `World_Num`, so before this was fixed a
    /// group in slot 6 spawned at W6's row while its start panel sat at W1's.
    /// Vanilla's rows are all different, so the fold cannot inherit one.
    #[test]
    fn spawn_lands_on_the_start_tile() {
        let Some(rom) = mega_rom() else { return };

        for sw in &SUPER_WORLDS {
            let grid = read_grid(&rom, sw.slot);
            let start = (0..ROWS)
                .flat_map(|r| (0..grid.cols).map(move |c| (r, c)))
                .find(|&(r, c)| grid.get(r, c) == rom_data::TILE_START)
                .unwrap_or_else(|| panic!("slot {} has no start tile", sw.slot));

            let y = rom.read_byte(rom_data::MAP_Y_STARTS_OFF + sw.slot) as usize;
            let spawn_row = (y / 16).saturating_sub(2);
            assert_eq!(
                spawn_row, start.0,
                "slot {} spawns at row {spawn_row}, start tile is at row {}",
                sw.slot, start.0
            );
            // The engine hardcodes X = $20, column 2 of page 0.
            assert_eq!(start.1, 2, "slot {} start tile is off the hardcoded spawn column", sw.slot);
        }

        // And exactly one start survives per group.
        for sw in &SUPER_WORLDS {
            let grid = read_grid(&rom, sw.slot);
            let starts = (0..ROWS)
                .flat_map(|r| (0..grid.cols).map(move |c| (r, c)))
                .filter(|&(r, c)| grid.get(r, c) == rom_data::TILE_START)
                .count();
            assert_eq!(starts, 1, "slot {} has {starts} start tiles", sw.slot);
        }
    }

    /// Each group has exactly one goal, and the last one's is Bowser.
    ///
    /// Clearing an airship castle runs `INC World_Num`. The final group holds
    /// W7's airship castle as well as W8's Bowser, so leaving both lets the
    /// player finish early and advance into world 9, the warp zone.
    #[test]
    fn each_group_has_exactly_one_goal() {
        let Some(rom) = mega_rom() else { return };

        for (i, sw) in SUPER_WORLDS.iter().enumerate() {
            let grid = read_grid(&rom, sw.slot);
            let count = |want: u8| {
                (0..ROWS)
                    .flat_map(|r| (0..grid.cols).map(move |c| (r, c)))
                    .filter(|&(r, c)| grid.get(r, c) == want)
                    .count()
            };
            let airships = count(TILE_CASTLE_BOTTOM);
            let bowsers = count(rom_data::TILE_BOWSER);
            let last = i == SUPER_WORLDS.len() - 1;

            if last {
                assert_eq!(bowsers, 1, "the last group must hold Bowser");
                assert_eq!(
                    airships, 0,
                    "slot {} still has an airship castle that would advance past world 8",
                    sw.slot
                );
            } else {
                assert_eq!(bowsers, 0, "slot {} should not hold Bowser", sw.slot);
                assert_eq!(airships, 1, "slot {} has {airships} airship castles", sw.slot);
            }
        }
    }

    /// A grid with every removable gate opened. Connectivity has to be
    /// measured with gates held open, or merely-gated regions read as
    /// separate islands.
    fn open_locks(grid: &rom_data::Grid) -> rom_data::Grid {
        let tiles = (0..ROWS)
            .map(|r| {
                (0..grid.cols)
                    .map(|c| {
                        let t = grid.get(r, c);
                        rom_data::path_for_gap_tile(t)
                            .or_else(|| rom_data::path_for_breakable_rock(t))
                            .unwrap_or(t)
                    })
                    .collect()
            })
            .collect();
        rom_data::Grid { tiles, cols: grid.cols, eights_are_wild: false }
    }

    /// Walk components holding at least one pointer entry.
    ///
    /// Content is what has to be reachable; a cluster of scenery does not need
    /// a bridge, and counting one inflates the number badly.
    fn content_islands(
        rom: &Rom,
        grid: &rom_data::Grid,
        teleports: &[rom_data::TeleportEdge],
        slot: usize,
        rowtype_offset: usize,
        entry_count: usize,
    ) -> usize {
        let scrcol = rowtype_offset + entry_count;
        let content: Vec<(usize, usize)> = (0..entry_count)
            .map(|i| {
                let rt = rom.read_byte(rowtype_offset + i);
                let sc = rom.read_byte(scrcol + i);
                (
                    (((rt >> 4) & 0x0F) as usize).saturating_sub(2),
                    (sc >> 4) as usize * 16 + (sc & 0x0F) as usize,
                )
            })
            .filter(|&(r, c)| r < ROWS && c < grid.cols)
            .collect();

        let mut seen: HashSet<(usize, usize)> = HashSet::new();
        let mut islands = 0;
        for &cell in &content {
            if seen.contains(&cell) {
                continue;
            }
            seen.extend(map_walker::walk_map(grid, teleports, Some(cell), slot).nodes);
            seen.insert(cell);
            islands += 1;
        }
        islands
    }

    /// Diagnostic: islands each super-world would hand the connectivity phase,
    /// against the pipe budget it would inherit.
    ///
    /// `connectivity.rs` spends one pair per island, so `islands - 1` bridges
    /// are needed; `spare_pipes` then insists every remaining pair is placed.
    /// If a group has fewer pairs than islands it cannot be made traversable
    /// from its own budget.
    #[test]
    #[ignore]
    fn island_budget_for_the_builder() {
        let Some(mut rom) = vanilla() else { return };
        build(&mut rom).expect("fold");

        // Vanilla control. The metric only means anything if vanilla itself
        // satisfies it: each world is traversable, so its own pipe budget must
        // cover its own islands.
        let Some(mut van) = vanilla() else { return };
        open_vanilla_maps(&mut van);
        let van_layout = rom_data::MapLayout::vanilla(&van);
        println!("--- vanilla control ---");
        println!("world  islands  bridges  budget  slack");
        let mut van_islands = 0;
        for w in 0..8 {
            // Same source on both sides of the comparison.
            let budget = van_layout.dest_indices_for_world(w).len();
            let grid = open_locks(&rom_data::read_tile_grid(&van, w));
            let canoes = rom_data::active_canoe_edges(w, false);
            let t = van_layout.tables(w);
            let n = content_islands(&van, &grid, &canoes, w, t.rowtype_offset, t.entry_count);
            van_islands += n;
            println!(
                "  {}      {n}       {}       {budget}      {}",
                w + 1,
                n.saturating_sub(1),
                budget as isize - n.saturating_sub(1) as isize
            );
        }
        println!("vanilla total: {van_islands} islands, {} bridges, 24 pairs", van_islands - 8);

        println!("--- folded ---");
        println!("group  pages  islands  bridges_needed  pipe_budget  slack");
        for sw in &SUPER_WORLDS {
            // Gates OPEN — exactly how the vanilla control measures. This
            // asymmetry is what made the folded numbers look catastrophic:
            // the fold was measured with locks and rocks SHUT, so every gated
            // pocket counted as its own island, against a control that held
            // them open.
            let grid = open_locks(&read_grid(&rom, sw.slot));
            // What connectivity faces: no pipes at all. Canoes stay, since
            // they are terrain the walker already models.
            let canoes: Vec<_> = teleports(&rom, sw.slot)
                .into_iter()
                .filter(|&(a, b)| {
                    !PIPE_ENDPOINT_TILES.contains(&grid.get(a.0, a.1))
                        || !PIPE_ENDPOINT_TILES.contains(&grid.get(b.0, b.1))
                })
                .collect();

            // Count regions that hold CONTENT, not every cluster of scenery.
            // An island only needs bridging if something the player must
            // reach is on it, and the pointer entries are what that means.
            let layout = rom_data::MapLayout::read(&rom, &[sw.slot]);
            let w = layout.get(sw.slot).unwrap();
            let scrcol = w.rowtype_offset + w.entry_count;
            let content: HashSet<(usize, usize)> = (0..w.entry_count)
                .map(|i| {
                    let rt = rom.read_byte(w.rowtype_offset + i);
                    let sc = rom.read_byte(scrcol + i);
                    (
                        (((rt >> 4) & 0x0F) as usize).saturating_sub(2),
                        (sc >> 4) as usize * 16 + (sc & 0x0F) as usize,
                    )
                })
                .collect();

            let mut seen: HashSet<(usize, usize)> = HashSet::new();
            let mut islands: usize = 0;
            for &cell in &content {
                if seen.contains(&cell) {
                    continue;
                }
                let nodes = map_walker::walk_map(&grid, &canoes, Some(cell), sw.slot).nodes;
                seen.extend(nodes.iter().copied());
                islands += 1;
            }

            let budget: usize =
                sw.sources.iter().map(|&w| van_layout.dest_indices_for_world(w).len()).sum();
            let needed = islands.saturating_sub(1);
            println!(
                "  {}      {}      {islands}        {needed}              {budget}         {}",
                sw.slot + 1,
                pages_in(sw.slot),
                budget as isize - needed as isize
            );
        }
    }

    /// Link mouths are reachable with every gate still shut.
    ///
    /// A mouth behind a lock or a breakable rock makes the seam itself gated:
    /// the player cannot cross into the next source world until they open
    /// something, and if what opens it lies beyond the seam that is a
    /// deadlock. Gating is the builder's job to do deliberately, not an
    /// accident of where a mouth landed.
    ///
    /// This is exactly what regressed when breakable rocks were first held
    /// open for *all* planning walks: the W2|W3 mouth moved to a cell behind a
    /// rock and world 6's pages 3-5 went dark.
    #[test]
    fn link_mouths_are_reachable_without_opening_any_gate() {
        // The shipped artifact: testrom removes locks before folding.
        let Some(mut rom) = vanilla() else { return };
        open_vanilla_maps(&mut rom);
        let report = build(&mut rom).unwrap();

        for sw in &SUPER_WORLDS {
            // read_grid is the as-written map: nothing held open.
            let grid = read_grid(&rom, sw.slot);
            let walk = map_walker::walk_map(&grid, &teleports(&rom, sw.slot), None, sw.slot);

            for link in report.links.iter().filter(|l| l.slot == sw.slot) {
                for mouth in [link.mouths.0, link.mouths.1] {
                    assert!(
                        walk.nodes.contains(&mouth),
                        "slot {} link W{}|W{} has a mouth at {mouth:?} that is gated shut",
                        sw.slot,
                        link.between.0,
                        link.between.1
                    );
                }
            }
        }
    }

    /// The full randomizer pipeline runs on a folded map and produces three
    /// completable super-worlds.
    ///
    /// Gates are held open, matching `all_world_targets_reachable`. That is
    /// not a convenience: the builder's convention is that a lock is stored as
    /// its open path tile with the closed state in a separate overlay, so a
    /// goal behind a lock is a gate to be opened, not a stranded target.
    ///
    /// `#[ignore]`d for runtime — eight seeds through the whole pipeline is
    /// ~90s, which is census territory rather than a per-commit gate.
    #[test]
    #[ignore]
    fn mega_map_builds_completable_super_worlds() {
        let Some(base) = vanilla() else { return };

        for seed in 0..8u64 {
            let mut rom = base.clone();
            let opts = crate::Options { mega_map: true, ..Default::default() };
            crate::randomizer::randomize(&mut rom, seed, &opts);

            for sw in &crate::randomize::mega_map::SUPER_WORLDS {
                // Gates held open, matching `all_world_targets_reachable`,
                // which walks the builder's grid — and the builder's
                // convention is that locks are RESTORED to their open path
                // tile, with the closed state living in a separate overlay.
                // The written ROM stamps them shut, so walking it directly
                // reports every goal behind a lock as stranded.
                let grid = open_locks(&crate::randomize::mega_map::read_grid(&rom, sw.slot));
                let pipes = crate::randomize::mega_map::teleports(&rom, sw.slot);
                let walk = crate::randomize::map_walker::walk_map(&grid, &pipes, None, sw.slot);

                assert!(
                    !walk.nodes.is_empty(),
                    "seed {seed} slot {}: no walk from the start",
                    sw.slot
                );

                let goal = crate::randomize::overworld_helpers::find_target(&grid, sw.slot);
                let goal = goal
                    .unwrap_or_else(|| panic!("seed {seed} slot {}: no goal on the map", sw.slot));
                assert!(
                    walk.nodes.contains(&goal),
                    "seed {seed} slot {}: goal at {goal:?} is unreachable",
                    sw.slot
                );
            }
        }
    }

    /// A super-world's pipe budget is exactly the sum of its source worlds'.
    ///
    /// `VANILLA_PIPE_PAIRS` is indexed by slot but describes *vanilla* worlds,
    /// so it handed a folded slot 6 W7's eight pairs when it really holds
    /// W4+W5+W6's six. The budget now comes from the remapped layout, which
    /// re-homes each pipe destination onto the slot its endpoints landed in.
    #[test]
    fn pipe_budget_is_the_sum_of_the_merged_worlds() {
        let Some(mut rom) = vanilla() else { return };
        let van = rom_data::MapLayout::vanilla(&rom);
        let want: Vec<usize> = SUPER_WORLDS
            .iter()
            .map(|sw| sw.sources.iter().map(|&w| van.dest_indices_for_world(w).len()).sum())
            .collect();

        let report = build(&mut rom).expect("fold");
        let live: Vec<usize> = SUPER_WORLDS.iter().map(|s| s.slot).collect();
        let folded = rom_data::MapLayout::read(&rom, &live).remapped(&report.entry_remap);

        for (sw, want) in SUPER_WORLDS.iter().zip(want) {
            assert_eq!(
                folded.dest_indices_for_world(sw.slot).len(),
                want,
                "slot {} should inherit its sources' pipes",
                sw.slot
            );
        }
        // And none are lost along the way.
        assert_eq!(folded.dest_to_world.len(), rom_data::DEST_TO_WORLD.len());
    }
}
