//! What a world map is made of: its tile vocabulary and its shape.
//!
//! Measures a [`WorldMap`], so the same code describes vanilla today and a
//! generated map later — the numbers vanilla produces are the generator's
//! targets (`docs/procedural_maps_design.md`, milestone 2).
//!
//! ```sh
//! cargo test --release --lib map_inventory -- --ignored --nocapture
//! ```

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::randomize::overworld::build::BREAKABLE_ROCKS;
use crate::randomize::overworld::helpers::find_target;
use crate::randomize::overworld::map_walker::walk_map;
use crate::randomize::rom_data::{
    self, BACKGROUND_TILES, Grid, MAP_TILE_GRIDS, Pos, TeleportEdge, VALID_BLANK_TILES, VALID_HORZ,
    VALID_VERT,
};

use super::WorldMap;

/// What a tile is doing on the map.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Role {
    /// A cell a pointer-table entry sits on: level, fort, pipe, start, ...
    Entry,
    /// An empty walkable node.
    Blank,
    HorizPath,
    VertPath,
    /// Closed until a fortress opens it: a lock, or water drawn over a path.
    Gap,
    /// Opened by a hammer.
    Rock,
    /// Where the canoe docks.
    Dock,
    /// Explicitly not walkable (`BACKGROUND_TILES`).
    Background,
    /// Everything else: art with no walk meaning.
    Scenery,
}

fn role(map: &WorldMap, entries: &HashSet<Pos>, pos: Pos) -> Role {
    let t = map.grid.get(pos.0, pos.1);
    if entries.contains(&pos) {
        Role::Entry
    } else if VALID_BLANK_TILES.contains(&t) {
        Role::Blank
    } else if VALID_HORZ.contains(&t) {
        Role::HorizPath
    } else if VALID_VERT.contains(&t) {
        Role::VertPath
    } else if map.gaps.iter().any(|&(p, _)| p == pos) {
        Role::Gap
    } else if BREAKABLE_ROCKS.iter().any(|&(closed, _)| closed == t) {
        Role::Rock
    } else if t == 0x4B {
        Role::Dock
    } else if BACKGROUND_TILES.contains(&t) {
        Role::Background
    } else {
        Role::Scenery
    }
}

/// How the walkable cells connect.
#[derive(Debug)]
struct Shape {
    /// Nodes reachable from the start by walking, pipe or canoe.
    nodes: usize,
    /// Walk links between neighbouring nodes (pipes and canoe not counted).
    walk_edges: usize,
    /// Nodes by number of walk links: `[0, 1, 2, 3+]`. 1 is a dead end, 2 a
    /// corridor, 3+ a junction; 0 is a node reached only by pipe or canoe.
    degree: [usize; 4],
    /// Independent loops in the walk graph: `edges - nodes + pieces`.
    loops: usize,
    /// Sizes of the pieces the walk graph falls into, largest first. More
    /// than one means islands, joined only by pipe or canoe.
    pieces: Vec<usize>,
    /// Fewest hops from the start to the airship or castle.
    start_to_target: Option<usize>,
    /// Breakable rocks, opened for this measurement.
    rocks: usize,
}

/// Measure a world's shape. Gaps and rocks are opened first: a gap is a lock
/// the builder may or may not keep, and a rock is a shortcut, so neither is a
/// wall to the map's shape.
fn shape(map: &WorldMap, world_idx: usize, pipe_pairs: &[TeleportEdge]) -> Shape {
    let mut grid: Grid = map.grid.clone();
    for &((r, c), path) in &map.gaps {
        grid.set(r, c, path);
    }
    let mut rocks = 0;
    for r in 0..grid.rows() {
        for c in 0..grid.cols {
            for (closed, open) in BREAKABLE_ROCKS {
                if grid.get(r, c) == closed {
                    grid.set(r, c, open);
                    rocks += 1;
                }
            }
        }
    }

    let walk = walk_map(&grid, pipe_pairs, None, world_idx);

    // Only real nodes: an entry's cell or a blank. The walker also steps onto
    // the scenery cell at column 0, left of the start, at the end of the
    // entrance path stub; that is not a place a player stands.
    let entries: HashSet<Pos> = map.entry_pos.iter().copied().collect();
    let is_node =
        |p: &Pos| entries.contains(p) || VALID_BLANK_TILES.contains(&map.grid.get(p.0, p.1));

    // Walk links, undirected and deduplicated. The walker does not expand out
    // of the airship (it is a sink), so take each link from whichever end
    // recorded it.
    let mut links: HashSet<(Pos, Pos)> = HashSet::new();
    for (&from, edges) in &walk.edges {
        if !is_node(&from) {
            continue;
        }
        for e in edges.iter().filter(|e| e.path_pos.is_some() && is_node(&e.dest)) {
            links.insert(if from < e.dest { (from, e.dest) } else { (e.dest, from) });
        }
    }
    let mut adj: HashMap<Pos, Vec<Pos>> =
        walk.nodes.iter().filter(|n| is_node(n)).map(|&n| (n, Vec::new())).collect();
    for &(a, b) in &links {
        adj.entry(a).or_default().push(b);
        adj.entry(b).or_default().push(a);
    }

    let mut degree = [0usize; 4];
    for neighbours in adj.values() {
        degree[neighbours.len().min(3)] += 1;
    }

    let mut seen: HashSet<Pos> = HashSet::new();
    let mut pieces = Vec::new();
    let mut starts: Vec<Pos> = adj.keys().copied().collect();
    starts.sort();
    for start in starts {
        if !seen.insert(start) {
            continue;
        }
        let mut size = 0;
        let mut stack = vec![start];
        while let Some(n) = stack.pop() {
            size += 1;
            for &m in &adj[&n] {
                if seen.insert(m) {
                    stack.push(m);
                }
            }
        }
        pieces.push(size);
    }
    pieces.sort_unstable_by(|a, b| b.cmp(a));

    let start_to_target =
        find_target(&grid, world_idx).and_then(|t| walk.distances.get(&t).copied());

    Shape {
        nodes: adj.len(),
        walk_edges: links.len(),
        degree,
        loops: (links.len() + pieces.len()).saturating_sub(adj.len()),
        pieces,
        start_to_target,
        rocks,
    }
}

#[test]
#[ignore]
fn map_inventory() {
    let Some(rom) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes")
        .ok()
        .and_then(|d| crate::rom::Rom::from_bytes(&d).ok())
    else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    // Default flags: no extra hammer rocks, no 8s-are-Wild canoe.
    let maps = super::vanilla::load(&rom, false, false);
    let pipes = rom_data::read_pipe_pairs(&rom);

    println!("\n== Shape (gaps and rocks open) ==");
    println!(
        "{:<3} {:>5} {:>6} {:>6} {:>5} {:>5} {:>5} {:>5} {:>6} {:>5} {:>6}  pieces",
        "W", "nodes", "/scrn", "links", "deg0", "dead", "corr", "junc", "loops", "rocks", "s->t"
    );
    for (wi, map) in maps.iter().enumerate() {
        let s = shape(map, wi, pipes.get(&wi).map(Vec::as_slice).unwrap_or(&[]));
        let screens = MAP_TILE_GRIDS[wi].screens;
        println!(
            "W{:<2} {:>5} {:>6.1} {:>6} {:>5} {:>5} {:>5} {:>5} {:>6} {:>5} {:>6}  {:?}",
            wi + 1,
            s.nodes,
            s.nodes as f64 / screens as f64,
            s.walk_edges,
            s.degree[0],
            s.degree[1],
            s.degree[2],
            s.degree[3],
            s.loops,
            s.rocks,
            s.start_to_target.map_or("-".to_string(), |d| d.to_string()),
            s.pieces,
        );
    }

    println!("\n== Tile vocabulary (role: tile x count) ==");
    for (wi, map) in maps.iter().enumerate() {
        let entries: HashSet<Pos> = map.entry_pos.iter().copied().collect();
        let mut by_role: BTreeMap<Role, BTreeMap<u8, usize>> = BTreeMap::new();
        for r in 0..map.grid.rows() {
            for c in 0..map.grid.cols {
                let t = map.grid.get(r, c);
                *by_role.entry(role(map, &entries, (r, c))).or_default().entry(t).or_default() += 1;
            }
        }
        println!("W{}", wi + 1);
        for (role, tiles) in &by_role {
            let total: usize = tiles.values().sum();
            let list: Vec<String> = tiles.iter().map(|(t, n)| format!("{t:02X}x{n}")).collect();
            println!("  {:<11} {:>4}  {}", format!("{role:?}"), total, list.join(" "));
        }
    }
}
