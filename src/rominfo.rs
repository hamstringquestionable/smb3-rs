//! Read-only lookups over a ROM, for people and tools: the Rust replacement
//! for `tools/rom_map.py`. Native-only, like `testrom`.
//!
//! Everything here is read through `rom_data::level_index`, the same model a
//! randomizer module uses, so the report and the randomizer cannot disagree
//! about where a level's bytes are. Names are the given ROM's own: on a
//! randomized ROM, `1-1` is whatever level now sits in 1-1's slot.

use std::fmt::Write;

use crate::randomize::items::powerups;
use crate::randomize::overworld::node_catalog::{EntryView, NodeCatalog, level_name_key};
use crate::randomize::rom_data::level_index::{
    self, Area, ChainEnd, DeadAlt, Object, object_name, power_blocks, read_commands, read_objects,
};
use crate::randomize::rom_data::{HB_EXCLUDE_ENTRIES, UNLISTED_LEVELS};
use crate::rom::Rom;

/// Every level name `--level` accepts, with its kind.
pub fn list_levels(rom_bytes: &[u8]) -> Result<Vec<String>, String> {
    let rom = Rom::from_bytes_lax(rom_bytes, true).map_err(|e| e.to_string())?;
    Ok(views(&rom)
        .iter()
        .filter(|e| e.level_entry.as_ref().is_some_and(|le| level_index::trace(&rom, le).is_ok()))
        .map(|e| format!("{:<8} {}", e.name, e.kind_label))
        .collect())
}

/// Everything in the level called `name`: where it is on the map, every area
/// a junction reaches, and each area's header, powerup blocks and objects.
pub fn level_report(rom_bytes: &[u8], name: &str) -> Result<String, String> {
    let rom = Rom::from_bytes_lax(rom_bytes, true).map_err(|e| e.to_string())?;
    let views = views(&rom);
    let want = level_name_key(name);
    let entry = views
        .iter()
        .find(|e| level_name_key(&e.name) == want)
        .ok_or_else(|| format!("unknown level {name:?} (try --list)"))?;
    let le = entry
        .level_entry
        .as_ref()
        .ok_or_else(|| format!("{} is a {} and has no level data", entry.name, entry.kind_label))?;

    let mut out = String::new();
    let _ = writeln!(out, "{} — {}", entry.name, entry.kind_label);
    if entry.world_idx == usize::MAX {
        let _ = writeln!(out, "  no pointer table entry: the engine loads it by address");
    } else {
        let (row, col) = entry.grid_pos;
        let _ = writeln!(
            out,
            "  World {} entry {}, map row {row} col {col}, tile ${:02X}",
            entry.world_idx + 1,
            entry.entry_idx,
            entry.tile
        );
    }
    let _ = writeln!(
        out,
        "  loaded with: tileset {}, layout ${:02X}{:02X}, objects ${:02X}{:02X}",
        le.tileset, le.lay_hi, le.lay_lo, le.obj_hi, le.obj_lo
    );

    if HB_EXCLUDE_ENTRIES.contains(&(u16::from_be_bytes([le.obj_hi, le.obj_lo]), le.tileset)) {
        let _ = writeln!(out, "  unused: the randomizer keeps it out of the Hammer Bro pool");
    }

    let (areas, end) = level_index::trace(&rom, le)
        .map_err(|dead| format!("{} does not lead to a level area: {dead:?}", entry.name))?;

    let shared: Vec<&str> = views
        .iter()
        .filter(|v| v.name != entry.name)
        .filter(|v| {
            v.level_entry.as_ref().is_some_and(|o| {
                level_index::trace(&rom, o)
                    .is_ok_and(|(a, _)| a[0].header.offset == areas[0].header.offset)
            })
        })
        .map(|v| v.name.as_str())
        .collect();
    if !shared.is_empty() {
        let _ = writeln!(out, "  same main area as: {}", shared.join(", "));
    }

    let mut bosses = Vec::new();
    for (i, area) in areas.iter().enumerate() {
        out.push('\n');
        write_area(&mut out, &rom, i, area, &mut bosses);
    }

    out.push('\n');
    let _ = writeln!(out, "Chain: {}", describe_end(end));
    let _ = writeln!(
        out,
        "Bosses: {}",
        if bosses.is_empty() { "none".to_string() } else { bosses.join(", ") }
    );
    Ok(out)
}

/// The catalog of `rom`, beta stages included, flattened for display, plus
/// the levels the engine loads without a pointer table entry (the Coin Ship).
fn views(rom: &Rom) -> Vec<EntryView> {
    let mut views = NodeCatalog::build(rom, true).entry_views();
    views.extend(UNLISTED_LEVELS.iter().map(|(name, entry)| EntryView {
        name: name.to_string(),
        world_idx: usize::MAX,
        entry_idx: usize::MAX,
        tile: 0,
        grid_pos: (usize::MAX, usize::MAX),
        kind_label: "unlisted",
        is_numbered_level: false,
        level_entry: Some(entry.clone()),
    }));
    views
}

fn write_area(out: &mut String, rom: &Rom, i: usize, area: &Area, bosses: &mut Vec<&'static str>) {
    let h = &area.header;
    let role = if i == 0 { "main" } else { "sub-area" };
    let _ = writeln!(
        out,
        "Area {i} ({role}): header 0x{:05X}, layout ${:04X}, tileset {}, objects ${:04X}",
        h.offset, area.layout_ptr, area.tileset, area.objects_ptr
    );
    let bytes: Vec<String> = h.bytes.iter().map(|b| format!("{b:02X}")).collect();
    let _ = writeln!(out, "  header bytes: {}", bytes.join(" "));
    let _ = writeln!(
        out,
        "  {} screens{}, music {} ({}) at 0x{:05X}, time {}",
        h.screens(),
        if h.vertical() { " (vertical)" } else { "" },
        h.music(),
        h.music_name().unwrap_or("?"),
        h.music_offset(),
        h.time(rom).map_or("none".to_string(), |t| t.to_string()),
    );
    let _ = writeln!(
        out,
        "  alt: layout ${:04X}, objects ${:04X}, tileset {}",
        h.alt_layout(),
        h.alt_objects(),
        h.alt_tileset()
    );

    let commands = read_commands(rom, area);
    let junctions = commands.iter().filter(|c| c.is_junction()).count();
    let _ = writeln!(
        out,
        "  commands: {} at 0x{:05X}..0x{:05X}, {junctions} junction(s)",
        commands.len(),
        h.offset + level_index::LEVEL_HEADER_SIZE,
        area.terminator
    );

    let blocks = power_blocks(area, &commands);
    let _ = writeln!(out, "  powerup blocks: {}", blocks.len());
    for b in &blocks {
        let c = &b.command;
        let _ = writeln!(
            out,
            "    0x{:05X}  {:<16} screen {:>2} col {:>2} row {:>2}{}",
            b.shape_offset(),
            b.name,
            c.screen(),
            c.column(),
            c.row(),
            randomizer_rule(b.shape_offset())
        );
    }

    match read_objects(rom, area.objects_ptr, area.tileset) {
        None => {
            let _ = writeln!(out, "  objects: none (pointer is not into the object data)");
        }
        Some(objects) => {
            let _ = writeln!(out, "  objects: {}", objects.len());
            for o in &objects {
                write_object(out, o);
                if let Some(name) = object_name(o.id).filter(|n| is_boss(n))
                    && !bosses.contains(&name)
                {
                    bosses.push(name);
                }
            }
        }
    }
}

fn write_object(out: &mut String, o: &Object) {
    let _ = writeln!(
        out,
        "    0x{:05X}  ${:02X} {:<22} x ${:02X} (screen {:>2} col {:>2})  y ${:02X}",
        o.offset,
        o.id,
        object_name(o.id).unwrap_or("?"),
        o.x,
        o.x >> 4,
        o.x & 0x0F,
        o.y
    );
}

/// What the powerup randomizer does differently with the block at `offset`.
fn randomizer_rule(offset: usize) -> &'static str {
    if powerups::PROTECTED_OFFSETS.contains(&offset) {
        "  [protected: never rerolled]"
    } else if powerups::FLOWER_OR_LEAF_OFFSETS.contains(&offset) {
        "  [rerolled to flower or leaf only]"
    } else if powerups::AIRSHIP_QBLOCK_OFFSETS.contains(&offset) {
        "  [airship: no star when Koopalings are hammer-vulnerable]"
    } else {
        ""
    }
}

/// Koopaling, Bowser and the three Boom-Booms.
fn is_boss(name: &str) -> bool {
    name.starts_with("BOSS_KOOPALING")
        || name.starts_with("BOSS_BOWSER")
        || name.starts_with("BOOMBOOM")
}

fn describe_end(end: ChainEnd) -> String {
    match end {
        ChainEnd::NoAlt => "the last area's alt layout is $0000 (its exits leave the level)".into(),
        ChainEnd::LoopsTo(i) => format!("the last area's alt pointer loops back to area {i}"),
        ChainEnd::Dead(dead) => {
            format!("the last area's alt pointer is dead: {}", describe_dead(dead))
        }
    }
}

fn describe_dead(dead: DeadAlt) -> String {
    match dead {
        DeadAlt::Tileset(t) => format!("tileset {t} is not a level tileset"),
        DeadAlt::NoRegion(t) => format!("tileset {t}'s bank holds no level data"),
        DeadAlt::NotAHeader(o) => format!("the bytes at 0x{o:05X} are not a level header"),
        DeadAlt::Unterminated(o) => format!("the commands at 0x{o:05X} run off the bank"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vanilla() -> Option<Vec<u8>> {
        std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()
    }

    /// Every name `--list` offers produces a report.
    #[test]
    fn every_listed_level_reports() {
        let Some(bytes) = vanilla() else { return };
        let names = list_levels(&bytes).unwrap();
        assert_eq!(names.len(), 324, "322 traced entries plus the Coin Ship and Unused Level 5");
        for line in names {
            let name = line.split_whitespace().next().unwrap();
            level_report(&bytes, name).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn a_report_lists_each_area_and_accepts_either_fortress_spelling() {
        let Some(bytes) = vanilla() else { return };
        let r = level_report(&bytes, "1-1").unwrap();
        assert!(r.contains("Area 0 (main): header 0x1FB92"), "{r}");
        assert!(r.contains("Area 1 (sub-area): header 0x1FCA3"), "{r}");
        assert!(r.contains("loops back to area 0"), "{r}");
        assert_eq!(level_report(&bytes, "6-F1"), level_report(&bytes, "6f1"));
        assert!(level_report(&bytes, "9-9").is_err());
        assert!(level_report(&bytes, "3HB18").unwrap().contains("unused:"));
        assert!(!level_report(&bytes, "3HB17").unwrap().contains("unused:"));
    }
}
