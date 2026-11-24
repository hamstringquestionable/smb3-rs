//! Builds [`GROUPS`](super::GROUPS) from the vanilla ROM.
//!
//! Every horizontal area is rendered with the engine's own loader
//! (`terrain::load`). A candidate group is a set of look-alike blocks: a
//! 4-connected cluster (rows, mostly), or for the levels in [`SCATTERED`] the
//! blocks within [`NEAR`] tiles of each other. Each cell must be open above and
//! below (hit from below, room for the item), and hold either a free power-up
//! (one the reroll may change, drawn by its own command) or a plain block.
//!
//! A group is kept only if the renderer agrees with what a swap should draw:
//! every power-up moved to every other cell, and a dozen whole shuffles. When a
//! power-up sits at the end of a plain run that stops short of it, lengthening
//! the run is tried first; cells that still draw wrong are left out.

use std::collections::{BTreeMap, BTreeSet};

use rand::SeedableRng;
use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;

use super::{GROUPS, Slot, arrange};
use crate::randomize::items::powerups::holds_free_power_up;
use crate::randomize::overworld::node_catalog::NodeCatalog;
use crate::randomize::rom_data::level_index::terrain::{self, ROWS, SCREEN_BYTES, Terrain};
use crate::randomize::rom_data::level_index::{Area, power_blocks, read_commands, trace};
use crate::rom::Rom;

/// Levels whose power-up sits in a pattern of blocks that do not touch,
/// picked by hand from the census: 1-2's and 2-4's note blocks, 3-4's ?
/// blocks. A distance rule over every level mostly catches scenery.
const SCATTERED: &[&str] = &["1-2/0", "2-4/0", "3-4/0"];
/// How far apart (rows and columns) blocks of a scattered pattern may be.
const NEAR: usize = 6;
/// Whole shuffles rendered per group, on top of every single move.
const SHUFFLES: u64 = 12;

/// Which blocks look alike: ? blocks, bricks, note blocks, bouncing wood.
fn look(tile: u8) -> Option<u8> {
    match tile {
        0x60..=0x65 => Some(0),
        0x67..=0x70 => Some(1),
        0x2E..=0x31 => Some(2),
        0x72..=0x75 => Some(3),
        _ => None,
    }
}

/// A flower, leaf, star or 1-up holder of any look.
fn holds_power_up(tile: u8) -> bool {
    matches!(tile, 0x60..=0x62 | 0x68..=0x6A | 0x6E | 0x2F..=0x31 | 0x73..=0x75)
}

/// A block with nothing worth moving: coin ? block, brick, coin brick, plain
/// note or wood block. Vines, P-switches, 10-coin and coin/star blocks are
/// neither, and stay out of every group.
fn plain(tile: u8) -> bool {
    matches!(tile, 0x63 | 0x65 | 0x67 | 0x6B | 0x2E | 0x72)
}

/// A generated group, before it is printed as a [`super::Group`].
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Found {
    level: String,
    prep: Vec<(usize, u8)>,
    slots: Vec<Slot>,
}

fn at(t: &Terrain, row: usize, col: usize) -> u8 {
    t.tile(col / 16, row, col % 16)
}

fn index(row: usize, col: usize) -> usize {
    col / 16 * SCREEN_BYTES + row * 16 + col % 16
}

/// Every group in the vanilla ROM, in level order. Prints what it left out.
pub(super) fn generate(rom: &Rom) -> Vec<Found> {
    let views = NodeCatalog::build(rom, true).entry_views();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for view in &views {
        let Some(entry) = view.level_entry.as_ref() else { continue };
        let Ok((areas, _)) = trace(rom, entry) else { continue };
        for (i, area) in areas.iter().enumerate() {
            if area.header.vertical() || !seen.insert(area.header.offset) {
                continue;
            }
            let level = format!("{}/{i}", view.name);
            out.extend(area_groups(rom, area, &level));
        }
    }
    out
}

fn area_groups(rom: &Rom, area: &Area, level: &str) -> Vec<Found> {
    let Ok(t) = terrain::load(rom, area) else { return Vec::new() };
    let commands = read_commands(rom, area);
    let movers: BTreeSet<usize> = power_blocks(area, &commands)
        .iter()
        .filter(|b| holds_free_power_up(b))
        .map(|b| b.command.offset)
        .collect();
    let width = area.header.screens() as usize * 16;
    let open = |row: Option<usize>, col: usize| match row {
        Some(r) if r < ROWS => {
            let x = at(&t, r, col);
            x < t.attr[(x >> 6) as usize]
        }
        _ => true,
    };
    let usable = |r: usize, c: usize| {
        look(at(&t, r, c)).is_some() && open(r.checked_sub(1), c) && open(Some(r + 1), c)
    };
    // The last one-block command drawn at a cell: a fixed shape, or a
    // variable one of length 1.
    let occupant = |r: usize, c: usize| {
        commands
            .iter()
            .rev()
            .find(|cmd| {
                let b = cmd.bytes;
                !cmd.is_junction()
                    && (b[0] & 0x1F) as usize == r
                    && b[1] as usize == c
                    && (cmd.fixed_shape().is_some() || b[2] & 0x0F == 0)
            })
            .map(|cmd| cmd.offset)
    };
    let slot = |r: usize, c: usize| -> Option<Slot> {
        let tile = at(&t, r, c);
        let command = occupant(r, c);
        if holds_power_up(tile) {
            let o = command?;
            movers.contains(&o).then_some((r as u8, c as u8, Some(o)))
        } else if plain(tile) {
            Some((r as u8, c as u8, command))
        } else {
            None
        }
    };

    // Candidate cell sets: clusters, then the hand-picked scattered patterns.
    let blocks: Vec<(usize, usize)> = (0..ROWS)
        .flat_map(|r| (0..width).map(move |c| (r, c)))
        .filter(|&(r, c)| look(at(&t, r, c)).is_some())
        .collect();
    let linked = |a: (usize, usize), b: (usize, usize), near: usize| {
        look(at(&t, a.0, a.1)) == look(at(&t, b.0, b.1))
            && a.0.abs_diff(b.0) <= near
            && a.1.abs_diff(b.1) <= near
            && (near > 1 || a.0.abs_diff(b.0) + a.1.abs_diff(b.1) == 1)
    };
    let mut candidates = clusters(&blocks, |a, b| linked(a, b, 1));
    if SCATTERED.contains(&level) {
        candidates.extend(clusters(&blocks, |a, b| linked(a, b, NEAR)));
    }

    let mut used = BTreeSet::new();
    let mut out = Vec::new();
    for cells in candidates {
        let slots: Vec<Slot> = cells
            .into_iter()
            .filter(|&(r, c)| usable(r, c) && !used.contains(&(r, c)))
            .filter_map(|(r, c)| slot(r, c))
            .collect();
        let Some(found) = settle(rom, area, &t, &commands, level, slots) else { continue };
        used.extend(found.slots.iter().map(|s| (s.0 as usize, s.1 as usize)));
        out.push(found);
    }
    out
}

/// Groups of `blocks` under the link relation, by flood fill.
fn clusters(
    blocks: &[(usize, usize)],
    linked: impl Fn((usize, usize), (usize, usize)) -> bool,
) -> Vec<Vec<(usize, usize)>> {
    let mut done = vec![false; blocks.len()];
    let mut out = Vec::new();
    for s in 0..blocks.len() {
        if done[s] {
            continue;
        }
        done[s] = true;
        let mut group = vec![s];
        let mut k = 0;
        while k < group.len() {
            let a = blocks[group[k]];
            k += 1;
            for j in 0..blocks.len() {
                if !done[j] && linked(a, blocks[j]) {
                    done[j] = true;
                    group.push(j);
                }
            }
        }
        out.push(group.into_iter().map(|j| blocks[j]).collect());
    }
    out
}

fn worth_keeping(slots: &[Slot]) -> bool {
    slots.len() >= 3 && slots.iter().any(|s| s.2.is_some())
}

/// Turn a candidate into a group the renderer agrees with, or `None`.
fn settle(
    rom: &Rom,
    area: &Area,
    t: &Terrain,
    commands: &[crate::randomize::rom_data::level_index::Command],
    level: &str,
    mut slots: Vec<Slot>,
) -> Option<Found> {
    let tile = |s: &Slot| at(t, s.0 as usize, s.1 as usize);
    let power = |s: &Slot| holds_power_up(tile(s));
    if !slots.iter().any(power) {
        return None;
    }
    // Cells with no command of their own must all show the same block, the
    // one a power-up leaves behind.
    let mut counts = BTreeMap::<u8, usize>::new();
    for s in slots.iter().filter(|s| s.2.is_none()) {
        *counts.entry(tile(s)).or_default() += 1;
    }
    let base = counts.iter().max_by_key(|e| e.1).map(|e| *e.0);
    slots.retain(|s| s.2.is_some() || Some(tile(s)) == base);
    if !worth_keeping(&slots) {
        return None;
    }

    // A plain run ending just before (or starting just after) a power-up,
    // drawn before it: lengthening the run puts a block under the power-up.
    let mut preps: Vec<Vec<(usize, u8)>> = vec![Vec::new()];
    for s in slots.iter().filter(|s| power(s)) {
        let at_stream = commands.iter().position(|c| Some(c.offset) == s.2)?;
        for c in &commands[..at_stream] {
            let b = c.bytes;
            let length = b[2] & 0x0F;
            if c.is_junction() || b[0] & 0x1F != s.0 || b[2] & 0xF0 == 0 || length == 0x0F {
                continue;
            }
            if b[1] as usize + length as usize + 1 == s.1 as usize {
                preps.push(vec![(c.offset + 2, b[2] + 1)]);
            } else if b[1] == s.1.wrapping_add(1) {
                preps.push(vec![(c.offset + 1, b[1] - 1), (c.offset + 2, b[2] + 1)]);
            }
        }
    }

    // No prep, each run alone, then every run together (two power-ups at
    // either end): the most cells wins, then the fewest bytes.
    if preps.len() > 2 {
        let all: Vec<(usize, u8)> = preps.concat();
        let offsets: BTreeSet<usize> = all.iter().map(|p| p.0).collect();
        if offsets.len() == all.len() {
            preps.push(all);
        }
    }
    let mut best: Option<Found> = None;
    for prep in preps {
        let mut slots = slots.clone();
        loop {
            match check(rom, area, t, &prep, &slots) {
                Ok(()) => break,
                Err(bad) if !bad.is_empty() => {
                    let mut i = 0;
                    slots.retain(|_| {
                        i += 1;
                        !bad.contains(&(i - 1))
                    });
                    if !worth_keeping(&slots) || !slots.iter().any(power) {
                        slots.clear();
                        break;
                    }
                }
                Err(_) => {
                    slots.clear();
                    break;
                }
            }
        }
        let better = best.as_ref().is_none_or(|b| {
            (slots.len(), std::cmp::Reverse(prep.len()))
                > (b.slots.len(), std::cmp::Reverse(b.prep.len()))
        });
        if !slots.is_empty() && better {
            best = Some(Found { level: level.to_string(), prep, slots });
        }
    }
    if best.is_none() {
        eprintln!("left out: {level} row {} (no arrangement draws right)", slots[0].0);
    }
    best
}

/// Render the identity, every power-up moved to every cell, and some whole
/// shuffles. `Err(cells)` names the target cells that drew wrong; an empty
/// set means the group as a whole is wrong.
fn check(
    rom: &Rom,
    area: &Area,
    t: &Terrain,
    prep: &[(usize, u8)],
    slots: &[Slot],
) -> Result<(), BTreeSet<usize>> {
    let tile = |s: &Slot| at(t, s.0 as usize, s.1 as usize);
    let base = slots.iter().find(|s| s.2.is_none()).map(tile);
    let original: Vec<Option<usize>> = slots.iter().map(|s| s.2).collect();
    let drawn = |arrangement: &[Option<usize>]| {
        let mut r = rom.clone();
        arrange(&mut r, prep, slots, arrangement);
        terrain::load(&r, area).map(|t| t.tiles).unwrap_or_default()
    };
    let wanted = |arrangement: &[Option<usize>]| {
        let mut want = t.tiles.clone();
        for (s, command) in slots.iter().zip(arrangement) {
            let from = command.and_then(|o| slots.iter().find(|x| x.2 == Some(o)));
            want[index(s.0 as usize, s.1 as usize)] = match from {
                Some(x) => tile(x),
                None => base.unwrap_or(0),
            };
        }
        want
    };
    if drawn(&original) != t.tiles {
        return Err(BTreeSet::new());
    }
    let mut bad = BTreeSet::new();
    for a in (0..slots.len()).filter(|&a| holds_power_up(tile(&slots[a]))) {
        for b in 0..slots.len() {
            if a == b || tile(&slots[a]) == tile(&slots[b]) {
                continue;
            }
            let mut arrangement = original.clone();
            arrangement.swap(a, b);
            if drawn(&arrangement) != wanted(&arrangement) {
                bad.insert(b);
            }
        }
    }
    if !bad.is_empty() {
        return Err(bad);
    }
    let mut rng = ChaCha8Rng::seed_from_u64(0);
    for _ in 0..SHUFFLES {
        let mut arrangement = original.clone();
        arrangement.shuffle(&mut rng);
        if drawn(&arrangement) != wanted(&arrangement) {
            return Err(BTreeSet::new());
        }
    }
    Ok(())
}

/// `found` as the source of [`GROUPS`].
fn source(found: &[Found]) -> String {
    let mut out = String::new();
    for f in found {
        let prep: Vec<String> =
            f.prep.iter().map(|(o, v)| format!("({o:#07X}, 0x{v:02X})")).collect();
        let slots: Vec<String> = f
            .slots
            .iter()
            .map(|(r, c, o)| match o {
                Some(o) => format!("({r}, {c}, Some({o:#07X}))"),
                None => format!("({r}, {c}, None)"),
            })
            .collect();
        out += &format!(
            "    Group {{ level: \"{}\", prep: &[{}], slots: &[{}] }},\n",
            f.level,
            prep.join(", "),
            slots.join(", ")
        );
    }
    out
}

/// [`GROUPS`] is exactly what `generate` builds from the vanilla ROM. On a
/// mismatch it prints the replacement table body. Skips without the ROM.
#[test]
fn table_is_current() {
    let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else { return };
    let rom = Rom::from_bytes_lax(&bytes, true).unwrap();
    let found = generate(&rom);
    let current: Vec<Found> = GROUPS
        .iter()
        .map(|g| Found { level: g.level.into(), prep: g.prep.to_vec(), slots: g.slots.to_vec() })
        .collect();
    if found != current {
        panic!("GROUPS is stale; replace its body with:\n{}", source(&found));
    }
}
