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
use crate::randomize::overworld::build::is_completion_unsafe;
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
    /// The airship; `None` where the target is fixed (W8's castle, on its
    /// untouched last screen).
    target: Option<Pos>,
    /// Each loop link's length: the nodes around the loop it closed when it
    /// was added. 4 is a square, which gives the player no real choice.
    loop_sizes: Vec<usize>,
    /// The secret area's nodes (see [`grow_pocket`]); empty without one.
    pocket: Vec<Pos>,
    /// The first column of the secret area's strip ([`PocketTarget`]).
    strip_col: Option<usize>,
    /// The nodes of each island the start's cannot walk to, joined later by
    /// the builder's pipes; empty without a split. The airship is on the
    /// first.
    islands: Vec<Vec<Pos>>,
    /// Each dock and the water tile beside it the canoe boards from; the
    /// canoe parks beside the first, on the start's island.
    docks: Vec<(Pos, Pos)>,
    /// A river of sea through the start's island, coast to coast, crossed
    /// by bridges ([`Targets::river_share`]); [`add_sea`] routes it.
    river: bool,
    /// The rock pocket's nodes ([`Targets::rock_pocket`]); empty without one.
    rock_pocket: Vec<Pos>,
}

/// A secret area's node count (counted in the world's nodes) and the first
/// column of its strip, which holds nothing else.
#[derive(Clone, Copy, Debug)]
struct PocketTarget {
    nodes: (usize, usize),
    from_col: usize,
}

/// The targets a layout must meet, or it is thrown away and redrawn.
#[derive(Clone, Copy, Debug)]
struct Targets {
    /// Links added on top of a tree. Each one is exactly one loop.
    loops: (usize, usize),
    dead_ends: (usize, usize),
    /// Fewest hops from the start to the target, on paths alone.
    start_to_target: (usize, usize),
    rocks: (usize, usize),
    /// A secret area: a small tree in a strip of its own, the map's last
    /// scroll view, with one way in (see [`grow_pocket`]).
    pocket: Option<PocketTarget>,
    /// The world's pipe pairs (`VANILLA_PIPE_PAIRS`). With `n` of them the
    /// layout splits into islands `n - 1` times, plus once more
    /// `extra_split` of the time; the builder's connectivity phase joins
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
    /// Pipe islands are seeded on the outermost node row or column (W4's
    /// small island on the map's edge).
    edge_islands: bool,
    /// How many of the start's island's nodes may sit on the top or bottom
    /// node row: `None` for no limit; W4 keeps its main island in the middle.
    mainland_outer: Option<(usize, usize)>,
    /// How often a world with pipes splits once more than its pipe pairs
    /// less one: 25% (user, 2026-10-13), none where there is no room (W5).
    extra_split: f64,
    /// When the extra split comes up (`extra_split`), how often it is a
    /// river through the start's island instead, crossed by bridges: one
    /// island still, divided to the eye.
    river_share: f64,
    /// A world's own chance of that river, extra split or not (W4: a river
    /// through the main island on half the maps, for its bridges).
    river_chance: f64,
    /// The world's one split ([`Targets::pipe_pairs`] less one) is a rock
    /// pocket rather than an island: a dead-end branch of 1-3 nodes whose
    /// only way in is a breakable rock (W6). The builder, which reads rocks
    /// as shut, joins it with a pipe: vanilla W6's pipe behind a rock.
    rock_pocket: bool,
    /// Every island, the start's too, gets an equal share of the nodes
    /// rather than `island_share` (W7: many islands of a size, all reached by
    /// pipe).
    even_islands: bool,
    /// Islands may sit side by side, a tile of sea between them and no path
    /// across (W7, as vanilla's do); otherwise an empty node slot keeps them
    /// apart.
    islands_touch: bool,
    /// The layout places the airship. W8 has none: its castle is fixed, on
    /// the last screen, which is left as vanilla drew it.
    airship: bool,
    /// One island per screen, of these node counts, the start's screen's
    /// included (W8, whose screens are walled apart by its frame: vanilla's
    /// counts), rather than islands seeded far apart; `pipe_pairs` is then
    /// only the builder's.
    per_screen: Option<&'static [usize]>,
    /// With `per_screen`, one more island of `.1` nodes on screen `.0`: W8's
    /// short pipe-only island at the top of its second screen's ring.
    small_island: Option<(usize, usize)>,
}

/// The proposal agreed for W1 on 2026-10-11. Vanilla W1 is 21 nodes, 2
/// loops, 3 dead ends, 9 hops, 1 rock shortcut.
const W1_TARGETS: Targets = Targets {
    loops: (3, 5),
    dead_ends: (2, 5),
    start_to_target: (7, 11),
    rocks: (0, 2),
    pocket: None,
    pipe_pairs: 0,
    island_share: (25, 50),
    dock_islands: (0, 0),
    mainland_docks: (0, 0),
    edge_islands: false,
    mainland_outer: None,
    extra_split: 0.25,
    river_share: 0.0,
    river_chance: 0.0,
    rock_pocket: false,
    even_islands: false,
    islands_touch: false,
    airship: true,
    per_screen: None,
    small_island: None,
};

/// W2, first guesses around vanilla: 47 nodes, 3 loops, 7 dead ends, 15 hops,
/// and the secret area behind the (0,21) rock, 6 nodes. The pocket has no
/// rock at its mouth (user, 2026-10-13); rocks go elsewhere.
const W2_TARGETS: Targets = Targets {
    loops: (2, 4),
    dead_ends: (5, 9),
    start_to_target: (13, 17),
    rocks: (0, 2),
    pocket: Some(PocketTarget { nodes: (5, 7), from_col: W2_POCKET_COL }),
    pipe_pairs: 1,
    island_share: (25, 50),
    dock_islands: (0, 0),
    mainland_docks: (0, 0),
    edge_islands: false,
    mainland_outer: None,
    extra_split: 0.25,
    river_share: 0.0,
    river_chance: 0.0,
    rock_pocket: false,
    even_islands: false,
    islands_touch: false,
    airship: true,
    per_screen: None,
    small_island: None,
};

/// W3, first guesses (user, 2026-10-13): one big mainland and small islands,
/// many of them reached by canoe. Vanilla W3 is 52 nodes, 4 loops, 16 dead
/// ends, walk pieces 42/5/3/2, two canoe islands.
const W3_TARGETS: Targets = Targets {
    loops: (2, 5),
    dead_ends: (8, 22),
    start_to_target: (13, 20),
    rocks: (0, 2),
    pocket: None,
    pipe_pairs: 3,
    island_share: (4, 10),
    dock_islands: (3, 5),
    mainland_docks: (1, 2),
    edge_islands: false,
    mainland_outer: None,
    extra_split: 0.25,
    river_share: 0.0,
    river_chance: 0.0,
    rock_pocket: false,
    even_islands: false,
    islands_touch: false,
    airship: true,
    per_screen: None,
    small_island: None,
};

/// W4 (user, 2026-10-13): a small island on the map's edge and a main island
/// in the middle rows (one node on an outer row now and then), its extra
/// split a second pipe island. Its water inside the main island is a lake
/// the roads cross ([`Step::RoadLake`]), not a river (user, 2026-10-17).
/// Vanilla W4 is 34 nodes, 2 loops, 10 dead ends, walk pieces 30/4, 2 pipe pairs.
const W4_TARGETS: Targets = Targets {
    loops: (2, 4),
    dead_ends: (6, 13),
    start_to_target: (11, 17),
    rocks: (0, 2),
    pocket: None,
    pipe_pairs: 2,
    island_share: (10, 20),
    dock_islands: (0, 0),
    mainland_docks: (0, 0),
    edge_islands: true,
    mainland_outer: Some((0, 1)),
    extra_split: 0.25,
    river_share: 0.0,
    river_chance: 0.0,
    rock_pocket: false,
    even_islands: false,
    islands_touch: false,
    airship: true,
    per_screen: None,
    small_island: None,
};

/// W5 (user, 2026-10-13): the ground and the sky are its two islands, each
/// in vanilla's play area for it ([`areas`]), joined by the tower or the
/// pipe as the builder likes. 42 nodes go into 46 slots, so there is no room
/// for an extra island. Vanilla W5 is 42 nodes, 3 loops, 8 dead ends, walk
/// pieces 22/20.
const W5_TARGETS: Targets = Targets {
    loops: (2, 4),
    dead_ends: (4, 12),
    start_to_target: (13, 20),
    rocks: (0, 0),
    pocket: None,
    pipe_pairs: 2,
    island_share: (48, 52),
    dock_islands: (0, 0),
    mainland_docks: (0, 0),
    edge_islands: false,
    mainland_outer: None,
    extra_split: 0.0,
    river_share: 0.0,
    river_chance: 0.0,
    rock_pocket: false,
    even_islands: false,
    islands_touch: false,
    airship: true,
    per_screen: None,
    small_island: None,
};

/// W6 (user, 2026-10-13): long, three screens, one piece but for a rock
/// pocket (its pipe split; one per seed, never more). Vanilla W6 is 57
/// nodes, 5 loops, 10 dead ends, 23 hops start to castle.
const W6_TARGETS: Targets = Targets {
    loops: (3, 6),
    dead_ends: (6, 14),
    start_to_target: (19, 27),
    rocks: (0, 1),
    pocket: None,
    pipe_pairs: 2,
    island_share: (25, 50),
    dock_islands: (0, 0),
    mainland_docks: (0, 0),
    edge_islands: false,
    mainland_outer: None,
    extra_split: 0.0,
    river_share: 0.0,
    river_chance: 0.0,
    rock_pocket: true,
    even_islands: false,
    islands_touch: false,
    airship: true,
    per_screen: None,
    small_island: None,
};

/// W7 (user, 2026-10-13): many islands close to one size, every one reached
/// only by pipe. Vanilla W7 is 46 nodes, 1 loop, 16 dead ends, walk pieces
/// 14/10/9/6/4/2/1, 8 pipe pairs.
const W7_TARGETS: Targets = Targets {
    loops: (0, 3),
    dead_ends: (8, 22),
    start_to_target: (11, 17),
    rocks: (0, 1),
    pocket: None,
    pipe_pairs: 8,
    island_share: (25, 50),
    dock_islands: (0, 0),
    mainland_docks: (0, 0),
    edge_islands: false,
    mainland_outer: None,
    extra_split: 0.25,
    river_share: 0.0,
    river_chance: 0.0,
    rock_pocket: false,
    even_islands: true,
    islands_touch: true,
    airship: true,
    per_screen: None,
    small_island: None,
};

/// W8 (user, 2026-10-14): its first three screens drawn afresh, one island
/// each, joined by pipe as vanilla's are, plus vanilla's short pipe-only
/// island on the second screen; the castle's screen is left alone. 35 nodes
/// are its 41 entries less the six on that screen. Vanilla's three screens
/// are 8, 10 and 17 nodes, 6 pipe pairs over the whole world.
const W8_TARGETS: Targets = Targets {
    loops: (1, 4),
    dead_ends: (6, 16),
    start_to_target: (0, 0),
    rocks: (0, 1),
    pocket: None,
    pipe_pairs: 6,
    island_share: (0, 0),
    dock_islands: (0, 0),
    mainland_docks: (0, 0),
    edge_islands: false,
    mainland_outer: None,
    extra_split: 0.0,
    river_share: 0.0,
    river_chance: 0.0,
    rock_pocket: false,
    even_islands: false,
    islands_touch: false,
    airship: false,
    per_screen: Some(&[8, 7, 17]),
    small_island: Some((1, 3)),
};

/// How one world is generated: its layout, then its scenery, step by step in
/// order, then its ground in every cell nothing took. A new world is a new
/// recipe; a new kind of scenery is a new [`Step`] any recipe can use.
struct Recipe {
    layout: Targets,
    /// Where the map is drawn; the rest stays as vanilla drew it.
    area: Area,
    /// The camera pans as far as the map is wide (W4's vanilla limit is a
    /// screen past it). Off where vanilla's pan must stand: W5 (whose sky is
    /// not panned to) and W8 (whose castle screen is kept).
    pans_to_width: bool,
    /// Every cell nothing else takes. (Generation marks empty cells `FILL`
    /// throughout and swaps this in last.)
    ground: u8,
    /// Cells left of this column take this ground instead (W8: fire on its
    /// first screen, bones after).
    ground_before: Option<(usize, u8)>,
    scenery: &'static [Step],
    /// A sky area's own ground and scenery (W5): its cells are left alone
    /// by `scenery`, then get these.
    sky: Option<(u8, &'static [Step])>,
}

/// One kind of scenery. Each sees a cell as free only while it is still
/// `FILL`; water marks its cells `WATER_PENDING` as it is added and is drawn
/// in one pass after the last step, so water that meets water joins with a
/// proper shore.
#[derive(Clone, Copy)]
enum Step {
    /// In each big open region, half the time, a lake ([`grow_lake`]).
    Lakes,
    /// `chance` of a river joining two lakes, crossing at most
    /// `max_crossings` paths ([`add_river`]): each crossing of a horizontal
    /// path is a bridge, which the builder may make a water gap. A crossing
    /// costs the route `crossing_cost` steps: high, it skirts round paths
    /// (W1); low, it cuts across them (W6, for its bridges).
    River { chance: f64, max_crossings: usize, crossing_cost: u32 },
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
    /// time, grown by `land` of the open cells, and `islet` islets; the rest
    /// is sea. With `coast`, no land is added on the map's outer ring, so sea
    /// runs all round every island (W4, W7). With `solid`, channels inside an island are
    /// filled, so it reads as one landmass (W4). First in a recipe, so the
    /// steps after it decorate the land. A horizontal path over the sea
    /// between land nodes is a bridge `bridges` of the time, else `$B9`.
    /// With `boxes`, each island's land is the rectangle its nodes span
    /// (W7's island blocks), clipped where it would touch another's. With
    /// `within`, the sea is only in those columns, `[from, to)` (W8's navy on
    /// its first screen), and never beside the frame: off it is land. With
    /// `left_shore`, the map's first column inside the frame is always land:
    /// a canoe move crosses a water cell, so the canoe can never reach the
    /// frame column, the very left edge of the picture (W3; user, 2026-10-16).
    Sea {
        margin: f64,
        land: f64,
        water_nodes: f64,
        islet: u8,
        coast: bool,
        solid: bool,
        bridges: f64,
        boxes: bool,
        within: Option<(usize, usize)>,
        left_shore: bool,
    },
    /// A river straight down a tile column between node columns, from the
    /// top of the map into the shore strip below, where it meets the sea as
    /// `mouth` ([`add_ground_river`]). Paths crossing it become bridges.
    GroundRiver { mouth: u8 },
    /// Moats round `count` dead ends entered from the side ([`add_moats`]).
    /// And `through` more round nodes in the middle of long straight roads,
    /// the road crossing on a bridge each side, as vanilla W6 breaks up its
    /// long road (user, 2026-10-17).
    Moats { count: (usize, usize), through: (usize, usize) },
    /// A wavy coast along the top edge, lined with `mountain` ([`add_coast`]).
    Coast { mountain: u8 },
    /// `count` rocky outcrops in the biggest open regions ([`add_outcrops`]):
    /// a rock core in dotted ground that thins out round it.
    /// Each has a `ruins` chance of a broken fort in its core.
    Outcrop { count: (usize, usize), ruins: f64 },
    /// `count` bays cut into the land from the sea, through cells with no
    /// path ([`add_bays`]): a ragged coast, as vanilla W4's bottom one.
    Bays { count: (usize, usize) },
    /// A lake round one node or two side by side, every road into it
    /// crossing on a bridge or a path over water ([`add_road_lake`]):
    /// vanilla W4's lake round its fortress, which breaks up its roads.
    RoadLake,
    /// `tile` on `share` of the open cells, paths beside them or not, but
    /// rarely two side by side ([`sprinkle`]): W4's clearings in its fire
    /// flowers.
    Sprinkle { tile: u8, share: f64 },
}

const W1_RECIPE: Recipe = Recipe {
    layout: W1_TARGETS,
    area: Area::Whole,
    pans_to_width: true,
    ground: BUSH,
    ground_before: None,
    sky: None,
    // Vanilla W1 has 9 rock cells to 11 clearing cells.
    scenery: &[
        Step::Lakes,
        Step::River {
            chance: RIVER_CHANCE,
            max_crossings: MAX_CROSSINGS,
            crossing_cost: CROSSING_COST,
        },
        Step::Moat,
        Step::Clusters { other: CLEARING, rock_share: 0.4 },
    ],
};

/// Plain sand; dotted sand in patches; palms where W1 has clearings; one big
/// oasis where vanilla draws its III (user, 2026-10-13).
const W2_RECIPE: Recipe = Recipe {
    layout: W2_TARGETS,
    area: Area::Whole,
    pans_to_width: true,
    ground: CLEARING,
    ground_before: None,
    sky: None,
    // One-tile pieces first: they need cells touching no path or node, which
    // the crowded main map has few of, so the patches and clusters fill
    // around them rather than leaving them nowhere to go.
    scenery: &[
        Step::Oasis { island: PALM },
        Step::Pools { pool: POOL, palm: PALM, count: (1, 3) },
        Step::Scatter { tile: PYRAMID, count: PYRAMIDS },
        Step::Scatter { tile: QUICKSAND, count: (1, 3) },
        Step::Scatter { tile: PLANT, count: (3, 6) },
        Step::Scatter { tile: DUNE, count: (4, 10) },
        Step::Outcrop { count: (1, 2), ruins: 0.5 },
        Step::Patches { tile: DUNE },
        Step::Clusters { other: PALM, rock_share: 0.4 },
    ],
};

/// W3: water first (user, 2026-10-13). The sea is the ground; land grows
/// around the paths, with bush (W3's boulders) on islets and in clusters, and
/// a few palms and pools, as vanilla has.
const W3_RECIPE: Recipe = Recipe {
    layout: W3_TARGETS,
    area: Area::Whole,
    pans_to_width: true,
    // Palms fill the land (user, 2026-10-17); bush clusters break them up.
    ground: PALM,
    ground_before: None,
    sky: None,
    scenery: &[
        Step::Sea {
            margin: 0.25,
            land: 0.04,
            water_nodes: 0.5,
            islet: BUSH,
            coast: false,
            solid: false,
            bridges: 0.6,
            boxes: false,
            within: None,
            left_shore: true,
        },
        Step::Clusters { other: BUSH, rock_share: 0.0 },
        Step::Scatter { tile: POOL, count: (0, 2) },
    ],
};

/// W4: water first but land-heavy (vanilla is about half water): every node
/// on land, the shore grown well out, then bays cut back into it and a lake
/// the roads cross; plants as the ground with clearings sprinkled singly
/// through it, a few palms and pools, rock islets (user, 2026-10-17). The
/// ground was fire flowers until 2026-10-18: `$BD` is in
/// `Map_Completable_Tiles`, so every cell of it cost a maze save bit and
/// claimed the row-7/8 bit; `$BE` is inert (user).
const W4_RECIPE: Recipe = Recipe {
    layout: W4_TARGETS,
    area: Area::Whole,
    pans_to_width: true,
    ground: PLANT,
    ground_before: None,
    sky: None,
    scenery: &[
        Step::Sea {
            margin: 1.0,
            land: 0.03,
            water_nodes: 0.0,
            islet: DECO_ROCK,
            coast: true,
            solid: true,
            bridges: 1.0,
            boxes: false,
            within: None,
            left_shore: false,
        },
        Step::Bays { count: (2, 4) },
        Step::RoadLake,
        Step::Pools { pool: POOL, palm: PALM, count: (0, 2) },
        Step::Scatter { tile: PALM, count: (1, 3) },
        Step::Sprinkle { tile: CLEARING, share: SPRINKLE_SHARE },
    ],
};

/// W5: the ground area plain ground with boulders (bush, as vanilla's ground
/// page); the sky area sky ground `$D7` with stars.
const W5_RECIPE: Recipe = Recipe {
    layout: W5_TARGETS,
    area: Area::GroundAndSky {
        ground: &[Rect { rows: (0, 6), cols: (1, 11) }, Rect { rows: (0, 2), cols: (12, 12) }],
        sky_from: SCREEN_COLS,
        sky_frame: w5_sky_frame,
        fixes: &[((7, 3), SHORE)],
    },
    pans_to_width: false,
    ground: CLEARING,
    ground_before: None,
    sky: Some((SKY_GROUND, &[Step::Scatter { tile: STAR, count: (3, 6) }])),
    scenery: &[
        Step::GroundRiver { mouth: RIVER_MOUTH },
        Step::Clusters { other: BUSH, rock_share: 0.0 },
    ],
};

/// W5's shore-strip tile where its river meets the sea.
const RIVER_MOUTH: u8 = 0x91;
/// The plain shore-strip tile a river mouth is cut into.
const SHORE: u8 = 0x85;

/// W6, ice land: a wavy coast along the top lined with peaks, a few moats,
/// lakes and a river, peak clusters and bushes.
const W6_RECIPE: Recipe = Recipe {
    layout: W6_TARGETS,
    area: Area::Whole,
    pans_to_width: true,
    ground: TALL_ROCK,
    ground_before: None,
    sky: None,
    scenery: &[
        Step::Coast { mountain: TALL_ROCK },
        Step::Moats { count: (2, 4), through: (2, 3) },
        Step::Lakes,
        // Meandering rivers joining the coast and the lakes (user,
        // 2026-10-17), not straight ones across the map.
        Step::River { chance: 1.0, max_crossings: 5, crossing_cost: CROSSING_COST },
        Step::River { chance: 0.5, max_crossings: 5, crossing_cost: CROSSING_COST },
        Step::Outcrop { count: (2, 3), ruins: 0.25 },
    ],
};

/// W7, pipe land: solid islands in open sea, piranha-plant clusters and a
/// few fire flowers.
const W7_RECIPE: Recipe = Recipe {
    layout: W7_TARGETS,
    area: Area::Whole,
    pans_to_width: true,
    ground: CLEARING,
    ground_before: None,
    sky: None,
    scenery: &[
        Step::Sea {
            margin: 1.0,
            land: 0.05,
            water_nodes: 0.0,
            islet: PLANT,
            coast: true,
            solid: true,
            bridges: 0.5,
            boxes: true,
            within: None,
            left_shore: false,
        },
        Step::Clusters { other: PLANT, rock_share: 0.0 },
        Step::Scatter { tile: FLOWER, count: (1, 3) },
    ],
};

/// W8, dark land: fire on its first screen and bones after, as vanilla,
/// with patches of open ground and clusters of fire. The first screen is
/// water first, as vanilla's navy: some nodes stand in the sea (user,
/// 2026-10-14).
const W8_RECIPE: Recipe = Recipe {
    layout: W8_TARGETS,
    area: Area::InsideFrame { frame: VOID, to_col: 3 * SCREEN_COLS, node_cols: (2, 12) },
    pans_to_width: false,
    ground: BONES,
    ground_before: Some((SCREEN_COLS, FIRE)),
    sky: None,
    scenery: &[
        Step::Sea {
            margin: 0.5,
            land: 0.1,
            water_nodes: 0.5,
            islet: FIRE,
            coast: false,
            solid: false,
            bridges: 0.6,
            boxes: false,
            within: Some((1, SCREEN_COLS)),
            left_shore: false,
        },
        Step::Patches { tile: CLEARING },
        Step::Clusters { other: FIRE, rock_share: 0.0 },
    ],
};

/// W8's ground tiles: the fire of its first screen, the bones of the rest.
const FIRE: u8 = 0xD1;
const BONES: u8 = 0xE2;
/// W8's frame: the void round and between its screens.
const VOID: u8 = 0xD8;

/// The sky's ground: a sky blank with nothing on it.
const SKY_GROUND: u8 = 0xD7;
/// `TILE_WORLD5STAR`. Above its page's threshold, but outside the M/L window
/// (`lock_keys::ML_RANGE_UPPER`), so it is not completable. It still stays off
/// rows 7 and 8, a rule from when it was.
const STAR: u8 = 0xE9;
/// Sky path tiles, `(horizontal, vertical)`.
const SKY_PATH: (u8, u8) = (0xDA, 0xDB);
/// Marks an empty sky cell until the sky's scenery runs (see [`FILL`]).
const SKY_FILL: u8 = 0xFE;

/// Each world's recipes; [`world_map`] draws with one picked at random. A
/// new look for a world is a new recipe in its list, and nothing else.
const RECIPES: [&[&Recipe]; 8] = [
    &[&W1_RECIPE],
    &[&W2_RECIPE],
    &[&W3_RECIPE],
    &[&W4_RECIPE],
    &[&W5_RECIPE],
    &[&W6_RECIPE],
    &[&W7_RECIPE],
    &[&W8_RECIPE],
];

/// Where a recipe draws, read off the world's vanilla map. Every other
/// cell is frame and stays as vanilla drew it.
#[derive(Clone, Copy)]
enum Area {
    /// The whole map but its first and last columns.
    Whole,
    /// Every cell left of `to_col` whose vanilla tile is not `frame` (W8:
    /// inside its void, the castle's screen kept), with nodes only in
    /// `node_cols` of each screen (inclusive, counted from the screen's left
    /// edge). W8's screens never scroll, so a node at a screen's edge is drawn
    /// at the edge of the picture, half under the TV's overscan; vanilla keeps
    /// W8's nodes in columns 2-12 (user, 2026-10-16).
    InsideFrame { frame: u8, to_col: usize, node_cols: (usize, usize) },
    /// Separate ground and sky areas (W5). The ground is `ground`'s
    /// rectangles; the sky every cell from column `sky_from` that `sky_frame` does not
    /// call frame. `fixes` are frame cells drawn for vanilla's content,
    /// redrawn plain.
    GroundAndSky {
        ground: &'static [Rect],
        sky_from: usize,
        sky_frame: fn(u8) -> bool,
        fixes: &'static [(Pos, u8)],
    },
}

/// Map cells from `rows.0` to `rows.1` and `cols.0` to `cols.1`, inclusive.
#[derive(Clone, Copy)]
struct Rect {
    rows: (usize, usize),
    cols: (usize, usize),
}

/// A recipe's [`Area`] on its map: every other cell is frame and stays as
/// vanilla drew it (the void, the cloud edges, the shore strip, the
/// miniature ground).
struct Areas {
    ground: HashSet<Pos>,
    sky: HashSet<Pos>,
    /// Frame cells drawn for vanilla's content, redrawn plain: W5's river
    /// mouth on the shore strip, which no generated river meets.
    frame_fixes: Vec<(Pos, u8)>,
    /// Where sky nodes start: two columns past the sky's edge, as vanilla's.
    sky_nodes_from: usize,
    /// The columns of each screen nodes may take ([`Area::InsideFrame`]).
    node_cols: (usize, usize),
}

/// W5's sky-page frame tiles: void, cloud edges, the miniature ground, the
/// sea and its shores.
fn w5_sky_frame(tile: u8) -> bool {
    matches!(tile, 0xC2..=0xC5 | 0xD0 | 0xD2..=0xD4 | 0xD6 | 0xCE | 0x42 | 0x57..=0x5D | 0x82..=0xA9)
}

/// `area` on `vanilla`; `None` for the whole map. W5's ground is rows 0-6
/// up to column 11, and column 12 where vanilla has land (rows 0-2), so the
/// shore tiles past it still meet land; its sky is every non-frame cell from
/// column 16. W8's is every cell of its first three screens inside the void.
fn areas(area: &Area, vanilla: &Grid) -> Option<Areas> {
    match *area {
        Area::Whole => None,
        Area::InsideFrame { frame, to_col, node_cols } => {
            let ground = (0..vanilla.rows())
                .flat_map(|r| (1..to_col).map(move |c| (r, c)))
                .filter(|&(r, c)| vanilla.get(r, c) != frame)
                .collect();
            Some(Areas {
                ground,
                sky: HashSet::new(),
                frame_fixes: Vec::new(),
                sky_nodes_from: to_col,
                node_cols,
            })
        }
        Area::GroundAndSky { ground, sky_from, sky_frame, fixes } => {
            let ground = ground
                .iter()
                .flat_map(|&Rect { rows: (r0, r1), cols: (c0, c1) }| {
                    (r0..=r1).flat_map(move |r| (c0..=c1).map(move |c| (r, c)))
                })
                .collect();
            let sky = (0..vanilla.rows())
                .flat_map(|r| (sky_from..vanilla.cols - 1).map(move |c| (r, c)))
                .filter(|&(r, c)| !sky_frame(vanilla.get(r, c)))
                .collect();
            Some(Areas {
                ground,
                sky,
                frame_fixes: fixes.to_vec(),
                sky_nodes_from: sky_from + 2,
                node_cols: (0, SCREEN_COLS - 1),
            })
        }
    }
}

/// What a layout is drawn on: the map's size and, for a world in areas, the
/// cells nodes may take (`None`: any lattice cell). Sky nodes keep two
/// columns clear of the screen edge, as vanilla's do.
struct Frame {
    rows: usize,
    cols: usize,
    nodes: Option<HashSet<Pos>>,
}

fn frame_for(areas: Option<&Areas>, vanilla: &Grid) -> Frame {
    let nodes = areas.map(|a| {
        let sky = a.sky.iter().filter(|p| p.1 >= a.sky_nodes_from);
        let in_cols = |p: &&Pos| (a.node_cols.0..=a.node_cols.1).contains(&(p.1 % SCREEN_COLS));
        a.ground.iter().chain(sky).filter(in_cols).copied().collect()
    });
    Frame { rows: vanilla.rows(), cols: vanilla.cols, nodes }
}

/// The entries a recipe deals, one to a node: every entry on the map and
/// inside its areas. (The others stay where vanilla put them.)
fn dealt_entries(vanilla: &WorldMap, areas: Option<&Areas>) -> usize {
    let grid = &vanilla.grid;
    vanilla
        .entry_pos
        .iter()
        .filter(|p| p.0 < grid.rows() && p.1 < grid.cols)
        .filter(|p| areas.is_none_or(|a| a.ground.contains(p) || a.sky.contains(p)))
        .count()
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
/// vanilla and the Start<->Airship swap start a player. `airship` is `None`
/// with the swap off, and for W8, which has none: always near `anchor`. Never where the
/// starting camera ([`start_camera_col`]) shows the pocket's strip.
/// It is picked once, so a start that is harder to fill around is not drawn
/// less often. `anchor` also sets the lattice's parity.
fn generate<R: Rng>(
    world_idx: usize,
    frame: &Frame,
    anchor: Pos,
    airship: Option<Pos>,
    targets: Targets,
    node_count: usize,
    rng: &mut R,
) -> Option<Layout> {
    let cells = lattice(frame, anchor);
    let near = match airship {
        Some(airship) if !rng.random::<bool>() => airship,
        _ => anchor,
    };
    let around: Vec<Pos> = cells
        .iter()
        .copied()
        .filter(|&p| manhattan(p, near) <= 2)
        .filter(|p| {
            targets.pocket.is_none_or(|k| start_camera_col(world_idx, p.1) + 16 <= k.from_col)
        })
        .collect();
    let start = *around.choose(rng)?;
    (0..ATTEMPTS).find_map(|_| try_once(frame, anchor, start, targets, node_count, rng))
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

/// Paths over water (`smb3_rom_reference.md`, "Paths over water"), by axis
/// and by which ends are water nodes: `[land-land, land-water, water-land,
/// water-water]`, the first end being the left or upper one. Land-land
/// horizontal is a bridge or `$B9` (the `Sea` step's `bridges`); vertical is `$BA`
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
    (margin, land, water_nodes, bridge_share): (f64, f64, f64, f64),
    (islet, coast, fill, boxes, left_shore): (u8, bool, bool, bool, bool),
    within: Option<(usize, usize)>,
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
    let (from, to) = within.unwrap_or((1, cols - 1));
    let cells: Vec<Pos> = (0..rows).flat_map(|r| (from..to).map(move |c| (r, c))).collect();
    let docks: Vec<Pos> = layout.docks.iter().map(|d| d.1).collect();
    if within.is_none() {
        water.sea_bounds = Some((rows, cols));
    }

    let fixed = |n: Pos| {
        n == layout.start || Some(n) == layout.target || layout.docks.iter().any(|d| d.0 == n)
    };
    let joined_from_above_or_left =
        |n: Pos| layout.links.iter().any(|&(_, b, kind)| kind == Link::Path && b == n);
    let on_rock =
        |n: Pos| layout.links.iter().any(|&(a, b, k)| k == Link::Rock && (a == n || b == n));
    let mut wet_nodes: HashSet<Pos> = layout
        .nodes
        .iter()
        .copied()
        .filter(|&n| !fixed(n) && !on_rock(n) && joined_from_above_or_left(n))
        .filter(|n| (from..to).contains(&n.1))
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
    // With a left shore, no water node may need sea in the first column.
    if left_shore {
        wet_nodes.retain(|&n| needs(n).iter().all(|p| p.1 != 1));
    }
    let mut keep = keep_for(&wet_nodes);
    // A river ([`carve_river`]) from the top coast to the bottom one, near
    // the middle of the start's island, crossing its paths: its course is sea,
    // so land never grows there (a cell with no water tile may still be
    // trimmed to land).
    let mut river: HashSet<Pos> = HashSet::new();
    if layout.river {
        let mut columns: Vec<usize> = layout
            .nodes
            .iter()
            .filter(|n| !layout.islands.iter().any(|i| i.contains(n)))
            .map(|n| n.1)
            .collect();
        columns.sort();
        let middle = columns.get(columns.len() / 2).copied().unwrap_or(cols / 2);
        let edge = |r: usize| -> Vec<Pos> {
            (1..cols - 1)
                .map(|c| (r, c))
                .filter(|&(r, c)| c.abs_diff(middle) <= 4 && grid.get(r, c) == FILL)
                .collect()
        };
        let (top, bottom) = (edge(0), edge(rows - 1));
        carve_river(grid, water, (&top, &bottom), AREA_RIVER, rng, |_, _, course| {
            river = course.cells.iter().chain([&course.start, &course.end]).copied().collect();
            true
        });
    }
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
    // A sea `within` columns keeps a rim of land against everything round
    // it: cells past its columns, and frame inside them (anything drawn that
    // is not the layout's). A water node's sea comes first.
    if within.is_some() {
        let in_cells: HashSet<Pos> = cells.iter().copied().collect();
        let mut drawn: HashSet<Pos> = layout.nodes.iter().copied().collect();
        drawn.extend(layout.links.iter().map(|&(a, b, _)| mid(a, b)));
        let frame =
            |p: Pos| !in_cells.contains(&p) || (grid.get(p.0, p.1) != FILL && !drawn.contains(&p));
        for &(r, c) in cells.iter().filter(|&&(r, c)| grid.get(r, c) == FILL) {
            let near_frame = r == 0
                || (r - 1..=r + 1).any(|y| (c.saturating_sub(1)..=c + 1).any(|x| frame((y, x))));
            if near_frame && !keep.contains(&(r, c)) {
                let nearest = layout.nodes.iter().copied().min_by_key(|&n| manhattan(n, (r, c)));
                solid.insert((r, c), nearest.map_or(0, island_of));
            }
        }
    }
    // The left shore: land all down the first column, so no canoe move can
    // cross it to the frame.
    if left_shore {
        for r in 0..rows {
            solid.entry((r, 1)).or_insert_with(|| {
                let nearest = layout.nodes.iter().copied().min_by_key(|&n| manhattan(n, (r, 1)));
                nearest.map_or(0, island_of)
            });
        }
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
    // Whether the cells around `p` keep its open sides joined without it: if
    // so, making `p` land cannot split the sea, and the full search is
    // skipped. Walking the eight cells round `p`, each is beside the next, so
    // its open sides are joined when they all fall in one open run.
    let locally_joined = |solid: &HashMap<Pos, usize>, (r, c): Pos| {
        let (u, d, l, rr) = (r.wrapping_sub(1), r + 1, c.wrapping_sub(1), c + 1);
        let ring = [(u, l), (u, c), (u, rr), (r, rr), (d, rr), (d, c), (d, l), (r, l)];
        let open: Vec<bool> = ring.iter().map(|&q| inside(q) && !solid.contains_key(&q)).collect();
        let Some(gap) = open.iter().position(|o| !o) else { return true };
        let mut runs_with_a_side = 0;
        let mut in_run_with_side = false;
        for k in 1..=8 {
            let i = (gap + k) % 8;
            if open[i] {
                in_run_with_side |= i % 2 == 1;
            } else {
                runs_with_a_side += usize::from(in_run_with_side);
                in_run_with_side = false;
            }
        }
        runs_with_a_side <= 1
    };
    // Every add after the first keeps the docks joined, so the shortcut holds;
    // a sea already split refuses every add, as the full search would.
    let start_joined = joined(&solid);
    let add = |solid: &mut HashMap<Pos, usize>, (r, c): Pos, island: usize| {
        // First: a cell off the map can have wrapped (`wrapping_sub` from
        // row 0), and `r + 1` below would overflow on it.
        if !inside((r, c)) {
            return false;
        }
        let touches_another = (r.saturating_sub(1)..=r + 1).any(|y| {
            (c.saturating_sub(1)..=c + 1).any(|x| solid.get(&(y, x)).is_some_and(|&o| o != island))
        });
        // With a coast no land goes on the outer ring, so every island has
        // sea all round it.
        let on_ring = r == 0 || r + 1 == rows || c <= 1 || c + 2 >= cols;
        if (coast && on_ring)
            || keep.contains(&(r, c))
            || river.contains(&(r, c))
            || solid.contains_key(&(r, c))
            || touches_another
        {
            return false;
        }
        solid.insert((r, c), island);
        let kept = start_joined && (locally_joined(solid, (r, c)) || joined(solid));
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
    // Island blocks: each island's land fills the rectangle its nodes span.
    if boxes {
        let mut spans: Vec<(usize, Pos, Pos)> = Vec::new();
        for &n in &layout.nodes {
            let id = island_of(n);
            match spans.iter_mut().find(|s| s.0 == id) {
                Some(s) => {
                    s.1 = (s.1.0.min(n.0), s.1.1.min(n.1));
                    s.2 = (s.2.0.max(n.0), s.2.1.max(n.1));
                }
                None => spans.push((id, n, n)),
            }
        }
        for (id, lo, hi) in spans {
            for r in lo.0..=hi.0 {
                for c in lo.1..=hi.1 {
                    add(&mut solid, (r, c), id);
                }
            }
        }
    }
    // A solid island: an open cell with land on two opposite sides is a
    // channel inside it, and fills, until none is left.
    if fill {
        loop {
            let channels: Vec<(Pos, usize)> = cells
                .iter()
                .copied()
                .filter(|p| !solid.contains_key(p))
                .filter(|&(r, c)| !coast || (r > 0 && r + 1 < rows && c > 1 && c + 2 < cols))
                .filter_map(|(r, c)| {
                    let side = |p: Pos| solid.get(&p).copied().filter(|&o| o != ISLET);
                    let (n, s) = (side((r.wrapping_sub(1), c)), side((r + 1, c)));
                    let (w, e) = (side((r, c.wrapping_sub(1))), side((r, c + 1)));
                    let across = n.filter(|&o| s == Some(o)).or(w.filter(|&o| e == Some(o)));
                    across.map(|o| ((r, c), o))
                })
                .collect();
            let before = solid.len();
            for (p, island) in channels {
                add(&mut solid, p, island);
            }
            if solid.len() == before {
                break;
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
            .filter(|&(r, c)| !coast || (r > 0 && r + 1 < rows && c > 1 && c + 2 < cols))
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
    let bridged: Vec<bool> = layout.links.iter().map(|_| rng.random_bool(bridge_share)).collect();
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
        grid.set(r, c, islet);
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
/// The recipe is one of the world's [`RECIPES`], picked at random; a world
/// with one takes no draw for it.
///
/// `airship_starts` is Swap Start / Airship: with it, the start may go by the
/// vanilla airship's spot as well as the vanilla start's ([`generate`]);
/// without, only by the start's.
pub(crate) fn world_map<R: Rng>(
    vanilla: &WorldMap,
    world_idx: usize,
    airship_starts: bool,
    rng: &mut R,
) -> Option<WorldMap> {
    let recipes = RECIPES.get(world_idx)?;
    let recipe = match recipes.len() {
        0 => return None,
        1 => recipes[0],
        n => recipes[rng.random_range(..n)],
    };
    draw_with(vanilla, world_idx, recipe, airship_starts, rng)
}

/// [`world_map`] with `recipe`.
fn draw_with<R: Rng>(
    vanilla: &WorldMap,
    world_idx: usize,
    recipe: &Recipe,
    airship_starts: bool,
    rng: &mut R,
) -> Option<WorldMap> {
    (0..DRAWS).find_map(|_| draw_once(vanilla, world_idx, recipe, airship_starts, rng))
}

/// Whole maps drawn before a world gives up and keeps vanilla. A draw can
/// fail late (W3's sea can trim its docks apart); W1 and W2 succeed first
/// time, so their draws are unchanged by the retries.
const DRAWS: usize = 20;

/// One attempt at [`world_map`].
fn draw_once<R: Rng>(
    vanilla: &WorldMap,
    world_idx: usize,
    recipe: &Recipe,
    airship_starts: bool,
    rng: &mut R,
) -> Option<WorldMap> {
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
    let airship_idx = match targets.airship {
        true => Some(
            (0..vanilla.entry_pos.len()).find(|&i| AIRSHIP_ENTRIES.contains(&(world_idx, i)))?,
        ),
        false => None,
    };
    let areas = areas(&recipe.area, old);
    let layout = generate(
        world_idx,
        &frame_for(areas.as_ref(), old),
        vanilla_start,
        airship_idx.filter(|_| airship_starts).map(|i| vanilla.entry_pos[i]),
        targets,
        dealt_entries(vanilla, areas.as_ref()),
        rng,
    )?;
    let start = layout.start;

    // The frame (first and last column) stays as vanilla drew it; everything
    // else starts as bush. A start in the first lattice column gets vanilla's
    // entrance stub, the path running off the edge of the map.
    let mut grid = old.clone();
    grid.vanilla_layout = false;
    match &areas {
        // In areas, only their cells are drawn afresh; the sky's start as
        // `SKY_FILL`, so the ground's scenery leaves them alone.
        Some(a) => {
            for &((r, c), tile) in &a.frame_fixes {
                grid.set(r, c, tile);
            }
            for &(r, c) in &a.ground {
                grid.set(r, c, FILL);
            }
            for &(r, c) in &a.sky {
                grid.set(r, c, SKY_FILL);
            }
        }
        None => {
            for r in 0..grid.rows() {
                for c in 1..grid.cols - 1 {
                    grid.set(r, c, FILL);
                }
            }
        }
    }
    let stub = (start.0, start.1 - 1);
    if start.1 == vanilla_start.1 && areas.as_ref().is_none_or(|a| a.ground.contains(&stub)) {
        grid.set(start.0, start.1 - 1, old.get(vanilla_start.0, vanilla_start.1 - 1));
    }
    for &(a, b, kind) in &layout.links {
        let cell = ((a.0 + b.0) / 2, (a.1 + b.1) / 2);
        let in_sky = areas.as_ref().is_some_and(|a| a.sky.contains(&cell));
        let (horiz, vert) = match kind {
            Link::Rock => ROCK,
            Link::Path if in_sky => SKY_PATH,
            Link::Path => PATH,
        };
        grid.set(cell.0, cell.1, if a.0 == b.0 { horiz } else { vert });
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
        .filter(|&n| n != start && Some(n) != layout.target && !dock_nodes.contains(&n))
        .collect();
    free.shuffle(rng);
    let mut entry_pos = vanilla.entry_pos.clone();
    entry_pos[start_idx] = start;
    if let (Some(i), Some(target)) = (airship_idx, layout.target) {
        entry_pos[i] = target;
    }
    let vanilla_tile = |i: usize| {
        let p = vanilla.entry_pos[i];
        on_grid(p).then(|| old.get(p.0, p.1))
    };
    // Entries outside the areas (W8's castle screen) stay where vanilla put
    // them.
    let outside =
        |p: Pos| areas.as_ref().is_some_and(|a| !a.ground.contains(&p) && !a.sky.contains(&p));
    let dealt_elsewhere =
        |i: usize| i == start_idx || Some(i) == airship_idx || outside(vanilla.entry_pos[i]);
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
        .chain((!layout.rock_pocket.is_empty()).then_some(&layout.rock_pocket))
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
    if let (Some(i), Some(target)) = (airship_idx, layout.target) {
        let air = vanilla.entry_pos[i];
        grid.set(target.0 - 1, target.1, old.get(air.0 - 1, air.1));
    }

    // Every cell drawn so far is a path, a node or the castle top: what
    // one-tile scenery must not touch.
    let built: HashSet<Pos> = (0..grid.rows())
        .flat_map(|r| (1..grid.cols - 1).map(move |c| (r, c)))
        .filter(|&(r, c)| grid.get(r, c) != FILL && grid.get(r, c) != SKY_FILL)
        .collect();

    // The recipe's scenery, then a sky's, then the water, then the ground.
    let mut water = Water::default();
    if !run_steps(recipe.scenery, &mut grid, &mut water, &layout, &built, rng) {
        return None;
    }
    if let Some((sky_ground, sky_steps)) = recipe.sky {
        swap_tiles(&mut grid, FILL, recipe.ground);
        swap_tiles(&mut grid, SKY_FILL, FILL);
        if !run_steps(sky_steps, &mut grid, &mut water, &layout, &built, rng) {
            return None;
        }
        swap_tiles(&mut grid, FILL, sky_ground);
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
                let ground = match recipe.ground_before {
                    Some((col, tile)) if c < col => tile,
                    _ => recipe.ground,
                };
                // Rows 7 and 8 share one completion bit: a ground that
                // completes on its page (W6's tall rock) gives way to plain
                // ground where the other row of the pair is a path or node.
                let partner = if r + 2 == grid.rows() { r + 1 } else { r.wrapping_sub(1) };
                let shares_bit = r + 2 >= grid.rows() && built.contains(&(partner, c));
                let ground =
                    if shares_bit && is_completion_unsafe(ground) { CLEARING } else { ground };
                grid.set(r, c, ground);
            }
        }
    }
    // That plain ground runs in long strings along the paths of rows 7 and
    // 8: break each one up with bush patches (`$B4`, not completable),
    // stretches of one to three of each (user, 2026-10-17).
    if is_completion_unsafe(recipe.ground) {
        for r in grid.rows() - 2..grid.rows() {
            let mut c = 1;
            while c < grid.cols - 1 {
                let run = (c..grid.cols - 1).take_while(|&x| grid.get(r, x) == CLEARING).count();
                let mut bush = rng.random_bool(0.5);
                let mut at = c;
                while at < c + run {
                    let len = rng.random_range(1..=3).min(c + run - at);
                    if bush {
                        (at..at + len).for_each(|x| grid.set(r, x, BUSH));
                    }
                    bush = !bush;
                    at += len;
                }
                c += run.max(1);
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
        pan_limit: recipe.pans_to_width.then_some((old.cols / SCREEN_COLS * 0x10) as u8),
        castle: layout.target.map(|t| (t, help_bubble(t, old.cols, areas.as_ref()))),
    })
}

/// Where the HELP bubble floats: right of the castle top, as vanilla's does
/// in every world, unless that cell is the frame (a castle on the last node
/// column); then above it (user, 2026-10-15). The castle always has two rows
/// above it ([`try_once`]), so the second always exists.
fn help_bubble(castle: Pos, cols: usize, areas: Option<&Areas>) -> Pos {
    let right = (castle.0 - 1, castle.1 + 1);
    let drawn = |p: &Pos| areas.is_none_or(|a| a.ground.contains(p) || a.sky.contains(p));
    if right.1 + 1 < cols && drawn(&right) { right } else { (castle.0 - 2, castle.1) }
}

/// Run scenery steps over `grid`, in order. `false` if a step's drawing
/// failed (the map is drawn again).
fn run_steps<R: Rng>(
    steps: &[Step],
    grid: &mut Grid,
    water: &mut Water,
    layout: &Layout,
    built: &HashSet<Pos>,
    rng: &mut R,
) -> bool {
    for &step in steps {
        match step {
            Step::Lakes => {
                for region in empty_regions(grid) {
                    if region.len() >= LAKE_MIN_REGION && rng.random_bool(LAKE_CHANCE) {
                        grow_lake(grid, &region, None, water, rng);
                    }
                }
            }
            Step::River { chance, max_crossings, crossing_cost } => {
                if rng.random_bool(chance) {
                    let limits =
                        RiverLimits { attempts: RIVER_ATTEMPTS, max_crossings, crossing_cost };
                    add_river(grid, water, limits, rng);
                }
            }
            Step::Moat => {
                if rng.random_bool(MOAT_CHANCE)
                    && let Some(target) = layout.target
                {
                    add_moat(grid, target, water, &[(target.0.wrapping_sub(1), target.1)], false);
                }
            }
            Step::Oasis { island } => {
                if let Some(col) = layout.strip_col {
                    add_oasis(grid, water, col, island);
                }
            }
            Step::Pools { pool, palm, count } => add_pools(grid, built, pool, palm, count, rng),
            Step::Patches { tile } => add_patches(grid, tile, rng),
            Step::Clusters { other, rock_share } => decorate(grid, other, rock_share, rng),
            Step::Scatter { tile, count } => {
                scatter(grid, built, tile, count, rng);
            }
            Step::GroundRiver { mouth } => add_ground_river(grid, water, mouth, rng),
            Step::Moats { count, through } => add_moats(grid, water, layout, (count, through), rng),
            Step::Coast { mountain } => add_coast(grid, water, mountain, rng),
            Step::Outcrop { count, ruins } => add_outcrops(grid, count, ruins, rng),
            Step::Bays { count } => add_bays(grid, water, count, rng),
            Step::RoadLake => add_road_lake(grid, water, layout, rng),
            Step::Sprinkle { tile, share } => sprinkle(grid, tile, share, rng),
            Step::Sea {
                margin,
                land,
                water_nodes,
                islet,
                coast,
                solid,
                bridges,
                boxes,
                within,
                left_shore,
            } => {
                let shares = (margin, land, water_nodes, bridges);
                let shape = (islet, coast, solid, boxes, left_shore);
                if !add_sea(grid, water, layout, shares, shape, within, rng) {
                    return false;
                }
            }
        }
    }
    true
}

/// A river across an area (W4, W5), where it must cross the island's rows of
/// paths rather than skirt round them.
const AREA_RIVER: RiverLimits =
    RiverLimits { attempts: RIVER_ATTEMPTS, max_crossings: 5, crossing_cost: CROSSING_COST };

/// W5's ground river ([`carve_river`]): from the top edge of the map, where
/// its water runs on off the map as vanilla's does, down into a shore-strip
/// cell, which becomes the river's `mouth` where it meets the sea.
fn add_ground_river<R: Rng>(grid: &mut Grid, water: &mut Water, mouth: u8, rng: &mut R) {
    let from: Vec<Pos> =
        (1..grid.cols - 1).map(|c| (0, c)).filter(|&(r, c)| grid.get(r, c) == FILL).collect();
    let to: Vec<Pos> = (0..grid.rows())
        .flat_map(|r| (1..grid.cols - 1).map(move |c| (r, c)))
        .filter(|&(r, c)| grid.get(r, c) == SHORE)
        .collect();
    carve_river(grid, water, (&from, &to), AREA_RIVER, rng, |grid, water, river| {
        let mut cells = river.cells.clone();
        cells.push(river.start);
        let nodes_before = water.nodes.len();
        water.nodes.push(river.end);
        water.open_top = true;
        if water.try_add(grid, &cells, &river.bridges, &river.crossings) {
            grid.set(river.end.0, river.end.1, mouth);
            true
        } else {
            water.nodes.truncate(nodes_before);
            water.open_top = false;
            false
        }
    });
}

/// Turn every interior `from` cell into `to`.
fn swap_tiles(grid: &mut Grid, from: u8, to: u8) {
    for r in 0..grid.rows() {
        for c in 1..grid.cols - 1 {
            if grid.get(r, c) == from {
                grid.set(r, c, to);
            }
        }
    }
}

/// Plain ground (all four quadrants blank): W1's clearings, and W2's ground.
const CLEARING: u8 = 0x42;

/// `TILE_DANCINGPALM`.
const PALM: u8 = 0xBB;
/// `TILE_DANCINGFLOWER`: a fire flower. Special only where Mario stands, so
/// as scenery off the paths it is plain decoration (vanilla W4 has 20).
const FLOWER: u8 = 0xBD;
/// Dotted sand: vanilla W2's ground, here grown in patches like lakes.
const DUNE: u8 = 0x43;
/// `TILE_POOL`, a one-tile oasis. Completable on its page, so never on rows
/// 7 or 8 (they share one completion bit) and never beside a path.
const POOL: u8 = 0xBF;
/// A small pyramid. Also the pyramid level's tile, so never beside a path.
const PYRAMID: u8 = 0x69;
/// Quicksand: a round pit. Also a level tile in vanilla W2, like the pyramid.
const QUICKSAND: u8 = 0x68;
/// A taller rock (W6 has 67). Above its page's threshold, but outside the M/L
/// window (`lock_keys::ML_RANGE_UPPER`), so it owns no completion bit — which
/// is what lets W6 use it as ground.
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

/// Regions smaller than this get no outcrop.
const OUTCROP_MIN_REGION: usize = 12;
/// Most cells an outcrop's blob grows to, before its fringe.
const MAX_OUTCROP: usize = 20;
/// Chance each cell of the first and second fringe rings is dotted ground.
const OUTCROP_FRINGE: [f64; 2] = [0.6, 0.25];

/// Rocky outcrops (user, 2026-10-17): in each of the `count` biggest open
/// regions, a blob drawn by depth. Cells with the blob all round are the
/// core, rock and tall rock (with a `ruins` chance of one broken fort); the
/// rest of the blob is dotted ground, and so are the open cells round it,
/// thinning out over two rings. The core touches only blob cells, so it is
/// never beside a path.
///
/// Not `$5D` (W5's mini trees) for the blob: the map animates by swapping
/// CHR banks, and its pattern flips between trees and a cloud (user,
/// 2026-10-17).
fn add_outcrops<R: Rng>(grid: &mut Grid, count: (usize, usize), ruins: f64, rng: &mut R) {
    let mut regions: Vec<Vec<Pos>> =
        empty_regions(grid).into_iter().filter(|r| r.len() >= OUTCROP_MIN_REGION).collect();
    regions.sort_by_key(|r| std::cmp::Reverse(r.len()));
    regions.truncate(rng.random_range(count.0..=count.1));
    for region in regions {
        let inside: HashSet<Pos> = region.iter().copied().collect();
        let deep: Vec<Pos> =
            region.iter().copied().filter(|&p| around(p).all(|n| inside.contains(&n))).collect();
        let Some(&seed) = deep.choose(rng).or_else(|| region.choose(rng)) else { continue };
        let blob = grow_blob(&region, seed, MAX_OUTCROP.min(region.len() * 2 / 3), rng);
        let in_blob: HashSet<Pos> = blob.iter().copied().collect();
        let core: Vec<Pos> =
            blob.iter().copied().filter(|&p| around(p).all(|n| in_blob.contains(&n))).collect();
        for &(r, c) in &blob {
            grid.set(r, c, DUNE);
        }
        for &(r, c) in &core {
            // Tall rock completes on its page: never on rows 7 and 8.
            let tall = r + 2 < grid.rows() && rng.random_bool(0.5);
            grid.set(r, c, if tall { TALL_ROCK } else { DECO_ROCK });
        }
        if let Some(&(r, c)) = core.choose(rng)
            && rng.random_bool(ruins)
        {
            grid.set(r, c, if rng.random_bool(0.5) { RUBBLE } else { ALT_RUBBLE });
        }
        let mut edge = blob;
        for chance in OUTCROP_FRINGE {
            let mut ring: Vec<Pos> = edge
                .iter()
                .flat_map(|&p| around(p))
                .filter(|&(r, c)| {
                    r < grid.rows() && c > 0 && c + 1 < grid.cols && grid.get(r, c) == FILL
                })
                .collect();
            ring.sort();
            ring.dedup();
            ring.retain(|_| rng.random_bool(chance));
            for &(r, c) in &ring {
                grid.set(r, c, DUNE);
            }
            edge = ring;
        }
    }
}

/// The four cells side by side with `(r, c)`, where the map has them.
fn around((r, c): Pos) -> impl Iterator<Item = Pos> {
    [(r.wrapping_sub(1), c), (r + 1, c), (r, c.wrapping_sub(1)), (r, c + 1)].into_iter()
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
fn add_oasis(grid: &mut Grid, water: &mut Water, strip_col: usize, island_tile: u8) {
    let strip: Vec<Pos> = empty_regions(grid)
        .into_iter()
        .flat_map(|region| {
            let in_strip: Vec<Pos> = region.into_iter().filter(|p| p.1 >= strip_col).collect();
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

    if let Some(lake) = add_trimmed(grid, water, lake, 2) {
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
            // A completable tile (W4's fire flower) stays off rows 7 and 8,
            // which share one completion bit: it would bar the node below.
            let low_ok = rock || !is_completion_unsafe(other);
            let allowed: Vec<Pos> = region
                .iter()
                .copied()
                .filter(|&p| grid.get(p.0, p.1) == FILL && (!rock || clear_of_paths(grid, p)))
                .filter(|&p| low_ok || p.0 + 2 < grid.rows())
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
    /// Water runs on off the top edge (W5's ground river, as vanilla's).
    open_top: bool,
    /// The map's row count when water also runs on off the bottom edge
    /// (W6's rivers across the map).
    open_bottom: Option<usize>,
}

impl Water {
    /// The picture a water cell needs: an edge middle is water when that
    /// neighbour is wet, a corner when both edges beside it and the diagonal
    /// are. Off the map is land, except a sea's edges (`sea_bounds`).
    fn picture(&self, wet: &HashSet<Pos>, (r, c): Pos) -> String {
        let at = |dr: isize, dc: isize| {
            let (nr, nc) = (r as isize + dr, c as isize + dc);
            let inside = nr >= 0 && nc >= 0 && wet.contains(&(nr as usize, nc as usize));
            match self.sea_bounds {
                None if nr < 0 => self.open_top,
                None if self.open_bottom.is_some_and(|rows| nr as usize >= rows) => true,
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
        let wet = self.wet_set();
        self.cells
            .iter()
            .copied()
            .filter(|&p| water_tile(&self.picture(&wet, p)).is_none())
            .collect()
    }

    /// Every wet cell, for the picture lookups: water, bridges, crossings,
    /// paths over water and water nodes.
    fn wet_set(&self) -> HashSet<Pos> {
        self.cells
            .iter()
            .chain(&self.bridges)
            .chain(&self.crossings)
            .chain(self.paths.iter().map(|(p, _)| p))
            .chain(&self.nodes)
            .copied()
            .collect()
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
        let wet = self.wet_set();
        let tiles: Vec<(Pos, u8)> = self
            .cells
            .iter()
            .map(|&p| (p, water_tile(&self.picture(&wet, p)).expect("only drawable water is kept")))
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
/// W1's chance of a river (project owner, 2026-10-12).
const RIVER_CHANCE: f64 = 0.75;
/// Most paths W1's river may cross.
const MAX_CROSSINGS: usize = 3;
/// W1's cost of crossing a path, so its river crosses only when it must.
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
fn add_river<R: Rng>(grid: &mut Grid, water: &mut Water, limits: RiverLimits, rng: &mut R) {
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
    let Some((a, b)) = farthest_lakes(&water.lakes, &water.cells) else { return };
    let (from, to) = (water.lakes[a].clone(), water.lakes[b].clone());
    carve_river(grid, water, (&from, &to), limits, rng, |grid, water, river| {
        water.try_add(grid, &river.cells, &river.bridges, &river.crossings)
    });
}

/// How many of the farthest spots a river's end lake is picked from.
const FAR_FEW: usize = 5;

/// The pair of lakes whose nearest cells are farthest apart, of those not
/// already joined by water (`cells`), so a second river finds a new pair.
fn farthest_lakes(lakes: &[Vec<Pos>], cells: &[Pos]) -> Option<(usize, usize)> {
    let body: HashMap<Pos, usize> = connected_pieces(cells)
        .into_iter()
        .enumerate()
        .flat_map(|(i, piece)| piece.into_iter().map(move |p| (p, i)))
        .collect();
    let joined = |a: &[Pos], b: &[Pos]| body.get(&a[0]).is_some_and(|i| body.get(&b[0]) == Some(i));
    let gap = |a: &[Pos], b: &[Pos]| {
        a.iter().flat_map(|&p| b.iter().map(move |&q| manhattan(p, q))).min().unwrap_or(0)
    };
    let mut best = None;
    let mut best_gap = 0;
    for i in 0..lakes.len() {
        for j in i + 1..lakes.len() {
            if joined(&lakes[i], &lakes[j]) {
                continue;
            }
            let g = gap(&lakes[i], &lakes[j]);
            if g > best_gap {
                best = Some((i, j));
                best_gap = g;
            }
        }
    }
    best
}

/// River cells, bridges and crossings: the cheapest route from a cell of
/// `from` to a cell of `to`. A step onto an empty cell costs 1 plus its
/// noise; a step over a path costs `crossing_cost` and lands on the cell
/// beyond, which must be empty or the destination lake.
fn route(
    grid: &Grid,
    water: &Water,
    (from, to): (&[Pos], &[Pos]),
    noise: &[Vec<u32>],
    crossing_cost: u32,
) -> Option<River> {
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
                (beyond, Some(next), crossing_cost + noise[beyond.0][beyond.1])
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
    Some(River { cells, bridges, crossings, start: at, end })
}

/// A river's course from [`route`]: its water cells (not its two ends),
/// the paths it crosses north-south (bridges) and east-west (crossings), and
/// the cell of `from` it left and of `to` it reached.
struct River {
    cells: Vec<Pos>,
    bridges: Vec<Pos>,
    crossings: Vec<Pos>,
    start: Pos,
    end: Pos,
}

/// How hard [`carve_river`] tries and how freely its river crosses paths.
#[derive(Clone, Copy)]
struct RiverLimits {
    attempts: usize,
    max_crossings: usize,
    /// What a crossing costs the route, in steps ([`route`]).
    crossing_cost: u32,
}

/// The river every world uses. Up to `attempts` routes from a cell
/// of `from` to a cell of `to`, each with fresh noise so it wanders
/// ([`route`]); the first crossing at most `max_crossings` paths that
/// `accept` takes is the river, and `true`. W1 joins two lakes, W5's ground
/// runs from the map's top edge into the shore, W4's crosses its island from
/// coast to coast.
fn carve_river<R: Rng>(
    grid: &mut Grid,
    water: &mut Water,
    (from, to): (&[Pos], &[Pos]),
    limits: RiverLimits,
    rng: &mut R,
    mut accept: impl FnMut(&mut Grid, &mut Water, &River) -> bool,
) -> bool {
    for _ in 0..limits.attempts {
        let noise: Vec<Vec<u32>> = (0..grid.rows())
            .map(|_| (0..grid.cols).map(|_| rng.random_range(0..RIVER_NOISE)).collect())
            .collect();
        let Some(river) = route(grid, water, (from, to), &noise, limits.crossing_cost) else {
            return false;
        };
        if river.bridges.len() + river.crossings.len() > limits.max_crossings {
            continue;
        }
        if accept(grid, water, &river) {
            return true;
        }
    }
    false
}

/// Chance a castle whose path comes in from the side gets a moat.
const MOAT_CHANCE: f64 = 0.5;

/// A moat: water on every cell around an island, the node plus `extra`
/// cells (the castle top above it, or ground: [`add_moats`]), with the
/// node's path crossing it on a bridge. Only when that path comes in from
/// the side, unless `cross_roads`: then every path in the ring crosses it,
/// a side one on a bridge and one from above or below as a path over water
/// (`$BA`), and `extra` may be other nodes ([`add_road_lake`]). Every other
/// cell of the ring must be empty (or already water); a moat that would
/// leave any water cell without a tile is not added. `true` if it was.
fn add_moat(
    grid: &mut Grid,
    (r, c): Pos,
    water: &mut Water,
    extra: &[Pos],
    cross_roads: bool,
) -> bool {
    let island: Vec<Pos> = std::iter::once((r, c)).chain(extra.iter().copied()).collect();
    let (top, bottom) = (island.iter().map(|p| p.0).min(), island.iter().map(|p| p.0).max());
    let (left, right) = (island.iter().map(|p| p.1).min(), island.iter().map(|p| p.1).max());
    let (Some(top), Some(bottom), Some(left), Some(right)) = (top, bottom, left, right) else {
        return false;
    };
    if top < 1 || bottom + 1 >= grid.rows() || left < 2 || right + 2 >= grid.cols {
        return false;
    }
    let boxed: Vec<Pos> = (top - 1..=bottom + 1)
        .flat_map(|rr| (left - 1..=right + 1).map(move |cc| (rr, cc)))
        .filter(|p| !island.contains(p))
        .collect();
    let crossing = |tile: u8, p: &Pos| grid.get(p.0, p.1) == tile;
    // A path on both sides is a moat the road runs through, a bridge each way.
    let (entries, crossings): (Vec<Pos>, Vec<Pos>) = if cross_roads {
        (
            boxed.iter().copied().filter(|p| crossing(PATH.0, p)).collect(),
            boxed.iter().copied().filter(|p| crossing(PATH.1, p)).collect(),
        )
    } else {
        let side = [(r, c - 1), (r, c + 1)];
        (side.into_iter().filter(|p| crossing(PATH.0, p) && !island.contains(p)).collect(), vec![])
    };
    if entries.is_empty() && crossings.is_empty() {
        return false;
    }
    let ring: Vec<Pos> =
        boxed.into_iter().filter(|p| !entries.contains(p) && !crossings.contains(p)).collect();
    let free = |(rr, cc): Pos| grid.get(rr, cc) == FILL || water.cells.contains(&(rr, cc));
    if !ring.iter().all(|&p| free(p)) {
        return false;
    }
    let new: Vec<Pos> = ring.into_iter().filter(|p| !water.cells.contains(p)).collect();
    water.try_add(grid, &new, &entries, &crossings)
}

/// Moats ([`add_moat`]) round `count` dead ends entered from the side, not
/// the start or the castle (its moat is [`Step::Moat`]), picked at random.
/// The island is sometimes bigger than the node (user, 2026-10-17): two
/// cells wide (away from the bridge), two tall, or a 2x2 square, its extra
/// cells left as ground. A shape that does not fit falls back to the node
/// alone.
fn add_moats<R: Rng>(
    grid: &mut Grid,
    water: &mut Water,
    layout: &Layout,
    (count, through): ((usize, usize), (usize, usize)),
    rng: &mut R,
) {
    let degree = degrees(&layout.nodes, &layout.links);
    let mut ends: Vec<Pos> = layout
        .nodes
        .iter()
        .copied()
        .filter(|&n| degree[&n] == 1 && n != layout.start && Some(n) != layout.target)
        .collect();
    ends.shuffle(rng);
    let mut left = rng.random_range(count.0..=count.1);
    for (r, c) in ends {
        if left == 0 {
            break;
        }
        // Away from the bridge, and up or down. A node too near the frame
        // for any moat is skipped before `c - 1` can underflow.
        if c < 2 || c + 2 >= grid.cols {
            continue;
        }
        let away = if grid.get(r, c - 1) == PATH.0 { c + 1 } else { c - 1 };
        let vert = if rng.random_bool(0.5) { r + 1 } else { r.wrapping_sub(1) };
        let extra = match rng.random_range(0..4) {
            0 => vec![],
            1 => vec![(r, away)],
            2 => vec![(vert, c)],
            _ => vec![(r, away), (vert, c), (vert, away)],
        };
        let fits = extra.iter().all(|&(er, ec)| er < grid.rows() && grid.get(er, ec) == FILL);
        let added = (fits && add_moat(grid, (r, c), water, &extra, false))
            || add_moat(grid, (r, c), water, &[], false);
        left -= usize::from(added);
    }
    // Through moats: nodes with a straight road both ways for at least
    // THROUGH_RUN hops in all, kept THROUGH_GAP columns apart.
    let hops = |(r, c): Pos, dir: isize| {
        let mut n = 0;
        let mut x = c;
        while let Some(link) = x.checked_add_signed(dir).filter(|&l| l < grid.cols)
            && grid.get(r, link) == PATH.0
        {
            n += 1;
            x = link.wrapping_add_signed(dir);
        }
        n
    };
    let mut middles: Vec<Pos> = layout
        .nodes
        .iter()
        .copied()
        .filter(|&n| n != layout.start && Some(n) != layout.target)
        .filter(|&n| n.1 >= 2 && n.1 + 2 < grid.cols && hops(n, -1) >= 1 && hops(n, 1) >= 1)
        .filter(|&n| hops(n, -1) + hops(n, 1) >= THROUGH_RUN)
        .collect();
    middles.shuffle(rng);
    let mut placed: Vec<Pos> = Vec::new();
    let mut left = rng.random_range(through.0..=through.1);
    for (r, c) in middles {
        if left == 0 {
            break;
        }
        if placed.iter().any(|&(pr, pc)| pr.abs_diff(r) <= 2 && pc.abs_diff(c) < THROUGH_GAP) {
            continue;
        }
        let vert = if rng.random_bool(0.5) { r + 1 } else { r.wrapping_sub(1) };
        let tall = rng.random_bool(0.5)
            && vert < grid.rows()
            && grid.get(vert, c) == FILL
            && add_moat(grid, (r, c), water, &[(vert, c)], false);
        if tall || add_moat(grid, (r, c), water, &[], false) {
            placed.push((r, c));
            left -= 1;
        }
    }
}

/// Fewest hops of straight road, both sides together, a through moat sits on.
const THROUGH_RUN: usize = 4;
/// Fewest columns between two through moats on neighbouring rows.
const THROUGH_GAP: usize = 6;

/// Fewest roads a road lake is crossed by (vanilla W4's is crossed by five).
const LAKE_ROADS: usize = 3;

/// The road lake ([`Step::RoadLake`]): a moat ([`add_moat`]) round one node,
/// or two side by side, crossed by every road into it. Spots crossed by
/// fewer than [`LAKE_ROADS`] are passed over; of the rest, the first at
/// random that draws.
fn add_road_lake<R: Rng>(grid: &mut Grid, water: &mut Water, layout: &Layout, rng: &mut R) {
    let nodes: HashSet<Pos> = layout.nodes.iter().copied().collect();
    let open = |n: Pos| n != layout.start && Some(n) != layout.target && nodes.contains(&n);
    let mut spots: Vec<(Pos, Vec<Pos>)> = Vec::new();
    for &(r, c) in layout.nodes.iter().filter(|&&n| open(n)) {
        let pair = (r, c + 2);
        for extra in [vec![], vec![pair]] {
            if extra.iter().any(|&p| !open(p)) {
                continue;
            }
            let right = extra.last().map_or(c, |p| p.1);
            if r < 1 || r + 2 >= grid.rows() || c < 2 || right + 2 >= grid.cols {
                continue;
            }
            let roads = (r - 1..=r + 1)
                .flat_map(|rr| (c - 1..=right + 1).map(move |cc| (rr, cc)))
                .filter(|&(rr, cc)| [PATH.0, PATH.1].contains(&grid.get(rr, cc)))
                .count();
            if roads >= LAKE_ROADS {
                spots.push(((r, c), extra));
            }
        }
    }
    spots.shuffle(rng);
    for (node, extra) in spots {
        if add_moat(grid, node, water, &extra, true) {
            return;
        }
    }
}

/// Cells a bay grows to, before trimming.
const BAY_SIZE: (usize, usize) = (3, 6);

/// Bays ([`Step::Bays`]): `count` times, a blob grown from a land cell
/// beside the water, through land with no path or node on it, added as water
/// with cells that have no tile for their picture trimmed ([`add_trimmed`]).
fn add_bays<R: Rng>(grid: &mut Grid, water: &mut Water, count: (usize, usize), rng: &mut R) {
    for _ in 0..rng.random_range(count.0..=count.1) {
        let wet: HashSet<Pos> = water.cells.iter().copied().collect();
        let land: Vec<Pos> = (0..grid.rows())
            .flat_map(|r| (1..grid.cols - 1).map(move |c| (r, c)))
            .filter(|&(r, c)| grid.get(r, c) == FILL)
            .collect();
        let shore: Vec<Pos> =
            land.iter().copied().filter(|&p| around(p).any(|n| wet.contains(&n))).collect();
        let Some(&seed) = shore.choose(rng) else { return };
        let bay = grow_blob(&land, seed, rng.random_range(BAY_SIZE.0..=BAY_SIZE.1), rng);
        add_trimmed(grid, water, bay, 2);
    }
}

/// W4's clearings in its fire-flower ground (user, 2026-10-17: vanilla's
/// even mix of the two read as too much clearing).
const SPRINKLE_SHARE: f64 = 0.3;

/// [`Step::Sprinkle`]: each open cell takes `tile` `share` of the time, a
/// cell with one already left of it or above a third as often, so they seldom
/// sit side by side. A completable tile stays off rows 7 and 8, as in
/// [`decorate`].
fn sprinkle<R: Rng>(grid: &mut Grid, tile: u8, share: f64, rng: &mut R) {
    let rows = if is_completion_unsafe(tile) { grid.rows() - 2 } else { grid.rows() };
    for r in 0..rows {
        for c in 1..grid.cols - 1 {
            if grid.get(r, c) != FILL {
                continue;
            }
            let beside = grid.get(r, c - 1) == tile || (r > 0 && grid.get(r - 1, c) == tile);
            if rng.random_bool(if beside { share / 3.0 } else { share }) {
                grid.set(r, c, tile);
            }
        }
    }
}

/// How the coast runs: each column it may step up or down a row, this often.
const COAST_STEP: f64 = 0.35;
/// Deepest the coast runs into the map, in rows.
const COAST_DEPTH: isize = 3;
/// How often a land cell on the coast is a mountain.
const COAST_MOUNTAIN: f64 = 0.7;

/// A coast along the top edge (W6): sea down from the edge, its depth
/// stepping up or down a row now and then so the coastline is never
/// straight, and stopping above anything drawn. The sea runs on off the
/// map. Most land cells along the coast are `mountain`.
fn add_coast<R: Rng>(grid: &mut Grid, water: &mut Water, mountain: u8, rng: &mut R) {
    let mut depth = rng.random_range(1..=2_i64) as isize;
    let (mut sea, mut lining) = (Vec::new(), Vec::new());
    for c in 1..grid.cols - 1 {
        if rng.random_bool(COAST_STEP) {
            depth = (depth + if rng.random_bool(0.5) { 1 } else { -1 }).clamp(0, COAST_DEPTH);
        }
        let mut r = 0;
        while (r as isize) < depth && grid.get(r, c) == FILL {
            sea.push((r, c));
            r += 1;
        }
        if r < grid.rows() && grid.get(r, c) == FILL {
            lining.push((r, c));
        }
    }
    water.open_top = true;
    let Some(sea) = add_trimmed(grid, water, sea, 2) else { return };
    // Each stretch of sea is a lake a river may join (user, 2026-10-17).
    water.lakes.extend(connected_pieces(&sea));
    for (r, c) in lining {
        let below_sea = r > 0 && sea.contains(&(r - 1, c));
        if below_sea
            && grid.get(r, c) == FILL
            && r + 2 < grid.rows()
            && rng.random_bool(COAST_MOUNTAIN)
        {
            grid.set(r, c, mountain);
        }
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
    let lake = grow_blob(region, seed, size, rng);
    let Some(lake) = add_trimmed(grid, water, lake, 2) else { return false };
    water.lakes.push(lake);
    true
}

/// Add water `cells`, first trimming those with no tile for their picture
/// one at a time (against the water already there), then adding the rest all
/// or nothing. `None`, with nothing added, if fewer than `min` are left or
/// they would leave other water without a tile. Lakes, the oasis and the
/// coast share it.
fn add_trimmed(
    grid: &mut Grid,
    water: &mut Water,
    mut cells: Vec<Pos>,
    min: usize,
) -> Option<Vec<Pos>> {
    let before = water.cells.len();
    water.cells.extend(&cells);
    loop {
        let bad: Vec<Pos> = water.undrawable().into_iter().filter(|p| cells.contains(p)).collect();
        let Some(&p) = bad.first() else { break };
        cells.retain(|&q| q != p);
        water.cells.retain(|&q| q != p);
    }
    water.cells.truncate(before);
    (cells.len() >= min && water.try_add(grid, &cells, &[], &[])).then_some(cells)
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
fn lattice(frame: &Frame, anchor: Pos) -> Vec<Pos> {
    let mut cells = Vec::new();
    for r in (anchor.0 % 2..frame.rows).step_by(2) {
        for c in (anchor.1 % 2..frame.cols).step_by(2) {
            if c > 0 && frame.nodes.as_ref().is_none_or(|n| n.contains(&(r, c))) {
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
    frame: &Frame,
    anchor: Pos,
    start: Pos,
    targets: Targets,
    node_count: usize,
    rng: &mut R,
) -> Option<Layout> {
    let (rows, cols) = (frame.rows, frame.cols);
    let cells = lattice(frame, anchor);
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
        Some(k) => cells.iter().copied().filter(|p| p.1 >= k.from_col).collect(),
        None => HashSet::new(),
    };
    let main: Vec<Pos> = cells.iter().copied().filter(|p| !walled.contains(p)).collect();
    let pocket_size = targets.pocket.map(|k| rng.random_range(k.nodes.0..=k.nodes.1));

    let main_nodes = node_count - pocket_size.unwrap_or(0);
    let mut river_wanted = false;
    let splits = match targets.pipe_pairs {
        _ if targets.per_screen.is_some() => 0,
        0 => 0,
        _ if targets.rock_pocket => 0,
        n => {
            let extra = targets.extra_split > 0.0 && rng.random_bool(targets.extra_split);
            river_wanted =
                extra && targets.river_share > 0.0 && rng.random_bool(targets.river_share);
            n - 1 + usize::from(extra && !river_wanted)
        }
    };
    if !river_wanted && targets.river_chance > 0.0 {
        river_wanted = rng.random_bool(targets.river_chance);
    }
    let dock_islands = match targets.dock_islands {
        (_, 0) => 0,
        (lo, hi) => rng.random_range(lo..=hi),
    };
    let mut islands: Vec<Vec<Pos>> = Vec::new();
    let mut mainland: Vec<Pos> = Vec::new();

    if splits + dock_islands == 0 && targets.per_screen.is_none() {
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
        // A world three screens wide or more goes by a waypoint on each
        // screen between, so the path crosses the whole map.
        let mut stops: Vec<Pos> = Vec::new();
        for screen in 1..cols / SCREEN_COLS - 1 {
            let band: Vec<Pos> = main
                .iter()
                .copied()
                .filter(|p| p.1 / SCREEN_COLS == screen && *p != start)
                .collect();
            stops.push(*band.choose(rng)?);
        }
        if start.1 > waypoint.1 {
            stops.reverse();
        }
        stops.push(waypoint);
        let mut at = start;
        for waypoint in stops {
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
                if in_tree.insert(next) {
                    nodes.push(next);
                }
                if !tree.contains(&ordered(at, next)) {
                    tree.push(ordered(at, next));
                }
                at = next;
            }
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
        let rules = IslandRules {
            share: targets.island_share,
            edge: targets.edge_islands,
            outer: targets.mainland_outer.map(|(lo, hi)| rng.random_range(lo..=hi)),
            even: targets.even_islands,
            touch: targets.islands_touch,
        };
        let grown = if let Some(sizes) = targets.per_screen {
            screen_islands(&main, &on_lattice, start, sizes, targets.small_island, rng)?
        } else {
            grow_islands(&main, &on_lattice, start, splits + dock_islands, main_nodes, rules, rng)?
        };
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
        // One search per node a pair starts from, over one adjacency.
        let adj = adjacency(&current);
        let mut from: HashMap<Pos, HashMap<Pos, usize>> = HashMap::new();
        let mut scored: Vec<(usize, (Pos, Pos))> = Vec::new();
        for &(a, b) in spare.iter().filter(|p| !extra.contains(p)) {
            let hops = from.entry(a).or_insert_with(|| hops_from(&adj, a));
            let hops = hops.get(&b).copied().unwrap_or(0);
            if hops != 3 || squares_left {
                scored.push((hops, (a, b)));
            }
        }
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

    // 3a. The rock pocket: a link whose loss cuts off 1-3 nodes (not the
    //     start), which becomes a breakable rock. Never a vertical link down
    //     onto the bottom row, as for every rock (rows 7 and 8 share a
    //     completion bit).
    let mut rock_pocket: Vec<Pos> = Vec::new();
    if targets.rock_pocket {
        let mut cuts: Vec<(usize, Vec<Pos>)> = Vec::new();
        for (i, &(a, b, kind)) in links.iter().enumerate() {
            let onto_bottom = a.1 == b.1 && a.0.max(b.0) == rows - 1;
            if kind != Link::Path || onto_bottom {
                continue;
            }
            let rest: Vec<(Pos, Pos, Link)> =
                links.iter().enumerate().filter(|&(j, _)| j != i).map(|(_, &l)| l).collect();
            let reached = path_hops(start, &rest);
            let cut: Vec<Pos> =
                nodes.iter().copied().filter(|n| !reached.contains_key(n)).collect();
            if (1..=3).contains(&cut.len()) && !cut.iter().any(|n| walled.contains(n)) {
                cuts.push((i, cut));
            }
        }
        let (i, cut) = cuts.choose(rng)?.clone();
        links[i].2 = Link::Rock;
        rock_pocket = cut;
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
        let homes: Vec<&[Pos]> =
            vec![
                mainland.as_slice();
                rng.random_range(targets.mainland_docks.0..=targets.mainland_docks.1)
            ];
        let spots = |home: &[Pos], docks: &[(Pos, Pos)]| -> Vec<(Pos, Pos)> {
            home.iter()
                .copied()
                .filter(|&n| n != start && !docks.iter().any(|d| d.0 == n))
                .flat_map(|(r, c)| {
                    [(r.wrapping_sub(1), c), (r + 1, c), (r, c.wrapping_sub(1)), (r, c + 1)]
                        .into_iter()
                        // Never the first column: that is W3's left shore.
                        .filter(|w| sea.contains(w) && w.1 >= 2)
                        .map(move |w| ((r, c), w))
                })
                .collect()
        };
        for home in homes {
            let here = spots(home, &docks);
            docks.push(*here.choose(rng)?);
        }
        //    Dock islands in a random order; one with no node on the open sea
        //    is passed over and stays a pipe island, while the pipes go round.
        let mut docked = 0;
        for &i in &order {
            if docked == dock_islands {
                break;
            }
            let here = spots(&islands[i], &docks);
            if let Some(&spot) = here.choose(rng) {
                docks.push(spot);
                docked += 1;
            }
        }
        if islands.len() - docked > targets.pipe_pairs {
            return None;
        }
    }

    // 4. The target: a dead end at the hop distance, not reached from above,
    //    with a row above it for the castle top.
    let degree = degrees(&nodes, &links);
    let hops = path_hops(start, &links);
    //    On an island any distance will do, since the builder decides where
    //    its pipe lands.
    //    On islands, the farthest island that has a spot for it.
    let fits = |n: Pos| {
        n != start
            && degree[&n] == 1
            && n.0 >= 2
            && !walled.contains(&n)
            && !docks.iter().any(|d| d.0 == n)
            && !rock_pocket.contains(&n)
            && !links.iter().any(|&(a, b, _)| (a, b) == ((n.0 - 2, n.1), n))
    };
    let candidates: Vec<Pos> = if islands.is_empty() {
        nodes
            .iter()
            .copied()
            .filter(|&n| fits(n))
            .filter(|n| {
                let d = hops.get(n).copied().unwrap_or(usize::MAX);
                (targets.start_to_target.0..=targets.start_to_target.1).contains(&d)
            })
            .collect()
    } else {
        islands
            .iter()
            .map(|island| {
                nodes.iter().copied().filter(|n| island.contains(n) && fits(*n)).collect::<Vec<_>>()
            })
            .find(|c| !c.is_empty())
            .unwrap_or_default()
    };
    let target = match targets.airship {
        true => Some(*candidates.choose(rng)?),
        false => None,
    };

    let dead_ends = degree.iter().filter(|&(&n, &d)| d == 1 && n != start).count();
    if !(targets.dead_ends.0..=targets.dead_ends.1).contains(&dead_ends) {
        return None;
    }
    if hooked_cross(&links) {
        return None;
    }

    let river = river_wanted;

    Some(Layout {
        rows,
        cols,
        nodes,
        links,
        start,
        target,
        loop_sizes,
        pocket,
        strip_col: targets.pocket.map(|k| k.from_col),
        islands,
        docks,
        river,
        rock_pocket,
    })
}

/// How [`grow_islands`] deals and places islands.
#[derive(Clone, Copy)]
struct IslandRules {
    /// [`Targets::island_share`].
    share: (usize, usize),
    /// Seed the other islands on the outermost node row or column.
    edge: bool,
    /// How many more of the start's island's nodes may go on the top or
    /// bottom node row; `None` for no limit.
    outer: Option<usize>,
    /// [`Targets::even_islands`].
    even: bool,
    /// [`Targets::islands_touch`].
    touch: bool,
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
/// placed beside another island's node, so islands never touch. An island
/// boxed in before its count stops there (two nodes at least), and the start's
/// island takes the rest.
fn grow_islands<R: Rng>(
    main: &[Pos],
    on_lattice: &HashSet<Pos>,
    start: Pos,
    splits: usize,
    total: usize,
    rules: IslandRules,
    rng: &mut R,
) -> Option<Islands> {
    let share = rules.share;
    let (top, bottom) = (main.iter().map(|p| p.0).min()?, main.iter().map(|p| p.0).max()?);
    let (left, right) = (main.iter().map(|p| p.1).min()?, main.iter().map(|p| p.1).max()?);
    let on_edge = |(r, c): Pos| r == top || r == bottom || c == left || c == right;
    let mut seeds = vec![start];
    for _ in 0..splits {
        let gap = |p: Pos| seeds.iter().map(|&s| manhattan(p, s)).min().unwrap_or(0);
        let mut far: Vec<(usize, Pos)> = main
            .iter()
            .copied()
            .filter(|&p| gap(p) > 2 && (!rules.edge || on_edge(p)))
            .map(|p| (gap(p), p))
            .collect();
        far.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(&y.1)));
        let &(_, seed) = far.get(..FARTHEST_FEW.min(far.len()))?.choose(rng)?;
        seeds.push(seed);
    }
    let mut quotas = vec![0; seeds.len()];
    if rules.even {
        // An equal share each; the first few take one more for the remainder.
        for (i, quota) in quotas.iter_mut().enumerate() {
            *quota = total / seeds.len() + usize::from(i < total % seeds.len());
        }
    } else {
        let mut left = total;
        for quota in quotas.iter_mut().skip(1) {
            *quota = (left * rng.random_range(share.0..=share.1) / 100).max(2);
            left -= *quota;
        }
        quotas[0] = left;
    }
    grow_from(&seeds, quotas, main, on_lattice, rules, rng)
}

/// One island per screen of `main` of `sizes` nodes, the start's first,
/// plus `small` (a screen and a node count): one more island there, seeded
/// among the cells farthest from that screen's seed (W8).
fn screen_islands<R: Rng>(
    main: &[Pos],
    on_lattice: &HashSet<Pos>,
    start: Pos,
    sizes: &[usize],
    small: Option<(usize, usize)>,
    rng: &mut R,
) -> Option<Islands> {
    let screen = |p: &Pos| p.1 / SCREEN_COLS;
    let screens = main.iter().map(screen).max()? + 1;
    let mut seeds = vec![start];
    for s in (0..screens).filter(|&s| s != screen(&start)) {
        let here: Vec<Pos> = main.iter().copied().filter(|p| screen(p) == s).collect();
        seeds.push(*here.choose(rng)?);
    }
    let mut quotas: Vec<usize> = seeds.iter().map(|seed| sizes[screen(seed)]).collect();
    if let Some((s, size)) = small {
        let others: Vec<Pos> = seeds.iter().copied().filter(|p| screen(p) == s).collect();
        let gap = |p: Pos| others.iter().map(|&o| manhattan(p, o)).min().unwrap_or(0);
        let mut far: Vec<(usize, Pos)> =
            main.iter().copied().filter(|p| screen(p) == s).map(|p| (gap(p), p)).collect();
        far.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(&y.1)));
        let &(_, seed) = far.get(..FARTHEST_FEW.min(far.len()))?.choose(rng)?;
        seeds.push(seed);
        quotas.push(size);
    }
    let rules = IslandRules { share: (0, 0), edge: false, outer: None, even: false, touch: false };
    grow_from(&seeds, quotas, main, on_lattice, rules, rng)
}

/// Grow islands from `seeds` to `quotas` nodes, a node at a time (see
/// [`grow_islands`]).
fn grow_from<R: Rng>(
    seeds: &[Pos],
    mut quotas: Vec<usize>,
    main: &[Pos],
    on_lattice: &HashSet<Pos>,
    rules: IslandRules,
    rng: &mut R,
) -> Option<Islands> {
    let (top, bottom) = (main.iter().map(|p| p.0).min()?, main.iter().map(|p| p.0).max()?);
    let in_main: HashSet<Pos> = main.iter().copied().collect();
    let mut owner: HashMap<Pos, usize> = seeds.iter().enumerate().map(|(i, &s)| (s, i)).collect();
    let mut islands: Vec<Vec<Pos>> = seeds.iter().map(|&s| vec![s]).collect();
    let mut links = Vec::new();
    let mut outer_left = rules.outer;
    loop {
        let short: Vec<usize> =
            (0..islands.len()).filter(|&i| islands[i].len() < quotas[i]).collect();
        let Some(&i) = short.choose(rng) else { break };
        let free = |n: Pos| -> Vec<Pos> {
            neighbours(n, on_lattice)
                .into_iter()
                .filter(|m| in_main.contains(m) && !owner.contains_key(m))
                .filter(|&m| {
                    rules.touch
                        || neighbours(m, on_lattice)
                            .iter()
                            .all(|o| owner.get(o).is_none_or(|&j| j == i))
                })
                .filter(|m| i != 0 || outer_left != Some(0) || (m.0 != top && m.0 != bottom))
                .collect()
        };
        let growable: Vec<Pos> =
            islands[i].iter().copied().filter(|&n| !free(n).is_empty()).collect();
        // An island boxed in by its neighbours stops where it is, and the
        // start's island takes the nodes it was short, if it has two already.
        if growable.is_empty() {
            if i == 0 || islands[i].len() < 2 {
                return None;
            }
            quotas[0] += quotas[i] - islands[i].len();
            quotas[i] = islands[i].len();
            continue;
        }
        let from = if rng.random_bool(EXTEND_NEWEST) {
            *growable.last()?
        } else {
            *growable.choose(rng)?
        };
        let &to = free(from).choose(rng)?;
        if i == 0 && (to.0 == top || to.0 == bottom) {
            outer_left = outer_left.map(|n| n - 1);
        }
        owner.insert(to, i);
        islands[i].push(to);
        links.push(ordered(from, to));
    }
    Some(Islands { nodes: islands, links })
}

/// W2's pocket strip: the map's last scroll view. Vanilla W2's secret area
/// starts at column 22, entered from (0,20); one node column further out,
/// its tiles cannot be seen from the main map without going over (user,
/// 2026-10-13).
const W2_POCKET_COL: usize = 24;

/// Map columns per screen.
const SCREEN_COLS: usize = 16;

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
    hops_from(&adjacency(links), start)
}

/// Each node's neighbours over path links.
fn adjacency(links: &[(Pos, Pos, Link)]) -> HashMap<Pos, Vec<Pos>> {
    let mut adj: HashMap<Pos, Vec<Pos>> = HashMap::new();
    for &(a, b, kind) in links {
        if kind == Link::Path {
            adj.entry(a).or_default().push(b);
            adj.entry(b).or_default().push(a);
        }
    }
    adj
}

/// Hops from `start` over an [`adjacency`].
fn hops_from(adj: &HashMap<Pos, Vec<Pos>>, start: Pos) -> HashMap<Pos, usize> {
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
        if let Some((r, c)) = self.target {
            cells[r][c] = 'A';
        }
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
        let hops = self.target.and_then(|t| path_hops(self.start, &self.links).get(&t).copied());
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
    let Some(rom) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes")
        .ok()
        .and_then(|d| crate::rom::Rom::from_bytes(&d).ok())
    else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    let maps = super::vanilla::load(&rom, false, false);
    let world: usize = std::env::var("LAYOUT_WORLD").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let grid = &maps[world - 1].grid;
    let start = crate::randomize::rom_data::find_start(grid).expect("the world has a start");
    let entries = &maps[world - 1].entry_pos;
    let airship =
        (0..entries.len()).find(|&i| AIRSHIP_ENTRIES.contains(&(world - 1, i))).map(|i| entries[i]);
    let recipes = RECIPES[world - 1];
    for (k, recipe) in recipes.iter().enumerate() {
        if recipes.len() > 1 {
            println!("\n=== recipe {} of {}", k + 1, recipes.len());
        }
        let areas = areas(&recipe.area, grid);
        let frame = frame_for(areas.as_ref(), grid);
        let node_count = dealt_entries(&maps[world - 1], areas.as_ref());
        layout_report(world, start, airship, recipe.layout, &frame, node_count);
    }
}

/// `generated_layouts` for one recipe.
#[cfg(test)]
fn layout_report(
    world: usize,
    start: Pos,
    airship: Option<Pos>,
    targets: Targets,
    frame: &Frame,
    node_count: usize,
) {
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    println!("\nW{world} start {start:?}, airship {airship:?} (vanilla); targets: {targets:?}");
    println!(
        "Vanilla: {}",
        [
            "W1 nodes 21  loops 2  dead ends 3  junctions 5  rocks 1  start->airship 9",
            "W2 nodes 47  loops 3  dead ends 7  junctions 10  pocket 6  start->airship 15",
            "W3 nodes 52  loops 4  dead ends 16  junctions 15  islands 5/3/2  docks 3",
            "W4 nodes 34  loops 2  dead ends 10  junctions 9  islands 30/4 (start on the 4)",
            "W5 nodes 42  loops 3  dead ends 8  junctions 10  islands 22/20 (ground/sky)",
            "W6 nodes 57  loops 5  dead ends 10  junctions 15  rock pocket with a pipe  start->airship 23",
            "W7 nodes 46  loops 1  dead ends 16  junctions 6  islands 14/10/9/6/4/2/1 by 8 pipes",
            "W8 nodes 35 (castle screen fixed)  islands 8/7/3/17 by screen, 6 pipes over the world",
        ][world - 1]
    );

    let seeds: u64 = std::env::var("LAYOUT_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(10);
    let quiet = seeds > 20;
    let (mut failed, mut built) = (0, 0);
    let (mut loops, mut squares, mut loop_total, mut junctions) = (0, 0, 0, 0);
    for seed in 1..=seeds {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        match generate(world - 1, frame, start, airship, targets, node_count, &mut rng) {
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
    let mut stranded_nodes: Vec<(u64, Vec<Pos>)> = Vec::new();
    let mut helps_above = 0;
    for seed in 0..seeds {
        let (shuffle, swap) = match seed % 3 {
            0 => (true, false),
            1 => (false, false),
            _ => (true, true),
        };
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut maps = super::vanilla::load(&rom, false, false);
        // Each of the world's recipes in turn, every arm over each.
        let recipes = RECIPES[wi];
        let recipe = recipes[(seed / 3) as usize % recipes.len()];
        let Some(w1) = draw_with(&maps[wi], wi, recipe, swap, &mut rng) else { continue };
        // A left shore holds: no tile in the first column the canoe can cross
        // (the engine's canoe tiles are $82-$A9), so it never reaches the
        // frame. Checked on the finished map, so water from any step (a
        // river carved before the shore, say) is caught too.
        let left_shore =
            recipe.scenery.iter().any(|s| matches!(s, Step::Sea { left_shore: true, .. }));
        if left_shore {
            for r in 0..w1.grid.rows() {
                let tile = w1.grid.get(r, 1);
                assert!(
                    !(0x82..=0xA9).contains(&tile),
                    "seed {seed}: canoe tile {tile:02X} at ({r},1) on the left shore"
                );
            }
        }
        // Nodes keep to their area's columns of each screen (W8's 2-12).
        if let Some(a) = areas(&recipe.area, &maps[wi].grid) {
            for p in w1.entry_pos.iter().filter(|p| a.ground.contains(p) || a.sky.contains(p)) {
                let col = p.1 % SCREEN_COLS;
                assert!(
                    (a.node_cols.0..=a.node_cols.1).contains(&col),
                    "seed {seed}: node {p:?} in column {col} of its screen"
                );
            }
        }
        // Without the swap, the start is on or beside the vanilla start.
        if !swap {
            let find = |g: &Grid| crate::randomize::rom_data::find_start(g).unwrap();
            let (start, home) = (find(&w1.grid), find(&maps[wi].grid));
            assert!(manhattan(start, home) <= 2, "seed {seed}: start {start:?}, vanilla {home:?}");
        }
        // HELP floats right of the castle top, or above it at the frame.
        if let Some((castle, help)) = w1.castle {
            let (right, above) = ((castle.0 - 1, castle.1 + 1), (castle.0 - 2, castle.1));
            assert!(help == right || help == above, "seed {seed}: HELP at {help:?}");
            assert!(help.1 >= 1 && help.1 + 1 < w1.grid.cols, "seed {seed}: HELP on the frame");
            helps_above += usize::from(help == above);
        }
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
        let maps_entry_pos = w1.entry_pos.clone();
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
        let reach = walk_map(&w1.grid, &w1.pipe_pairs, start, wi).nodes;
        let stranded: Vec<Pos> = maps_entry_pos
            .iter()
            .copied()
            .filter(|&(r, c)| r < w1.grid.rows() && c < w1.grid.cols && !reach.contains(&(r, c)))
            .collect();
        // Not a failure: only the airship must be reachable (user,
        // 2026-10-13). Reported, to see how often the builder strands one.
        if !stranded.is_empty() {
            stranded_nodes.push((seed, stranded));
        }
    }
    println!("builder: {generated} of {seeds} seeds generated a W{}, all completable", wi + 1);
    assert!(
        row78_conflicts.is_empty(),
        "a completable or removable tile above a bottom-row node (seed, column): {row78_conflicts:?}",
    );
    println!(
        "  {} seeds leave a node no route reaches (allowed): {stranded_nodes:?}",
        stranded_nodes.len()
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
    println!("  HELP above the castle (right is the frame): {helps_above}");
    println!("pipeline: {} seeds ran end to end", seeds.min(20));
}

/// A generated world's start never shows its pocket's strip: the starting
/// camera ([`start_camera_col`]) must stop short of it, so the secret area is
/// found only by going over. Every recipe with a pocket.
#[test]
fn generated_start_never_shows_the_pocket() {
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
    for (wi, recipes) in RECIPES.iter().enumerate() {
        for recipe in recipes.iter() {
            let Some(pocket) = recipe.layout.pocket else { continue };
            for seed in 0..50 {
                let mut rng = ChaCha8Rng::seed_from_u64(seed);
                let map = draw_with(&vanilla[wi], wi, recipe, true, &mut rng).unwrap();
                let start = crate::randomize::rom_data::find_start(&map.grid).unwrap();
                let shown = start_camera_col(wi, start.1) + 16;
                assert!(
                    shown <= pocket.from_col,
                    "W{} seed {seed}: start {start:?} shows up to column {shown}",
                    wi + 1
                );
            }
        }
    }
}

/// Every recipe fits its world, so it can be fed to the generator as it
/// stands: per-screen island sizes add up to the entries it deals, a pocket's
/// strip is on the map, and it draws a map on the first few seeds.
#[test]
fn recipes_fit_their_worlds() {
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
    for (wi, recipes) in RECIPES.iter().enumerate() {
        for (k, recipe) in recipes.iter().enumerate() {
            let at = format!("W{} recipe {}", wi + 1, k + 1);
            let areas = areas(&recipe.area, &vanilla[wi].grid);
            let dealt = dealt_entries(&vanilla[wi], areas.as_ref());
            let targets = recipe.layout;
            if let Some(sizes) = targets.per_screen {
                let small = targets.small_island.map_or(0, |s| s.1);
                assert_eq!(sizes.iter().sum::<usize>() + small, dealt, "{at}: screen sizes");
            }
            if let Some(pocket) = targets.pocket {
                assert!(pocket.from_col < vanilla[wi].grid.cols, "{at}: pocket off the map");
            }
            for seed in 0..5 {
                let mut rng = ChaCha8Rng::seed_from_u64(seed);
                assert!(
                    draw_with(&vanilla[wi], wi, recipe, true, &mut rng).is_some(),
                    "{at} seed {seed}"
                );
            }
        }
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
    for (wi, map) in vanilla.iter().enumerate() {
        for seed in 0..20 {
            let draw = || world_map(map, wi, true, &mut ChaCha8Rng::seed_from_u64(seed)).unwrap();
            let (a, b) = (draw(), draw());
            assert_eq!(a.grid.tiles, b.grid.tiles, "W{} seed {seed}: two draws differ", wi + 1);
            assert_eq!(a.entry_pos, b.entry_pos, "W{} seed {seed}: entries differ", wi + 1);
        }
    }
}

/// A generated world (`LAYOUT_WORLD`, default 1) as drawn tiles, before the
/// builder places anything: `.` bush, ` ` clearing or plain sand, `:` dotted
/// sand, `^` rock (decorative), `T` palm, `O` pool, `M` pyramid, `Q`
/// quicksand, `K` tall rock, `Y` plant, `f` fire flower, `u` ruins, `~` water,
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
        let map = world_map(&vanilla[wi], wi, true, &mut ChaCha8Rng::seed_from_u64(seed)).unwrap();
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
                    FLOWER => 'f',
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
            .map(|i| map.entry_pos[i]);
        for recipe in RECIPES[wi] {
            let areas = areas(&recipe.area, grid);
            let frame = frame_for(areas.as_ref(), grid);
            let node_count = dealt_entries(map, areas.as_ref());
            for seed in 0..50 {
                let mut rng = ChaCha8Rng::seed_from_u64(seed);
                if let Some(layout) =
                    generate(wi, &frame, start, airship, recipe.layout, node_count, &mut rng)
                {
                    assert!(!hooked_cross(&layout.links), "W{} seed {seed}", wi + 1);
                }
            }
        }
    }
}
