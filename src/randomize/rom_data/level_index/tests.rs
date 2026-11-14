use super::areas::Trace;
use super::*;
use crate::randomize::overworld::node_catalog::NodeCatalog;
use crate::rom::Rom;

fn load_rom() -> Option<Rom> {
    let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
    Rom::from_bytes(&data).ok()
}

/// `(name, trace)` for every catalog entry with a layout, beta stages included.
fn traces(rom: &Rom) -> Vec<(String, Result<Trace, DeadAlt>)> {
    let mut out = Vec::new();
    for e in NodeCatalog::build(rom, true).entries {
        if let Some(le) = &e.level_entry
            && (le.lay_lo, le.lay_hi) != (0, 0)
        {
            out.push((e.name.clone(), trace(rom, le)));
        }
    }
    out
}

/// The area list, for reading. `cargo test --lib print_level_areas -- --ignored --nocapture`
#[test]
#[ignore]
fn print_level_areas() {
    let Some(rom) = load_rom() else { return };
    for (name, t) in traces(&rom) {
        match t {
            Err(dead) => eprintln!("{name:8} NOT A LEVEL AREA: {dead:?}"),
            Ok((areas, end)) => {
                let list: Vec<String> = areas
                    .iter()
                    .map(|a| {
                        format!(
                            "0x{:05X}..0x{:05X} ts{} obj ${:04X} {}scr",
                            a.header.offset,
                            a.terminator,
                            a.tileset,
                            a.objects_ptr,
                            a.header.screens()
                        )
                    })
                    .collect();
                eprintln!("{name:8} {} | {end:?}", list.join("  ->  "));
            }
        }
    }
}

/// Every distinct area's bytes, header through terminator, as `(start, end, names)`.
fn spans(rom: &Rom) -> Vec<(usize, usize, Vec<String>)> {
    let mut by_header: std::collections::BTreeMap<usize, (usize, Vec<String>)> = Default::default();
    for (name, t) in traces(rom) {
        let Ok((areas, _)) = t else { continue };
        for a in areas {
            let e = by_header.entry(a.header.offset).or_insert((a.terminator, vec![]));
            assert_eq!(e.0, a.terminator, "{name}: one header, two command lengths");
            if !e.1.contains(&name) {
                e.1.push(name.clone());
            }
        }
    }
    by_header.into_iter().map(|(s, (t, n))| (s, t, n)).collect()
}

/// Every pointer table entry with a layout leads to a real level area.
#[test]
fn every_entry_with_a_layout_is_a_level_area() {
    let Some(rom) = load_rom() else { return };
    let t = traces(&rom);
    assert_eq!(t.len(), 322);
    for (name, t) in t {
        assert!(t.is_ok(), "{name}: {:?}", t.err());
    }
}

/// Only two vanilla chains end on a pointer the engine never follows, and
/// both name tileset 0 (the map's).
#[test]
fn the_dead_alt_pointers_are_the_known_two() {
    let Some(rom) = load_rom() else { return };
    let dead: Vec<(String, ChainEnd)> = traces(&rom)
        .into_iter()
        .filter_map(|(name, t)| {
            let (_, end) = t.ok()?;
            matches!(end, ChainEnd::Dead(_)).then_some((name, end))
        })
        .collect();
    let want = ChainEnd::Dead(DeadAlt::Tileset(0));
    assert_eq!(dead, vec![("3-9".to_string(), want), ("3HB18".to_string(), want)]);
}

/// No area's commands run into another's. The one exception is vanilla's:
/// 3HB18 shares its siblings' layout pointer (`$B3E7`) but names tileset 3
/// where they name 1, so the pointer resolves in the Hilly bank, inside 7-3's
/// last command. It loads a garbled arena; the pipeline corrects it to
/// tileset 1 (`qol::fix_3hb18_tileset`), but these tests read the vanilla ROM,
/// where it stays. A walk that sizes a junction (`$E0`+) by the tileset table
/// instead fails this in Ice/Sky.
#[test]
fn areas_do_not_overlap_except_3hb18() {
    let Some(rom) = load_rom() else { return };
    let s = spans(&rom);
    assert_eq!(s.len(), 224);
    let overlaps: Vec<(usize, usize)> =
        s.windows(2).filter(|w| w[1].0 <= w[0].1).map(|w| (w[0].0, w[1].0)).collect();
    assert_eq!(overlaps, vec![(0x21294, 0x213F7), (0x213F7, 0x213FB)]);
}

/// The headers a writer must leave alone are exactly 3HB18's and the W1 arena
/// its byte 8 lands in. 7-3 is not among them: its header is its own, even
/// though 3HB18 sits inside its commands.
#[test]
fn the_shared_headers_are_3hb18_and_the_w1_arena() {
    let Some(rom) = load_rom() else { return };
    let shared: Vec<usize> = shared_headers(&every_level(&rom)).into_iter().collect();
    assert_eq!(shared, vec![0x213F7, 0x213FB]);
}

/// Every object any vanilla area loads is one the disassembly names: a
/// missing name means either a misread pointer or a gap in the table. The one
/// exception is vanilla's: 6A places `$AB`, which has no `OBJ_` constant; its
/// init slot (PRG005) is `ObjInit_FireJetRight`, "doesn't really work".
#[test]
fn every_vanilla_object_but_6as_ab_has_a_name() {
    let Some(rom) = load_rom() else { return };
    let mut unnamed = std::collections::BTreeSet::new();
    for (name, t) in traces(&rom) {
        let (areas, _) = t.unwrap();
        for a in &areas {
            for o in read_objects(&rom, a.objects_ptr, a.tileset).unwrap_or_default() {
                if object_name(o.id).is_none() {
                    unnamed.insert(format!("{name} 0x{:05X} ${:02X}", o.offset, o.id));
                }
            }
        }
    }
    let want: std::collections::BTreeSet<String> = ["6A 0x0D80B $AB".to_string()].into();
    assert_eq!(unnamed, want);
}

/// The boss the catalog's classification implies is in the level's objects:
/// a Boom-Boom in every fortress, a Koopaling in every airship, Bowser in 8B.
/// Ties what the index reads to what the catalog decided independently.
#[test]
fn every_fortress_airship_and_castle_has_its_boss() {
    use crate::randomize::overworld::node_catalog::NodeKind;
    let Some(rom) = load_rom() else { return };
    for e in NodeCatalog::build(&rom, false).entries {
        let boss = match e.kind {
            NodeKind::Fortress => "BOOMBOOM",
            NodeKind::Airship => "BOSS_KOOPALING",
            NodeKind::Bowser => "BOSS_BOWSER",
            _ => continue,
        };
        let (areas, _) = trace(&rom, e.level_entry.as_ref().unwrap()).unwrap();
        let found = areas.iter().any(|a| {
            read_objects(&rom, a.objects_ptr, a.tileset)
                .unwrap_or_default()
                .iter()
                .any(|o| object_name(o.id).is_some_and(|n| n.starts_with(boss)))
        });
        assert!(found, "{} ({:?}) has no {boss}", e.name, e.kind);
    }
}
