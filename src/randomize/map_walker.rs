/// BFS map walker for overworld connectivity analysis.
///
/// Traverses SMB3 overworld maps using the game's 2-tile movement model
/// (node → path tile → node). Supports pipe teleport edges, chokepoint
/// detection, and fortress progression simulation.
///
/// Shared ROM constants, data structures, and helpers live in `rom_data.rs`.

use std::collections::{HashMap, HashSet, VecDeque};

#[cfg(test)]
use crate::rom::Rom;

use super::rom_data::{
    self, BACKGROUND_TILES, VALID_HORZ, VALID_VERT,
    Grid,
};

#[cfg(test)]
use super::rom_data::{FX_WORLD_TABLE, TILE_FORTRESS, TILE_START};



/// Movement directions: (delta_row, delta_col, is_horizontal).
const DIRECTIONS: [(i8, i8, bool); 4] = [
    (0, 1, true),   // right
    (0, -1, true),  // left
    (1, 0, false),  // down
    (-1, 0, false), // up
];

// ---------------------------------------------------------------------------
// Data structures (walker-specific)
// ---------------------------------------------------------------------------

/// An edge in the walk graph.
/// Fields are populated during BFS and consumed by test-only visualization/analysis.
#[allow(dead_code)]
pub(super) struct Edge {
    pub dest: (usize, usize),
    /// Path tile position (None for pipe teleport edges).
    pub path_pos: Option<(usize, usize)>,
}

/// Result of a BFS map walk.
pub(super) struct WalkResult {
    pub nodes: HashSet<(usize, usize)>,
    /// Edge graph — populated during BFS, consumed by test-only chokepoint analysis.
    #[allow(dead_code)]
    pub edges: HashMap<(usize, usize), Vec<Edge>>,
    #[allow(dead_code)]
    pub path_tiles: HashSet<(usize, usize)>,
}

/// A step in fortress progression simulation.
/// Fields beyond `nodes` exist for debugging inspection via `--nocapture`.
#[cfg(test)]
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
    let start = match start_pos.or_else(|| rom_data::find_start(grid)) {
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
#[cfg(test)]
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

// ANSI color codes (test-only visualization)
#[cfg(test)]
const RESET: &str = "\x1b[0m";
#[cfg(test)]
const DIM: &str = "\x1b[2m";
#[cfg(test)]
const BRIGHT_GREEN: &str = "\x1b[1;32m";
#[cfg(test)]
const BRIGHT_RED: &str = "\x1b[1;31m";
#[cfg(test)]
const BRIGHT_CYAN: &str = "\x1b[1;36m";
#[cfg(test)]
const YELLOW: &str = "\x1b[33m";
#[cfg(test)]
const BRIGHT_WHITE: &str = "\x1b[1;37m";

/// Render a colored ASCII debug visualization of a world's overworld grid.
///
/// Shows reachable nodes, walked paths, chokepoints, and pipe positions
/// using ANSI terminal colors.
#[cfg(test)]
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

/// Find fortress positions by scanning the tile grid for fortress tiles ($67).
/// Returns sorted positions in row-major order (deterministic).
#[cfg(test)]
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
#[cfg(test)]
pub(super) fn render_progression(
    rom: &Rom,
    world_idx: usize,
    pipe_pairs: &[((usize, usize), (usize, usize))],
) -> String {
    let mut grid = rom_data::read_tile_grid(rom, world_idx);
    let fx_slots = rom_data::read_fx_slots(rom);

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

/// Simulate fortress progression for a world.
///
/// Iteratively walks the map, beats the lowest-ordinal reachable fortress,
/// opens its FX slot (replacing the lock/bridge tile), and re-walks.
/// Uses hardcoded FORTRESS_ENTRIES — for post-redistribution use, call
/// `simulate_progression_with` and pass explicit fortress positions.
#[cfg(test)]
pub(super) fn simulate_progression(
    rom: &Rom,
    world_idx: usize,
    pipe_pairs: &[((usize, usize), (usize, usize))],
) -> Vec<ProgressionStep> {
    let world_forts = rom_data::read_fortress_positions(rom, world_idx);
    let fx_assignments = rom_data::read_world_fx_assignments(rom);
    let world_fx = fx_assignments[world_idx].clone();
    simulate_progression_with(rom, world_idx, pipe_pairs, &world_forts, &world_fx)
}

/// Simulate fortress progression with explicit fortress positions and FX assignments.
///
/// Use this after redistribution when FORTRESS_ENTRIES no longer reflects reality.
#[cfg(test)]
pub(super) fn simulate_progression_with(
    rom: &Rom,
    world_idx: usize,
    pipe_pairs: &[((usize, usize), (usize, usize))],
    world_forts: &[(usize, usize)],
    world_fx: &[u8],
) -> Vec<ProgressionStep> {
    let mut grid = rom_data::read_tile_grid(rom, world_idx);
    let fx_slots = rom_data::read_fx_slots(rom);

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
    use super::rom_data;

    #[test]
    fn test_find_start_all_worlds() {
        let rom_data_bytes = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data_bytes.is_err() {
            return;
        }
        let rom = Rom::from_bytes(&rom_data_bytes.unwrap()).unwrap();

        for wi in 0..8 {
            let grid = rom_data::read_tile_grid(&rom, wi);
            let start = rom_data::find_start(&grid);
            assert!(
                start.is_some(),
                "World {} should have a START tile",
                wi + 1
            );
        }
    }

    #[test]
    fn test_walk_w1_reachable() {
        let rom_data_bytes = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data_bytes.is_err() {
            return;
        }
        let rom = Rom::from_bytes(&rom_data_bytes.unwrap()).unwrap();

        let grid = rom_data::read_tile_grid(&rom, 0);
        let pipes = rom_data::read_pipe_pairs(&rom);
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
        let rom_data_bytes = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data_bytes.is_err() {
            return;
        }
        let rom = Rom::from_bytes(&rom_data_bytes.unwrap()).unwrap();

        let grid = rom_data::read_tile_grid(&rom, 6);

        // Walk without pipes — should be very limited
        let result_no_pipes = walk_map(&grid, &[], None);

        // Walk with pipes — should reach many more
        let pipes = rom_data::read_pipe_pairs(&rom);
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
        assert_eq!(rom_data::dest_indices_for_world(0).len(), 0); // W1: no pipes
        assert_eq!(rom_data::dest_indices_for_world(1).len(), 1); // W2: 1 pair
        assert_eq!(rom_data::dest_indices_for_world(4).len(), 2); // W5: 1 regular + 1 spiral tower
        assert_eq!(rom_data::dest_indices_for_world(6).len(), 8); // W7: 8 pairs
        assert_eq!(rom_data::dest_indices_for_world(7).len(), 6); // W8: 6 pairs
    }

    #[test]
    fn test_chokepoints_w1() {
        let rom_data_bytes = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data_bytes.is_err() {
            return;
        }
        let rom = Rom::from_bytes(&rom_data_bytes.unwrap()).unwrap();

        let grid = rom_data::read_tile_grid(&rom, 0);
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
        let rom_data_bytes = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data_bytes.is_err() {
            return;
        }
        let rom = Rom::from_bytes(&rom_data_bytes.unwrap()).unwrap();
        let all_pipes = rom_data::read_pipe_pairs(&rom);

        for wi in 0..8 {
            let grid = rom_data::read_tile_grid(&rom, wi);
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
        let rom_data_bytes = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data_bytes.is_err() {
            return;
        }
        let rom = Rom::from_bytes(&rom_data_bytes.unwrap()).unwrap();
        let all_pipes = rom_data::read_pipe_pairs(&rom);
        let pipes = all_pipes.get(&5).cloned().unwrap_or_default();

        let output = render_progression(&rom, 5, &pipes);
        print!("{output}");
    }

    #[test]
    fn test_render_randomized_seed() {
        let rom_data_bytes = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data_bytes.is_err() {
            return;
        }
        let mut rom = Rom::from_bytes(&rom_data_bytes.unwrap()).unwrap();

        let mut options = crate::randomizer::Options::default();
        options.shuffle_fortresses = true;
        options.fortress_redistribute = crate::randomizer::FortressRedistribute::CrossWorld;
        options.shuffle_pipes = true;
        let seed = 42;
        crate::randomizer::randomize(&mut rom, seed, &options);

        let all_pipes = rom_data::read_pipe_pairs(&rom);

        println!("\n\x1b[1;33m=== Randomized seed {seed} (fortresses + pipes) ===\x1b[0m\n");

        // Debug view of all 8 worlds
        for wi in 0..8 {
            let grid = rom_data::read_tile_grid(&rom, wi);
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
