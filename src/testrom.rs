//! Test-ROM builder: assemble a playtest ROM from orthogonal, composable knobs.
//!
//! This exists because playtesting needs ROMs that the randomizer proper would
//! never produce: a specific level parked on tile 1, a map with every lock
//! removed, a fortress reachable without clearing three levels first. Those
//! edits used to be a prose procedure re-derived by hand each time; here they
//! are code, driven by [`TestRomSpec`].
//!
//! The knobs are deliberately independent — what to place, what to walk
//! through, where to start, and whether the base map is vanilla or randomized
//! are four separate axes, so level testing, overworld testing, airship
//! testing, and lock testing are all combinations rather than named modes.

use crate::ips;
use crate::randomize::node_catalog::{EntryView, NodeCatalog};
use crate::randomize::rom_data::{self, LevelEntry, WORLDS};
use crate::randomize::world_order::WORLD_INIT_OPERAND;
use crate::rom::Rom;
use crate::{randomize_rom, Options};

/// Bytes per map screen: 9 rows × 16 columns, stored screen-major.
const SCREEN_BYTES: usize = 144;

/// File-offset window covering PRG010–011 of the ROM.
///
/// Practice-patch records outside this window rewrite PRG006/PRG012 (enemy
/// data and the overworld map) and would clobber whatever the randomizer just
/// produced, so the movement patch is applied as a subset over this range only.
const MOVEMENT_RECORD_RANGE: (usize, usize) = (0x14010, 0x18010);

// Lock/gap classification and the path each restores to come from
// `rom_data::tiles` — see `is_lock`, `is_water_gap`, `path_for_gap_tile`. The
// inverse is lossy (a vertical path variant comes back as the plain vertical
// path), which for a test ROM is a cosmetic difference on a tile you are about
// to walk over.

// ---------------------------------------------------------------------------
// Seed requirements
// ---------------------------------------------------------------------------

/// A class of map tile a requirement can look for.
///
/// Deliberately a small named set plus a raw-byte escape hatch, rather than an
/// expression language — add a class when a test actually needs one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileClass {
    Lock,
    Gap,
    Fortress,
    Level,
    Pipe,
    ToadHouse,
    Airship,
    Bowser,
    Raw(u8),
}

impl TileClass {
    fn parse(s: &str) -> Result<Self, String> {
        if let Some(hex) = s.strip_prefix("tile:") {
            let hex = hex.trim_start_matches("0x").trim_start_matches("0X");
            return u8::from_str_radix(hex, 16)
                .map(TileClass::Raw)
                .map_err(|_| format!("bad tile byte {s:?} (expected e.g. tile:0x54)"));
        }
        Ok(match s {
            "lock" => TileClass::Lock,
            "gap" => TileClass::Gap,
            "fortress" | "fort" => TileClass::Fortress,
            "level" => TileClass::Level,
            "pipe" => TileClass::Pipe,
            "toadhouse" => TileClass::ToadHouse,
            "airship" => TileClass::Airship,
            "bowser" => TileClass::Bowser,
            _ => {
                return Err(format!(
                    "unknown tile class {s:?} (lock, gap, fortress, level, pipe, \
                     toadhouse, airship, bowser, or tile:0xNN)"
                ))
            }
        })
    }

    fn matches(&self, tile: u8) -> bool {
        match self {
            TileClass::Lock => rom_data::is_lock(tile),
            TileClass::Gap => rom_data::is_water_gap(tile),
            TileClass::Fortress => rom_data::is_fortress(tile),
            TileClass::Level => rom_data::is_numbered_level(tile),
            TileClass::Pipe => tile == rom_data::TILE_PIPE,
            TileClass::ToadHouse => tile == rom_data::TILE_TOAD_HOUSE,
            TileClass::Airship => tile == rom_data::TILE_AIRSHIP,
            TileClass::Bowser => tile == rom_data::TILE_BOWSER,
            TileClass::Raw(b) => tile == *b,
        }
    }

    fn label(&self) -> String {
        match self {
            TileClass::Lock => "lock".into(),
            TileClass::Gap => "gap".into(),
            TileClass::Fortress => "fortress".into(),
            TileClass::Level => "level".into(),
            TileClass::Pipe => "pipe".into(),
            TileClass::ToadHouse => "toad house".into(),
            TileClass::Airship => "airship".into(),
            TileClass::Bowser => "bowser".into(),
            TileClass::Raw(b) => format!("tile {b:#04X}"),
        }
    }
}

/// "at least `min_count` of `what` in world `world_idx`, optionally restricted
/// to one screen." Parsed from `lock@w8:s2>=2`.
pub struct Requirement {
    pub what: TileClass,
    pub world_idx: usize,
    /// `None` matches anywhere in the world.
    pub screen: Option<usize>,
    pub min_count: usize,
}

impl Requirement {
    /// Parse `<class>@w<N>[:s<M>][>=<K>]`, e.g. `lock@w8:s2` or `fort@w3>=2`.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let bad = || {
            format!("bad --require {spec:?} (expected e.g. lock@w8:s2, fort@w3>=2, tile:0x54@w8)")
        };

        let (head, min_count) = match spec.split_once(">=") {
            Some((h, n)) => (h, n.trim().parse::<usize>().map_err(|_| bad())?),
            None => (spec, 1),
        };
        if min_count == 0 {
            return Err("--require count must be at least 1".to_string());
        }

        let (what, loc) = head.rsplit_once('@').ok_or_else(bad)?;
        let what = TileClass::parse(what.trim())?;

        let (world, screen) = match loc.split_once(':') {
            Some((w, s)) => (w, Some(s)),
            None => (loc, None),
        };

        let world_num: usize = world
            .trim()
            .trim_start_matches(['w', 'W'])
            .parse()
            .map_err(|_| bad())?;
        if !(1..=8).contains(&world_num) {
            return Err(format!("world must be 1-8 in --require {spec:?}"));
        }
        let world_idx = world_num - 1;

        let screen = match screen {
            Some(s) => {
                let n: usize = s
                    .trim()
                    .trim_start_matches(['s', 'S'])
                    .parse()
                    .map_err(|_| bad())?;
                let screens = rom_data::MAP_TILE_GRIDS[world_idx].screens;
                if n >= screens {
                    return Err(format!(
                        "W{world_num} has {screens} screen(s) (0-{}), got s{n}",
                        screens - 1
                    ));
                }
                Some(n)
            }
            None => None,
        };

        Ok(Requirement { what, world_idx, screen, min_count })
    }

    /// Positions in the target ROM matching this requirement.
    fn find(&self, rom: &Rom) -> Vec<(usize, usize)> {
        let grid = rom_data::read_tile_grid(rom, self.world_idx);
        let (lo, hi) = match self.screen {
            Some(s) => (s * 16, s * 16 + 16),
            None => (0, grid.cols),
        };
        (0..grid.rows())
            .flat_map(|r| (lo..hi.min(grid.cols)).map(move |c| (r, c)))
            .filter(|&(r, c)| self.what.matches(grid.get(r, c)))
            .collect()
    }

    /// One-line English rendering, for progress and error messages.
    pub fn describe(&self) -> String {
        let where_ = match self.screen {
            Some(s) => format!("W{} screen {s}", self.world_idx + 1),
            None => format!("W{}", self.world_idx + 1),
        };
        if self.min_count == 1 {
            format!("a {} on {where_}", self.what.label())
        } else {
            format!("{}× {} on {where_}", self.min_count, self.what.label())
        }
    }
}

/// Outcome of a seed search.
pub struct SeedHit {
    pub seed: u64,
    pub tried: usize,
    /// Human-readable account of what matched, and a census of the target area
    /// so the match can be eyeballed rather than trusted.
    pub report: Vec<String>,
}

/// Find the lowest seed at or after `from` whose randomized output satisfies
/// `req`, trying at most `limit` seeds.
///
/// Rejection sampling on purpose: biasing the builder to *place* the feature
/// would produce a map the randomizer never generates, which is worthless for
/// verifying a fix. This keeps the real distribution and just skips seeds that
/// don't happen to exercise the thing under test.
///
/// Runs entirely in memory — only the winning seed is ever written to disk.
pub fn search_seed(
    vanilla: &[u8],
    options: &Options,
    req: &Requirement,
    from: u64,
    limit: usize,
) -> Result<Option<SeedHit>, String> {
    for i in 0..limit {
        let seed = from.wrapping_add(i as u64);
        let rom = randomize_rom(vanilla, seed, options, None)?;
        let hits = req.find(&rom);
        if hits.len() >= req.min_count {
            let mut report = vec![format!(
                "seed {seed}: {} — {} match(es)",
                req.describe(),
                hits.len()
            )];
            for (r, c) in &hits {
                report.push(format!("  {} at W{} ({r},{c})", req.what.label(), req.world_idx + 1));
            }
            report.extend(census(&rom, req));
            return Ok(Some(SeedHit { seed, tried: i + 1, report }));
        }
    }
    Ok(None)
}

/// Count the notable tiles in the requirement's target area.
///
/// A predicate can pass while the ROM still fails to test what you meant — a
/// lock with no fortress beside it is broken by a hammer, not by a fortress
/// clear, which is a different code path. Printing the surroundings makes the
/// match checkable instead of trusted.
fn census(rom: &Rom, req: &Requirement) -> Vec<String> {
    let classes = [
        TileClass::Fortress,
        TileClass::Lock,
        TileClass::Gap,
        TileClass::Level,
        TileClass::Pipe,
    ];
    let counts: Vec<String> = classes
        .iter()
        .map(|c| {
            let probe = Requirement { what: *c, min_count: 1, screen: req.screen, world_idx: req.world_idx };
            (c, probe.find(rom).len())
        })
        .filter(|(_, n)| *n > 0)
        .map(|(c, n)| format!("{} {}", n, c.label()))
        .collect();

    let where_ = match req.screen {
        Some(s) => format!("W{} screen {s}", req.world_idx + 1),
        None => format!("W{}", req.world_idx + 1),
    };
    vec![
        format!("  {where_} contains: {}", counts.join(", ")),
        "  (eyeball it: tools/map_viz.py <rom.nes> --world N)".to_string(),
    ]
}

/// Where the map and level layout come from before test edits are applied.
pub enum Base {
    /// Untouched vanilla ROM. The right base for testing a level's contents.
    Vanilla,
    /// Randomizer output. The right base for testing the overworld itself.
    Randomized { seed: u64, options: Box<Options> },
}

/// One `--place` request: a level name and where to park it.
pub struct Placement {
    /// Level name as the catalog spells it, e.g. `6F1`, `8B`, `7A`, `1-4`.
    pub level: String,
    /// Target numbered tile (1-based). `None` means "next free slot".
    pub slot: Option<u8>,
}

/// A full description of the test ROM to build.
pub struct TestRomSpec {
    pub base: Base,
    /// Levels to park on numbered tiles of the starting world.
    pub placements: Vec<Placement>,
    /// Replace *every* numbered level in the starting world with this one.
    pub place_all: Option<String>,
    /// Starting world, 1-based. `None` leaves the ROM's own starting world.
    pub world: Option<u8>,
    /// Whole IPS patches to apply to the base, in order, before any test edits.
    ///
    /// Test edits deliberately land *after* these, so `--place`/`--keep-locks`
    /// win over anything a patch writes to the same bytes. Use this for the
    /// full practice ROM (level select + warp whistles) or any patch in
    /// `patches/`; `movement_patch` is the narrow subset instead.
    pub extra_patches: Vec<Vec<u8>>,
    /// Keep `extra_patches` from touching the 8 overworld map grids.
    ///
    /// The full practice patch rewrites the maps (49 of its records land in
    /// the grid region) and removes lock tiles along the way, which destroys
    /// any test that needs a lock. This snapshots the grids before patching
    /// and restores them after, so the patch's code changes land but its map
    /// does not — for a randomized base that also protects the built map,
    /// which is the hazard that made the old procedure apply only a subset.
    pub protect_map: bool,
    /// Practice-patch bytes supplying open map movement (walk over level,
    /// fortress and lock tiles without entering or clearing them).
    pub movement_patch: Option<Vec<u8>>,
    /// Apply the movement records that don't clash with randomizer patches
    /// instead of refusing outright. Yields *partial* open movement.
    pub walk_skip_conflicts: bool,
    /// Replace lock tiles with walkable path.
    pub remove_locks: bool,
    /// Replace water-gap tiles with bridges.
    pub remove_gaps: bool,
    /// Item IDs to start with, up to 3 (the trampoline's slot count).
    pub starting_items: Vec<u8>,
    /// Starting lives. Only written when `starting_items` is non-empty, since
    /// the two share one trampoline.
    pub starting_lives: u8,
    /// Let the Hammer item break fortress lock tiles on the map.
    pub hammer_breaks_locks: bool,
    /// Let the Hammer item break water-gap (bridge) tiles on the map.
    pub hammer_breaks_bridges: bool,
    /// Include the 9 unreferenced beta stages as placeable names.
    pub include_beta: bool,
}

/// Result of a build: the ROM bytes plus a human-readable account of what was
/// changed, so the caller can report it without re-deriving anything.
#[derive(Debug)]
pub struct TestRom {
    pub bytes: Vec<u8>,
    pub report: Vec<String>,
}

/// Normalize a level name for matching: case-insensitive, dashes optional.
///
/// The catalog spells fortresses `6F1` while the CLI and docs have long used
/// `6-F1`; both must resolve, and neither spelling is worth making canonical.
fn norm(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '-' && *c != '_' && !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Catalog of the vanilla ROM, used to resolve source level names.
///
/// Always built from vanilla even when the base is randomized: `--place 6F1`
/// means "the level that is 6F1 in the original game", regardless of where the
/// randomizer has since moved it.
fn source_catalog(vanilla: &[u8], include_beta: bool) -> Result<Vec<EntryView>, String> {
    let rom = Rom::from_bytes_lax(vanilla, true).map_err(|e| e.to_string())?;
    Ok(NodeCatalog::build(&rom, include_beta).entry_views())
}

/// Resolve a level name to the data that travels with it when placed.
fn resolve(catalog: &[EntryView], name: &str) -> Result<LevelEntry, String> {
    let want = norm(name);
    let hit = catalog
        .iter()
        .find(|e| norm(&e.name) == want)
        .ok_or_else(|| format!("unknown level {name:?} (try --list)"))?;

    hit.level_entry.clone().ok_or_else(|| {
        format!("{name:?} is a {} and has no level data to place", hit.kind_label)
    })
}

/// Numbered-level slots in a world, ordered by level number.
///
/// Read from the *target* ROM so this works on a randomized base, where the
/// numbered tiles no longer sit where vanilla put them.
fn numbered_slots(rom: &Rom, world_idx: usize, include_beta: bool) -> Vec<(u8, usize)> {
    let mut slots: Vec<(u8, usize)> = NodeCatalog::build(rom, include_beta)
        .entry_views()
        .into_iter()
        .filter(|e| {
            e.world_idx == world_idx
                && e.is_numbered_level
                && rom_data::is_numbered_level(e.tile)
        })
        .map(|e| (e.tile - 2, e.entry_idx))
        .collect();
    slots.sort_unstable();
    slots.dedup_by_key(|(num, _)| *num);
    slots
}

/// Rewrite lock and/or water-gap tiles across all 8 world maps.
fn open_map(rom: &mut Rom, remove_locks: bool, remove_gaps: bool) -> usize {
    let mut swaps: Vec<(u8, u8)> = Vec::new();
    for tile in 0u8..=0xFF {
        let wanted = (remove_locks && rom_data::is_lock(tile))
            || (remove_gaps && rom_data::is_water_gap(tile));
        if let Some(path) = rom_data::path_for_gap_tile(tile).filter(|_| wanted) {
            swaps.push((tile, path));
        }
    }
    if swaps.is_empty() {
        return 0;
    }

    let mut changed = 0;
    for world_idx in 0..8 {
        let grid = rom_data::read_tile_grid(rom, world_idx);
        for row in 0..grid.rows() {
            for col in 0..grid.cols {
                let tile = grid.get(row, col);
                if let Some(&(_, path)) = swaps.iter().find(|(lock, _)| *lock == tile) {
                    // Grids are stored screen-major (144 bytes per screen), so
                    // the offset is not `base + row * columns + col`.
                    rom.write_byte(rom_data::map_tile_offset(world_idx, row, col), path);
                    changed += 1;
                }
            }
        }
    }
    changed
}

/// Records in `patch` that would land on bytes the randomizer has already
/// written — i.e. free space our own 6502 patches are using.
///
/// The free-space map in `rom_data` describes *vanilla*. Once a randomizer
/// pass has claimed a region, an outside IPS applied on top silently shreds
/// it: the practice patch's movement records overlap `fx_screen_check` by 19
/// bytes, which corrupts the cross-screen lock routine on any randomized ROM.
/// Ordering can't fix it — the two genuinely want the same bytes — so the only
/// honest options are to refuse or to drop the patch.
fn collisions(rom: &Rom, patch: &[u8], range: Option<(usize, usize)>) -> Result<Vec<String>, String> {
    let mut found = Vec::new();
    for rec in ips::parse_ips_records(patch)? {
        if range.is_some_and(|(lo, hi)| !(lo..hi).contains(&rec.offset)) {
            continue;
        }
        let end = rec.offset + rec.payload.len();
        for w in rom.writes_in_range(rec.offset, end) {
            let desc = format!("0x{:05X}..0x{end:05X} overlaps `{}`", rec.offset, w.tag);
            if !found.contains(&desc) {
                found.push(desc);
            }
        }
    }
    Ok(found)
}

/// Apply only the map-movement records of a practice patch.
///
/// With `skip_conflicts`, records landing on randomizer-owned bytes are
/// dropped instead of shredding them. Returns the skipped count so the caller
/// can say so out loud — a silently degraded movement patch is its own trap.
fn apply_movement(
    rom: &mut Rom,
    patch: &[u8],
    skip_conflicts: bool,
) -> Result<(usize, usize, usize), String> {
    let (start, end) = MOVEMENT_RECORD_RANGE;
    let rom_len = rom.output_bytes().len();

    let mut applied = 0;
    let mut written = 0;
    let mut skipped = 0;
    for rec in ips::parse_ips_records(patch)? {
        if !(start..end).contains(&rec.offset) {
            continue;
        }
        if skip_conflicts
            && !rom
                .writes_in_range(rec.offset, rec.offset + rec.payload.len())
                .is_empty()
        {
            skipped += 1;
            continue;
        }
        if rec.offset + rec.payload.len() > rom_len {
            return Err(format!(
                "movement record at 0x{:06X} (+{}) exceeds ROM size {rom_len}",
                rec.offset,
                rec.payload.len()
            ));
        }
        rom.write_range(rec.offset, &rec.payload);
        applied += 1;
        written += rec.payload.len();
    }
    Ok((applied, written, skipped))
}

/// Build a test ROM from vanilla bytes and a spec.
pub fn build(vanilla: &[u8], spec: &TestRomSpec) -> Result<TestRom, String> {
    let mut report = Vec::new();

    // 1. Base ROM — vanilla, or a full randomizer run.
    //
    // The randomized `Rom` is kept rather than round-tripped through bytes, so
    // its write log survives; `collisions` below relies on it to catch patches
    // landing on top of randomizer-owned free space.
    let mut rom = match &spec.base {
        Base::Vanilla => {
            report.push("base: vanilla".to_string());
            Rom::from_bytes_lax(vanilla, true).map_err(|e| e.to_string())?
        }
        Base::Randomized { seed, options } => {
            report.push(format!("base: randomized (seed {seed})"));
            randomize_rom(vanilla, *seed, options, None)?
        }
    };

    // 1b. Whole IPS patches, before any test edit, so test edits win on
    //     overlapping bytes.
    if !spec.extra_patches.is_empty() {
        let saved_grids: Option<Vec<Vec<u8>>> = spec.protect_map.then(|| {
            rom_data::MAP_TILE_GRIDS
                .iter()
                .map(|g| rom.read_range(g.file_offset, g.screens * SCREEN_BYTES).to_vec())
                .collect()
        });

        for patch in &spec.extra_patches {
            let clashes = collisions(&rom, patch, None)?;
            if !clashes.is_empty() {
                return Err(format!(
                    "this patch would overwrite randomizer patches:\n       {}\n       \
                     A third-party IPS uses the ROM's free space for its own code, and \n       \
                     rom_data's free-space map describes vanilla, not a patched ROM.\n       \
                     Build on a vanilla base, or drop the conflicting randomizer feature.",
                    clashes.join("\n       ")
                ));
            }
            let records = ips::parse_ips_records(patch)?;
            let bytes: usize = records.iter().map(|r| r.payload.len()).sum();
            rom.apply_ips_patch(patch)?;
            report.push(format!("patch: {} records ({bytes} bytes)", records.len()));
        }

        if let Some(grids) = saved_grids {
            let mut restored = 0;
            for (g, saved) in rom_data::MAP_TILE_GRIDS.iter().zip(&grids) {
                restored += saved
                    .iter()
                    .enumerate()
                    .filter(|(i, b)| rom.read_byte(g.file_offset + i) != **b)
                    .count();
                rom.write_range(g.file_offset, saved);
            }
            report.push(format!("map protected: {restored} byte(s) reverted"));
        }
    }

    // 2. Starting world. Resolved before placement so placements land in the
    //    world the player actually boots into.
    let world_idx = match spec.world {
        Some(w) => {
            let idx = (w as usize) - 1;
            rom.write_byte(WORLD_INIT_OPERAND, idx as u8);
            report.push(format!("start world: W{w}"));
            idx
        }
        None => rom.read_byte(WORLD_INIT_OPERAND) as usize,
    };
    if world_idx >= 8 {
        return Err(format!("starting world index {world_idx} out of range"));
    }

    // 3. Level placement.
    if spec.place_all.is_some() || !spec.placements.is_empty() {
        let catalog = source_catalog(vanilla, spec.include_beta)?;
        let slots = numbered_slots(&rom, world_idx, spec.include_beta);
        let world = &WORLDS[world_idx];

        if let Some(name) = &spec.place_all {
            let entry = resolve(&catalog, name)?;
            for (_, entry_idx) in &slots {
                rom_data::write_entry(&mut rom, world, *entry_idx, &entry);
            }
            report.push(format!(
                "placed {name} on all {} numbered tiles of W{}",
                slots.len(),
                world_idx + 1
            ));
        }

        let mut next_free = 0usize;
        for placement in &spec.placements {
            let entry = resolve(&catalog, &placement.level)?;
            let (num, entry_idx) = match placement.slot {
                Some(want) => *slots
                    .iter()
                    .find(|(num, _)| *num == want)
                    .ok_or_else(|| {
                        format!(
                            "W{} has no numbered tile {want} (available: {})",
                            world_idx + 1,
                            slots
                                .iter()
                                .map(|(n, _)| n.to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })?,
                None => {
                    let slot = *slots.get(next_free).ok_or_else(|| {
                        format!(
                            "W{} has only {} numbered tiles; too many --place entries",
                            world_idx + 1,
                            slots.len()
                        )
                    })?;
                    next_free += 1;
                    slot
                }
            };
            rom_data::write_entry(&mut rom, world, entry_idx, &entry);
            report.push(format!(
                "placed {} on W{}-{num}",
                placement.level,
                world_idx + 1
            ));
        }
    }

    // 4. Open the map up.
    let opened = open_map(&mut rom, spec.remove_locks, spec.remove_gaps);
    if opened > 0 {
        let what = match (spec.remove_locks, spec.remove_gaps) {
            (true, true) => "locks + water gaps",
            (true, false) => "locks",
            _ => "water gaps",
        };
        report.push(format!("removed {opened} {what} across all 8 maps"));
    }
    if !spec.remove_locks {
        report.push("locks: kept".to_string());
    }

    // 5. Open movement (walk over level/fortress/lock tiles).
    match &spec.movement_patch {
        Some(patch) => {
            let clashes = collisions(&rom, patch, Some(MOVEMENT_RECORD_RANGE))?;
            if !clashes.is_empty() && !spec.walk_skip_conflicts {
                return Err(format!(
                    "the open-movement patch would overwrite randomizer patches:\n       {}\n       \
                     Their free space is the same space — applying both corrupts the routine.\n       \
                     --walk-skip-conflicts applies only the records that don't clash\n       \
                     (partial open movement); --no-walk drops it entirely.",
                    clashes.join("\n       ")
                ));
            }
            let (applied, written, skipped) =
                apply_movement(&mut rom, patch, spec.walk_skip_conflicts)?;
            report.push(format!(
                "open movement: {applied} records ({written} bytes)"
            ));
            if skipped > 0 {
                report.push(format!(
                    "  WARNING: {skipped} record(s) skipped — they overlap randomizer \
                     patches.\n           Open movement is PARTIAL; verify in-game that you \
                     can actually walk."
                ));
            }
        }
        None => report.push("open movement: off".to_string()),
    }

    // 6. Hammer tile-breaking. Applied here rather than via `Options` so it
    //    works on a vanilla base too — testing what a hammer does to a lock
    //    shouldn't require randomizing the map first.
    if spec.hammer_breaks_locks || spec.hammer_breaks_bridges {
        crate::randomize::qol::hammer_breaks_tiles(
            &mut rom,
            spec.hammer_breaks_locks,
            spec.hammer_breaks_bridges,
        );
        let what = match (spec.hammer_breaks_locks, spec.hammer_breaks_bridges) {
            (true, true) => "locks + bridges",
            (true, false) => "locks",
            _ => "bridges",
        };
        report.push(format!("hammer breaks: {what}"));
    }

    // 7. Starting inventory. Last, mirroring the randomizer's own ordering —
    //    the trampoline overwrites title-screen bytes and must win.
    if !spec.starting_items.is_empty() {
        let items: Vec<u8> = spec.starting_items.iter().copied().take(3).collect();
        // Seed only drives the intro-skip menu music; any fixed value is fine
        // for a test ROM and keeps output reproducible.
        crate::randomize::qol::write_starting_items(&mut rom, 0, spec.starting_lives, &items);
        report.push(format!(
            "inventory: {} ({} lives)",
            items
                .iter()
                .map(|&id| crate::item_display_name(id))
                .collect::<Vec<_>>()
                .join(", "),
            spec.starting_lives
        ));
        if spec.starting_items.len() > 3 {
            report.push(format!(
                "  note: {} extra item(s) dropped — only 3 inventory slots exist",
                spec.starting_items.len() - 3
            ));
        }
    }

    Ok(TestRom { bytes: rom.output_bytes().to_vec(), report })
}

/// All placeable level names, for `--list`.
pub fn list_levels(vanilla: &[u8], include_beta: bool) -> Result<Vec<String>, String> {
    let catalog = source_catalog(vanilla, include_beta)?;
    Ok(catalog
        .iter()
        .filter(|e| e.level_entry.is_some())
        .map(|e| format!("{:<8} {}", e.name, e.kind_label))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VANILLA: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";

    fn vanilla() -> Option<Vec<u8>> {
        std::fs::read(VANILLA).ok()
    }

    fn spec() -> TestRomSpec {
        TestRomSpec {
            base: Base::Vanilla,
            placements: Vec::new(),
            place_all: None,
            world: None,
            extra_patches: Vec::new(),
            protect_map: false,
            movement_patch: None,
            walk_skip_conflicts: false,
            remove_locks: false,
            remove_gaps: false,
            starting_items: Vec::new(),
            starting_lives: 5,
            hammer_breaks_locks: false,
            hammer_breaks_bridges: false,
            include_beta: false,
        }
    }

    #[test]
    fn name_normalization_accepts_both_spellings() {
        assert_eq!(norm("6-F1"), norm("6f1"));
        assert_eq!(norm("8B"), norm("8b"));
        assert_eq!(norm("8-Tank"), norm("8tank"));
    }

    #[test]
    fn placement_writes_the_source_levels_pointers() {
        let Some(van) = vanilla() else { return };

        // 6F1 is W6's first fortress: FORTRESS_ENTRIES lists it at (5, 9).
        let src = Rom::from_bytes_lax(&van, true).unwrap();
        let want = rom_data::read_entry(&src, &WORLDS[5], 9);

        let built = build(
            &van,
            &TestRomSpec {
                placements: vec![Placement { level: "6-F1".into(), slot: Some(1) }],
                world: Some(1),
                ..spec()
            },
        )
        .unwrap();

        let out = Rom::from_bytes_lax(&built.bytes, true).unwrap();
        let slot = numbered_slots(&out, 0, false)
            .into_iter()
            .find(|(num, _)| *num == 1)
            .expect("W1 has a tile 1");
        assert_eq!(rom_data::read_entry(&out, &WORLDS[0], slot.1), want);
    }

    /// Regression: map grids are screen-major, so a row-major offset walks off
    /// into the wrong screen and corrupts unrelated tiles. Unlocking must leave
    /// zero lock bytes behind and must not touch anything else.
    #[test]
    fn unlocking_clears_every_lock_and_nothing_else() {
        let Some(van) = vanilla() else { return };

        let built = build(&van, &TestRomSpec { remove_locks: true, ..spec() }).unwrap();
        let out = Rom::from_bytes_lax(&built.bytes, true).unwrap();

        let locks: Vec<u8> = rom_data::LOCK_TILES.to_vec();
        for world_idx in 0..8 {
            let grid = rom_data::read_tile_grid(&out, world_idx);
            for row in 0..grid.rows() {
                for col in 0..grid.cols {
                    assert!(
                        !locks.contains(&grid.get(row, col)),
                        "lock survived at W{} ({row},{col})",
                        world_idx + 1
                    );
                }
            }
        }

        // Every changed byte must be a lock→path swap, nothing collateral.
        for (i, (before, after)) in van.iter().zip(built.bytes.iter()).enumerate() {
            if before != after {
                assert!(
                    rom_data::path_for_gap_tile(*before) == Some(*after),
                    "unexpected write at 0x{i:05X}: {before:#04X} -> {after:#04X}"
                );
            }
        }
    }

    #[test]
    fn gaps_and_locks_are_independent_knobs() {
        let Some(van) = vanilla() else { return };

        let locks_only = build(&van, &TestRomSpec { remove_locks: true, ..spec() }).unwrap();
        for (i, (before, after)) in van.iter().zip(locks_only.bytes.iter()).enumerate() {
            assert!(
                before == after || *before != 0x9D,
                "water gap at 0x{i:05X} cleared despite remove_gaps = false"
            );
        }

        let gaps_only = build(&van, &TestRomSpec { remove_gaps: true, ..spec() }).unwrap();
        for (i, (before, after)) in van.iter().zip(gaps_only.bytes.iter()).enumerate() {
            assert!(
                before == after || *before == 0x9D,
                "non-gap byte changed at 0x{i:05X} despite remove_locks = false"
            );
        }
    }

    #[test]
    fn starting_world_is_written() {
        let Some(van) = vanilla() else { return };
        let built = build(&van, &TestRomSpec { world: Some(7), ..spec() }).unwrap();
        assert_eq!(built.bytes[WORLD_INIT_OPERAND], 6);
    }

    /// The inventory bytes must actually reach the trampoline, and asking for
    /// items must not disturb the map.
    #[test]
    fn starting_items_are_written_and_touch_nothing_on_the_map() {
        let Some(van) = vanilla() else { return };

        let built = build(
            &van,
            &TestRomSpec {
                starting_items: vec![0x0B, 0x03, 0x02], // hammer, leaf, fire
                starting_lives: 25,
                ..spec()
            },
        )
        .unwrap();

        // Each item is written by an `LDA #item / STA $7D80+i` pair.
        for (i, item) in [0x0Bu8, 0x03, 0x02].into_iter().enumerate() {
            let pair = [0xA9, item, 0x8D, 0x80 + i as u8, 0x7D];
            assert!(
                built.bytes.windows(5).any(|w| w == pair),
                "no write found for item {item:#04X} in slot {i}"
            );
        }
        assert!(built.bytes.windows(2).any(|w| w == [0xA9, 25]), "lives not written");

        for world_idx in 0..8 {
            let before = Rom::from_bytes_lax(&van, true).unwrap();
            let after = Rom::from_bytes_lax(&built.bytes, true).unwrap();
            assert_eq!(
                rom_data::read_tile_grid(&before, world_idx).tiles,
                rom_data::read_tile_grid(&after, world_idx).tiles,
                "starting items changed W{}'s map",
                world_idx + 1
            );
        }
    }

    /// Locks must survive `--keep-locks` even when the hammer can break them —
    /// that combination is the whole point of lock-FX testing.
    #[test]
    fn hammer_breaks_locks_leaves_the_locks_in_place() {
        let Some(van) = vanilla() else { return };

        let built = build(
            &van,
            &TestRomSpec { hammer_breaks_locks: true, remove_locks: false, ..spec() },
        )
        .unwrap();

        let out = Rom::from_bytes_lax(&built.bytes, true).unwrap();
        let locks: Vec<u8> = rom_data::LOCK_TILES.to_vec();
        let surviving: usize = (0..8)
            .map(|w| {
                let grid = rom_data::read_tile_grid(&out, w);
                (0..grid.rows())
                    .flat_map(|r| (0..grid.cols).map(move |c| (r, c)))
                    .filter(|&(r, c)| locks.contains(&grid.get(r, c)))
                    .count()
            })
            .sum();
        assert!(surviving > 0, "hammer patch removed the locks it should break at runtime");
    }

    #[test]
    fn requirement_parses_every_documented_form() {
        let r = Requirement::parse("lock@w8:s2").unwrap();
        assert_eq!(r.what, TileClass::Lock);
        assert_eq!(r.world_idx, 7);
        assert_eq!(r.screen, Some(2));
        assert_eq!(r.min_count, 1);

        let r = Requirement::parse("fort@w3>=2").unwrap();
        assert_eq!(r.what, TileClass::Fortress);
        assert_eq!(r.world_idx, 2);
        assert_eq!(r.screen, None);
        assert_eq!(r.min_count, 2);

        assert_eq!(Requirement::parse("tile:0x54@w8").unwrap().what, TileClass::Raw(0x54));
        assert_eq!(Requirement::parse("tile:54@w8").unwrap().what, TileClass::Raw(0x54));
    }

    #[test]
    fn requirement_rejects_nonsense_rather_than_guessing() {
        for bad in [
            "lock",             // no world
            "lock@w9:s0",       // world out of range
            "lock@w1:s7",       // W1 has 1 screen
            "banana@w1",        // unknown class
            "lock@w1>=0",       // pointless count
            "tile:zz@w1",       // bad hex
        ] {
            assert!(Requirement::parse(bad).is_err(), "should have rejected {bad:?}");
        }
    }

    /// W1 has a single screen, so `s0` is valid and `s1` is not — the check is
    /// per-world, not a fixed maximum.
    #[test]
    fn screen_bound_is_per_world() {
        assert!(Requirement::parse("lock@w1:s0").is_ok());
        assert!(Requirement::parse("lock@w1:s1").is_err());
        assert!(Requirement::parse("lock@w8:s3").is_ok()); // W8 has 4 screens
        assert!(Requirement::parse("lock@w8:s4").is_err());
    }

    /// The predicate must be evaluated against the randomized map, and the
    /// seed it reports must actually satisfy it.
    #[test]
    fn search_returns_a_seed_that_really_matches() {
        let Some(van) = vanilla() else { return };

        let req = Requirement::parse("lock@w8:s2").unwrap();
        let hit = search_seed(&van, &Options::default(), &req, 1, 8)
            .unwrap()
            .expect("a dark-page lock should appear within 8 seeds");

        let rom = randomize_rom(&van, hit.seed, &Options::default(), None).unwrap();
        assert!(
            req.find(&rom).len() >= req.min_count,
            "reported seed {} does not satisfy the predicate",
            hit.seed
        );
        // Matches are reported, not just counted — a silent pass is worse than
        // no predicate (a lock with no fortress beside it tests a different
        // code path than a fortress clear).
        assert!(hit.report.iter().any(|l| l.contains("lock at W8")));
        assert!(hit.report.iter().any(|l| l.contains("contains:")));
    }

    #[test]
    fn search_gives_up_cleanly_when_nothing_matches() {
        let Some(van) = vanilla() else { return };
        // W1 has no Bowser castle, so this can never be satisfied.
        let req = Requirement::parse("bowser@w1").unwrap();
        assert!(search_seed(&van, &Options::default(), &req, 1, 3).unwrap().is_none());
    }

    #[test]
    fn unknown_level_name_is_an_error() {
        let Some(van) = vanilla() else { return };
        let err = build(
            &van,
            &TestRomSpec {
                placements: vec![Placement { level: "9-9".into(), slot: None }],
                ..spec()
            },
        )
        .unwrap_err();
        assert!(err.contains("unknown level"), "got: {err}");
    }

    /// A hammer bro / toad house has no placeable level data; the failure
    /// should name the kind rather than silently writing garbage pointers.
    #[test]
    fn placing_a_non_level_entry_explains_why() {
        let Some(van) = vanilla() else { return };
        let err = build(
            &van,
            &TestRomSpec {
                placements: vec![Placement { level: "1S".into(), slot: None }],
                ..spec()
            },
        )
        .unwrap_err();
        assert!(err.contains("start") || err.contains("unknown"), "got: {err}");
    }
}
