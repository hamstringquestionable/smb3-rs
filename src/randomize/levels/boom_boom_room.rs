//! Boom-Boom Room: the floating platforms in a fortress boss room are dealt
//! new places and widths.
//!
//! Every fortress boss room has the same shell: one screen wide (the screen
//! Boom-Boom is placed on), a wall in column 15, the ceiling at row 15 and the
//! floor a few rows below the open space. A platform is one layout command, a
//! run of blocks, so moving it is rewriting the command in place: no bytes
//! move, and no free space is spent. See `docs/smb3_rom_reference.md`,
//! "Boom-Boom Boss Rooms: Floating Platforms".
//!
//! Two rooms with no platforms in vanilla, 1F and 8F1, get the commands in
//! their area that draw nothing a later command does not draw over
//! (`steals_draw_nothing` checks this with the engine's loader), rewritten
//! into platforms. 5F2 has such a command too, but the room's background is
//! drawn after it, so a platform there would be painted over. 2F, 3F1, 5F1,
//! 7F1 and the three World 8 rooms have none, and are left alone.
//!
//! 8F1's backdrop command is also the one the Frog Suit softlock fix
//! (`items::frog_softlocks`) repurposes, so with that fix on 8F1 deals only
//! its other command: see [`commands`].
//!
//! Draws from its own substream ([`BOOM_BOOM_ROOM_SALT`]), so the option
//! moves no other part of the seed.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::rom::Rom;

/// The option's own RNG stream.
pub(crate) const BOOM_BOOM_ROOM_SALT: u64 = 0x424F_4F4D_524D_5631; // "BOOMRMV1"

/// How a room's platform commands are encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// Fortress (TS2) group 0 `$E0`: `[row, b1, $E0, width-1]`.
    FortressRun,
    /// Ice/sky (TS12) group 3 `$1n`: `[$60|row, b1, $10|width-1]`.
    IceRun,
}

/// A platform: its row and its first and last column within the room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Platform {
    pub(crate) row: u8,
    pub(crate) first: u8,
    pub(crate) last: u8,
}

/// One boss room.
pub(crate) struct Room {
    /// The level and area: a label for the write log, and where the tests
    /// look.
    pub(crate) level: &'static str,
    pub(crate) area: usize,
    /// The screen within the area: a command's `b1` high nibble.
    pub(crate) screen: u8,
    /// The first solid row under the room.
    pub(crate) floor: u8,
    /// Boom-Boom's spawn tile, row and column within the room (its object's
    /// Y low nibble + 16, and X low nibble).
    pub(crate) boss: (u8, u8),
    shape: Shape,
    /// The commands this room deals, by file offset.
    pub(crate) commands: &'static [usize],
    /// Solid ground inside the room that stays put (6F2's ledge, which
    /// Boom-Boom stands on).
    pub(crate) fixed: &'static [Platform],
    /// Cells a later command draws over (windows, mostly), so a platform
    /// there would not show. Rows `floor - 6 ..= floor - 3`, as `(row,
    /// column)`. `covered_cells_are_current` keeps it right.
    pub(crate) covered: &'static [(u8, u8)],
}

/// Platforms sit in rows `floor - 6 ..= floor - 3`: vanilla's 19-22 over a
/// row-25 floor, which leaves Big Mario room under the lowest.
const HIGHEST: u8 = 6;
const LOWEST: u8 = 3;
/// A platform this many rows above the floor or less is a jump from it.
const FLOOR_REACH: u8 = 5;
/// Columns 0-1 stay open for the entrance; column 15 is the wall.
const FIRST_COLUMN: u8 = 2;
const LAST_COLUMN: u8 = 14;
const MAX_WIDTH: u8 = 3;
/// Whole-room redeals before giving up and leaving the room as it is.
const ATTEMPTS: usize = 10_000;

#[rustfmt::skip]
pub(crate) const ROOMS: &[Room] = &[
    Room { level: "1F",  area: 0, screen: 9,  floor: 25, boss: (23, 13), shape: Shape::FortressRun,
           commands: &[0x2A99E], fixed: &[], covered: &[(19, 3), (19, 7), (19, 11)] },
    Room { level: "3F2", area: 1, screen: 5,  floor: 26, boss: (24, 12), shape: Shape::FortressRun,
           commands: &[0x2AB88, 0x2AB8C], fixed: &[], covered: &[] },
    Room { level: "4F1", area: 0, screen: 8,  floor: 25, boss: (23, 12), shape: Shape::FortressRun,
           commands: &[0x2B755], fixed: &[], covered: &[] },
    Room { level: "4F2", area: 0, screen: 8,  floor: 25, boss: (23, 13), shape: Shape::FortressRun,
           commands: &[0x2B678, 0x2B67C, 0x2B680], fixed: &[], covered: &[(19, 5), (19, 9), (20, 5), (20, 9)] },
    Room { level: "6F1", area: 0, screen: 7,  floor: 25, boss: (23, 12), shape: Shape::FortressRun,
           commands: &[0x2B203, 0x2B207], fixed: &[], covered: &[] },
    Room { level: "6F2", area: 1, screen: 1,  floor: 25, boss: (20, 13), shape: Shape::IceRun,
           commands: &[0x22CF0, 0x22CF3], fixed: &[Platform { row: 22, first: 11, last: 14 }], covered: &[] },
    Room { level: "6F3", area: 0, screen: 10, floor: 25, boss: (23, 12), shape: Shape::FortressRun,
           commands: &[0x2B11E, 0x2B122, 0x2B126, 0x2B12A, 0x2B12E], fixed: &[], covered: &[] },
    Room { level: "7F2", area: 1, screen: 3,  floor: 25, boss: (23, 12), shape: Shape::FortressRun,
           commands: &[0x2B031, 0x2B035, 0x2B039], fixed: &[], covered: &[] },
    Room { level: "8F1", area: 0, screen: 14, floor: 26, boss: (24, 12), shape: Shape::FortressRun,
           commands: &[0x2BA21, 0x2BA85], fixed: &[], covered: &[] },
];

/// The commands `room` deals. With `frog_fix` (the Frog Suit softlock fix
/// will run), 8F1 gives up its backdrop command, which that fix repurposes
/// into 8F's row-7 brick, and deals one platform instead of two. Taking it
/// here would leave 8F unfixed: the fix only lands on vanilla bytes.
pub(crate) fn commands(room: &Room, frog_fix: bool) -> &'static [usize] {
    let backdrop = &[crate::randomize::items::frog_softlocks::BACKDROP_8F];
    match room.commands.strip_suffix(backdrop) {
        Some(rest) if frog_fix => rest,
        _ => room.commands,
    }
}

/// Deal every room.
pub(crate) fn randomize(rom: &mut Rom, seed: u64, frog_fix: bool) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ BOOM_BOOM_ROOM_SALT);
    for room in ROOMS {
        let room = &Room { commands: commands(room, frog_fix), ..*room };
        if let Some(platforms) = deal(room, &mut rng) {
            rom.push_tag(&format!("{}/{}", room.level, room.area));
            write(rom, room, &platforms);
            rom.pop_tag();
        }
    }
}

/// A layout for `room` that keeps every rule, or `None` (the room stays as
/// it is) if none turns up in [`ATTEMPTS`] tries.
fn deal(room: &Room, rng: &mut ChaCha8Rng) -> Option<Vec<Platform>> {
    let rows = room.floor - HIGHEST..=room.floor - LOWEST;
    for _ in 0..ATTEMPTS {
        let platforms: Vec<Platform> = room
            .commands
            .iter()
            .map(|_| {
                let width = rng.random_range(1..=MAX_WIDTH);
                let first = rng.random_range(FIRST_COLUMN..=LAST_COLUMN + 1 - width);
                Platform { row: rng.random_range(rows.clone()), first, last: first + width - 1 }
            })
            .collect();
        if keeps_rules(room, &platforms) {
            return Some(platforms);
        }
    }
    None
}

/// The rules a layout must keep, checked together.
///
/// - **Clear of Boom-Boom**: no platform on the 2-by-2 tiles it spawns on
///   (the spawn tile down and right), or the column either side. Above its
///   head is allowed: vanilla 6F3 has a block there.
/// - **Apart**: two platforms within 2 rows of each other leave at least
///   one empty column between them, so none merge into a wall.
/// - **Reachable**: a platform is a jump from the floor ([`FLOOR_REACH`]
///   rows or less above it), or from a reachable platform 1-3 rows below it
///   and at most 3 columns to the side, not under it. The `?` ball can land
///   on any of them, so a platform Mario cannot get onto could hold it.
pub(crate) fn keeps_rules(room: &Room, platforms: &[Platform]) -> bool {
    let (boss_row, boss_col) = room.boss;
    let near_boss = |p: &Platform| {
        (boss_row..=boss_row + 1).contains(&p.row)
            && p.last + 1 >= boss_col
            && p.first <= boss_col + 2
    };
    if platforms.iter().any(near_boss) {
        return false;
    }
    let covers = |p: &Platform| {
        room.covered.iter().any(|&(r, c)| r == p.row && (p.first..=p.last).contains(&c))
    };
    if platforms.iter().any(covers) {
        return false;
    }
    let all: Vec<Platform> = platforms.iter().chain(room.fixed).copied().collect();
    for (i, p) in all.iter().enumerate() {
        for q in &all[i + 1..] {
            if p.row.abs_diff(q.row) < 3 && p.last + 1 >= q.first && q.last + 1 >= p.first {
                return false;
            }
        }
    }
    let mut reached: Vec<bool> = all.iter().map(|p| p.row + FLOOR_REACH >= room.floor).collect();
    loop {
        let mut grew = false;
        for i in 0..all.len() {
            if reached[i] {
                continue;
            }
            let p = all[i];
            let step = all.iter().zip(&reached).any(|(q, &r)| {
                let gap = if q.first > p.last {
                    q.first - p.last
                } else {
                    p.first.saturating_sub(q.last)
                };
                r && (1..=3).contains(&(q.row.saturating_sub(p.row))) && (1..=4).contains(&gap)
            });
            if step {
                reached[i] = true;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    reached.iter().all(|&r| r)
}

/// Rewrite each command as its platform.
fn write(rom: &mut Rom, room: &Room, platforms: &[Platform]) {
    for (&offset, p) in room.commands.iter().zip(platforms) {
        let b1 = room.screen << 4 | p.first;
        let width = p.last - p.first;
        match room.shape {
            Shape::FortressRun => rom.write_range(offset, &[p.row, b1, 0xE0, width]),
            Shape::IceRun => rom.write_range(offset, &[0x60 | p.row, b1, 0x10 | width]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::overworld::node_catalog::NodeCatalog;
    use crate::randomize::rom_data::level_index::terrain::{self, SCREEN_BYTES, Terrain};
    use crate::randomize::rom_data::level_index::{Area, read_commands, read_objects, trace};

    fn vanilla() -> Option<Rom> {
        let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Some(Rom::from_bytes_lax(&bytes, true).unwrap())
    }

    fn area(rom: &Rom, room: &Room) -> Area {
        let views = NodeCatalog::build(rom, true).entry_views();
        let view = views.iter().find(|v| v.name == room.level).unwrap();
        let (areas, _) = trace(rom, view.level_entry.as_ref().unwrap()).unwrap();
        areas[room.area].clone()
    }

    fn solid(t: &Terrain, screen: u8, row: u8, col: u8) -> bool {
        let tile = t.tile(screen as usize, row as usize, col as usize);
        tile >= t.attr[(tile >> 6) as usize + 4]
    }

    /// What vanilla has where the table says: the commands in this room's
    /// area, Boom-Boom where `boss` puts it, the floor row solid and the
    /// rows above it open apart from the vanilla platforms and `fixed`.
    #[test]
    fn table_matches_vanilla() {
        let Some(rom) = vanilla() else { return };
        for room in ROOMS {
            let area = area(&rom, room);
            let commands = read_commands(&rom, &area);
            for &offset in room.commands {
                let c = commands.iter().find(|c| c.offset == offset);
                let len = if room.shape == Shape::IceRun { 3 } else { 4 };
                assert_eq!(c.map(|c| c.len), Some(len), "{}: {offset:#X}", room.level);
            }
            let boss = read_objects(&rom, area.objects_ptr, area.tileset)
                .unwrap()
                .into_iter()
                .find(|o| (0x4B..=0x4C).contains(&o.id))
                .unwrap_or_else(|| panic!("{}: no Boom-Boom", room.level));
            assert_eq!(boss.x >> 4, room.screen, "{}", room.level);
            assert_eq!((boss.y & 0x0F | 0x10, boss.x & 0x0F), room.boss, "{}", room.level);
            let t = terrain::load(&rom, &area).unwrap();
            for col in 0..15 {
                assert!(solid(&t, room.screen, room.floor, col), "{} floor col {col}", room.level);
                assert!(!solid(&t, room.screen, room.boss.0, col.max(2)), "{}", room.level);
            }
        }
    }

    /// The vanilla layouts keep the rules, so the rules are not stricter
    /// than the game's own design. (The steal rooms have no vanilla layout.)
    #[test]
    fn vanilla_layouts_keep_the_rules() {
        let Some(rom) = vanilla() else { return };
        for room in ROOMS.iter().filter(|r| !["1F", "8F1"].contains(&r.level)) {
            let t = terrain::load(&rom, &area(&rom, room)).unwrap();
            let platforms = platforms_in(&t, room);
            assert_eq!(platforms.len(), room.commands.len(), "{}: {platforms:?}", room.level);
            // 3F2's two blocks are 6 and 5 rows over its row-26 floor, with
            // nothing under them; FLOOR_REACH is kept at 5 on purpose, so the
            // dealt layouts are never harder to get onto than that.
            if room.level != "3F2" {
                assert!(keeps_rules(room, &platforms), "{}: {platforms:?}", room.level);
            }
        }
    }

    /// The platforms the terrain shows in a room's platform rows: runs of
    /// solid tiles in columns 1-14, less `fixed`.
    fn platforms_in(t: &Terrain, room: &Room) -> Vec<Platform> {
        let mut out = Vec::new();
        for row in room.floor - HIGHEST..=room.floor - LOWEST {
            let mut col = 1;
            while col <= LAST_COLUMN {
                if solid(t, room.screen, row, col) {
                    let first = col;
                    while col < LAST_COLUMN && solid(t, room.screen, row, col + 1) {
                        col += 1;
                    }
                    let p = Platform { row, first, last: col };
                    if !room.fixed.contains(&p) {
                        out.push(p);
                    }
                }
                col += 1;
            }
        }
        out
    }

    /// `covered` is exactly the cells where a 1-wide platform from one of the
    /// room's commands does not show, with the room's other commands parked
    /// at row 0, column 0, up in the wall where they cannot reach the cell.
    /// Fails printing the replacement rows.
    #[test]
    fn covered_cells_are_current() {
        let Some(rom) = vanilla() else { return };
        let mut stale = false;
        for room in ROOMS {
            let area = area(&rom, room);
            let mut found = Vec::new();
            for row in room.floor - HIGHEST..=room.floor - LOWEST {
                for col in FIRST_COLUMN..=LAST_COLUMN {
                    let hidden = (0..room.commands.len()).any(|i| {
                        let mut probe = Rom::from_bytes_lax(rom.output_bytes(), true).unwrap();
                        let mut at =
                            vec![Platform { row: 0, first: 0, last: 0 }; room.commands.len()];
                        at[i] = Platform { row, first: col, last: col };
                        write(&mut probe, room, &at);
                        !solid(&terrain::load(&probe, &area).unwrap(), room.screen, row, col)
                    });
                    if hidden {
                        found.push((row, col));
                    }
                }
            }
            if found != room.covered {
                stale = true;
                println!("{}: covered: &{found:?}", room.level);
            }
        }
        assert!(!stale, "covered tables are stale; see above");
    }

    /// The commands taken for 1F and 8F1 draw nothing that survives:
    /// with one taken out, the area loads to the same tiles.
    #[test]
    fn steals_draw_nothing() {
        let Some(rom) = vanilla() else { return };
        for room in ROOMS.iter().filter(|r| ["1F", "8F1"].contains(&r.level)) {
            let area = area(&rom, room);
            let before = terrain::load(&rom, &area).unwrap().tiles;
            for &offset in room.commands {
                let mut bytes = rom.output_bytes().to_vec();
                let tail = bytes[offset + 4..=area.terminator].to_vec();
                bytes[offset..offset + tail.len()].copy_from_slice(&tail);
                let without = Rom::from_bytes_lax(&bytes, true).unwrap();
                let after = terrain::load(&without, &area).unwrap().tiles;
                let n = area.header.screens() as usize * SCREEN_BYTES;
                assert!(before[..n] == after[..n], "{}: {offset:#X} draws something", room.level);
            }
        }
    }

    /// Over many seeds, every room's dealt layout loads as dealt: the
    /// platforms are exactly the solid runs in the room, they keep the
    /// rules, and every tile outside the room is as vanilla left it.
    #[test]
    fn dealt_rooms_load_as_dealt() {
        let Some(vanilla) = vanilla() else { return };
        let seeds: u64 =
            std::env::var("CENSUS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
        let areas: Vec<Area> = ROOMS.iter().map(|r| area(&vanilla, r)).collect();
        let before: Vec<Vec<u8>> =
            areas.iter().map(|a| terrain::load(&vanilla, a).unwrap().tiles).collect();
        for seed in 0..seeds {
            let mut rom = Rom::from_bytes_lax(vanilla.output_bytes(), true).unwrap();
            let mut rng = ChaCha8Rng::seed_from_u64(seed ^ BOOM_BOOM_ROOM_SALT);
            for (i, room) in ROOMS.iter().enumerate() {
                let dealt = deal(room, &mut rng)
                    .unwrap_or_else(|| panic!("seed {seed}: {} found no layout", room.level));
                write(&mut rom, room, &dealt);
                let t = terrain::load(&rom, &areas[i]).unwrap();
                let mut seen = platforms_in(&t, room);
                let mut want = dealt.clone();
                seen.sort_by_key(|p| (p.row, p.first));
                want.sort_by_key(|p| (p.row, p.first));
                assert_eq!(seen, want, "seed {seed}: {}", room.level);
                assert!(keeps_rules(room, &seen), "seed {seed}: {}", room.level);
                let n = areas[i].header.screens() as usize * SCREEN_BYTES;
                let screen = room.screen as usize;
                for (k, (&now, &was)) in t.tiles[..n].iter().zip(&before[i]).enumerate() {
                    let (s, r, c) = (k / SCREEN_BYTES, k % SCREEN_BYTES / 16, k % 16);
                    let in_room = s == screen && (16..room.floor as usize).contains(&r) && c < 15;
                    if !in_room {
                        assert_eq!(now, was, "seed {seed}: {} s{s} r{r} c{c}", room.level);
                    }
                }
            }
        }
    }
}
