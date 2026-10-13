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
//! and rocks from the links, each pointer entry on a node showing its own
//! tile, then water (lakes, a river, a moat) and decoration clusters
//! (`docs/procedural_maps_design.md`, milestone 3). Reached only through the
//! hidden `generated_worlds` option.

use std::collections::{HashMap, HashSet, VecDeque};

use rand::Rng;
use rand::seq::{IndexedRandom, SliceRandom};

use crate::randomize::overworld::pickup::blank_tile_from_neighbors;
use crate::randomize::rom_data::{
    AIRSHIP_ENTRIES, BRIDGE_TILE, Grid, Pos, ROCK_BREAK_H, ROCK_BREAK_V, TILE_START,
    VALID_BLANK_TILES,
};

use super::WorldMap;
use crate::randomize::overworld::start_airship_swap::start_camera_col;

/// What a link between two neighbouring nodes is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Link {
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
struct Layout {
    rows: usize,
    cols: usize,
    /// Every node, start and target included.
    nodes: Vec<Pos>,
    /// Links between nodes two tiles apart, each pair stored once.
    links: Vec<(Pos, Pos, Link)>,
    start: Pos,
    /// The airship (the castle in W8).
    target: Pos,
    /// Each loop link's length: the nodes around the loop it closed when it
    /// was added. 4 is a square, which gives the player no real choice.
    loop_sizes: Vec<usize>,
    /// The secret area's nodes (see [`grow_pocket`]); empty without one.
    pocket: Vec<Pos>,
    /// The nodes of each island the start's cannot walk to, joined later by
    /// the builder's pipes; empty without a split. The airship is on the
    /// first.
    islands: Vec<Vec<Pos>>,
    /// Each dock and the water tile beside it the canoe boards from; the
    /// canoe parks beside the first, on the start's island.
    docks: Vec<(Pos, Pos)>,
}

/// The targets a layout must meet, or it is thrown away and redrawn.
#[derive(Clone, Copy, Debug)]
struct Targets {
    /// The world's pointer-entry count: one node per entry. Spare nodes hold
    /// Hammer Bro levels, as in vanilla.
    nodes: usize,
    /// Links added on top of a tree. Each one is exactly one loop.
    loops: (usize, usize),
    dead_ends: (usize, usize),
    /// Fewest hops from the start to the target, on paths alone.
    start_to_target: (usize, usize),
    rocks: (usize, usize),
    /// A secret area's node count, counted in `nodes`: a small tree in the
    /// map's last scroll view with one way in (see [`grow_pocket`]).
    pocket: Option<(usize, usize)>,
    /// The world's pipe pairs (`VANILLA_PIPE_PAIRS`). With `n` of them the
    /// layout splits into islands `n - 1` times, plus once more
    /// [`EXTRA_SPLIT`] of the time; the builder's connectivity phase joins
    /// each island with a pair and leaves any spare for routing.
    pipe_pairs: usize,
    /// Each island past the start's gets this share, in percent, of the nodes
    /// not yet dealt: (25, 50) makes islands of a size, a low range one big
    /// mainland with small satellites.
    island_share: (usize, usize),
    /// Islands reached by canoe, on top of the pipe splits: each gets a dock
    /// (`$4B`, `TILE_DOCK`), and the start's island gets `mainland_docks` more.
    dock_islands: (usize, usize),
    mainland_docks: (usize, usize),
}

/// How often a world with pipes splits once more than its pipe pairs less
/// one (user, 2026-10-13).
const EXTRA_SPLIT: f64 = 0.25;

/// The proposal agreed for W1 on 2026-10-11. Vanilla W1 is 21 nodes, 2
/// loops, 3 dead ends, 9 hops, 1 rock shortcut.
const W1_TARGETS: Targets = Targets {
    nodes: 21,
    loops: (3, 5),
    dead_ends: (2, 5),
    start_to_target: (7, 11),
    rocks: (0, 2),
    pocket: None,
    pipe_pairs: 0,
    island_share: (25, 50),
    dock_islands: (0, 0),
    mainland_docks: (0, 0),
};

/// W2, first guesses around vanilla: 47 nodes, 3 loops, 7 dead ends, 15 hops,
/// and the secret area behind the (0,21) rock, 6 nodes. The pocket has no
/// rock at its mouth (user, 2026-10-13); rocks go elsewhere.
const W2_TARGETS: Targets = Targets {
    nodes: 47,
    loops: (2, 4),
    dead_ends: (5, 9),
    start_to_target: (13, 17),
    rocks: (0, 2),
    pocket: Some((5, 7)),
    pipe_pairs: 1,
    island_share: (25, 50),
    dock_islands: (0, 0),
    mainland_docks: (0, 0),
};

/// W3, first guesses (user, 2026-10-13): one big mainland and small islands,
/// many of them reached by canoe. Vanilla W3 is 52 nodes, 4 loops, 16 dead
/// ends, walk pieces 42/5/3/2, two canoe islands.
const W3_TARGETS: Targets = Targets {
    nodes: 52,
    loops: (2, 5),
    dead_ends: (8, 22),
    start_to_target: (13, 20),
    rocks: (0, 2),
    pocket: None,
    pipe_pairs: 3,
    island_share: (4, 10),
    dock_islands: (3, 5),
    mainland_docks: (1, 2),
};

/// How one world is generated: its layout, then its scenery, step by step in
/// order, then its ground in every cell nothing took. A new world is a new
/// recipe; a new kind of scenery is a new [`Step`] any recipe can use.
struct Recipe {
    layout: Targets,
    /// Every cell nothing else takes. (Generation marks empty cells `FILL`
    /// throughout and swaps this in last.)
    ground: u8,
    scenery: &'static [Step],
}

/// One kind of scenery. Each sees a cell as free only while it is still
/// `FILL`; water marks its cells `WATER_PENDING` as it is added and is drawn
/// in one pass after the last step, so water that meets water joins with a
/// proper shore.
#[derive(Clone, Copy)]
enum Step {
    /// In each big open region, half the time, a lake ([`grow_lake`]).
    Lakes,
    /// Most of the time a river joining two lakes ([`add_river`]).
    River,
    /// Sometimes a moat beside the castle ([`add_moat`]).
    Moat,
    /// The pocket strip's open space as one lake, `island` on an island at
    /// its deepest cell ([`add_oasis`]).
    Oasis { island: u8 },
    /// `count` one-tile `pool`s, each with a `palm` or two beside it
    /// ([`add_pools`]).
    Pools { pool: u8, palm: u8, count: (usize, usize) },
    /// Patches of `tile` grown in the bigger open regions ([`add_patches`]).
    Patches { tile: u8 },
    /// Clusters in each open region ([`decorate`]): rock, `rock_share` of the
    /// time, else `other`.
    Clusters { other: u8, rock_share: f64 },
    /// A few single tiles off the paths ([`scatter`]).
    Scatter { tile: u8, count: (usize, usize) },
    /// Water as the ground ([`add_sea`]): `water_nodes` of the nodes that can
    /// be drawn in the sea, land on each land node's side `margin` of the
    /// time, grown by `land` of the open cells, and bush islets; the rest is
    /// sea. First in a recipe, so the steps after it decorate the land.
    Sea { margin: f64, land: f64, water_nodes: f64 },
}

const W1_RECIPE: Recipe = Recipe {
    layout: W1_TARGETS,
    ground: BUSH,
    // Vanilla W1 has 9 rock cells to 11 clearing cells.
    scenery: &[
        Step::Lakes,
        Step::River,
        Step::Moat,
        Step::Clusters { other: CLEARING, rock_share: 0.4 },
    ],
};

/// Plain sand; dotted sand in patches; palms where W1 has clearings; one big
/// oasis where vanilla draws its III (user, 2026-10-13).
const W2_RECIPE: Recipe = Recipe {
    layout: W2_TARGETS,
    ground: CLEARING,
    // One-tile pieces first: they need cells touching no path or node, which
    // the crowded main map has few of, so the patches and clusters fill
    // around them rather than leaving them nowhere to go.
    scenery: &[
        Step::Oasis { island: PALM },
        Step::Pools { pool: POOL, palm: PALM, count: (1, 3) },
        Step::Scatter { tile: PYRAMID, count: PYRAMIDS },
        Step::Scatter { tile: QUICKSAND, count: (1, 3) },
        Step::Scatter { tile: TALL_ROCK, count: (2, 4) },
        Step::Scatter { tile: RUBBLE, count: (1, 3) },
        Step::Scatter { tile: ALT_RUBBLE, count: (1, 2) },
        Step::Scatter { tile: PLANT, count: (3, 6) },
        Step::Scatter { tile: DUNE, count: (4, 10) },
        Step::Patches { tile: DUNE },
        Step::Clusters { other: PALM, rock_share: 0.4 },
    ],
};

/// W3: water first (user, 2026-10-13). The sea is the ground; land grows
/// around the paths, with bush (W3's boulders) on islets and in clusters, and
/// a few palms and pools, as vanilla has.
const W3_RECIPE: Recipe = Recipe {
    layout: W3_TARGETS,
    ground: CLEARING,
    scenery: &[
        Step::Sea { margin: 0.25, land: 0.04, water_nodes: 0.5 },
        Step::Clusters { other: BUSH, rock_share: 0.0 },
        Step::Scatter { tile: PALM, count: (1, 3) },
        Step::Scatter { tile: POOL, count: (0, 2) },
    ],
};

/// The recipe for `world_idx`, for the worlds that have one.
fn recipe(world_idx: usize) -> Option<&'static Recipe> {
    match world_idx {
        0 => Some(&W1_RECIPE),
        1 => Some(&W2_RECIPE),
        2 => Some(&W3_RECIPE),
        _ => None,
    }
}

/// Layouts drawn before a world gives up and keeps vanilla.
const ATTEMPTS: usize = 1000;

/// How often the tree grows from its newest node rather than from any node.
/// A first guess, to be judged on the ASCII layouts.
const EXTEND_NEWEST: f64 = 0.75;

/// How many of the farthest-apart candidate pairs a loop link is picked from.
const FARTHEST_FEW: usize = 3;

/// Most squares (four nodes in a ring, which give no real route choice) a
/// layout may have. Not banned outright: one now and then reads fine.
const MAX_SQUARES: usize = 1;

/// Draw layouts until one meets `targets`, or give up after [`ATTEMPTS`].
///
/// The start goes on, or one node from, `anchor` (the vanilla start) or
/// `airship` (the vanilla airship), a coin picking which: the two places
/// vanilla and the Start<->Airship swap start a player. Never where the
/// starting camera ([`start_camera_col`]) shows the pocket's strip.
/// It is picked once, so a start that is harder to fill around is not drawn
/// less often. `anchor` also sets the lattice's parity.
fn generate<R: Rng>(
    world_idx: usize,
    rows: usize,
    cols: usize,
    anchor: Pos,
    airship: Pos,
    targets: Targets,
    rng: &mut R,
) -> Option<Layout> {
    let cells = lattice(rows, cols, anchor);
    let near = if rng.random::<bool>() { anchor } else { airship };
    let around: Vec<Pos> = cells
        .iter()
        .copied()
        .filter(|&p| manhattan(p, near) <= 2)
        .filter(|p| targets.pocket.is_none() || start_camera_col(world_idx, p.1) + 16 <= POCKET_COL)
        .collect();
    let start = *around.choose(rng)?;
    (0..ATTEMPTS).find_map(|_| try_once(rows, cols, anchor, start, targets, rng))
}

/// Marks an empty cell while a map is generated: every interior cell starts
/// as it, and what water and decoration leave becomes the recipe's ground at
/// the end. No map uses the byte, so a recipe can use any real tile, the bush
/// included, as scenery.
const FILL: u8 = 0xFF;
/// The dancing bush (`TILE_DANCINGBUSH`): vanilla W1 fills its empty space
/// with it, and W3's rock islets are drawn with it.
const BUSH: u8 = 0xB4;
/// `TILE_DOCK`: the canoe is boarded from here, and the only tile it lands at.
const DOCK: u8 = 0x4B;

/// How often a horizontal path over a sea between two land nodes is a
/// bridge, which can become a water gap; otherwise `$B9`, a path over water.
/// Vanilla W3 has 5 bridges and 8 horizontal paths over water.
const BRIDGE_SHARE: f64 = 0.4;

/// Paths over water (`smb3_rom_reference.md`, "Paths over water"), by axis
/// and by which ends are water nodes: `[land-land, land-water, water-land,
/// water-water]`, the first end being the left or upper one. Land-land
/// horizontal is a bridge or `$B9` ([`BRIDGE_SHARE`]); vertical is `$BA`
/// ([`CROSSING`]).
const HORZ_OVER_WATER: [u8; 4] = [0xB9, 0xB7, 0xB8, 0xAC];
const VERT_OVER_WATER: [u8; 4] = [CROSSING, 0xAA, 0xAB, 0xB0];

/// Bush islets out in open water per sea map.
const ISLETS: (usize, usize) = (2, 4);
/// The owner [`add_sea`] gives an islet, and sea trimmed back to land: no
/// island grows from it.
const ISLET: usize = usize::MAX;

/// Water first (W3).
///
/// **Water nodes.** `water_nodes` of the nodes that can be are drawn in the
/// sea, not on land: a node with a path from its left or from above (the path
/// the pickup's island blank, `$AE` `$B5` `$AF` `$B6`, keys off), and not the
/// start, the airship or a dock. Its sides and the sides of its paths stay
/// sea, and its paths take the over-water tile for their ends
/// ([`HORZ_OVER_WATER`], [`VERT_OVER_WATER`]), so a run of them reads as open
/// water while the walker sees one island.
///
/// **Land** is what is drawn so far (paths, nodes, the castle top), each side
/// of a land node `margin` of the time, then `land` of the open cells grown
/// on from the shore, then a few one-cell islets of bush out in the open.
/// Every other cell is sea. Land belongs to the island it grew from and never
/// touches another island's, so separate islands always show water between;
/// it never takes a cell that must stay sea (a dock's water, a water node's
/// sides), nor one whose loss would cut one dock's water off from another's.
///
/// A path with sea both sides takes its over-water tile. Sea cells with no
/// tile for their picture become land; `false` if that takes a cell that must
/// stay sea or cuts the docks apart (the draw is retried).
fn add_sea<R: Rng>(
    grid: &mut Grid,
    water: &mut Water,
    layout: &Layout,
    (margin, land, water_nodes): (f64, f64, f64),
    rng: &mut R,
) -> bool {
    let (rows, cols) = (grid.rows(), grid.cols);
    let inside = |(r, c): Pos| r < rows && c >= 1 && c + 1 < cols;
    let around =
        |(r, c): Pos| [(r.wrapping_sub(1), c), (r + 1, c), (r, c.wrapping_sub(1)), (r, c + 1)];
    let mid = |a: Pos, b: Pos| ((a.0 + b.0) / 2, (a.1 + b.1) / 2);
    // A link's two cells beside it, across its axis.
    let beside = |a: Pos, b: Pos| {
        let (r, c) = mid(a, b);
        if a.0 == b.0 { [(r.wrapping_sub(1), c), (r + 1, c)] } else { [(r, c - 1), (r, c + 1)] }
    };
    let cells: Vec<Pos> = (0..rows).flat_map(|r| (1..cols - 1).map(move |c| (r, c))).collect();
    let docks: Vec<Pos> = layout.docks.iter().map(|d| d.1).collect();
    water.sea_bounds = Some((rows, cols));

    let fixed =
        |n: Pos| n == layout.start || n == layout.target || layout.docks.iter().any(|d| d.0 == n);
    let joined_from_above_or_left =
        |n: Pos| layout.links.iter().any(|&(_, b, kind)| kind == Link::Path && b == n);
    let on_rock =
        |n: Pos| layout.links.iter().any(|&(a, b, k)| k == Link::Rock && (a == n || b == n));
    let mut wet_nodes: HashSet<Pos> = layout
        .nodes
        .iter()
        .copied()
        .filter(|&n| !fixed(n) && !on_rock(n) && joined_from_above_or_left(n))
        .filter(|_| rng.random_bool(water_nodes))
        .collect();
    // The cells a water node needs kept sea: its sides, and its paths' sides.
    let empty: HashSet<Pos> =
        cells.iter().copied().filter(|&(r, c)| grid.get(r, c) == FILL).collect();
    let needs = |n: Pos| {
        let mut cells: Vec<Pos> = around(n).into_iter().filter(|p| empty.contains(p)).collect();
        for &(a, b, _) in layout.links.iter().filter(|&&(a, b, _)| a == n || b == n) {
            cells.extend(beside(a, b).into_iter().filter(|&p| inside(p)));
        }
        cells
    };
    let keep_for = |wet_nodes: &HashSet<Pos>| {
        let mut keep: HashSet<Pos> = docks.iter().copied().collect();
        for &n in wet_nodes {
            keep.extend(needs(n));
        }
        keep
    };
    let mut keep = keep_for(&wet_nodes);
    let nodes_before = water.nodes.len();
    water.nodes.extend(&wet_nodes);

    // Land, keyed by the island it belongs to (0 the start's, k + 1 the
    // layout's k-th), so islands never grow into each other: no land is
    // placed touching another island's, diagonals included.
    let island_of =
        |p: Pos| layout.islands.iter().position(|i| i.contains(&p)).map_or(0, |k| k + 1);
    let mut solid: HashMap<Pos, usize> = HashMap::new();
    for &(r, c) in cells.iter().filter(|&&(r, c)| grid.get(r, c) != FILL) {
        let nearest = layout.nodes.iter().copied().min_by_key(|&n| manhattan(n, (r, c)));
        solid.insert((r, c), nearest.map_or(0, island_of));
    }

    // Whether every dock's water is still one sea, with `solid` as land.
    let joined = |solid: &HashMap<Pos, usize>| {
        let Some(&first) = docks.first() else { return true };
        let mut seen = HashSet::from([first]);
        let mut queue = vec![first];
        while let Some(p) = queue.pop() {
            for n in around(p) {
                if inside(n) && !solid.contains_key(&n) && seen.insert(n) {
                    queue.push(n);
                }
            }
        }
        docks.iter().all(|w| seen.contains(w))
    };
    let add = |solid: &mut HashMap<Pos, usize>, (r, c): Pos, island: usize| {
        let touches_another = (r.saturating_sub(1)..=r + 1).any(|y| {
            (c.saturating_sub(1)..=c + 1).any(|x| solid.get(&(y, x)).is_some_and(|&o| o != island))
        });
        if !inside((r, c))
            || keep.contains(&(r, c))
            || solid.contains_key(&(r, c))
            || touches_another
        {
            return false;
        }
        solid.insert((r, c), island);
        let kept = joined(solid);
        if !kept {
            solid.remove(&(r, c));
        }
        kept
    };

    for &n in layout.nodes.iter().filter(|n| !wet_nodes.contains(n)) {
        for p in around(n) {
            if rng.random_bool(margin) {
                add(&mut solid, p, island_of(n));
            }
        }
    }
    let target = (cells.iter().filter(|p| !solid.contains_key(p)).count() as f64 * land) as usize;
    let (mut grown, mut tries) = (0, 0);
    while grown < target && tries < 4 * target {
        tries += 1;
        let shore: Vec<(Pos, usize)> = cells
            .iter()
            .copied()
            .filter(|p| !solid.contains_key(p))
            .filter_map(|p| {
                around(p)
                    .iter()
                    .filter(|n| !wet_nodes.contains(n))
                    .find_map(|n| solid.get(n))
                    .filter(|&&o| o != ISLET)
                    .map(|&o| (p, o))
            })
            .collect();
        let Some(&(p, island)) = shore.choose(rng) else { break };
        grown += usize::from(add(&mut solid, p, island));
    }
    let mut islets = Vec::new();
    for _ in 0..rng.random_range(ISLETS.0..=ISLETS.1) {
        let open: Vec<Pos> = cells
            .iter()
            .copied()
            .filter(|&(r, c)| {
                (r.saturating_sub(2)..=r + 2)
                    .all(|y| (c.saturating_sub(2)..=c + 2).all(|x| !solid.contains_key(&(y, x))))
            })
            .collect();
        let Some(&p) = open.choose(rng) else { break };
        if add(&mut solid, p, ISLET) {
            islets.push(p);
        }
    }

    // The sea, trimmed until every cell of it has a tile. Whether a
    // land-to-land horizontal path over it is a bridge is decided once per
    // link.
    let bridged: Vec<bool> = layout.links.iter().map(|_| rng.random_bool(BRIDGE_SHARE)).collect();
    let mut sea: Vec<Pos> = cells.iter().copied().filter(|p| !solid.contains_key(p)).collect();
    let paths_before = water.paths.len();
    loop {
        if !joined(&solid) {
            return false;
        }
        let in_sea: HashSet<Pos> = sea.iter().copied().collect();
        let (mut bridges, mut crossings) = (Vec::new(), Vec::new());
        water.paths.truncate(paths_before);
        for (&(a, b, kind), &bridge) in layout.links.iter().zip(&bridged) {
            if kind != Link::Path || !beside(a, b).iter().all(|p| in_sea.contains(p)) {
                continue;
            }
            let ends =
                usize::from(wet_nodes.contains(&a)) * 2 + usize::from(wet_nodes.contains(&b));
            let cell = mid(a, b);
            match (a.0 == b.0, ends) {
                (true, 0) if bridge => bridges.push(cell),
                (false, 0) => crossings.push(cell),
                (true, _) => water.paths.push((cell, HORZ_OVER_WATER[ends])),
                (false, _) => water.paths.push((cell, VERT_OVER_WATER[ends])),
            }
        }
        if water.try_add(grid, &sea, &bridges, &crossings) {
            break;
        }
        let before = (water.cells.len(), water.bridges.len(), water.crossings.len());
        water.cells.extend(&sea);
        water.bridges.extend(&bridges);
        water.crossings.extend(&crossings);
        let bad: Vec<Pos> = water.undrawable().into_iter().filter(|p| in_sea.contains(p)).collect();
        water.cells.truncate(before.0);
        water.bridges.truncate(before.1);
        water.crossings.truncate(before.2);
        if bad.is_empty() || bad.iter().any(|p| docks.contains(p)) {
            return false;
        }
        // A cell a water node needs kept sea has no tile: that node goes back
        // to land, and the cell can then be trimmed like any other.
        let stuck: Vec<Pos> = bad.iter().copied().filter(|p| keep.contains(p)).collect();
        if !stuck.is_empty() {
            let demoted: Vec<Pos> = wet_nodes
                .iter()
                .copied()
                .filter(|&n| needs(n).iter().any(|p| stuck.contains(p)))
                .collect();
            for n in &demoted {
                wet_nodes.remove(n);
            }
            keep = keep_for(&wet_nodes);
            water.nodes.truncate(nodes_before);
            water.nodes.extend(&wet_nodes);
            continue;
        }
        sea.retain(|p| !bad.contains(p));
        solid.extend(bad.into_iter().map(|p| (p, ISLET)));
    }
    for (r, c) in islets {
        grid.set(r, c, BUSH);
    }
    true
}
/// Path and rock tiles for a link, by direction: `(horizontal, vertical)`.
const PATH: (u8, u8) = (0x45, 0x46);
const ROCK: (u8, u8) = (ROCK_BREAK_H, ROCK_BREAK_V);

/// Generate a new map for `world_idx`, drawn over `vanilla` (that world's
/// vanilla map, which supplies the frame, the start, and each entry's tile).
/// `None` if no draw succeeded in [`DRAWS`] tries; the caller keeps vanilla then.
///
/// Worlds 1 to 3 so far.
pub(crate) fn world_map<R: Rng>(
    vanilla: &WorldMap,
    world_idx: usize,
    rng: &mut R,
) -> Option<WorldMap> {
    (0..DRAWS).find_map(|_| draw_once(vanilla, world_idx, rng))
}

/// Whole maps drawn before a world gives up and keeps vanilla. A draw can
/// fail late (W3's sea can trim its docks apart); W1 and W2 succeed first
/// time, so their draws are unchanged by the retries.
const DRAWS: usize = 20;

/// One attempt at [`world_map`].
fn draw_once<R: Rng>(vanilla: &WorldMap, world_idx: usize, rng: &mut R) -> Option<WorldMap> {
    let recipe = recipe(world_idx)?;
    let targets = recipe.layout;
    let old = &vanilla.grid;
    let on_grid = |p: Pos| p.0 < old.rows() && p.1 < old.cols;
    let start_idx = (0..vanilla.entry_pos.len()).find(|&i| {
        on_grid(vanilla.entry_pos[i]) && {
            let p = vanilla.entry_pos[i];
            old.get(p.0, p.1) == TILE_START
        }
    })?;
    let vanilla_start = vanilla.entry_pos[start_idx];
    let airship_idx =
        (0..vanilla.entry_pos.len()).find(|&i| AIRSHIP_ENTRIES.contains(&(world_idx, i)))?;
    let layout = generate(
        world_idx,
        old.rows(),
        old.cols,
        vanilla_start,
        vanilla.entry_pos[airship_idx],
        targets,
        rng,
    )?;
    let start = layout.start;

    // The frame (first and last column) stays as vanilla drew it; everything
    // else starts as bush. A start in the first lattice column gets vanilla's
    // entrance stub, the path running off the edge of the map.
    let mut grid = old.clone();
    grid.vanilla_layout = false;
    for r in 0..grid.rows() {
        for c in 1..grid.cols - 1 {
            grid.set(r, c, FILL);
        }
    }
    if start.1 == vanilla_start.1 {
        grid.set(start.0, start.1 - 1, old.get(vanilla_start.0, vanilla_start.1 - 1));
    }
    for &(a, b, kind) in &layout.links {
        let (horiz, vert) = if kind == Link::Rock { ROCK } else { PATH };
        grid.set((a.0 + b.0) / 2, (a.1 + b.1) / 2, if a.0 == b.0 { horiz } else { vert });
    }

    // Entries onto nodes: the start and the airship go where the layout put
    // them, and docks take vanilla's dock entries, then spare Hammer Bro levels
    // (drawn as docks below). The rest are dealt at random onto the remaining
    // nodes. Entries off the grid (unused rows in the pointer table) stay where
    // they are.
    let dock_nodes: Vec<Pos> = layout.docks.iter().map(|d| d.0).collect();
    let mut free: Vec<Pos> = layout
        .nodes
        .iter()
        .copied()
        .filter(|&n| n != start && n != layout.target && !dock_nodes.contains(&n))
        .collect();
    free.shuffle(rng);
    let mut entry_pos = vanilla.entry_pos.clone();
    entry_pos[start_idx] = start;
    entry_pos[airship_idx] = layout.target;
    let vanilla_tile = |i: usize| {
        let p = vanilla.entry_pos[i];
        on_grid(p).then(|| old.get(p.0, p.1))
    };
    let dealt_elsewhere = |i: usize| i == start_idx || i == airship_idx;
    let pinned_entries: Vec<usize> = (0..entry_pos.len())
        .filter(|&i| vanilla_tile(i) == Some(DOCK))
        .chain(
            (0..entry_pos.len())
                .filter(|&i| !dealt_elsewhere(i))
                .filter(|&i| vanilla_tile(i).is_some_and(|t| VALID_BLANK_TILES.contains(&t))),
        )
        .collect();
    if pinned_entries.iter().filter(|&&i| vanilla_tile(i) == Some(DOCK)).count() > dock_nodes.len()
    {
        return None; // A vanilla dock entry with no dock would draw a stray dock.
    }
    // Every island but the start's also takes a spare Hammer Bro level, on
    // its first free node: the pickup always lifts those, so the builder has
    // somewhere to land the island's pipe. An island of only fixed nodes (the
    // airship and a Toad House the flags keep) could never be joined.
    let anchors: Vec<Pos> = layout
        .islands
        .iter()
        .filter_map(|island| island.iter().copied().find(|n| free.contains(n)))
        .collect();
    free.retain(|n| !anchors.contains(n));
    let placed: Vec<usize> =
        pinned_entries.into_iter().take(dock_nodes.len() + anchors.len()).collect();
    if placed.len() < dock_nodes.len() + anchors.len() {
        return None;
    }
    for (&i, &p) in placed.iter().zip(dock_nodes.iter().chain(&anchors)) {
        entry_pos[i] = p;
    }
    for (i, pos) in entry_pos.iter_mut().enumerate() {
        if !dealt_elsewhere(i) && !placed.contains(&i) && on_grid(vanilla.entry_pos[i]) {
            *pos = free.pop()?;
        }
    }
    if !free.is_empty() {
        return None;
    }

    // Each entry shows its own tile. A blank one (a Hammer Bro level's cell)
    // is drawn for its neighbours once every path and water tile is final, at
    // the end; until then it keeps the entry's tile.
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
    // Docks, and the canoe joining them: from the first, where the canoe
    // parks, to every other (the walker opens all of them once the first is
    // reached on foot).
    for &(r, c) in &dock_nodes {
        grid.set(r, c, DOCK);
    }
    blanks.retain(|p| !dock_nodes.contains(p));
    if let Some((&first, rest)) = dock_nodes.split_first() {
        grid.canoe_edges = Some(rest.iter().map(|&d| (first, d)).collect());
    }

    // The castle top sits above the airship, as in vanilla.
    let air = vanilla.entry_pos[airship_idx];
    grid.set(layout.target.0 - 1, layout.target.1, old.get(air.0 - 1, air.1));

    // Every cell drawn so far is a path, a node or the castle top: what
    // one-tile scenery must not touch.
    let built: HashSet<Pos> = (0..grid.rows())
        .flat_map(|r| (1..grid.cols - 1).map(move |c| (r, c)))
        .filter(|&(r, c)| grid.get(r, c) != FILL)
        .collect();

    // The recipe's scenery, then its water, then its ground.
    let mut water = Water::default();
    for &step in recipe.scenery {
        match step {
            Step::Lakes => {
                for region in empty_regions(&grid) {
                    if region.len() >= LAKE_MIN_REGION && rng.random_bool(LAKE_CHANCE) {
                        grow_lake(&mut grid, &region, None, &mut water, rng);
                    }
                }
            }
            Step::River => {
                if rng.random_bool(RIVER_CHANCE) {
                    add_river(&mut grid, &mut water, rng);
                }
            }
            Step::Moat => {
                if rng.random_bool(MOAT_CHANCE) {
                    add_moat(&mut grid, layout.target, &mut water);
                }
            }
            Step::Oasis { island } => add_oasis(&mut grid, &mut water, island),
            Step::Pools { pool, palm, count } => {
                add_pools(&mut grid, &built, pool, palm, count, rng)
            }
            Step::Patches { tile } => add_patches(&mut grid, tile, rng),
            Step::Clusters { other, rock_share } => decorate(&mut grid, other, rock_share, rng),
            Step::Scatter { tile, count } => {
                scatter(&mut grid, &built, tile, count, rng);
            }
            Step::Sea { margin, land, water_nodes } => {
                if !add_sea(&mut grid, &mut water, &layout, (margin, land, water_nodes), rng) {
                    return None;
                }
            }
        }
    }
    water.draw(&mut grid);
    // Blanks last, so each reads its final paths: the pickup keeps a blank
    // it finds (`blank_tile_for`), and W3's water nodes need the island blank
    // their over-water path calls for.
    for &pos in &blanks {
        let tile = blank_tile_from_neighbors(&grid, world_idx, pos.0, pos.1);
        grid.set(pos.0, pos.1, tile);
    }
    for r in 0..grid.rows() {
        for c in 1..grid.cols - 1 {
            if grid.get(r, c) == FILL {
                grid.set(r, c, recipe.ground);
            }
        }
    }

    Some(WorldMap {
        grid,
        entry_pos,
        sprites: Vec::new(),
        hb_sprites: Vec::new(),
        gaps: Vec::new(),
        hammer_gated: Vec::new(),
        no_locks: water
            .crossings
            .iter()
            .copied()
            .chain(water.paths.iter().map(|&(p, _)| p))
            .collect(),
        canoe: layout.docks.first().map(|d| d.1),
    })
}

/// Plain ground (all four quadrants blank): W1's clearings, and W2's ground.
const CLEARING: u8 = 0x42;

/// `TILE_DANCINGPALM`.
const PALM: u8 = 0xBB;
/// Dotted sand: vanilla W2's ground, here grown in patches like lakes.
const DUNE: u8 = 0x43;
/// `TILE_POOL`, a one-tile oasis. Completable on its page, so never on rows
/// 7 or 8 (they share one completion bit) and never beside a path.
const POOL: u8 = 0xBF;
/// A small pyramid. Also the pyramid level's tile, so never beside a path.
const PYRAMID: u8 = 0x69;
/// Quicksand: a round pit. Also a level tile in vanilla W2, like the pyramid.
const QUICKSAND: u8 = 0x68;
/// A taller rock (W6 has 67). On its page it is a level-gate tile.
const TALL_ROCK: u8 = 0xEA;
/// A small desert plant.
const PLANT: u8 = 0xBE;
/// `TILE_FORTRUBBLE` and `TILE_ALTRUBBLE`: what a cleared mini-fortress
/// leaves, in two colours. Ruins here (user, 2026-10-13).
const RUBBLE: u8 = 0x60;
const ALT_RUBBLE: u8 = 0xE3;

/// Pyramids per desert map (vanilla W2 has 7 as scenery).
const PYRAMIDS: (usize, usize) = (2, 5);
/// Palms beside each pool.
const POOL_PALMS: (usize, usize) = (1, 2);
/// Regions at least this big may get a patch, this often.
const PATCH_MIN_REGION: usize = 6;
const PATCH_CHANCE: f64 = 0.9;

/// Whether `p` may take a one-tile piece of scenery: still empty, touching
/// no cell in `built` (paths, nodes, the castle top), and off rows 7 and 8.
fn scenery_spot(grid: &Grid, built: &HashSet<Pos>, (r, c): Pos) -> bool {
    let around = [(r.wrapping_sub(1), c), (r + 1, c), (r, c - 1), (r, c + 1)];
    grid.get(r, c) == FILL && r + 2 < grid.rows() && !around.iter().any(|p| built.contains(p))
}

/// Place a few of `tile`, one cell each, at random [`scenery_spot`]s.
fn scatter<R: Rng>(
    grid: &mut Grid,
    built: &HashSet<Pos>,
    tile: u8,
    count: (usize, usize),
    rng: &mut R,
) -> Vec<Pos> {
    let mut placed = Vec::new();
    for _ in 0..rng.random_range(count.0..=count.1) {
        let spots: Vec<Pos> = (0..grid.rows())
            .flat_map(|r| (1..grid.cols - 1).map(move |c| (r, c)))
            .filter(|&p| scenery_spot(grid, built, p))
            .collect();
        let Some(&(r, c)) = spots.choose(rng) else { break };
        grid.set(r, c, tile);
        placed.push((r, c));
    }
    placed
}

/// Small oases: a pool with a palm or two beside it.
fn add_pools<R: Rng>(
    grid: &mut Grid,
    built: &HashSet<Pos>,
    pool: u8,
    palm: u8,
    count: (usize, usize),
    rng: &mut R,
) {
    for (r, c) in scatter(grid, built, pool, count, rng) {
        let mut beside: Vec<Pos> = [(r.wrapping_sub(1), c), (r + 1, c), (r, c - 1), (r, c + 1)]
            .into_iter()
            .filter(|&(nr, nc)| nr < grid.rows() && nc > 0 && nc + 1 < grid.cols)
            .filter(|&(nr, nc)| grid.get(nr, nc) == FILL)
            .collect();
        beside.shuffle(rng);
        for (pr, pc) in beside.into_iter().take(rng.random_range(POOL_PALMS.0..=POOL_PALMS.1)) {
            grid.set(pr, pc, palm);
        }
    }
}

/// Patches of `tile` (W2's dotted sand), grown as blobs in the bigger open
/// regions.
fn add_patches<R: Rng>(grid: &mut Grid, tile: u8, rng: &mut R) {
    for region in empty_regions(grid) {
        if region.len() >= PATCH_MIN_REGION && rng.random_bool(PATCH_CHANCE) {
            let Some(&seed) = region.choose(rng) else { continue };
            let size = rng.random_range(3..=MAX_LAKE.min(region.len() / 2).max(3));
            for (r, c) in grow_blob(&region, seed, size, rng) {
                grid.set(r, c, tile);
            }
        }
    }
}

/// The big oasis: the largest open region in the pocket's strip, all of it
/// water, with `island_tile` on an island at its deepest cell when it has one (a
/// cell with water all round). Water cells with no tile for their picture
/// are trimmed back to ground. Takes no random draws: the strip decides it.
fn add_oasis(grid: &mut Grid, water: &mut Water, island_tile: u8) {
    let strip: Vec<Pos> = empty_regions(grid)
        .into_iter()
        .flat_map(|region| {
            let in_strip: Vec<Pos> = region.into_iter().filter(|p| p.1 >= POCKET_COL).collect();
            connected_pieces(&in_strip)
        })
        .max_by(|a, b| a.len().cmp(&b.len()).then(b.cmp(a)))
        .unwrap_or_default();
    let mut lake = strip;
    lake.sort();
    let inside: HashSet<Pos> = lake.iter().copied().collect();
    // How many rings of lake surround a cell: 1 or more is an island spot.
    let depth = |(r, c): Pos| {
        let mut k = 0;
        while r > k
            && c > k
            && (r - k - 1..=r + k + 1)
                .all(|y| (c - k - 1..=c + k + 1).all(|x| inside.contains(&(y, x))))
        {
            k += 1;
        }
        k
    };
    let island = lake.iter().copied().filter(|&p| depth(p) >= 1).max_by_key(|&p| depth(p));
    lake.retain(|&p| Some(p) != island);

    let before = water.cells.len();
    water.cells.extend(&lake);
    loop {
        let bad: Vec<Pos> = water.undrawable().into_iter().filter(|p| lake.contains(p)).collect();
        let Some(&p) = bad.first() else { break };
        lake.retain(|&q| q != p);
        water.cells.retain(|&q| q != p);
    }
    water.cells.truncate(before);
    if lake.len() >= 2 && water.try_add(grid, &lake, &[], &[]) {
        if let Some((r, c)) = island {
            grid.set(r, c, island_tile);
        }
        water.lakes.push(lake);
    }
}

/// `cells` split into 4-connected pieces.
fn connected_pieces(cells: &[Pos]) -> Vec<Vec<Pos>> {
    let mut left: HashSet<Pos> = cells.iter().copied().collect();
    let mut pieces = Vec::new();
    for &start in cells {
        if !left.remove(&start) {
            continue;
        }
        let mut piece = vec![start];
        let mut i = 0;
        while i < piece.len() {
            let (r, c) = piece[i];
            i += 1;
            for n in [(r.wrapping_sub(1), c), (r + 1, c), (r, c.wrapping_sub(1)), (r, c + 1)] {
                if left.remove(&n) {
                    piece.push(n);
                }
            }
        }
        pieces.push(piece);
    }
    pieces
}
/// A rock that never breaks. It looks exactly like a breakable one, so it is
/// only ever placed where no path or node touches it.
const DECO_ROCK: u8 = 0x53;

/// Scenery (milestone 3, decorations D1 + D2). The empty cells still bush
/// fill after the water is placed fall into separate regions; each region big
/// enough gets a few clusters of rock or `other` (W1's clearings, W2's
/// palms), grown as a blob from a random cell. Tiny regions, usually wedged
/// between paths, stay as they are.
fn decorate<R: Rng>(grid: &mut Grid, other: u8, rock_share: f64, rng: &mut R) {
    for region in empty_regions(grid) {
        if region.len() < MIN_DECORATED_REGION {
            continue;
        }
        for _ in 0..1 + region.len() / CELLS_PER_CLUSTER {
            let rock = rng.random_bool(rock_share);
            let allowed: Vec<Pos> = region
                .iter()
                .copied()
                .filter(|&p| grid.get(p.0, p.1) == FILL && (!rock || clear_of_paths(grid, p)))
                .collect();
            let Some(&seed) = allowed.choose(rng) else { continue };
            let size = rng.random_range(2..=MAX_CLUSTER.min(region.len() / 2).max(2));
            let tile = if rock { DECO_ROCK } else { other };
            for (r, c) in grow_blob(&allowed, seed, size, rng) {
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
    /// A path over a sea and the over-water tile it is drawn as
    /// ([`HORZ_OVER_WATER`], [`VERT_OVER_WATER`]): walkable, and it looks like
    /// water. Like a crossing it has no water-gap form, so no lock may take it.
    paths: Vec<(Pos, u8)>,
    /// Nodes drawn in the sea ([`add_sea`]): wet to the water beside them,
    /// though their own tile is the node's.
    nodes: Vec<Pos>,
    /// A sea's map size, `(rows, cols)` ([`add_sea`]): the open sea runs off
    /// the top, left and right edges and over the frame columns, as vanilla
    /// W3's does, and meets land only along the bottom. `None` for lakes,
    /// where everything off the map is land.
    sea_bounds: Option<(usize, usize)>,
}

impl Water {
    fn wet(&self, p: Pos) -> bool {
        self.cells.contains(&p)
            || self.bridges.contains(&p)
            || self.crossings.contains(&p)
            || self.paths.iter().any(|&(q, _)| q == p)
            || self.nodes.contains(&p)
    }

    /// The picture a water cell needs: an edge middle is water when that
    /// neighbour is wet, a corner when both edges beside it and the diagonal
    /// are. Off the map is land, except a sea's edges (`sea_bounds`).
    fn picture(&self, (r, c): Pos) -> String {
        let at = |dr: isize, dc: isize| {
            let (nr, nc) = (r as isize + dr, c as isize + dc);
            let inside = nr >= 0 && nc >= 0 && self.wet((nr as usize, nc as usize));
            match self.sea_bounds {
                None => inside,
                Some(_) if nr < 0 => true,
                Some((rows, _)) if nr as usize >= rows => false,
                Some((_, cols)) if nc <= 0 || nc as usize >= cols - 1 => true,
                Some(_) => inside,
            }
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

    /// Add water only if every water cell still has a tile afterwards, and
    /// mark the new cells taken on the grid; otherwise change nothing.
    fn try_add(
        &mut self,
        grid: &mut Grid,
        cells: &[Pos],
        bridges: &[Pos],
        crossings: &[Pos],
    ) -> bool {
        let before = (self.cells.len(), self.bridges.len(), self.crossings.len());
        self.cells.extend(cells);
        self.bridges.extend(bridges);
        self.crossings.extend(crossings);
        if !self.undrawable().is_empty() {
            self.cells.truncate(before.0);
            self.bridges.truncate(before.1);
            self.crossings.truncate(before.2);
            return false;
        }
        for &(r, c) in cells {
            grid.set(r, c, WATER_PENDING);
        }
        true
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
            grid.set(r, c, BRIDGE_TILE);
        }
        for &(r, c) in &self.crossings {
            grid.set(r, c, CROSSING);
        }
        for &((r, c), tile) in &self.paths {
            grid.set(r, c, tile);
        }
    }
}

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
/// Lakes grown for the river's ends stay even when no route works.
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
                let far = near.iter().map(|&q| manhattan(p, q)).min();
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
        if water.try_add(grid, &cells, &bridges, &crossings) {
            return;
        }
    }
}

/// How many of the farthest spots a river's end lake is picked from.
const FAR_FEW: usize = 5;

/// The pair of lakes whose nearest cells are farthest apart.
fn farthest_lakes(lakes: &[Vec<Pos>]) -> (usize, usize) {
    let gap = |a: &[Pos], b: &[Pos]| {
        a.iter().flat_map(|&p| b.iter().map(move |&q| manhattan(p, q))).min().unwrap_or(0)
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
    water.try_add(grid, &new, &[entry], &[]);
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
    let mut lake = grow_blob(region, seed, size, rng);
    // Trim against the water already there, then take it back out:
    // `try_add` makes the final, all-or-nothing addition.
    let before = water.cells.len();
    water.cells.extend(&lake);
    loop {
        let bad: Vec<Pos> = water.undrawable().into_iter().filter(|p| lake.contains(p)).collect();
        let Some(&p) = bad.first() else { break };
        lake.retain(|&q| q != p);
        water.cells.retain(|&q| q != p);
    }
    water.cells.truncate(before);
    if lake.len() < 2 || !water.try_add(grid, &lake, &[], &[]) {
        return false;
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

/// Steps between two cells, moving side to side.
fn manhattan(a: Pos, b: Pos) -> usize {
    a.0.abs_diff(b.0) + a.1.abs_diff(b.1)
}

fn touching(a: Pos, b: Pos) -> bool {
    manhattan(a, b) == 1
}

/// Grow a blob of up to `size` cells from `seed`, each new cell a random one
/// of `allowed` touching the blob so far. Stops early when none is left.
fn grow_blob<R: Rng>(allowed: &[Pos], seed: Pos, size: usize, rng: &mut R) -> Vec<Pos> {
    let mut blob = vec![seed];
    while blob.len() < size {
        let next: Vec<Pos> = allowed
            .iter()
            .copied()
            .filter(|p| !blob.contains(p) && blob.iter().any(|&c| touching(c, *p)))
            .collect();
        let Some(&p) = next.choose(rng) else { break };
        blob.push(p);
    }
    blob
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
/// Nodes sit two tiles apart, because a map move is two tiles: on the
/// vanilla start's row and column parity. Column 0 is left out: it is where
/// the start's entrance path runs off the edge of the map.
fn lattice(rows: usize, cols: usize, anchor: Pos) -> Vec<Pos> {
    let mut cells = Vec::new();
    for r in (anchor.0 % 2..rows).step_by(2) {
        for c in (anchor.1 % 2..cols).step_by(2) {
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
    anchor: Pos,
    start: Pos,
    targets: Targets,
    rng: &mut R,
) -> Option<Layout> {
    let cells = lattice(rows, cols, anchor);
    let on_lattice: HashSet<Pos> = cells.iter().copied().collect();

    // 1. A random tree grown from the start until it has the node count. Most
    //    steps extend the newest node that can still grow, which draws long
    //    corridors; the rest branch from any node. Picking from any node every
    //    time grows a blob around the start instead.
    let mut nodes = vec![start];
    let mut in_tree: HashSet<Pos> = HashSet::from([start]);
    let mut tree: Vec<(Pos, Pos)> = Vec::new();
    // The secret area's strip: the main tree never enters it, no loop touches
    // it, and the waypoint and airship ignore it, so it hangs off the main map
    // by one link. Empty without a pocket.
    let walled: HashSet<Pos> = match targets.pocket {
        Some(_) => cells.iter().copied().filter(|p| p.1 >= POCKET_COL).collect(),
        None => HashSet::new(),
    };
    let main: Vec<Pos> = cells.iter().copied().filter(|p| !walled.contains(p)).collect();
    let pocket_size = targets.pocket.map(|(lo, hi)| rng.random_range(lo..=hi));

    let main_nodes = targets.nodes - pocket_size.unwrap_or(0);
    let splits = match targets.pipe_pairs {
        0 => 0,
        n => n - 1 + usize::from(rng.random_bool(EXTRA_SPLIT)),
    };
    let dock_islands = match targets.dock_islands {
        (_, 0) => 0,
        (lo, hi) => rng.random_range(lo..=hi),
    };
    let mut islands: Vec<Vec<Pos>> = Vec::new();
    let mut mainland: Vec<Pos> = Vec::new();

    if splits + dock_islands == 0 {
        //    First a waypoint, reached by a winding path from the start, so every
        //    layout crosses the map. It lands in a staircase on the side away
        //    from the start's column: on the row farthest from the start's row
        //    only the far column qualifies, and each row nearer allows one more
        //    column in. (A start bottom-left puts it bottom-right.) Each step of
        //    the path moves toward the waypoint's column or row, chosen at random.
        // From the ordered lattice, never the set: a HashSet iterates in a
        // different order every run, which made the same seed draw different maps.
        let far_end = |values: Vec<usize>, from: usize| {
            let (lo, hi) = (*values.iter().min()?, *values.iter().max()?);
            Some(if from - lo < hi - from { hi } else { lo })
        };
        let far_row = far_end(main.iter().map(|p| p.0).collect(), start.0)?;
        let far_col = far_end(main.iter().map(|p| p.1).collect(), start.1)?;
        let corner: Vec<Pos> = main
            .iter()
            .copied()
            .filter(|&(r, c)| c.abs_diff(far_col) / 2 <= r.abs_diff(far_row) / 2)
            .collect();
        let waypoint = *corner.choose(rng)?;
        let mut at = start;
        while at != waypoint {
            let mut steps = Vec::new();
            if at.1 < waypoint.1 {
                steps.push((at.0, at.1 + 2));
            }
            if at.1 > waypoint.1 {
                steps.push((at.0, at.1 - 2));
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
        while nodes.len() < main_nodes {
            let free = |n: Pos| -> Vec<Pos> {
                neighbours(n, &on_lattice)
                    .into_iter()
                    .filter(|m| !in_tree.contains(m) && !walled.contains(m))
                    .collect()
            };
            let growable: Vec<Pos> =
                nodes.iter().copied().filter(|&n| !free(n).is_empty()).collect();
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
    } else {
        //    Islands instead: grown together from seeds spread apart, never
        //    side by side, so an empty band at least one node wide lies
        //    between any two for water or scenery to fill.
        let grown = grow_islands(
            &main,
            &on_lattice,
            start,
            splits + dock_islands,
            main_nodes,
            targets.island_share,
            rng,
        )?;
        for island in &grown.nodes {
            nodes.extend(island.iter().filter(|&&n| n != start));
            in_tree.extend(island);
        }
        tree.extend(grown.links);
        mainland = grown.nodes[0].clone();
        islands = grown.nodes[1..].to_vec();
    }
    //    The pocket, last: grown into the strip from a mouth beside a node
    //    the main tree reached.
    let pocket = match pocket_size {
        Some(size) => {
            let p = grow_pocket(&walled, &on_lattice, &in_tree, size, rng)?;
            tree.extend(p.links.iter().map(|&(a, b)| ordered(a, b)));
            p.nodes
        }
        None => Vec::new(),
    };
    nodes.extend(&pocket);
    in_tree.extend(&pocket);

    // 2. Extra links between neighbouring nodes: each one is a loop. None
    //    inside or into the pocket.
    let linked: HashSet<(Pos, Pos)> = tree.iter().copied().collect();
    let mut spare: Vec<(Pos, Pos)> = Vec::new();
    for &n in nodes.iter().filter(|n| !walled.contains(n)) {
        for m in neighbours(n, &on_lattice) {
            let pair = ordered(n, m);
            if in_tree.contains(&m)
                && !walled.contains(&m)
                && !linked.contains(&pair)
                && !spare.contains(&pair)
            {
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
    //    Never a vertical link down onto the bottom row either: its rock sits
    //    on row 7, and rows 7 and 8 share one completion bit, so the node
    //    below (a kept Toad House, say) would open the rock and never stay
    //    used.
    let mut rocks = rng.random_range(targets.rocks.0..=targets.rocks.1);
    let mut links: Vec<(Pos, Pos, Link)> = tree.iter().map(|&(a, b)| (a, b, Link::Path)).collect();
    for &(a, b) in &extra {
        let onto_bottom = a.1 == b.1 && a.0.max(b.0) == rows - 1;
        let kind = if rocks > 0 && !onto_bottom {
            rocks -= 1;
            Link::Rock
        } else {
            Link::Path
        };
        links.push((a, b, kind));
    }

    // 3b. Docks: one on each of `dock_islands` islands picked at random, and
    //     one or two on the start's island, each a node with a free tile
    //     beside it for the canoe's water. That tile is always in the open
    //     sea, the largest open area (paths and nodes cut it), so every dock
    //     is on the one sea; a tile in a pocket enclosed by paths would
    //     strand its dock. The canoe parks beside the first.
    let mut docks: Vec<(Pos, Pos)> = Vec::new();
    if dock_islands > 0 {
        let taken: HashSet<Pos> = nodes
            .iter()
            .copied()
            .chain(links.iter().map(|&(a, b, _)| ((a.0 + b.0) / 2, (a.1 + b.1) / 2)))
            .collect();
        let open = |(r, c): Pos| r < rows && c >= 1 && c + 1 < cols && !taken.contains(&(r, c));
        let mut sea: HashSet<Pos> = HashSet::new();
        let mut seen: HashSet<Pos> = HashSet::new();
        for p in (0..rows).flat_map(|r| (1..cols - 1).map(move |c| (r, c))) {
            if !open(p) || !seen.insert(p) {
                continue;
            }
            let mut area = vec![p];
            let mut i = 0;
            while i < area.len() {
                let (r, c) = area[i];
                i += 1;
                for n in [(r.wrapping_sub(1), c), (r + 1, c), (r, c.wrapping_sub(1)), (r, c + 1)] {
                    if open(n) && seen.insert(n) {
                        area.push(n);
                    }
                }
            }
            if area.len() > sea.len() {
                sea = area.into_iter().collect();
            }
        }
        let mut order: Vec<usize> = (0..islands.len()).collect();
        order.shuffle(rng);
        let mut homes: Vec<&[Pos]> =
            vec![
                mainland.as_slice();
                rng.random_range(targets.mainland_docks.0..=targets.mainland_docks.1)
            ];
        homes.extend(order.iter().take(dock_islands).map(|&i| islands[i].as_slice()));
        for home in homes {
            let spots: Vec<(Pos, Pos)> = home
                .iter()
                .copied()
                .filter(|&n| n != start && !docks.iter().any(|d| d.0 == n))
                .flat_map(|(r, c)| {
                    [(r.wrapping_sub(1), c), (r + 1, c), (r, c.wrapping_sub(1)), (r, c + 1)]
                        .into_iter()
                        .filter(|w| sea.contains(w))
                        .map(move |w| ((r, c), w))
                })
                .collect();
            docks.push(*spots.choose(rng)?);
        }
    }

    // 4. The target: a dead end at the hop distance, not reached from above,
    //    with a row above it for the castle top.
    let degree = degrees(&nodes, &links);
    let hops = path_hops(start, &links);
    //    On an island, the first: any distance, since the builder decides
    //    where its pipe lands.
    let candidates: Vec<Pos> = nodes
        .iter()
        .copied()
        .filter(|&n| n != start && degree[&n] == 1 && n.0 >= 2 && !walled.contains(&n))
        .filter(|&n| !docks.iter().any(|d| d.0 == n))
        .filter(|&n| match islands.first() {
            Some(far) => far.contains(&n),
            None => {
                let d = hops.get(&n).copied().unwrap_or(usize::MAX);
                (targets.start_to_target.0..=targets.start_to_target.1).contains(&d)
            }
        })
        .filter(|&n| !links.iter().any(|&(a, b, _)| (a, b) == ((n.0 - 2, n.1), n)))
        .collect();
    let &target = candidates.choose(rng)?;

    let dead_ends = degree.iter().filter(|&(&n, &d)| d == 1 && n != start).count();
    if !(targets.dead_ends.0..=targets.dead_ends.1).contains(&dead_ends) {
        return None;
    }
    if hooked_cross(&links) {
        return None;
    }

    Some(Layout { rows, cols, nodes, links, start, target, loop_sizes, pocket, islands, docks })
}

/// A layout's islands, the start's first, and the links that grew them.
struct Islands {
    nodes: Vec<Vec<Pos>>,
    links: Vec<(Pos, Pos)>,
}

/// Grow `splits + 1` islands over `main` with `total` nodes between them,
/// the first from `start`. Each later island's seed is among the
/// [`FARTHEST_FEW`] cells farthest from the seeds already down, and it gets
/// `share` percent of the nodes not yet dealt; the start's island keeps the
/// rest. They grow a node at a time, the island picked at random among those
/// short of their count, the same way the single tree grows. A node is never
/// placed beside another island's node, so islands never touch.
fn grow_islands<R: Rng>(
    main: &[Pos],
    on_lattice: &HashSet<Pos>,
    start: Pos,
    splits: usize,
    total: usize,
    share: (usize, usize),
    rng: &mut R,
) -> Option<Islands> {
    let mut seeds = vec![start];
    for _ in 0..splits {
        let gap = |p: Pos| seeds.iter().map(|&s| manhattan(p, s)).min().unwrap_or(0);
        let mut far: Vec<(usize, Pos)> =
            main.iter().copied().filter(|&p| gap(p) > 2).map(|p| (gap(p), p)).collect();
        far.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(&y.1)));
        let &(_, seed) = far.get(..FARTHEST_FEW.min(far.len()))?.choose(rng)?;
        seeds.push(seed);
    }
    let mut quotas = vec![0; seeds.len()];
    let mut left = total;
    for quota in quotas.iter_mut().skip(1) {
        *quota = (left * rng.random_range(share.0..=share.1) / 100).max(2);
        left -= *quota;
    }
    quotas[0] = left;

    let in_main: HashSet<Pos> = main.iter().copied().collect();
    let mut owner: HashMap<Pos, usize> = seeds.iter().enumerate().map(|(i, &s)| (s, i)).collect();
    let mut islands: Vec<Vec<Pos>> = seeds.iter().map(|&s| vec![s]).collect();
    let mut links = Vec::new();
    loop {
        let short: Vec<usize> =
            (0..islands.len()).filter(|&i| islands[i].len() < quotas[i]).collect();
        let Some(&i) = short.choose(rng) else { break };
        let free = |n: Pos| -> Vec<Pos> {
            neighbours(n, on_lattice)
                .into_iter()
                .filter(|m| in_main.contains(m) && !owner.contains_key(m))
                .filter(|&m| {
                    neighbours(m, on_lattice).iter().all(|o| owner.get(o).is_none_or(|&j| j == i))
                })
                .collect()
        };
        let growable: Vec<Pos> =
            islands[i].iter().copied().filter(|&n| !free(n).is_empty()).collect();
        let from = if rng.random_bool(EXTEND_NEWEST) {
            *growable.last()?
        } else {
            *growable.choose(rng)?
        };
        let &to = free(from).choose(rng)?;
        owner.insert(to, i);
        islands[i].push(to);
        links.push(ordered(from, to));
    }
    Some(Islands { nodes: islands, links })
}

/// The first column of the pocket's strip: the map's last scroll view, which
/// holds nothing else. Vanilla W2's secret area starts at column 22, entered
/// from (0,20); one node column further out, its tiles cannot be seen from
/// the main map without going over (user, 2026-10-13).
const POCKET_COL: usize = 24;

/// A secret area: a small tree in the strip, joined to the main map by one
/// link from a main node into its mouth (vanilla W2's area behind the (0,21)
/// rock, which this randomizer always opens).
struct Pocket {
    nodes: Vec<Pos>,
    /// The tree's links, the joining link first.
    links: Vec<(Pos, Pos)>,
}

/// Grow a [`Pocket`] of `size` nodes in `strip`, its mouth beside a node of
/// `main` (the main tree, already grown).
fn grow_pocket<R: Rng>(
    strip: &HashSet<Pos>,
    on_lattice: &HashSet<Pos>,
    main: &HashSet<Pos>,
    size: usize,
    rng: &mut R,
) -> Option<Pocket> {
    // Sorted: `strip` and `main` are sets, which iterate in a different order
    // every run.
    let mut doors: Vec<(Pos, Pos)> = strip
        .iter()
        .flat_map(|&m| neighbours(m, on_lattice).into_iter().map(move |o| (o, m)))
        .filter(|(o, _)| main.contains(o))
        .collect();
    doors.sort();
    let &(door, mouth) = doors.choose(rng)?;

    // Grown from the mouth, from any of its nodes, inside the strip only.
    let mut nodes = vec![mouth];
    let mut links = vec![(door, mouth)];
    while nodes.len() < size {
        let free = |n: Pos| -> Vec<Pos> {
            neighbours(n, on_lattice)
                .into_iter()
                .filter(|m| strip.contains(m) && !nodes.contains(m))
                .collect()
        };
        let growable: Vec<Pos> = nodes.iter().copied().filter(|&n| !free(n).is_empty()).collect();
        let &from = growable.choose(rng)?;
        let &to = free(from).choose(rng)?;
        nodes.push(to);
        links.push((from, to));
    }
    Some(Pocket { nodes, links })
}

/// Longest straight arm [`hooked_cross`] looks along: the lattice is five
/// nodes tall, so no arm runs further.
const MAX_ARM: usize = 2;

/// Whether the links draw a hooked cross: a node with an arm in all four
/// directions, each a clean corridor (every node on it has just its two
/// links) running straight for up to [`MAX_ARM`] links and then turning,
/// every turn the same way round (all clockwise, or all anticlockwise, so
/// both mirror images). It reads as a swastika, and no layout may contain
/// one (user, 2026-10-13). A junction whose arms branch on does not read as
/// one, so it is allowed. Rocks count: they draw in line.
fn hooked_cross(links: &[(Pos, Pos, Link)]) -> bool {
    // North, east, south, west: one step clockwise is the next one.
    const DIRS: [(isize, isize); 4] = [(-2, 0), (0, 2), (2, 0), (0, -2)];
    let linked: HashSet<(Pos, Pos)> = links.iter().map(|&(a, b, _)| ordered(a, b)).collect();
    let joined = |p: Pos, (dr, dc): (isize, isize)| {
        let (r, c) = (p.0 as isize + dr, p.1 as isize + dc);
        let q = (r >= 0 && c >= 0).then_some((r as usize, c as usize))?;
        linked.contains(&ordered(p, q)).then_some(q)
    };
    let degree = |p: Pos| DIRS.iter().filter(|&&d| joined(p, d).is_some()).count();
    let mut centres: Vec<Pos> = links.iter().flat_map(|&(a, b, _)| [a, b]).collect();
    centres.sort();
    centres.dedup();
    centres.into_iter().any(|centre| {
        [1, 3].into_iter().any(|turn| {
            (0..4).all(|i| {
                let (out, hook) = (DIRS[i], DIRS[(i + turn) % 4]);
                let mut at = centre;
                for _ in 0..MAX_ARM {
                    let Some(next) = joined(at, out) else { return false };
                    at = next;
                    if degree(at) != 2 {
                        return false;
                    }
                    if joined(at, hook).is_some() {
                        return true;
                    }
                }
                false
            })
        })
    })
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
    /// `|` paths, `R` a rock, `p` a node in the secret area, `x` `y` `z` a node
    /// on an island the start cannot walk to, `D` a dock, `~` the water it
    /// boards from, `c` where the canoe parks.
    fn ascii(&self) -> String {
        let mut cells = vec![vec![' '; self.cols]; self.rows];
        for &(r, c) in &self.nodes {
            cells[r][c] = 'o';
        }
        for &(r, c) in &self.pocket {
            cells[r][c] = 'p';
        }
        for (k, island) in self.islands.iter().enumerate() {
            for &(r, c) in island {
                cells[r][c] = ['x', 'y', 'z'][k.min(2)];
            }
        }
        for (k, &((r, c), (wr, wc))) in self.docks.iter().enumerate() {
            cells[r][c] = 'D';
            cells[wr][wc] = if k == 0 { 'c' } else { '~' };
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
    fn summary(&self) -> String {
        let hops = path_hops(self.start, &self.links).get(&self.target).copied();
        format!(
            "nodes {}  loops {} {:?}  dead ends {}  junctions {}  rocks {}  pocket {}  islands {:?}  docks {}  start->airship {}",
            self.nodes.len(),
            self.links.len() + 1 + self.islands.len() - self.nodes.len(),
            self.loop_sizes,
            self.dead_ends(),
            self.junctions(),
            self.links.iter().filter(|l| l.2 == Link::Rock).count(),
            self.pocket.len(),
            self.islands.iter().map(Vec::len).collect::<Vec<_>>(),
            self.docks.len(),
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

/// Prints each seed's layout as ASCII (`LAYOUT_SEEDS`, default 10) for
/// `LAYOUT_WORLD` (default 1). Past 20 seeds it prints only the tally, for
/// comparing generator changes.
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
    let world: usize = std::env::var("LAYOUT_WORLD").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let targets = recipe(world - 1).expect("LAYOUT_WORLD has a recipe").layout;
    let grid = &maps[world - 1].grid;
    let start = crate::randomize::rom_data::find_start(grid).expect("the world has a start");
    let entries = &maps[world - 1].entry_pos;
    let airship = (0..entries.len())
        .find(|&i| AIRSHIP_ENTRIES.contains(&(world - 1, i)))
        .map(|i| entries[i])
        .expect("the world has an airship");
    println!("\nW{world} start {start:?}, airship {airship:?} (vanilla); targets: {targets:?}");
    println!(
        "Vanilla: {}",
        [
            "W1 nodes 21  loops 2  dead ends 3  junctions 5  rocks 1  start->airship 9",
            "W2 nodes 47  loops 3  dead ends 7  junctions 10  pocket 6  start->airship 15",
            "W3 nodes 52  loops 4  dead ends 16  junctions 15  islands 5/3/2  docks 3",
        ][world - 1]
    );

    let seeds: u64 = std::env::var("LAYOUT_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(10);
    let quiet = seeds > 20;
    let (mut failed, mut built) = (0, 0);
    let (mut loops, mut squares, mut loop_total, mut junctions) = (0, 0, 0, 0);
    for seed in 1..=seeds {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        match generate(world - 1, grid.rows(), grid.cols, start, airship, targets, &mut rng) {
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
                    println!("\n--- seed {seed}: no layout in {ATTEMPTS} attempts");
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

/// The stranding check for a generated world (`CENSUS_WORLD`, default 1):
/// across seeds, the builder must
/// leave the airship reachable from the start (locks open, the builder's own
/// convention, and rocks shut), and the whole pipeline must run without
/// panicking. `CENSUS_SEEDS` seeds, default 50.
///
/// ```sh
/// CENSUS_SEEDS=500 cargo test --release --lib generated_is_completable -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn generated_is_completable() {
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
    let wi: usize =
        std::env::var("CENSUS_WORLD").ok().and_then(|s| s.parse().ok()).unwrap_or(1) - 1;

    // The builder alone, many seeds.
    // Three arms by seed: everything shuffled; Toad Houses and spades left
    // where the generator dealt them; Start <-> Airship swap on.
    let mut generated = 0;
    let (mut with_river, mut with_gap, mut with_moat) = (0, 0, 0);
    let mut row78_conflicts = Vec::new();
    let mut stranded_by_swap: Vec<u64> = Vec::new();
    for seed in 0..seeds {
        let (shuffle, swap) = match seed % 3 {
            0 => (true, false),
            1 => (false, false),
            _ => (true, true),
        };
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut maps = super::vanilla::load(&rom, false, false);
        let Some(w1) = world_map(&maps[wi], wi, &mut rng) else { continue };
        // Rows 7 and 8 share one completion bit: a completable or removable
        // tile on row 7 must never sit above a node on row 8.
        let last = w1.grid.rows() - 1;
        for &(r, c) in &w1.entry_pos {
            if r == last
                && crate::randomize::overworld::build::is_completion_unsafe(
                    w1.grid.get(last - 1, c),
                )
            {
                row78_conflicts.push((seed, c));
            }
        }
        let docks = w1.grid.canoe_edges.clone();
        maps[wi] = w1;
        generated += 1;
        let mut catalog = NodeCatalog::from_maps(&rom, maps, false);
        if swap {
            crate::randomize::overworld::start_airship_swap::pick_swaps(&mut catalog, &mut rng);
        }
        let flags = PickupFlags {
            shuffle_spade_games: shuffle,
            shuffle_toad_houses: shuffle,
            shuffle_hammer_bros: true,
        };
        let pickup = pick_up(&rom, &catalog, flags);
        let result = build(
            &rom,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            BuildFlags {
                shuffle_toad_houses: shuffle,
                shuffle_hammer_bros: true,
                ..Default::default()
            },
        );
        let w1 = &result.worlds[wi];
        assert_eq!(w1.grid.canoe_edges, docks, "seed {seed}: the builder lost the generated docks");
        for &(a, b) in docks.iter().flatten() {
            for (r, c) in [a, b] {
                assert_eq!(w1.grid.get(r, c), DOCK, "seed {seed}: dock ({r},{c}) was built over");
            }
        }
        let bridges: Vec<Pos> = (0..w1.grid.rows())
            .flat_map(|r| (0..w1.grid.cols).map(move |c| (r, c)))
            .filter(|&(r, c)| w1.grid.get(r, c) == BRIDGE_TILE)
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
        let castle = find_target(&w1.grid, wi).expect("the generated world has an airship");
        with_moat += usize::from(
            bridges.contains(&(castle.0, castle.1 - 1))
                || bridges.contains(&(castle.0, castle.1 + 1)),
        );
        let start = crate::randomize::rom_data::find_start(&w1.grid);
        let target = find_target(&w1.grid, wi).expect("the generated world has an airship");
        let reachable = walk_map(&w1.grid, &w1.pipe_pairs, start, wi).nodes.contains(&target);
        if swap && !reachable {
            stranded_by_swap.push(seed);
            continue;
        }
        assert!(reachable, "seed {seed}: the generated airship is unreachable from the start");
    }
    println!("builder: {generated} of {seeds} seeds generated a W{}, all completable", wi + 1);
    assert!(
        row78_conflicts.is_empty(),
        "a completable or removable tile above a bottom-row node (seed, column): {row78_conflicts:?}",
    );
    println!(
        "  stranded with Start <-> Airship swap on: {} of {} {stranded_by_swap:?}",
        stranded_by_swap.len(),
        seeds / 3
    );
    println!(
        "  {with_river} have water crossing a path, {with_moat} a moat; in {with_gap} the builder locked a bridge (a water gap)"
    );

    // The whole pipeline, fewer seeds: the writer and every later stage.
    let options =
        crate::Options { generated_worlds: 1 << wi, palettes: false, ..Default::default() };
    for seed in 0..seeds.min(20) {
        crate::generate_patched_rom(&raw, seed, &options, None)
            .unwrap_or_else(|e| panic!("seed {seed}: pipeline failed: {e}"));
    }
    println!("pipeline: {} seeds ran end to end", seeds.min(20));
}

/// A generated W2's start never shows the pocket's strip: the starting camera
/// ([`start_camera_col`]) must stop short of it, so the secret area is found
/// only by going over.
#[test]
fn generated_w2_start_never_shows_the_pocket() {
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
    for seed in 0..50 {
        let map = world_map(&vanilla[1], 1, &mut ChaCha8Rng::seed_from_u64(seed)).unwrap();
        let start = crate::randomize::rom_data::find_start(&map.grid).unwrap();
        let shown = start_camera_col(1, start.1) + 16;
        assert!(shown <= POCKET_COL, "seed {seed}: start {start:?} shows up to column {shown}");
    }
}

/// The same seed must draw the same map. Each `HashSet` iterates in its own
/// order, so a choice made from one shows up here as two different maps.
#[test]
fn generated_worlds_are_deterministic() {
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
    for (wi, map) in vanilla.iter().enumerate().take(2) {
        for seed in 0..20 {
            let draw = || world_map(map, wi, &mut ChaCha8Rng::seed_from_u64(seed)).unwrap();
            let (a, b) = (draw(), draw());
            assert_eq!(a.grid.tiles, b.grid.tiles, "W{} seed {seed}: two draws differ", wi + 1);
            assert_eq!(a.entry_pos, b.entry_pos, "W{} seed {seed}: entries differ", wi + 1);
        }
    }
}

/// A generated world (`LAYOUT_WORLD`, default 1) as drawn tiles, before the
/// builder places anything: `.` bush, ` ` clearing or plain sand, `:` dotted
/// sand, `^` rock (decorative), `T` palm, `O` pool, `M` pyramid, `Q`
/// quicksand, `K` tall rock, `Y` plant, `u` ruins, `~` water,
/// `=` bridge, `+` path over water, `-` `|` paths, `R` breakable rock, `o`
/// node, `S` start, `A` airship, `a` castle top, `#` frame.
///
/// ```sh
/// LAYOUT_WORLD=2 cargo test --release --lib generated_tiles -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn generated_tiles() {
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
    let wi: usize =
        std::env::var("LAYOUT_WORLD").ok().and_then(|s| s.parse().ok()).unwrap_or(1) - 1;
    for seed in 1..=seeds {
        let map = world_map(&vanilla[wi], wi, &mut ChaCha8Rng::seed_from_u64(seed)).unwrap();
        println!("\n--- seed {seed}");
        for r in 0..map.grid.rows() {
            let row: String = (0..map.grid.cols)
                .map(|c| match map.grid.get(r, c) {
                    BUSH => '.',
                    DOCK => 'D',
                    CLEARING => ' ',
                    DECO_ROCK => '^',
                    DUNE => ':',
                    PALM => 'T',
                    POOL => 'O',
                    PYRAMID => 'M',
                    QUICKSAND => 'Q',
                    TALL_ROCK => 'K',
                    PLANT => 'Y',
                    RUBBLE | ALT_RUBBLE => 'u',
                    0x45 | 0x49 => '-',
                    0x46 => '|',
                    0x51 | 0x52 => 'R',
                    0x02 => '#',
                    0x82..=0xA9 => '~',
                    BRIDGE_TILE => '=',
                    CROSSING => '+',
                    0xAC | 0xB7..=0xB9 => '_',
                    0xAA | 0xAB | 0xB0 => '!',
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
    // The lookup takes the first match, so the pictures must all differ.
    let distinct: HashSet<&str> = WATER_TILES.iter().map(|&(_, p)| p).collect();
    assert_eq!(distinct.len(), WATER_TILES.len(), "two water tiles share a picture");

    let pixel = |piece: u8, r: usize, c: usize| {
        let at = crate::randomize::rom_data::map_chr_offset(piece);
        ((rom[at + r] >> (7 - c)) & 1) | (((rom[at + r + 8] >> (7 - c)) & 1) << 1)
    };
    let base = crate::randomize::rom_data::PRG012_FILE_BASE;
    for &(tile, want) in &WATER_TILES {
        let t = tile as usize;
        let (ul, ll, ur, lr) =
            (rom[base + t], rom[base + 0x100 + t], rom[base + 0x200 + t], rom[base + 0x300 + t]);
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

/// The hooked-cross check catches both mirror images, at arm lengths one and
/// two, and leaves a plain cross alone.
#[test]
fn hooked_cross_is_caught() {
    let path = |a: Pos, b: Pos| (ordered(a, b).0, ordered(a, b).1, Link::Path);
    let c = (4, 8);
    let plain: Vec<_> = [(2, 8), (6, 8), (4, 6), (4, 10)].iter().map(|&n| path(c, n)).collect();
    assert!(!hooked_cross(&plain));

    // Clockwise hooks: north arm turns east, east turns south, and so on.
    let mut clockwise = plain.clone();
    clockwise.extend([path((2, 8), (2, 10)), path((4, 10), (6, 10))]);
    clockwise.extend([path((6, 8), (6, 6)), path((4, 6), (2, 6))]);
    assert!(hooked_cross(&clockwise));

    let mut anticlockwise = plain.clone();
    anticlockwise.extend([path((2, 8), (2, 6)), path((4, 10), (2, 10))]);
    anticlockwise.extend([path((6, 8), (6, 10)), path((4, 6), (6, 6))]);
    assert!(hooked_cross(&anticlockwise));

    // Arms two links long, hooked at the far end.
    let c = (4, 12);
    let mut long: Vec<_> = Vec::new();
    for (a, b, hook) in [
        ((2, 12), (0, 12), (0, 14)),
        ((4, 14), (4, 16), (6, 16)),
        ((6, 12), (8, 12), (8, 10)),
        ((4, 10), (4, 8), (2, 8)),
    ] {
        long.extend([path(c, a), path(a, b), path(b, hook)]);
    }
    assert!(hooked_cross(&long));

    // Mixed turns are not the shape.
    let mut mixed = plain;
    mixed.extend([path((2, 8), (2, 10)), path((4, 10), (6, 10))]);
    mixed.extend([path((6, 8), (6, 10)), path((4, 6), (2, 6))]);
    assert!(!hooked_cross(&mixed));
}

/// No generated layout contains a hooked cross ([`hooked_cross`]).
#[test]
fn generated_layouts_have_no_hooked_cross() {
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
    for (wi, map) in maps.iter().enumerate().take(3) {
        let grid = &map.grid;
        let start = crate::randomize::rom_data::find_start(grid).unwrap();
        let airship = (0..map.entry_pos.len())
            .find(|&i| AIRSHIP_ENTRIES.contains(&(wi, i)))
            .map(|i| map.entry_pos[i])
            .unwrap();
        let targets = recipe(wi).unwrap().layout;
        for seed in 0..50 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            if let Some(layout) =
                generate(wi, grid.rows(), grid.cols, start, airship, targets, &mut rng)
            {
                assert!(!hooked_cross(&layout.links), "W{} seed {seed}", wi + 1);
            }
        }
    }
}
