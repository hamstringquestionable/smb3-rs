//! The generated map source, layout stage: a world's nodes and links, with no
//! tiles yet.
//!
//! A layout is judged as ASCII before anything draws tiles or touches the ROM
//! (`docs/procedural_maps_design.md`, milestone 3):
//!
//! ```sh
//! cargo test --release --lib generated_layouts -- --ignored --nocapture
//! ```
//!
//! Test-only until the renderer exists; nothing here reaches a seed.

use std::collections::{HashMap, HashSet, VecDeque};

use rand::Rng;
use rand::seq::IndexedRandom;

use crate::randomize::rom_data::Pos;

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
    let on_lattice: HashSet<Pos> = lattice(rows, cols, start).into_iter().collect();

    // 1. A random tree grown from the start until it has the node count. Most
    //    steps extend the newest node that can still grow, which draws long
    //    corridors; the rest branch from any node. Picking from any node every
    //    time grows a blob around the start instead.
    let mut nodes = vec![start];
    let mut in_tree: HashSet<Pos> = HashSet::from([start]);
    let mut tree: Vec<(Pos, Pos)> = Vec::new();
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
    let loops = rng.random_range(targets.loops.0..=targets.loops.1);
    let extra: Vec<(Pos, Pos)> = spare.choose_multiple(rng, loops).copied().collect();
    if extra.len() < loops {
        return None;
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

    Some(Layout { rows, cols, nodes, links, start, target })
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
        let degree = degrees(&self.nodes, &self.links);
        let dead = degree.iter().filter(|&(&n, &d)| d == 1 && n != self.start).count();
        let junctions = degree.values().filter(|&&d| d >= 3).count();
        let rocks = self.links.iter().filter(|l| l.2 == Link::Rock).count();
        let loops = self.links.len() + 1 - self.nodes.len();
        let hops = path_hops(self.start, &self.links).get(&self.target).copied();
        format!(
            "nodes {}  loops {loops}  dead ends {dead}  junctions {junctions}  rocks {rocks}  \
             start->airship {}",
            self.nodes.len(),
            hops.map_or("-".to_string(), |h| h.to_string()),
        )
    }
}

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
    let mut failed = 0;
    for seed in 1..=seeds {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        match generate(w1.rows(), w1.cols, start, W1_TARGETS, 1000, &mut rng) {
            Some(layout) => {
                println!("\n--- seed {seed}: {}", layout.summary());
                print!("{}", layout.ascii());
            }
            None => {
                failed += 1;
                println!("\n--- seed {seed}: no layout in 1000 attempts");
            }
        }
    }
    println!("\n{failed} of {seeds} seeds found no layout");
}
