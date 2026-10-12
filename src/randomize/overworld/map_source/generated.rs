//! The generated map source: a world's layout (nodes and links), drawn into a
//! [`WorldMap`] the rest of the pipeline uses exactly like a vanilla one.
//!
//! Two stages. [`generate`] decides the layout, which can be judged as ASCII
//! before anything is drawn:
//!
//! ```sh
//! cargo test --release --lib generated_layouts -- --ignored --nocapture
//! ```
//!
//! [`world_map`] then draws it: frame and bush fill from the vanilla world, paths
//! and rocks from the links, and each pointer entry on a node showing its own
//! tile. No scenery yet (`docs/procedural_maps_design.md`, milestone 3).
//! Reached only through the hidden `generated_worlds` option.

use std::collections::{HashMap, HashSet, VecDeque};

use rand::Rng;
use rand::seq::{IndexedRandom, SliceRandom};

use crate::randomize::overworld::pickup::blank_tile_from_neighbors;
use crate::randomize::rom_data::{AIRSHIP_ENTRIES, Grid, Pos, TILE_START, VALID_BLANK_TILES};

use super::WorldMap;

/// What a link between two neighbouring nodes is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Link {
    /// An ordinary path.
    Path,
    /// A rock a hammer opens. Only ever a shortcut: the paths alone always
    /// connect every node.
    Rock,
}

/// One world's layout: where the nodes are and how they connect.
#[derive(Debug)]
// Reason: `rows`, `cols`, `start` and `loop_sizes` are read only by the ASCII
// view and the tally, which are test-only; the layout carries them so a test
// judges exactly what production generated.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) struct Layout {
    pub(super) rows: usize,
    pub(super) cols: usize,
    /// Every node, start and target included.
    pub(super) nodes: Vec<Pos>,
    /// Links between nodes two tiles apart, each pair stored once.
    pub(super) links: Vec<(Pos, Pos, Link)>,
    pub(super) start: Pos,
    /// The airship (the castle in W8).
    pub(super) target: Pos,
    /// Each loop link's length: the nodes around the loop it closed when it
    /// was added. 4 is a square, which gives the player no real choice.
    pub(super) loop_sizes: Vec<usize>,
}

/// The targets a layout must meet, or it is thrown away and redrawn.
#[derive(Clone, Copy, Debug)]
pub(super) struct Targets {
    /// The world's pointer-entry count: one node per entry. Spare nodes hold
    /// Hammer Bro levels, as in vanilla.
    pub(super) nodes: usize,
    /// Links added on top of a tree. Each one is exactly one loop.
    pub(super) loops: (usize, usize),
    pub(super) dead_ends: (usize, usize),
    /// Fewest hops from the start to the target, on paths alone.
    pub(super) start_to_target: (usize, usize),
    pub(super) rocks: (usize, usize),
}

/// The proposal agreed for W1 on 2026-10-11. Vanilla W1 is 21 nodes, 2
/// loops, 3 dead ends, 9 hops, 1 rock shortcut.
pub(super) const W1_TARGETS: Targets = Targets {
    nodes: 21,
    loops: (3, 5),
    dead_ends: (2, 5),
    start_to_target: (7, 11),
    rocks: (0, 2),
};

/// How often the tree grows from its newest node rather than from any node.
/// A first guess, to be judged on the ASCII layouts.
const EXTEND_NEWEST: f64 = 0.75;

/// How many of the farthest-apart candidate pairs a loop link is picked from.
const FARTHEST_FEW: usize = 3;

/// Most squares (four nodes in a ring, which give no real route choice) a
/// layout may have. Not banned outright: one now and then reads fine.
const MAX_SQUARES: usize = 1;

/// Draw layouts until one meets `targets`, or give up after `attempts`.
pub(super) fn generate<R: Rng>(
    rows: usize,
    cols: usize,
    start: Pos,
    targets: Targets,
    attempts: usize,
    rng: &mut R,
) -> Option<Layout> {
    (0..attempts).find_map(|_| try_once(rows, cols, start, targets, rng))
}

/// The dancing bush (`TILE_DANCINGBUSH`): what every empty cell becomes until
/// there is scenery. Vanilla W1 fills its empty space with it too.
const FILL: u8 = 0xB4;
/// Path and rock tiles for a link, by direction: `(horizontal, vertical)`.
const PATH: (u8, u8) = (0x45, 0x46);
const ROCK: (u8, u8) = (0x51, 0x52);

/// Generate a new map for `world_idx`, drawn over `vanilla` (that world's
/// vanilla map, which supplies the frame, the start, and each entry's tile).
/// `None` if no layout met the targets; the caller keeps vanilla then.
///
/// Only World 1 has targets so far.
pub(crate) fn world_map<R: Rng>(
    vanilla: &WorldMap,
    world_idx: usize,
    rng: &mut R,
) -> Option<WorldMap> {
    if world_idx != 0 {
        return None;
    }
    let old = &vanilla.grid;
    let on_grid = |p: Pos| p.0 < old.rows() && p.1 < old.cols;
    let start_idx = (0..vanilla.entry_pos.len()).find(|&i| {
        on_grid(vanilla.entry_pos[i]) && {
            let p = vanilla.entry_pos[i];
            old.get(p.0, p.1) == TILE_START
        }
    })?;
    let start = vanilla.entry_pos[start_idx];
    let layout = generate(old.rows(), old.cols, start, W1_TARGETS, 1000, rng)?;

    // Frame (first and last column) and the start's entrance stub stay as
    // vanilla drew them; everything else starts as plain ground.
    let mut grid = old.clone();
    grid.vanilla_layout = false;
    for r in 0..grid.rows() {
        for c in 1..grid.cols - 1 {
            if (r, c) != (start.0, start.1 - 1) {
                grid.set(r, c, FILL);
            }
        }
    }
    for &(a, b, kind) in &layout.links {
        let (horiz, vert) = if kind == Link::Rock { ROCK } else { PATH };
        grid.set((a.0 + b.0) / 2, (a.1 + b.1) / 2, if a.0 == b.0 { horiz } else { vert });
    }

    // Entries onto nodes: the start stays, the airship goes to the target,
    // the rest are dealt at random onto the remaining nodes. Entries off the
    // grid (unused rows in the pointer table) stay where they are.
    let airship_idx =
        (0..vanilla.entry_pos.len()).find(|&i| AIRSHIP_ENTRIES.contains(&(world_idx, i)))?;
    let mut free: Vec<Pos> =
        layout.nodes.iter().copied().filter(|&n| n != start && n != layout.target).collect();
    free.shuffle(rng);
    let mut entry_pos = vanilla.entry_pos.clone();
    entry_pos[airship_idx] = layout.target;
    for (i, pos) in entry_pos.iter_mut().enumerate() {
        if i != start_idx && i != airship_idx && on_grid(vanilla.entry_pos[i]) {
            *pos = free.pop()?;
        }
    }
    if !free.is_empty() {
        return None;
    }

    // Each entry shows its own tile. A blank one (a Hammer Bro level's cell)
    // is redrawn for its new neighbours, after every entry is down.
    let mut blanks = Vec::new();
    for (i, &pos) in entry_pos.iter().enumerate() {
        let from = vanilla.entry_pos[i];
        if !on_grid(from) {
            continue;
        }
        let tile = old.get(from.0, from.1);
        grid.set(pos.0, pos.1, tile);
        if VALID_BLANK_TILES.contains(&tile) {
            blanks.push(pos);
        }
    }
    for pos in blanks {
        let tile = blank_tile_from_neighbors(&grid, world_idx, pos.0, pos.1);
        grid.set(pos.0, pos.1, tile);
    }

    // The castle top sits above the airship, as in vanilla.
    let air = vanilla.entry_pos[airship_idx];
    grid.set(layout.target.0 - 1, layout.target.1, old.get(air.0 - 1, air.1));

    // Water, all drawn together at the end: lakes in the big open regions,
    // most of the time a river joining two lakes, sometimes a moat. Then the
    // decoration clusters fill what is left.
    let mut water = Water::default();
    for region in empty_regions(&grid) {
        if region.len() >= LAKE_MIN_REGION && rng.random_bool(LAKE_CHANCE) {
            grow_lake(&mut grid, &region, None, &mut water, rng);
        }
    }
    if rng.random_bool(RIVER_CHANCE) {
        add_river(&mut grid, &mut water, rng);
    }
    if rng.random_bool(MOAT_CHANCE) {
        add_moat(&mut grid, layout.target, &mut water);
    }
    decorate(&mut grid, rng);
    water.draw(&mut grid);

    Some(WorldMap {
        grid,
        entry_pos,
        sprites: Vec::new(),
        hb_sprites: Vec::new(),
        gaps: Vec::new(),
        hammer_gated: Vec::new(),
        no_locks: water.crossings.clone(),
    })
}

/// Plain ground (all four quadrants blank): W1's clearings.
const CLEARING: u8 = 0x42;
/// A rock that never breaks. It looks exactly like a breakable one, so it is
/// only ever placed where no path or node touches it.
const DECO_ROCK: u8 = 0x53;

/// Scenery (milestone 3, decorations D1 + D2). The empty cells still bush
/// fill after the water is placed fall into separate regions; each region big
/// enough gets a few clusters of clearing or rock, grown as a blob from a
/// random cell. Tiny regions, usually wedged between paths, stay bushes.
fn decorate<R: Rng>(grid: &mut Grid, rng: &mut R) {
    for region in empty_regions(grid) {
        if region.len() < MIN_DECORATED_REGION {
            continue;
        }
        for _ in 0..1 + region.len() / CELLS_PER_CLUSTER {
            let rock = rng.random_bool(ROCK_SHARE);
            let allowed: Vec<Pos> = region
                .iter()
                .copied()
                .filter(|&p| grid.get(p.0, p.1) == FILL && (!rock || clear_of_paths(grid, p)))
                .collect();
            let Some(&seed) = allowed.choose(rng) else { continue };
            let size = rng.random_range(2..=MAX_CLUSTER.min(region.len() / 2).max(2));
            let tile = if rock { DECO_ROCK } else { CLEARING };
            let mut cluster = vec![seed];
            while cluster.len() < size {
                let next: Vec<Pos> = allowed
                    .iter()
                    .copied()
                    .filter(|p| !cluster.contains(p) && cluster.iter().any(|&c| touching(c, *p)))
                    .collect();
                let Some(&p) = next.choose(rng) else { break };
                cluster.push(p);
            }
            for (r, c) in cluster {
                grid.set(r, c, tile);
            }
        }
    }
}

/// Regions at least this big may get a lake.
const LAKE_MIN_REGION: usize = 10;
/// Chance a big enough region gets one.
const LAKE_CHANCE: f64 = 0.5;
/// Most cells a lake grows to (it may end smaller once trimmed).
const MAX_LAKE: usize = 12;

/// Each water tile and the picture its graphics draw: water (`W`) or land
/// (`L`) at nine points, the corners, edge middles and centre, as rows top /
/// middle / bottom. Every water tile `$82`–`$A9` is a different picture, so a
/// lake is drawn by working out each cell's picture from its lake neighbours
/// and looking the tile up. `water_pictures_match_the_graphics` re-derives
/// every picture from the ROM's CHR.
const WATER_TILES: [(u8, &str); 40] = [
    (0x82, "LWL/WWW/LLL"),
    (0x83, "LLL/WWW/LWL"),
    (0x84, "LLL/LWW/LWW"),
    (0x85, "LLL/WWW/WWW"),
    (0x86, "LLL/WWL/WWL"),
    (0x87, "WWW/WWW/WWL"),
    (0x88, "WWW/WWW/LWW"),
    (0x89, "WWW/WWW/LWL"),
    (0x8A, "LWL/LWW/LWL"),
    (0x8B, "LWL/WWL/LWL"),
    (0x8C, "LWW/LWW/LWW"),
    (0x8D, "WWW/WWW/WWW"),
    (0x8E, "WWL/WWL/WWL"),
    (0x8F, "WWL/WWW/WWW"),
    (0x90, "LWW/WWW/WWW"),
    (0x91, "LWL/WWW/WWW"),
    (0x92, "WWL/WWW/WWL"),
    (0x93, "LWW/WWW/LWW"),
    (0x94, "LWW/LWW/LLL"),
    (0x95, "WWW/WWW/LLL"),
    (0x96, "WWL/WWL/LLL"),
    (0x97, "LWW/LWW/LWL"),
    (0x98, "WWL/WWL/LWL"),
    (0x99, "LLL/LWW/LWL"),
    (0x9A, "LLL/WWL/LWL"),
    (0x9B, "LLL/WWW/WWL"),
    (0x9C, "LLL/WWW/LWW"),
    (0x9D, "LWL/LWL/LWL"),
    (0x9E, "LWW/WWW/WWL"),
    (0x9F, "WWL/WWW/LWW"),
    (0xA0, "LWL/WWW/LWL"),
    (0xA1, "LLL/LWW/LLL"),
    (0xA2, "LLL/WWW/LLL"),
    (0xA3, "LLL/WWL/LLL"),
    (0xA4, "LWL/LWW/LWW"),
    (0xA5, "LWL/WWL/WWL"),
    (0xA6, "LWL/LWW/LLL"),
    (0xA7, "LWL/WWL/LLL"),
    (0xA8, "WWL/WWW/LLL"),
    (0xA9, "LWW/WWW/LLL"),
];

fn water_tile(picture: &str) -> Option<u8> {
    WATER_TILES.iter().find(|&&(_, p)| p == picture).map(|&(t, _)| t)
}

/// Everything wet on the map, drawn in one pass once lakes, river and moat
/// are placed, so water that meets other water joins it with a proper shore.
#[derive(Default)]
struct Water {
    /// Lake, river and moat cells.
    cells: Vec<Pos>,
    /// Each lake's cells, so a river can find its ends.
    lakes: Vec<Vec<Pos>>,
    /// A horizontal path crossing water that runs north-south: drawn as the
    /// bridge `$B3`. A lock the builder puts there is a water gap.
    bridges: Vec<Pos>,
    /// A vertical path crossing water that runs east-west: drawn as `$BA`
    /// (path over water, land at both ends). It has no water-gap form, so
    /// no lock may take it (`WorldMap::no_locks`).
    crossings: Vec<Pos>,
}

impl Water {
    fn wet(&self, p: Pos) -> bool {
        self.cells.contains(&p) || self.bridges.contains(&p) || self.crossings.contains(&p)
    }

    /// The picture a water cell needs: an edge middle is water when that
    /// neighbour is wet, a corner when both edges beside it and the diagonal
    /// are. Off the map is land.
    fn picture(&self, (r, c): Pos) -> String {
        let at = |dr: isize, dc: isize| {
            let (nr, nc) = (r as isize + dr, c as isize + dc);
            nr >= 0 && nc >= 0 && self.wet((nr as usize, nc as usize))
        };
        let w = |b: bool| if b { 'W' } else { 'L' };
        let (n, s, e, west) = (at(-1, 0), at(1, 0), at(0, 1), at(0, -1));
        format!(
            "{}{}{}/{}W{}/{}{}{}",
            w(n && west && at(-1, -1)),
            w(n),
            w(n && e && at(-1, 1)),
            w(west),
            w(e),
            w(s && west && at(1, -1)),
            w(s),
            w(s && e && at(1, 1)),
        )
    }

    /// Water cells whose picture has no water tile.
    fn undrawable(&self) -> Vec<Pos> {
        self.cells.iter().copied().filter(|&p| water_tile(&self.picture(p)).is_none()).collect()
    }

    fn draw(&self, grid: &mut Grid) {
        let tiles: Vec<(Pos, u8)> = self
            .cells
            .iter()
            .map(|&p| (p, water_tile(&self.picture(p)).expect("only drawable water is kept")))
            .collect();
        for ((r, c), t) in tiles {
            grid.set(r, c, t);
        }
        for &(r, c) in &self.bridges {
            grid.set(r, c, BRIDGE);
        }
        for &(r, c) in &self.crossings {
            grid.set(r, c, CROSSING);
        }
    }
}

/// The bridge over water running north-south, a horizontal path.
const BRIDGE: u8 = 0xB3;
/// A vertical path over water running east-west, land at both ends.
const CROSSING: u8 = 0xBA;
/// Stands in for a water cell until [`Water::draw`] picks its tile, so later
/// passes see the cell as taken.
const WATER_PENDING: u8 = 0x8D;
/// Chance a layout gets a river (project owner, 2026-10-12).
const RIVER_CHANCE: f64 = 0.75;
/// Most paths a river may cross.
const MAX_CROSSINGS: usize = 3;
/// Extra cost of crossing a path, so a river crosses only when it must.
const CROSSING_COST: u32 = 6;
/// Random extra cost per cell, up to this: what makes the river wander.
const RIVER_NOISE: u32 = 20;
/// Routes tried, each with fresh noise, before a layout goes without a river.
const RIVER_ATTEMPTS: usize = 10;
/// Smallest region a lake for a river's end may be grown in.
const END_LAKE_REGION: usize = 6;

/// A river joining two lakes. With two or more lakes it joins the two
/// farthest apart; with fewer it first grows the missing ones far from what
/// is there (like the layout's waypoint). The route is the cheapest path
/// through empty cells with random noise added, so it wanders. It crosses a
/// horizontal path straight north-south (a `$B3` bridge) and a vertical path
/// straight east-west (`$BA`), never a node. Water has no tile for a channel
/// ending on land, so a river always runs lake to lake.
fn add_river<R: Rng>(grid: &mut Grid, water: &mut Water, rng: &mut R) {
    while water.lakes.len() < 2 {
        let near: Vec<Pos> = water.lakes.iter().flatten().copied().collect();
        let mut spots: Vec<(usize, Pos, usize)> = Vec::new();
        for (i, region) in empty_regions(grid).iter().enumerate() {
            if region.len() < END_LAKE_REGION {
                continue;
            }
            for &p in region {
                let far = near.iter().map(|&q| p.0.abs_diff(q.0) + p.1.abs_diff(q.1)).min();
                spots.push((far.unwrap_or(0), p, i));
            }
        }
        if spots.is_empty() {
            return;
        }
        // The farthest few, or anywhere for a first lake.
        spots.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let pick = if near.is_empty() { spots.len() } else { FAR_FEW.min(spots.len()) };
        let &(_, seed, i) = spots[..pick].choose(rng).expect("spots is not empty");
        let region = empty_regions(grid).swap_remove(i);
        if !grow_lake(grid, &region, Some(seed), water, rng) {
            return;
        }
    }
    let (a, b) = farthest_lakes(&water.lakes);
    let (from, to) = (water.lakes[a].clone(), water.lakes[b].clone());
    for _ in 0..RIVER_ATTEMPTS {
        let noise: Vec<Vec<u32>> = (0..grid.rows())
            .map(|_| (0..grid.cols).map(|_| rng.random_range(0..RIVER_NOISE)).collect())
            .collect();
        let Some((cells, bridges, crossings)) = route(grid, water, &from, &to, &noise) else {
            return;
        };
        if bridges.len() + crossings.len() > MAX_CROSSINGS {
            continue;
        }
        let before = (water.cells.len(), water.bridges.len(), water.crossings.len());
        water.cells.extend(&cells);
        water.bridges.extend(&bridges);
        water.crossings.extend(&crossings);
        if water.undrawable().is_empty() {
            for &(r, c) in &cells {
                grid.set(r, c, WATER_PENDING);
            }
            return;
        }
        water.cells.truncate(before.0);
        water.bridges.truncate(before.1);
        water.crossings.truncate(before.2);
    }
}

/// How many of the farthest spots a river's end lake is picked from.
const FAR_FEW: usize = 5;

/// The pair of lakes whose nearest cells are farthest apart.
fn farthest_lakes(lakes: &[Vec<Pos>]) -> (usize, usize) {
    let gap = |a: &[Pos], b: &[Pos]| {
        a.iter()
            .flat_map(|p| b.iter().map(move |q| p.0.abs_diff(q.0) + p.1.abs_diff(q.1)))
            .min()
            .unwrap_or(0)
    };
    let mut best = (0, 1);
    let mut best_gap = 0;
    for i in 0..lakes.len() {
        for j in i + 1..lakes.len() {
            let g = gap(&lakes[i], &lakes[j]);
            if g > best_gap {
                best = (i, j);
                best_gap = g;
            }
        }
    }
    best
}

/// River cells, bridges and crossings: the cheapest route from a cell of
/// `from` to a cell of `to`. A step onto an empty cell costs 1 plus its
/// noise; a step over a path costs [`CROSSING_COST`] and lands on the cell
/// beyond, which must be empty or the destination lake.
fn route(
    grid: &Grid,
    water: &Water,
    from: &[Pos],
    to: &[Pos],
    noise: &[Vec<u32>],
) -> Option<(Vec<Pos>, Vec<Pos>, Vec<Pos>)> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    let (rows, cols) = (grid.rows() as isize, grid.cols as isize);
    let inside = |r: isize, c: isize| r >= 0 && r < rows && c >= 1 && c < cols - 1;
    let open = |p: Pos| grid.get(p.0, p.1) == FILL && !water.cells.contains(&p);
    let mut best: HashMap<Pos, u32> = HashMap::new();
    // How each cell was reached: the cell before it and the path crossed.
    let mut came: HashMap<Pos, (Pos, Option<Pos>)> = HashMap::new();
    let mut heap = BinaryHeap::new();
    for &p in from {
        best.insert(p, 0);
        heap.push(Reverse((0u32, p)));
    }
    let mut end = None;
    while let Some(Reverse((cost, p))) = heap.pop() {
        if best.get(&p).is_some_and(|&b| cost > b) {
            continue;
        }
        if to.contains(&p) {
            end = Some(p);
            break;
        }
        for (dr, dc) in [(-1isize, 0isize), (1, 0), (0, -1), (0, 1)] {
            let (r1, c1) = (p.0 as isize + dr, p.1 as isize + dc);
            if !inside(r1, c1) {
                continue;
            }
            let next = (r1 as usize, c1 as usize);
            let tile = grid.get(next.0, next.1);
            let (land, over, step) = if to.contains(&next) || open(next) {
                (next, None, 1 + noise[next.0][next.1])
            } else if (tile == PATH.0 && dr != 0) || (tile == PATH.1 && dc != 0) {
                let (r2, c2) = (r1 + dr, c1 + dc);
                if !inside(r2, c2) {
                    continue;
                }
                let beyond = (r2 as usize, c2 as usize);
                if !(to.contains(&beyond) || open(beyond)) {
                    continue;
                }
                (beyond, Some(next), CROSSING_COST + noise[beyond.0][beyond.1])
            } else {
                continue;
            };
            let total = cost + step;
            if best.get(&land).is_none_or(|&b| total < b) {
                best.insert(land, total);
                came.insert(land, (p, over));
                heap.push(Reverse((total, land)));
            }
        }
    }
    let end = end?;
    let (mut cells, mut bridges, mut crossings) = (Vec::new(), Vec::new(), Vec::new());
    let mut at = end;
    while let Some(&(prev, over)) = came.get(&at) {
        if !to.contains(&at) {
            cells.push(at);
        }
        if let Some(o) = over {
            if grid.get(o.0, o.1) == PATH.0 { bridges.push(o) } else { crossings.push(o) }
        }
        at = prev;
    }
    Some((cells, bridges, crossings))
}

/// Chance a castle whose path comes in from the side gets a moat.
const MOAT_CHANCE: f64 = 0.5;

/// A moat: water on every cell around the castle and its top, with the
/// castle's path crossing it on a bridge. Only when that path comes in from
/// the side, because a path from below would need a vertical bridge and none
/// exists. Every cell of the ring must be empty (or already water); a moat
/// that would leave any water cell without a tile is not added.
fn add_moat(grid: &mut Grid, (r, c): Pos, water: &mut Water) {
    if r < 2 || r + 1 >= grid.rows() || c < 2 || c + 2 >= grid.cols {
        return;
    }
    let Some(entry) =
        [(r, c - 1), (r, c + 1)].into_iter().find(|&(er, ec)| grid.get(er, ec) == PATH.0)
    else {
        return;
    };
    let ring: Vec<Pos> = (r - 2..=r + 1)
        .flat_map(|rr| (c - 1..=c + 1).map(move |cc| (rr, cc)))
        .filter(|&p| p != (r - 1, c) && p != (r, c) && p != entry)
        .collect();
    let free = |(rr, cc): Pos| grid.get(rr, cc) == FILL || water.cells.contains(&(rr, cc));
    if !ring.iter().all(|&p| free(p)) {
        return;
    }
    let new: Vec<Pos> = ring.into_iter().filter(|p| !water.cells.contains(p)).collect();
    let before = (water.cells.len(), water.bridges.len());
    water.cells.extend(&new);
    water.bridges.push(entry);
    if !water.undrawable().is_empty() {
        water.cells.truncate(before.0);
        water.bridges.truncate(before.1);
        return;
    }
    for &(rr, cc) in &new {
        grid.set(rr, cc, WATER_PENDING);
    }
}

/// Grow a lake inside `region`, from `seed` or a random cell, and add it to
/// `water`. Lake cells whose picture has no water tile (a lone cell, say) are
/// trimmed; a lake that trims below two cells, or that would leave other
/// water undrawable, is dropped. Returns whether a lake was added.
fn grow_lake<R: Rng>(
    grid: &mut Grid,
    region: &[Pos],
    seed: Option<Pos>,
    water: &mut Water,
    rng: &mut R,
) -> bool {
    let Some(seed) = seed.or_else(|| region.choose(rng).copied()) else { return false };
    let size = rng.random_range(4..=MAX_LAKE.min(region.len() / 2).max(4));
    let mut lake = vec![seed];
    while lake.len() < size {
        let next: Vec<Pos> = region
            .iter()
            .copied()
            .filter(|p| !lake.contains(p) && lake.iter().any(|&c| touching(c, *p)))
            .collect();
        let Some(&p) = next.choose(rng) else { break };
        lake.push(p);
    }
    let before = water.cells.len();
    water.cells.extend(&lake);
    loop {
        let bad: Vec<Pos> = water.undrawable().into_iter().filter(|p| lake.contains(p)).collect();
        let Some(&p) = bad.first() else { break };
        lake.retain(|&q| q != p);
        water.cells.retain(|&q| q != p);
    }
    if lake.len() < 2 || !water.undrawable().is_empty() {
        water.cells.truncate(before);
        return false;
    }
    for &(r, c) in &lake {
        grid.set(r, c, WATER_PENDING);
    }
    water.lakes.push(lake);
    true
}

/// Regions smaller than this stay bushes.
const MIN_DECORATED_REGION: usize = 3;
/// One more cluster per this many cells of region.
const CELLS_PER_CLUSTER: usize = 12;
/// Most cells one cluster grows to.
const MAX_CLUSTER: usize = 6;
/// Share of clusters that are rock piles rather than clearings. Vanilla W1
/// has 9 rock cells to 11 clearing cells.
const ROCK_SHARE: f64 = 0.4;

fn touching(a: Pos, b: Pos) -> bool {
    a.0.abs_diff(b.0) + a.1.abs_diff(b.1) == 1
}

/// The groups of bush-fill cells joined side to side, in grid order. The
/// first and last columns are the frame and never part of one.
fn empty_regions(grid: &Grid) -> Vec<Vec<Pos>> {
    let mut seen = vec![vec![false; grid.cols]; grid.rows()];
    let mut regions = Vec::new();
    for r in 0..grid.rows() {
        for c in 1..grid.cols - 1 {
            if seen[r][c] || grid.get(r, c) != FILL {
                continue;
            }
            seen[r][c] = true;
            let mut region = vec![(r, c)];
            let mut i = 0;
            while i < region.len() {
                let (pr, pc) = region[i];
                i += 1;
                let around = [
                    pr.checked_sub(1).map(|r| (r, pc)),
                    (pr + 1 < grid.rows()).then_some((pr + 1, pc)),
                    (pc > 1).then(|| (pr, pc - 1)),
                    (pc + 2 < grid.cols).then_some((pr, pc + 1)),
                ];
                for (nr, nc) in around.into_iter().flatten() {
                    if !seen[nr][nc] && grid.get(nr, nc) == FILL {
                        seen[nr][nc] = true;
                        region.push((nr, nc));
                    }
                }
            }
            regions.push(region);
        }
    }
    regions
}

/// No neighbour of `p` is a path or a node: every one is scenery or frame.
fn clear_of_paths(grid: &Grid, (r, c): Pos) -> bool {
    let around = [
        r.checked_sub(1).map(|r| (r, c)),
        (r + 1 < grid.rows()).then_some((r + 1, c)),
        c.checked_sub(1).map(|c| (r, c)),
        (c + 1 < grid.cols).then_some((r, c + 1)),
    ];
    around.into_iter().flatten().all(|(nr, nc)| {
        let t = grid.get(nr, nc);
        t == FILL || t == CLEARING || t == DECO_ROCK || nc == 0 || nc + 1 == grid.cols
    })
}
/// Nodes sit two tiles apart, because a map move is two tiles: on the start's
/// row and column parity. Column 0 is left out: it is where the start's
/// entrance path runs off the edge of the map.
fn lattice(rows: usize, cols: usize, start: Pos) -> Vec<Pos> {
    let mut cells = Vec::new();
    for r in (start.0 % 2..rows).step_by(2) {
        for c in (start.1 % 2..cols).step_by(2) {
            if c > 0 {
                cells.push((r, c));
            }
        }
    }
    cells
}

/// The lattice cells two tiles away in each direction.
fn neighbours(cell: Pos, on_lattice: &HashSet<Pos>) -> Vec<Pos> {
    let (r, c) = cell;
    let candidates = [
        r.checked_sub(2).map(|r| (r, c)),
        Some((r + 2, c)),
        c.checked_sub(2).map(|c| (r, c)),
        Some((r, c + 2)),
    ];
    candidates.into_iter().flatten().filter(|p| on_lattice.contains(p)).collect()
}

fn ordered(a: Pos, b: Pos) -> (Pos, Pos) {
    if a < b { (a, b) } else { (b, a) }
}

fn try_once<R: Rng>(
    rows: usize,
    cols: usize,
    start: Pos,
    targets: Targets,
    rng: &mut R,
) -> Option<Layout> {
    let cells = lattice(rows, cols, start);
    let on_lattice: HashSet<Pos> = cells.iter().copied().collect();

    // 1. A random tree grown from the start until it has the node count. Most
    //    steps extend the newest node that can still grow, which draws long
    //    corridors; the rest branch from any node. Picking from any node every
    //    time grows a blob around the start instead.
    let mut nodes = vec![start];
    let mut in_tree: HashSet<Pos> = HashSet::from([start]);
    let mut tree: Vec<(Pos, Pos)> = Vec::new();

    //    First a waypoint, reached by a winding path from the start, so every
    //    layout reaches the right-hand side. It lands in a staircase in the
    //    bottom-right corner: on the top row only the last column qualifies,
    //    and each row further down allows one more column in from the right.
    //    Each step of the path moves right or toward the waypoint's row,
    //    chosen at random.
    // From the ordered lattice, never the set: a HashSet iterates in a
    // different order every run, which made the same seed draw different maps.
    let last_col = cells.iter().map(|p| p.1).max()?;
    let first_row = cells.iter().map(|p| p.0).min()?;
    let corner: Vec<Pos> =
        cells.iter().copied().filter(|&(r, c)| (last_col - c) / 2 <= (r - first_row) / 2).collect();
    let waypoint = *corner.choose(rng)?;
    let mut at = start;
    while at != waypoint {
        let mut steps = Vec::new();
        if at.1 < waypoint.1 {
            steps.push((at.0, at.1 + 2));
        }
        if at.0 < waypoint.0 {
            steps.push((at.0 + 2, at.1));
        }
        if at.0 > waypoint.0 {
            steps.push((at.0 - 2, at.1));
        }
        let &next = steps.choose(rng)?;
        in_tree.insert(next);
        nodes.push(next);
        tree.push(ordered(at, next));
        at = next;
    }

    while nodes.len() < targets.nodes {
        let free = |n: Pos| -> Vec<Pos> {
            neighbours(n, &on_lattice).into_iter().filter(|m| !in_tree.contains(m)).collect()
        };
        let growable: Vec<Pos> = nodes.iter().copied().filter(|&n| !free(n).is_empty()).collect();
        let from = if rng.random_bool(EXTEND_NEWEST) {
            *growable.last()?
        } else {
            *growable.choose(rng)?
        };
        let &to = free(from).choose(rng)?;
        in_tree.insert(to);
        nodes.push(to);
        tree.push(ordered(from, to));
    }

    // 2. Extra links between neighbouring nodes: each one is a loop.
    let linked: HashSet<(Pos, Pos)> = tree.iter().copied().collect();
    let mut spare: Vec<(Pos, Pos)> = Vec::new();
    for &n in &nodes {
        for m in neighbours(n, &on_lattice) {
            let pair = ordered(n, m);
            if in_tree.contains(&m) && !linked.contains(&pair) && !spare.contains(&pair) {
                spare.push(pair);
            }
        }
    }
    //    Prefer pairs far apart along the links already there: a link between
    //    nodes 8 hops apart is a real second route, while one between nodes 3
    //    hops apart only closes a square and offers no choice. Pick among the
    //    farthest few, so layouts still vary. A square is allowed, but at most
    //    `MAX_SQUARES` per layout.
    let loops = rng.random_range(targets.loops.0..=targets.loops.1);
    let mut extra: Vec<(Pos, Pos)> = Vec::new();
    let mut loop_sizes: Vec<usize> = Vec::new();
    for _ in 0..loops {
        let current: Vec<(Pos, Pos, Link)> =
            tree.iter().chain(&extra).map(|&(a, b)| (a, b, Link::Path)).collect();
        let squares_left = loop_sizes.iter().filter(|&&s| s == 4).count() < MAX_SQUARES;
        let mut scored: Vec<(usize, (Pos, Pos))> = spare
            .iter()
            .filter(|p| !extra.contains(p))
            .map(|&(a, b)| (path_hops(a, &current).get(&b).copied().unwrap_or(0), (a, b)))
            .filter(|&(hops, _)| hops != 3 || squares_left)
            .collect();
        scored.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(&y.1)));
        let &(hops, pair) = scored.get(..FARTHEST_FEW.min(scored.len()))?.choose(rng)?;
        loop_sizes.push(hops + 1);
        extra.push(pair);
    }

    // 3. Some extra links become rocks. Never a tree link, so never required.
    let rocks = rng.random_range(targets.rocks.0..=targets.rocks.1).min(extra.len());
    let mut links: Vec<(Pos, Pos, Link)> = tree.iter().map(|&(a, b)| (a, b, Link::Path)).collect();
    for (i, &(a, b)) in extra.iter().enumerate() {
        links.push((a, b, if i < rocks { Link::Rock } else { Link::Path }));
    }

    // 4. The target: a dead end at the right distance, not reached from above,
    //    with a row above it for the castle top.
    let degree = degrees(&nodes, &links);
    let hops = path_hops(start, &links);
    let candidates: Vec<Pos> = nodes
        .iter()
        .copied()
        .filter(|&n| n != start && degree[&n] == 1 && n.0 >= 2)
        .filter(|&n| {
            let d = hops.get(&n).copied().unwrap_or(usize::MAX);
            (targets.start_to_target.0..=targets.start_to_target.1).contains(&d)
        })
        .filter(|&n| !links.iter().any(|&(a, b, _)| (a, b) == ((n.0 - 2, n.1), n)))
        .collect();
    let &target = candidates.choose(rng)?;

    let dead_ends = degree.iter().filter(|&(&n, &d)| d == 1 && n != start).count();
    if !(targets.dead_ends.0..=targets.dead_ends.1).contains(&dead_ends) {
        return None;
    }

    Some(Layout { rows, cols, nodes, links, start, target, loop_sizes })
}

fn degrees(nodes: &[Pos], links: &[(Pos, Pos, Link)]) -> HashMap<Pos, usize> {
    let mut degree: HashMap<Pos, usize> = nodes.iter().map(|&n| (n, 0)).collect();
    for &(a, b, _) in links {
        *degree.get_mut(&a).unwrap() += 1;
        *degree.get_mut(&b).unwrap() += 1;
    }
    degree
}

/// Hops from `start` over path links only: the route a player without a
/// hammer can walk.
fn path_hops(start: Pos, links: &[(Pos, Pos, Link)]) -> HashMap<Pos, usize> {
    let mut adj: HashMap<Pos, Vec<Pos>> = HashMap::new();
    for &(a, b, kind) in links {
        if kind == Link::Path {
            adj.entry(a).or_default().push(b);
            adj.entry(b).or_default().push(a);
        }
    }
    let mut hops = HashMap::from([(start, 0)]);
    let mut queue = VecDeque::from([start]);
    while let Some(n) = queue.pop_front() {
        for &m in adj.get(&n).map(Vec::as_slice).unwrap_or(&[]) {
            if !hops.contains_key(&m) {
                hops.insert(m, hops[&n] + 1);
                queue.push_back(m);
            }
        }
    }
    hops
}

#[cfg(test)]
impl Layout {
    /// One character per map tile: `S` start, `A` target, `o` node, `-` and
    /// `|` paths, `R` a rock.
    pub(super) fn ascii(&self) -> String {
        let mut cells = vec![vec![' '; self.cols]; self.rows];
        for &(r, c) in &self.nodes {
            cells[r][c] = 'o';
        }
        cells[self.start.0][self.start.1] = 'S';
        cells[self.target.0][self.target.1] = 'A';
        for &(a, b, kind) in &self.links {
            let mid = ((a.0 + b.0) / 2, (a.1 + b.1) / 2);
            cells[mid.0][mid.1] = match (kind, a.0 == b.0) {
                (Link::Rock, _) => 'R',
                (Link::Path, true) => '-',
                (Link::Path, false) => '|',
            };
        }
        cells.iter().map(|row| row.iter().collect::<String>() + "\n").collect()
    }

    /// The figures the inventory reports for a map, on this layout.
    pub(super) fn summary(&self) -> String {
        let hops = path_hops(self.start, &self.links).get(&self.target).copied();
        format!(
            "nodes {}  loops {} {:?}  dead ends {}  junctions {}  rocks {}  start->airship {}",
            self.nodes.len(),
            self.links.len() + 1 - self.nodes.len(),
            self.loop_sizes,
            self.dead_ends(),
            self.junctions(),
            self.links.iter().filter(|l| l.2 == Link::Rock).count(),
            hops.map_or("-".to_string(), |h| h.to_string()),
        )
    }

    fn dead_ends(&self) -> usize {
        degrees(&self.nodes, &self.links)
            .iter()
            .filter(|&(&n, &d)| d == 1 && n != self.start)
            .count()
    }

    fn junctions(&self) -> usize {
        degrees(&self.nodes, &self.links).values().filter(|&&d| d >= 3).count()
    }
}

/// Prints each seed's layout as ASCII (`LAYOUT_SEEDS`, default 10). Past 20
/// seeds it prints only the tally, for comparing generator changes.
#[test]
#[ignore]
fn generated_layouts() {
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    let Some(rom) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes")
        .ok()
        .and_then(|d| crate::rom::Rom::from_bytes(&d).ok())
    else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    let maps = super::vanilla::load(&rom, false, false);
    let w1 = &maps[0].grid;
    let start = crate::randomize::rom_data::find_start(w1).expect("W1 has a start");
    println!("\nW1 start (anchor): {start:?}; targets: {W1_TARGETS:?}");
    println!("Vanilla W1: nodes 21  loops 2  dead ends 3  junctions 5  rocks 1  start->airship 9");

    let seeds: u64 = std::env::var("LAYOUT_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(10);
    let quiet = seeds > 20;
    let (mut failed, mut built) = (0, 0);
    let (mut loops, mut squares, mut loop_total, mut junctions) = (0, 0, 0, 0);
    for seed in 1..=seeds {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        match generate(w1.rows(), w1.cols, start, W1_TARGETS, 1000, &mut rng) {
            Some(layout) => {
                built += 1;
                loops += layout.loop_sizes.len();
                squares += layout.loop_sizes.iter().filter(|&&s| s == 4).count();
                loop_total += layout.loop_sizes.iter().sum::<usize>();
                junctions += layout.junctions();
                if !quiet {
                    println!("\n--- seed {seed}: {}", layout.summary());
                    print!("{}", layout.ascii());
                }
            }
            None => {
                failed += 1;
                if !quiet {
                    println!("\n--- seed {seed}: no layout in 1000 attempts");
                }
            }
        }
    }
    println!("\n{failed} of {seeds} seeds found no layout");
    if built > 0 && loops > 0 {
        println!(
            "loops: {loops}, squares {:.0}%, mean loop size {:.1}; mean junctions {:.1}",
            100.0 * squares as f64 / loops as f64,
            loop_total as f64 / loops as f64,
            junctions as f64 / built as f64,
        );
    }
}

/// The stranding check for generated World 1: across seeds, the builder must
/// leave the airship reachable from the start (locks open, the builder's own
/// convention, and rocks shut), and the whole pipeline must run without
/// panicking. `CENSUS_SEEDS` seeds, default 50.
///
/// ```sh
/// CENSUS_SEEDS=500 cargo test --release --lib generated_w1_is_completable -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn generated_w1_is_completable() {
    use crate::randomize::overworld::build::{BuildFlags, OverworldData, build};
    use crate::randomize::overworld::helpers::find_target;
    use crate::randomize::overworld::map_walker::walk_map;
    use crate::randomize::overworld::node_catalog::NodeCatalog;
    use crate::randomize::overworld::pickup::{PickupFlags, pick_up};
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    let Ok(raw) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    let rom = crate::rom::Rom::from_bytes(&raw).unwrap();
    let seeds: u64 = std::env::var("CENSUS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(50);

    // The builder alone, many seeds.
    let mut generated = 0;
    let (mut with_river, mut with_gap, mut with_moat) = (0, 0, 0);
    for seed in 0..seeds {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut maps = super::vanilla::load(&rom, false, false);
        let Some(w1) = world_map(&maps[0], 0, &mut rng) else { continue };
        maps[0] = w1;
        generated += 1;
        let catalog = NodeCatalog::from_maps(&rom, maps, false);
        let flags = PickupFlags {
            shuffle_spade_games: true,
            shuffle_toad_houses: true,
            shuffle_hammer_bros: true,
        };
        let pickup = pick_up(&rom, &catalog, flags);
        let result = build(
            &rom,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            BuildFlags {
                shuffle_toad_houses: true,
                shuffle_hammer_bros: true,
                ..Default::default()
            },
        );
        let w1 = &result.worlds[0];
        let bridges: Vec<Pos> = (0..w1.grid.rows())
            .flat_map(|r| (0..w1.grid.cols).map(move |c| (r, c)))
            .filter(|&(r, c)| w1.grid.get(r, c) == BRIDGE)
            .collect();
        let crossings: Vec<Pos> = (0..w1.grid.rows())
            .flat_map(|r| (0..w1.grid.cols).map(move |c| (r, c)))
            .filter(|&(r, c)| w1.grid.get(r, c) == CROSSING)
            .collect();
        with_river += usize::from(!bridges.is_empty() || !crossings.is_empty());
        with_gap += usize::from(w1.locks.iter().any(|l| bridges.contains(&l.pos)));
        assert!(
            !w1.locks.iter().any(|l| crossings.contains(&l.pos)),
            "seed {seed}: a lock took a \\$BA crossing, which has no water-gap form",
        );
        let castle = find_target(&w1.grid, 0).expect("generated W1 has an airship");
        with_moat += usize::from(
            bridges.contains(&(castle.0, castle.1 - 1))
                || bridges.contains(&(castle.0, castle.1 + 1)),
        );
        let start = crate::randomize::rom_data::find_start(&w1.grid);
        let target = find_target(&w1.grid, 0).expect("generated W1 has an airship");
        assert!(
            walk_map(&w1.grid, &w1.pipe_pairs, start, 0).nodes.contains(&target),
            "seed {seed}: generated W1's airship is unreachable from the start",
        );
    }
    println!("builder: {generated} of {seeds} seeds generated a W1, all completable");
    println!(
        "  {with_river} have water crossing a path, {with_moat} a moat; in {with_gap} the builder locked a bridge (a water gap)"
    );

    // The whole pipeline, fewer seeds: the writer and every later stage.
    let options = crate::Options { generated_worlds: 1, palettes: false, ..Default::default() };
    for seed in 0..seeds.min(20) {
        crate::generate_patched_rom(&raw, seed, &options, None)
            .unwrap_or_else(|e| panic!("seed {seed}: pipeline failed: {e}"));
    }
    println!("pipeline: {} seeds ran end to end", seeds.min(20));
}

/// The same seed must draw the same map. Each `HashSet` iterates in its own
/// order, so a choice made from one shows up here as two different maps.
#[test]
fn generated_w1_is_deterministic() {
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    let Some(rom) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes")
        .ok()
        .and_then(|d| crate::rom::Rom::from_bytes(&d).ok())
    else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    let vanilla = super::vanilla::load(&rom, false, false);
    for seed in 0..20 {
        let draw = || world_map(&vanilla[0], 0, &mut ChaCha8Rng::seed_from_u64(seed)).unwrap();
        let (a, b) = (draw(), draw());
        assert_eq!(a.grid.tiles, b.grid.tiles, "seed {seed}: two draws differ");
        assert_eq!(a.entry_pos, b.entry_pos, "seed {seed}: two draws place entries differently");
    }
}

/// Generated W1 as drawn tiles, before the builder places anything:
/// `.` bush, ` ` clearing, `^` rock (decorative), `~` water, `=` bridge, `+` path over water, `-` `|` paths, `R` breakable
/// rock, `o` node, `S` start, `A` airship, `a` castle top, `#` frame.
///
/// ```sh
/// cargo test --release --lib generated_w1_tiles -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn generated_w1_tiles() {
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    let Some(rom) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes")
        .ok()
        .and_then(|d| crate::rom::Rom::from_bytes(&d).ok())
    else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    let vanilla = super::vanilla::load(&rom, false, false);
    let seeds: u64 = std::env::var("LAYOUT_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(6);
    for seed in 1..=seeds {
        let map = world_map(&vanilla[0], 0, &mut ChaCha8Rng::seed_from_u64(seed)).unwrap();
        println!("\n--- seed {seed}");
        for r in 0..map.grid.rows() {
            let row: String = (0..map.grid.cols)
                .map(|c| match map.grid.get(r, c) {
                    FILL => '.',
                    CLEARING => ' ',
                    DECO_ROCK => '^',
                    0x45 | 0x49 => '-',
                    0x46 => '|',
                    0x51 | 0x52 => 'R',
                    0x02 => '#',
                    0x82..=0xA9 => '~',
                    BRIDGE => '=',
                    CROSSING => '+',
                    TILE_START => 'S',
                    0xC9 => 'A',
                    0xC8 => 'a',
                    _ => 'o',
                })
                .collect();
            println!("  {row}");
        }
    }
}

/// Every picture in [`WATER_TILES`] matches what the tile's graphics draw.
/// A map tile is four 8x8 CHR pieces, listed at `0x18010` / `0x18110` /
/// `0x18210` / `0x18310` + tile as upper-left, lower-left, upper-right,
/// lower-right (column by column). The map's background CHR, frame 0, is page
/// `$14` onward (`0x40010 + 0x14 * 0x400`, 16 bytes a piece, two bit planes).
/// Colour 1 is water and colour 3 land; each of the nine points is the
/// majority of the two in a 3x3 block of pixels.
#[test]
fn water_pictures_match_the_graphics() {
    let Ok(rom) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    let chr = 0x40010 + 0x14 * 0x400;
    let pixel = |piece: u8, r: usize, c: usize| {
        let at = chr + piece as usize * 16;
        ((rom[at + r] >> (7 - c)) & 1) | (((rom[at + r + 8] >> (7 - c)) & 1) << 1)
    };
    for &(tile, want) in &WATER_TILES {
        let t = tile as usize;
        let (ul, ll, ur, lr) =
            (rom[0x18010 + t], rom[0x18110 + t], rom[0x18210 + t], rom[0x18310 + t]);
        let at = |r: usize, c: usize| {
            let piece = match (r < 8, c < 8) {
                (true, true) => ul,
                (false, true) => ll,
                (true, false) => ur,
                (false, false) => lr,
            };
            pixel(piece, r % 8, c % 8)
        };
        let point = |r0: usize, c0: usize| {
            let block: Vec<u8> = (r0..r0 + 3)
                .flat_map(|r| (c0..c0 + 3).map(move |c| (r, c)))
                .map(|(r, c)| at(r, c))
                .collect();
            let water = block.iter().filter(|&&p| p == 1).count();
            let land = block.iter().filter(|&&p| p == 3).count();
            if water > land { 'W' } else { 'L' }
        };
        let rows: Vec<String> =
            [0, 6, 13].iter().map(|&r| [0, 6, 13].iter().map(|&c| point(r, c)).collect()).collect();
        assert_eq!(rows.join("/"), want, "${tile:02X}");
    }
}
