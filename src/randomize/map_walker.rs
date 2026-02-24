/// Shared map walker for overworld connectivity analysis.
///
/// BFS-based walker that traverses SMB3 overworld maps using the game's
/// 2-tile movement model (node → path tile → node). Supports pipe teleport
/// edges, chokepoint detection, and fortress progression simulation.
///
/// Used by `pipes.rs` for pipe shuffle and will be used by future
/// lock/bridge shuffle.


use std::collections::{HashMap, HashSet, VecDeque};

use crate::rom::Rom;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Valid horizontal path tiles (Map_Object_Valid_Left/Right in PRG010).
pub(super) const VALID_HORZ: &[u8] = &[0x45, 0xB2, 0xB3, 0xAC, 0xB7, 0xB8, 0xDA, 0xB9, 0xE6];

/// Valid vertical path tiles (Map_Object_Valid_Down/Up in PRG010).
pub(super) const VALID_VERT: &[u8] = &[0x46, 0xB1, 0xAA, 0xAB, 0xB0, 0xDB, 0xBA];

/// Background / non-walkable tiles.
pub(super) const BACKGROUND_TILES: &[u8] = &[0xB4, 0xFF, 0x02];

/// Start tile ID.
pub(super) const TILE_START: u8 = 0xE5;

/// Pipe tile ID.
pub(super) const TILE_PIPE: u8 = 0xBC;

/// W5 Spiral Tower tile ID (functionally a pipe connecting screen 0 ↔ screen 1).
pub(super) const TILE_SPIRAL: u8 = 0x5F;

/// Number of rows in every overworld map.
pub(super) const ROWS: usize = 9;

/// Movement directions: (delta_row, delta_col, is_horizontal).
const DIRECTIONS: [(i8, i8, bool); 4] = [
    (0, 1, true),   // right
    (0, -1, true),  // left
    (1, 0, false),  // down
    (-1, 0, false), // up
];

// Pipe destination tables (PRG002)
pub(super) const PIPE_MAP_XHI: usize = 0x046AA;
pub(super) const PIPE_MAP_X: usize = 0x046C2;
pub(super) const PIPE_MAP_Y: usize = 0x046DA;
pub(super) const PIPE_MAP_SCRL_XHI: usize = 0x046F2;

/// Destination byte → world index (0-based). Only paired pipe destinations.
const DEST_TO_WORLD: &[(u8, usize)] = &[
    (0x00, 4),  // W5 (spiral tower)
    (0x01, 1),  // W2
    (0x02, 5), (0x03, 5),  // W6
    (0x04, 6), (0x05, 6), (0x06, 6), (0x07, 6),  // W7
    (0x08, 6), (0x09, 6), (0x0A, 6), (0x0B, 6),  // W7
    (0x0C, 7), (0x0D, 7), (0x0E, 7), (0x0F, 7), (0x10, 7), (0x11, 7),  // W8
    (0x12, 2), (0x13, 2), (0x14, 2),  // W3
    (0x15, 3), (0x16, 3),  // W4
    (0x17, 4),  // W5
];

/// Per-world map tile grid info.
pub(super) struct MapGridInfo {
    pub file_offset: usize,
    pub columns: usize,
    pub screens: usize,
}

pub(super) const MAP_TILE_GRIDS: [MapGridInfo; 8] = [
    MapGridInfo { file_offset: 0x185BA, columns: 16, screens: 1 },  // W1
    MapGridInfo { file_offset: 0x1864B, columns: 32, screens: 2 },  // W2
    MapGridInfo { file_offset: 0x1876C, columns: 48, screens: 3 },  // W3
    MapGridInfo { file_offset: 0x1891D, columns: 32, screens: 2 },  // W4
    MapGridInfo { file_offset: 0x18A3E, columns: 32, screens: 2 },  // W5
    MapGridInfo { file_offset: 0x18B5F, columns: 48, screens: 3 },  // W6
    MapGridInfo { file_offset: 0x18D10, columns: 32, screens: 2 },  // W7
    MapGridInfo { file_offset: 0x18E31, columns: 64, screens: 4 },  // W8
];

/// Pointer table locations per world.
pub(super) struct WorldTables {
    pub rowtype_offset: usize,
    pub entry_count: usize,
}

pub(super) const WORLDS: [WorldTables; 8] = [
    WorldTables { rowtype_offset: 0x19438, entry_count: 21 },
    WorldTables { rowtype_offset: 0x194BA, entry_count: 47 },
    WorldTables { rowtype_offset: 0x195D8, entry_count: 52 },
    WorldTables { rowtype_offset: 0x19714, entry_count: 34 },
    WorldTables { rowtype_offset: 0x197E4, entry_count: 42 },
    WorldTables { rowtype_offset: 0x198E4, entry_count: 57 },
    WorldTables { rowtype_offset: 0x19A3E, entry_count: 46 },
    WorldTables { rowtype_offset: 0x19B56, entry_count: 41 },
];

/// Known fortress entries (world_idx, entry_idx).
pub(super) const FORTRESS_ENTRIES: &[(usize, usize)] = &[
    (0, 11),
    (1, 13),
    (2, 13), (2, 34),
    (3, 9), (3, 16),
    (4, 12), (4, 31),
    (5, 9), (5, 27), (5, 48),
    (6, 5), (6, 40),
    (7, 7), (7, 10), (7, 26), (7, 36),
];

/// Known airship entries (world_idx, entry_idx).
pub(super) const AIRSHIP_ENTRIES: &[(usize, usize)] = &[
    (0, 17), (1, 36), (2, 49), (3, 6), (4, 35), (5, 53), (6, 43),
];

/// Bowser's castle entry.
pub(super) const BOWSER_ENTRY: (usize, usize) = (7, 40);

/// Map transition entries.
pub(super) const MAP_TRANSITIONS: &[(usize, usize)] = &[(4, 5)];

// FX table offsets (17 slots)
pub(super) const FX_MAP_LOC_ROW: usize = 0x14855;
pub(super) const FX_MAP_LOC: usize = 0x14866;
pub(super) const FX_MAP_TILE_REPLACE: usize = 0x14877;
pub(super) const FX_WORLD_TABLE: usize = 0x14888;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Mutable overworld tile grid.
pub(super) struct Grid {
    pub tiles: Vec<Vec<u8>>,
    pub rows: usize,
    pub cols: usize,
}

impl Grid {
    pub fn get(&self, row: usize, col: usize) -> u8 {
        self.tiles[row][col]
    }

    pub fn set(&mut self, row: usize, col: usize, tile: u8) {
        self.tiles[row][col] = tile;
    }

    /// Deep copy of the grid for testing lock placement scenarios.
    pub fn clone_grid(&self) -> Grid {
        Grid {
            tiles: self.tiles.clone(),
            rows: self.rows,
            cols: self.cols,
        }
    }
}

/// An edge in the walk graph.
pub(super) struct Edge {
    pub dest: (usize, usize),
    /// Path tile position (None for pipe teleport edges).
    pub path_pos: Option<(usize, usize)>,
    /// Path tile ID (0 for pipe edges).
    pub path_tile: u8,
}

/// Result of a BFS map walk.
pub(super) struct WalkResult {
    pub nodes: HashSet<(usize, usize)>,
    pub edges: HashMap<(usize, usize), Vec<Edge>>,
    pub path_tiles: HashSet<(usize, usize)>,
}

/// An FX slot (lock/bridge position and replacement tile).
pub(super) struct FxSlot {
    pub grid_row: usize,
    pub grid_col: usize,
    pub replace_tile: u8,
}

/// A step in fortress progression simulation.
#[allow(dead_code)]
pub(super) struct ProgressionStep {
    pub fort_idx: Option<usize>,
    pub fort_pos: Option<(usize, usize)>,
    pub fx_pos: Option<(usize, usize)>,
    pub fx_old_tile: Option<u8>,
    pub fx_new_tile: Option<u8>,
    pub nodes: HashSet<(usize, usize)>,
}

// ---------------------------------------------------------------------------
// ROM helpers
// ---------------------------------------------------------------------------

/// Read a 16-bit little-endian word from ROM.
pub(super) fn read_word(rom: &Rom, offset: usize) -> u16 {
    let lo = rom.read_byte(offset) as u16;
    let hi = rom.read_byte(offset + 1) as u16;
    (hi << 8) | lo
}

/// Compute sub-table file offsets for a world's pointer tables.
/// Returns (scrcol_offset, objsets_offset, layouts_offset).
pub(super) fn table_offsets(world: &WorldTables) -> (usize, usize, usize) {
    let n = world.entry_count;
    let scrcol = world.rowtype_offset + n;
    let objsets = scrcol + n;
    let layouts = objsets + n * 2;
    (scrcol, objsets, layouts)
}

/// Get the (grid_row, grid_col) for a pointer table entry.
pub(super) fn entry_grid_position(rom: &Rom, world: &WorldTables, idx: usize) -> (usize, usize) {
    let row_nibble = (rom.read_byte(world.rowtype_offset + idx) >> 4) & 0x0F;
    let scrcol = rom.read_byte(world.rowtype_offset + world.entry_count + idx);
    let screen = (scrcol >> 4) & 0x0F;
    let column = scrcol & 0x0F;
    let grid_row = (row_nibble as usize).wrapping_sub(2);
    let grid_col = screen as usize * 16 + column as usize;
    (grid_row, grid_col)
}

/// Compute the ROM file offset of a map tile at (row, col).
pub(super) fn map_tile_offset(world_idx: usize, row: usize, col: usize) -> usize {
    let info = &MAP_TILE_GRIDS[world_idx];
    let screen = col / 16;
    let col_in_screen = col % 16;
    info.file_offset + screen * 144 + row * 16 + col_in_screen
}

// ---------------------------------------------------------------------------
// Level entry helpers
// ---------------------------------------------------------------------------

/// PRG bank loaded at CPU $A000-$BFFF for each tileset (0-18).
pub(super) const PAGE_A000_BY_TILESET: [usize; 19] = [
    11, 15, 21, 16, 17, 19, 18, 18, 18, 20, 23, 19, 17, 19, 13, 26, 26, 26, 9,
];

/// Data that travels with a level when shuffled.
#[derive(Clone, Debug)]
pub(super) struct LevelEntry {
    pub tileset: u8,
    pub obj_lo: u8,
    pub obj_hi: u8,
    pub lay_lo: u8,
    pub lay_hi: u8,
}

/// Returns true if this map entry has a real level pointer (not a toad house,
/// bonus game, hand trap, or pipe junction).
pub(super) fn is_level_pointer(obj_ptr: u16, lay_ptr: u16) -> bool {
    obj_ptr >= 0xC000 && lay_ptr != 0x0000
}

/// Convert a layout CPU address ($A000-$BFFF) + tileset to a ROM file offset.
pub(super) fn layout_file_offset(cpu_addr: u16, tileset: u8) -> Option<usize> {
    if tileset as usize >= PAGE_A000_BY_TILESET.len() || cpu_addr < 0xA000 {
        return None;
    }
    let bank = PAGE_A000_BY_TILESET[tileset as usize];
    Some(bank * 0x2000 + 0x10 + (cpu_addr as usize - 0xA000))
}

/// Read the screen count from a level's 9-byte header.
/// Header byte 4, bits 3-0 = (num_screens - 1).
pub(super) fn level_screen_count(rom: &Rom, layout_offset: usize) -> u8 {
    (rom.read_byte(layout_offset + 4) & 0x0F) + 1
}

/// Read a LevelEntry from ROM for a given world and entry index.
pub(super) fn read_entry(rom: &Rom, world: &WorldTables, idx: usize) -> LevelEntry {
    let (_scrcol, objsets, layouts) = table_offsets(world);
    let obj_off = objsets + idx * 2;
    let lay_off = layouts + idx * 2;

    LevelEntry {
        tileset: rom.read_byte(world.rowtype_offset + idx) & 0x0F,
        obj_lo: rom.read_byte(obj_off),
        obj_hi: rom.read_byte(obj_off + 1),
        lay_lo: rom.read_byte(lay_off),
        lay_hi: rom.read_byte(lay_off + 1),
    }
}

/// Write a LevelEntry back to ROM for a given world and entry index.
/// Only the tileset (lower nibble of ByRowType) is updated — the upper
/// nibble (map row position) is preserved.
pub(super) fn write_entry(rom: &mut Rom, world: &WorldTables, idx: usize, entry: &LevelEntry) {
    let (_scrcol, objsets, layouts) = table_offsets(world);
    let obj_off = objsets + idx * 2;
    let lay_off = layouts + idx * 2;

    let old_brt = rom.read_byte(world.rowtype_offset + idx);
    let new_brt = (old_brt & 0xF0) | (entry.tileset & 0x0F);
    rom.write_byte(world.rowtype_offset + idx, new_brt);

    rom.write_byte(obj_off, entry.obj_lo);
    rom.write_byte(obj_off + 1, entry.obj_hi);
    rom.write_byte(lay_off, entry.lay_lo);
    rom.write_byte(lay_off + 1, entry.lay_hi);
}

// ---------------------------------------------------------------------------
// Grid reading
// ---------------------------------------------------------------------------

/// Read a world's tile grid from ROM as a mutable Grid.
pub(super) fn read_tile_grid(rom: &Rom, world_idx: usize) -> Grid {
    let info = &MAP_TILE_GRIDS[world_idx];
    let cols = info.columns;

    let mut tiles = Vec::with_capacity(ROWS);
    for r in 0..ROWS {
        let mut row = Vec::with_capacity(cols);
        for c in 0..cols {
            let screen = c / 16;
            let col_in_screen = c % 16;
            let offset = info.file_offset + screen * 144 + r * 16 + col_in_screen;
            row.push(rom.read_byte(offset));
        }
        tiles.push(row);
    }

    Grid { tiles, rows: ROWS, cols }
}

/// Find the START tile position in a grid.
pub(super) fn find_start(grid: &Grid) -> Option<(usize, usize)> {
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            if grid.get(r, c) == TILE_START {
                return Some((r, c));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Pipe data reading
// ---------------------------------------------------------------------------

/// Get destination table indices that belong to a given world.
pub(super) fn dest_indices_for_world(world_idx: usize) -> Vec<usize> {
    DEST_TO_WORLD
        .iter()
        .filter(|&&(_, w)| w == world_idx)
        .map(|&(d, _)| d as usize)
        .collect()
}

/// Read all pipe pairs from ROM destination tables, grouped by world.
/// Returns a map: world_idx → Vec of ((row_a, col_a), (row_b, col_b)).
pub(super) fn read_pipe_pairs(rom: &Rom) -> HashMap<usize, Vec<((usize, usize), (usize, usize))>> {
    let mut pipes_by_world: HashMap<usize, Vec<_>> = HashMap::new();

    for &(dest, world_idx) in DEST_TO_WORLD {
        let d = dest as usize;
        let xhi = rom.read_byte(PIPE_MAP_XHI + d);
        let x = rom.read_byte(PIPE_MAP_X + d);
        let y = rom.read_byte(PIPE_MAP_Y + d);

        let a_scr = ((xhi >> 4) & 0x0F) as usize;
        let b_scr = (xhi & 0x0F) as usize;
        let a_col = ((x >> 4) & 0x0F) as usize;
        let b_col = (x & 0x0F) as usize;
        let a_row_nib = ((y >> 4) & 0x0F) as usize;
        let b_row_nib = (y & 0x0F) as usize;

        let a_pos = (a_row_nib.wrapping_sub(2), a_scr * 16 + a_col);
        let b_pos = (b_row_nib.wrapping_sub(2), b_scr * 16 + b_col);

        pipes_by_world.entry(world_idx).or_default().push((a_pos, b_pos));
    }

    pipes_by_world
}

// ---------------------------------------------------------------------------
// BFS map walker
// ---------------------------------------------------------------------------

/// BFS walk from a start position, returning reachable nodes, edges, and path tiles.
///
/// Movement model: player moves 2 tiles at a time. The intermediate tile must
/// be a valid path tile for the movement direction. Pipes create bidirectional
/// teleport edges.
pub(super) fn walk_map(
    grid: &Grid,
    pipe_pairs: &[((usize, usize), (usize, usize))],
    start_pos: Option<(usize, usize)>,
) -> WalkResult {
    let start = match start_pos.or_else(|| find_start(grid)) {
        Some(s) => s,
        None => {
            return WalkResult {
                nodes: HashSet::new(),
                edges: HashMap::new(),
                path_tiles: HashSet::new(),
            };
        }
    };

    // Build pipe lookup: position → list of destinations
    let mut pipe_lookup: HashMap<(usize, usize), Vec<(usize, usize)>> = HashMap::new();
    for &(a, b) in pipe_pairs {
        pipe_lookup.entry(a).or_default().push(b);
        pipe_lookup.entry(b).or_default().push(a);
    }

    let mut nodes = HashSet::new();
    let mut edges: HashMap<(usize, usize), Vec<Edge>> = HashMap::new();
    let mut path_tiles = HashSet::new();
    let mut queue = VecDeque::new();

    nodes.insert(start);
    queue.push_back(start);

    while let Some((r, c)) = queue.pop_front() {
        edges.entry((r, c)).or_default();

        // Orthogonal movement: node → path tile → node (2 tiles)
        for &(dr, dc, is_horz) in &DIRECTIONS {
            let pr = r as i16 + dr as i16;
            let pc = c as i16 + dc as i16;
            if pr < 0 || pr >= grid.rows as i16 || pc < 0 || pc >= grid.cols as i16 {
                continue;
            }
            let (pr, pc) = (pr as usize, pc as usize);

            let path_tile = grid.get(pr, pc);
            let valid = if is_horz { VALID_HORZ } else { VALID_VERT };
            if !valid.contains(&path_tile) {
                continue;
            }

            let nr = r as i16 + 2 * dr as i16;
            let nc = c as i16 + 2 * dc as i16;
            if nr < 0 || nr >= grid.rows as i16 || nc < 0 || nc >= grid.cols as i16 {
                continue;
            }
            let (nr, nc) = (nr as usize, nc as usize);

            let dest_tile = grid.get(nr, nc);
            if BACKGROUND_TILES.contains(&dest_tile) {
                continue;
            }

            path_tiles.insert((pr, pc));
            edges.entry((r, c)).or_default().push(Edge {
                dest: (nr, nc),
                path_pos: Some((pr, pc)),
                path_tile,
            });

            if !nodes.contains(&(nr, nc)) {
                nodes.insert((nr, nc));
                queue.push_back((nr, nc));
            }
        }

        // Pipe edges: direct teleport
        if let Some(dests) = pipe_lookup.get(&(r, c)) {
            for &dest in dests {
                edges.entry((r, c)).or_default().push(Edge {
                    dest,
                    path_pos: None,
                    path_tile: 0,
                });
                if !nodes.contains(&dest) {
                    nodes.insert(dest);
                    queue.push_back(dest);
                }
            }
        }
    }

    WalkResult { nodes, edges, path_tiles }
}

// ---------------------------------------------------------------------------
// Chokepoint detection
// ---------------------------------------------------------------------------

/// Find path tiles whose removal disconnects the node graph (articulation points).
///
/// Tests each path tile by removing it and checking if BFS still reaches all nodes.
pub(super) fn find_chokepoints(result: &WalkResult) -> HashSet<(usize, usize)> {
    if result.nodes.is_empty() {
        return HashSet::new();
    }

    // Build adjacency: node → list of (neighbor, path_pos_or_none)
    let mut adj: HashMap<(usize, usize), Vec<((usize, usize), Option<(usize, usize)>)>> =
        HashMap::new();
    for (node, neighbors) in &result.edges {
        for edge in neighbors {
            adj.entry(*node).or_default().push((edge.dest, edge.path_pos));
        }
    }

    let start = *result.nodes.iter().next().unwrap();
    let mut chokepoints = HashSet::new();

    for &path_pos in &result.path_tiles {
        // BFS without using edges through this path tile
        let mut visited = HashSet::new();
        let mut q = VecDeque::new();
        visited.insert(start);
        q.push_back(start);

        while let Some(n) = q.pop_front() {
            if let Some(neighbors) = adj.get(&n) {
                for &(dest, pp) in neighbors {
                    if pp == Some(path_pos) {
                        continue;
                    }
                    if !visited.contains(&dest) {
                        visited.insert(dest);
                        q.push_back(dest);
                    }
                }
            }
        }

        if visited.len() < result.nodes.len() {
            chokepoints.insert(path_pos);
        }
    }

    chokepoints
}

// ---------------------------------------------------------------------------
// Debug visualization
// ---------------------------------------------------------------------------

// ANSI color codes
const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";
const BRIGHT_GREEN: &str = "\x1b[1;32m";
const BRIGHT_RED: &str = "\x1b[1;31m";
const BRIGHT_CYAN: &str = "\x1b[1;36m";
const YELLOW: &str = "\x1b[33m";
const BRIGHT_WHITE: &str = "\x1b[1;37m";
const _MAGENTA: &str = "\x1b[35m";

/// Render a colored ASCII debug visualization of a world's overworld grid.
///
/// Shows reachable nodes, walked paths, chokepoints, and pipe positions
/// using ANSI terminal colors.
#[allow(dead_code)]
pub(super) fn render_debug(
    grid: &Grid,
    walk: Option<&WalkResult>,
    chokepoints: Option<&HashSet<(usize, usize)>>,
    pipe_positions: Option<&HashSet<(usize, usize)>>,
    label: &str,
) -> String {
    let mut out = String::new();

    // Header
    out.push_str(&format!("\n{DIM}=== {label} ({} cols) ==={RESET}\n", grid.cols));

    // Column ruler
    out.push_str(&format!("{DIM}  "));
    for c in 0..grid.cols {
        out.push_str(&format!("{}", c % 10));
    }
    out.push_str(&format!("{RESET}\n"));

    // Grid rows
    for r in 0..grid.rows {
        out.push_str(&format!("{DIM}{r} {RESET}"));
        for c in 0..grid.cols {
            let tile = grid.get(r, c);
            let pos = (r, c);

            let is_node = walk.is_some_and(|w| w.nodes.contains(&pos));
            let is_path = walk.is_some_and(|w| w.path_tiles.contains(&pos));

            let (ch, color) = if tile == TILE_START {
                ('S', BRIGHT_GREEN)
            } else if chokepoints.is_some_and(|cp| cp.contains(&pos)) {
                ('!', BRIGHT_RED)
            } else if pipe_positions.is_some_and(|pp| pp.contains(&pos)) {
                ('P', BRIGHT_CYAN)
            } else if is_path {
                ('~', YELLOW)
            } else if is_node {
                ('*', BRIGHT_WHITE)
            } else if VALID_HORZ.contains(&tile) {
                ('-', DIM)
            } else if VALID_VERT.contains(&tile) {
                ('|', DIM)
            } else {
                ('.', DIM)
            };

            out.push_str(&format!("{color}{ch}{RESET}"));
        }
        out.push('\n');
    }

    // Legend
    out.push_str(&format!(
        "{DIM}{BRIGHT_GREEN}S{RESET}{DIM}=start {BRIGHT_WHITE}*{RESET}{DIM}=node \
         {YELLOW}~{RESET}{DIM}=path {BRIGHT_RED}!{RESET}{DIM}=choke \
         {BRIGHT_CYAN}P{RESET}{DIM}=pipe{RESET}\n"
    ));

    out
}

/// Fortress map tile ID.
const TILE_FORTRESS: u8 = 0x67;

/// Find fortress positions by scanning the tile grid for fortress tiles ($67).
/// Returns sorted positions in row-major order (deterministic).
fn find_fortress_tiles(grid: &Grid) -> Vec<(usize, usize)> {
    let mut positions = Vec::new();
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            if grid.get(r, c) == TILE_FORTRESS {
                positions.push((r, c));
            }
        }
    }
    positions.sort();
    positions
}

/// Render a step-by-step fortress progression visualization for a world.
///
/// Beats each reachable fortress in order, opens its lock/bridge via the FX
/// table, re-walks, and renders the grid at each step.
#[allow(dead_code)]
pub(super) fn render_progression(
    rom: &Rom,
    world_idx: usize,
    pipe_pairs: &[((usize, usize), (usize, usize))],
) -> String {
    let mut grid = read_tile_grid(rom, world_idx);
    let fx_slots = read_fx_slots(rom);

    // Scan map for fortress tiles ($67) to handle post-redistribution state
    let world_forts = find_fortress_tiles(&grid);
    let fort_count = world_forts.len();
    let base = FX_WORLD_TABLE + world_idx * 4;
    let world_fx: Vec<u8> = (0..fort_count.min(4))
        .map(|i| rom.read_byte(base + i))
        .collect();
    let mut beaten: HashSet<usize> = HashSet::new();
    let mut out = String::new();

    // Collect pipe positions for display
    let mut pipe_pos = HashSet::new();
    for &(a, b) in pipe_pairs {
        pipe_pos.insert(a);
        pipe_pos.insert(b);
    }

    // Initial walk
    let result = walk_map(&grid, pipe_pairs, None);
    let chokes = find_chokepoints(&result);
    out.push_str(&render_debug(
        &grid, Some(&result), Some(&chokes), Some(&pipe_pos),
        &format!("W{} — initial", world_idx + 1),
    ));

    loop {
        // Re-walk with current grid state to get fresh reachable set
        let result = walk_map(&grid, pipe_pairs, None);

        let reachable_forts: Vec<usize> = world_forts
            .iter()
            .enumerate()
            .filter(|(i, pos)| !beaten.contains(i) && result.nodes.contains(pos))
            .map(|(i, _)| i)
            .collect();

        if reachable_forts.is_empty() {
            break;
        }

        let fort_idx = reachable_forts[0];
        let fort_pos = world_forts[fort_idx];
        beaten.insert(fort_idx);

        let mut label = format!("W{} — beat fortress {} at ({},{})",
            world_idx + 1, fort_idx, fort_pos.0, fort_pos.1);

        if fort_idx < world_fx.len() {
            let slot_idx = world_fx[fort_idx] as usize;
            if slot_idx < fx_slots.len() {
                let slot = &fx_slots[slot_idx];
                let (fx_r, fx_c) = (slot.grid_row, slot.grid_col);
                let old = grid.get(fx_r, fx_c);
                grid.set(fx_r, fx_c, slot.replace_tile);
                label.push_str(&format!(
                    " → open ({},{}) ${:02X}→${:02X}",
                    fx_r, fx_c, old, slot.replace_tile
                ));
            }
        }

        let result = walk_map(&grid, pipe_pairs, None);
        let chokes = find_chokepoints(&result);
        out.push_str(&render_debug(
            &grid, Some(&result), Some(&chokes), Some(&pipe_pos), &label,
        ));
    }

    out
}

// ---------------------------------------------------------------------------
// Fortress progression simulation
// ---------------------------------------------------------------------------

/// Read all 17 FX slots from ROM.
pub(super) fn read_fx_slots(rom: &Rom) -> Vec<FxSlot> {
    let mut slots = Vec::with_capacity(17);
    for i in 0..17 {
        let loc_row = rom.read_byte(FX_MAP_LOC_ROW + i);
        let loc = rom.read_byte(FX_MAP_LOC + i);
        let replace_tile = rom.read_byte(FX_MAP_TILE_REPLACE + i);

        let grid_row = ((loc_row >> 4) as usize).wrapping_sub(2);
        let col_in_screen = ((loc >> 4) & 0x0F) as usize;
        let screen = (loc & 0x0F) as usize;

        slots.push(FxSlot {
            grid_row,
            grid_col: screen * 16 + col_in_screen,
            replace_tile,
        });
    }
    slots
}

/// Read FortressFX_W1-W8: which FX slots each world uses.
/// Returns array of 8 Vecs, one per world.
///
/// Each world has 4 bytes in the table, but only the first N are meaningful
/// where N = number of fortresses in that world. The rest are zero-padded.
/// We use the fortress count from FORTRESS_ENTRIES to know how many to read.
pub(super) fn read_world_fx_assignments(rom: &Rom) -> [Vec<u8>; 8] {
    let mut assignments: [Vec<u8>; 8] = Default::default();
    for wi in 0..8 {
        let fort_count = FORTRESS_ENTRIES.iter().filter(|&&(w, _)| w == wi).count();
        let base = FX_WORLD_TABLE + wi * 4;
        for i in 0..fort_count.min(4) {
            assignments[wi].push(rom.read_byte(base + i));
        }
    }
    assignments
}

/// Read grid positions of fortress entries for a world.
pub(super) fn read_fortress_positions(rom: &Rom, world_idx: usize) -> Vec<(usize, usize)> {
    let world = &WORLDS[world_idx];
    FORTRESS_ENTRIES
        .iter()
        .filter(|&&(w, _)| w == world_idx)
        .map(|&(_, ei)| entry_grid_position(rom, world, ei))
        .collect()
}

/// Simulate fortress progression for a world.
///
/// Iteratively walks the map, beats the lowest-ordinal reachable fortress,
/// opens its FX slot (replacing the lock/bridge tile), and re-walks.
/// Uses hardcoded FORTRESS_ENTRIES — for post-redistribution use, call
/// `simulate_progression_with` and pass explicit fortress positions.
pub(super) fn simulate_progression(
    rom: &Rom,
    world_idx: usize,
    pipe_pairs: &[((usize, usize), (usize, usize))],
) -> Vec<ProgressionStep> {
    let world_forts = read_fortress_positions(rom, world_idx);
    let fx_assignments = read_world_fx_assignments(rom);
    let world_fx = fx_assignments[world_idx].clone();
    simulate_progression_with(rom, world_idx, pipe_pairs, &world_forts, &world_fx)
}

/// Simulate fortress progression with explicit fortress positions and FX assignments.
///
/// Use this after redistribution when FORTRESS_ENTRIES no longer reflects reality.
pub(super) fn simulate_progression_with(
    rom: &Rom,
    world_idx: usize,
    pipe_pairs: &[((usize, usize), (usize, usize))],
    world_forts: &[(usize, usize)],
    world_fx: &[u8],
) -> Vec<ProgressionStep> {
    let mut grid = read_tile_grid(rom, world_idx);
    let fx_slots = read_fx_slots(rom);

    let mut beaten: HashSet<usize> = HashSet::new();
    let mut steps = Vec::new();

    // Initial walk
    let result = walk_map(&grid, pipe_pairs, None);
    steps.push(ProgressionStep {
        fort_idx: None,
        fort_pos: None,
        fx_pos: None,
        fx_old_tile: None,
        fx_new_tile: None,
        nodes: result.nodes.clone(),
    });

    loop {
        // Find reachable fortresses not yet beaten (use latest step's nodes)
        let current_nodes = &steps.last().unwrap().nodes;
        let reachable_forts: Vec<usize> = world_forts
            .iter()
            .enumerate()
            .filter(|(i, pos)| !beaten.contains(i) && current_nodes.contains(pos))
            .map(|(i, _)| i)
            .collect();

        if reachable_forts.is_empty() {
            break;
        }

        let fort_idx = reachable_forts[0];
        let fort_pos = world_forts[fort_idx];
        beaten.insert(fort_idx);

        let mut fx_pos = None;
        let mut fx_old = None;
        let mut fx_new = None;

        if fort_idx < world_fx.len() {
            let slot_idx = world_fx[fort_idx] as usize;
            if slot_idx < fx_slots.len() {
                let slot = &fx_slots[slot_idx];
                let (fx_r, fx_c) = (slot.grid_row, slot.grid_col);
                fx_old = Some(grid.get(fx_r, fx_c));
                fx_new = Some(slot.replace_tile);
                grid.set(fx_r, fx_c, slot.replace_tile);
                fx_pos = Some((fx_r, fx_c));
            }
        }

        let result = walk_map(&grid, pipe_pairs, None);
        steps.push(ProgressionStep {
            fort_idx: Some(fort_idx),
            fort_pos: Some(fort_pos),
            fx_pos,
            fx_old_tile: fx_old,
            fx_new_tile: fx_new,
            nodes: result.nodes,
        });
    }

    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_start_all_worlds() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }
        let rom = Rom::from_bytes(&rom_data.unwrap()).unwrap();

        for wi in 0..8 {
            let grid = read_tile_grid(&rom, wi);
            let start = find_start(&grid);
            assert!(
                start.is_some(),
                "World {} should have a START tile",
                wi + 1
            );
        }
    }

    #[test]
    fn test_walk_w1_reachable() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }
        let rom = Rom::from_bytes(&rom_data.unwrap()).unwrap();

        let grid = read_tile_grid(&rom, 0);
        let pipes = read_pipe_pairs(&rom);
        let w1_pipes = pipes.get(&0).cloned().unwrap_or_default();
        let result = walk_map(&grid, &w1_pipes, None);

        // W1 has 21 entries, most are reachable from start (no pipes needed)
        assert!(
            result.nodes.len() >= 15,
            "W1 should have at least 15 reachable nodes, got {}",
            result.nodes.len()
        );
    }

    #[test]
    fn test_walk_w7_needs_pipes() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }
        let rom = Rom::from_bytes(&rom_data.unwrap()).unwrap();

        let grid = read_tile_grid(&rom, 6);

        // Walk without pipes — should be very limited
        let result_no_pipes = walk_map(&grid, &[], None);

        // Walk with pipes — should reach many more
        let pipes = read_pipe_pairs(&rom);
        let w7_pipes = pipes.get(&6).cloned().unwrap_or_default();
        let result_with_pipes = walk_map(&grid, &w7_pipes, None);

        assert!(
            result_with_pipes.nodes.len() > result_no_pipes.nodes.len(),
            "W7 with pipes ({}) should reach more than without ({})",
            result_with_pipes.nodes.len(),
            result_no_pipes.nodes.len()
        );
    }

    #[test]
    fn test_dest_indices_for_world() {
        assert_eq!(dest_indices_for_world(0).len(), 0); // W1: no pipes
        assert_eq!(dest_indices_for_world(1).len(), 1); // W2: 1 pair
        assert_eq!(dest_indices_for_world(4).len(), 2); // W5: 1 regular + 1 spiral tower
        assert_eq!(dest_indices_for_world(6).len(), 8); // W7: 8 pairs
        assert_eq!(dest_indices_for_world(7).len(), 6); // W8: 6 pairs
    }

    #[test]
    fn test_chokepoints_w1() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }
        let rom = Rom::from_bytes(&rom_data.unwrap()).unwrap();

        let grid = read_tile_grid(&rom, 0);
        let result = walk_map(&grid, &[], None);
        let chokepoints = find_chokepoints(&result);

        // W1 has a linear path structure with many chokepoints
        assert!(
            !chokepoints.is_empty(),
            "W1 should have chokepoints (linear map)"
        );
    }

    #[test]
    fn test_render_debug_visual() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }
        let rom = Rom::from_bytes(&rom_data.unwrap()).unwrap();
        let all_pipes = read_pipe_pairs(&rom);

        for wi in 0..8 {
            let grid = read_tile_grid(&rom, wi);
            let pipes = all_pipes.get(&wi).cloned().unwrap_or_default();
            let result = walk_map(&grid, &pipes, None);
            let chokes = find_chokepoints(&result);

            let mut pipe_pos = HashSet::new();
            for &(a, b) in &pipes {
                pipe_pos.insert(a);
                pipe_pos.insert(b);
            }

            let label = format!("W{}", wi + 1);
            let output = render_debug(&grid, Some(&result), Some(&chokes), Some(&pipe_pos), &label);
            print!("{output}");
        }
    }

    #[test]
    fn test_render_progression_w6() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }
        let rom = Rom::from_bytes(&rom_data.unwrap()).unwrap();
        let all_pipes = read_pipe_pairs(&rom);
        let pipes = all_pipes.get(&5).cloned().unwrap_or_default();

        let output = render_progression(&rom, 5, &pipes);
        print!("{output}");
    }

    #[test]
    fn test_render_randomized_seed() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }
        let mut rom = Rom::from_bytes(&rom_data.unwrap()).unwrap();

        let mut options = crate::randomizer::Options::default();
        options.shuffle_fortresses = true;
        options.redistribute_fortresses = true;
        options.shuffle_pipes = true;
        let seed = 42;
        crate::randomizer::randomize(&mut rom, seed, &options);

        let all_pipes = read_pipe_pairs(&rom);

        println!("\n\x1b[1;33m=== Randomized seed {seed} (fortresses + pipes) ===\x1b[0m\n");

        // Debug view of all 8 worlds
        for wi in 0..8 {
            let grid = read_tile_grid(&rom, wi);
            let pipes = all_pipes.get(&wi).cloned().unwrap_or_default();
            let result = walk_map(&grid, &pipes, None);
            let chokes = find_chokepoints(&result);

            let mut pipe_pos = HashSet::new();
            for &(a, b) in &pipes {
                pipe_pos.insert(a);
                pipe_pos.insert(b);
            }

            let label = format!("W{} randomized", wi + 1);
            let output = render_debug(
                &grid, Some(&result), Some(&chokes), Some(&pipe_pos), &label,
            );
            print!("{output}");
        }

        // Progression view for worlds with locks (W1-W7)
        println!("\n\x1b[1;33m=== Progression views ===\x1b[0m\n");
        for wi in 0..7 {
            let pipes = all_pipes.get(&wi).cloned().unwrap_or_default();
            let output = render_progression(&rom, wi, &pipes);
            print!("{output}");
        }
    }
}
