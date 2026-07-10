//! Catalog build tests: count/shape invariants against the vanilla ROM plus a
//! `--ignored` catalog dump for visual inspection.

use std::collections::HashMap;

use super::*;
use crate::randomize::rom_data::{MAP_TILE_GRIDS, WORLDS};

fn load_rom() -> Option<Rom> {
    let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
    Rom::from_bytes(&data).ok()
}

#[test]
fn test_total_count() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = NodeCatalog::build(&rom, false);
    let expected: usize = WORLDS.iter().map(|w| w.entry_count).sum();
    assert_eq!(catalog.entries.len(), expected, "expected {expected} total entries");
}

#[test]
fn test_kind_counts() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = NodeCatalog::build(&rom, false);

    let count = |pred: fn(&NodeKind) -> bool| -> usize {
        catalog.entries.iter().filter(|e| pred(&e.kind)).count()
    };

    assert_eq!(count(|k| matches!(k, NodeKind::Fortress { .. })), 17, "fortresses");
    assert_eq!(count(|k| matches!(k, NodeKind::Airship)), 7, "airships");
    assert_eq!(count(|k| matches!(k, NodeKind::Bowser)), 1, "bowser");
    assert_eq!(count(|k| matches!(k, NodeKind::Start)), 8, "starts");
    assert_eq!(count(|k| matches!(k, NodeKind::Pipe { .. })), 48, "pipe endpoints");
}

#[test]
fn test_pipe_pairs_consistent() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = NodeCatalog::build(&rom, false);

    // Every dest_idx should appear exactly twice
    let mut dest_counts: HashMap<usize, usize> = HashMap::new();
    for e in &catalog.entries {
        if let NodeKind::Pipe { dest_idx, .. } = &e.kind {
            *dest_counts.entry(*dest_idx).or_insert(0) += 1;
        }
    }

    for (&dest, &count) in &dest_counts {
        assert_eq!(count, 2, "dest_idx {dest} should appear exactly twice, got {count}");
    }
    assert_eq!(dest_counts.len(), 24, "should have 24 unique dest indices");
}

#[test]
fn test_names_non_empty() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = NodeCatalog::build(&rom, false);

    for e in &catalog.entries {
        assert!(
            !e.name.is_empty(),
            "W{} entry {} has empty name",
            e.world_idx + 1, e.entry_idx,
        );
    }
}

#[test]
fn test_grid_positions_valid() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = NodeCatalog::build(&rom, false);

    for e in &catalog.entries {
        let (row, col) = e.grid_pos;
        let max_cols = MAP_TILE_GRIDS[e.world_idx].columns;
        // Non-level entries may have row >= 9 (out of bounds) — that's fine,
        // they're classified as HammerBro. But level-like entries must be valid.
        if e.kind.is_level_like() {
            assert!(
                row < 9 && col < max_cols,
                "W{} {} ({:?}) at ({},{}) is out of bounds (max cols {})",
                e.world_idx + 1, e.name, e.kind, row, col, max_cols,
            );
        }
    }
}

#[test]
fn test_level_entry_presence() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = NodeCatalog::build(&rom, false);

    for e in &catalog.entries {
        if e.kind.is_level_like() {
            assert!(
                e.level_entry.is_some(),
                "W{} {} ({:?}) should have level_entry",
                e.world_idx + 1, e.name, e.kind,
            );
        }
    }
}

#[test]
fn test_fortress_boomboom_offsets() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = NodeCatalog::build(&rom, false);

    for e in &catalog.entries {
        if let NodeKind::Fortress { boomboom_y_offset } = &e.kind {
            assert!(
                *boomboom_y_offset != 0,
                "W{} {} has zero boomboom_y_offset",
                e.world_idx + 1, e.name,
            );
        }
    }
}

#[test]
fn test_kind_totals_sum_to_340() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = NodeCatalog::build(&rom, false);

    // Aggregate counts plus the known vanilla fixed totals
    // (17 fortresses, 48 pipes, 7 airships, 1 bowser, 8 starts)
    // must cover all 340 pointer table entries.
    let levels: usize = catalog.entries.iter()
        .filter(|e| matches!(e.kind, NodeKind::Level))
        .count();
    let fixed: usize = catalog.entries.iter()
        .filter(|e| matches!(
            e.kind,
            NodeKind::ToadHouse | NodeKind::BonusGame | NodeKind::HammerBro | NodeKind::MapObject
        ))
        .count();

    let total = levels + 17 + 48 + 7 + 1 + 8 + fixed;
    assert_eq!(total, 340, "total should be 340, got {total}");
}

/// Print the full catalog for visual inspection.
/// Run with: cargo test -- test_print_catalog --ignored --nocapture
#[test]
#[ignore]
fn test_print_catalog() {
    let rom = match load_rom() {
        Some(r) => r,
        None => {
            eprintln!("ROM not found, skipping");
            return;
        }
    };
    let catalog = NodeCatalog::build(&rom, false);

    let mut current_world = usize::MAX;
    for e in &catalog.entries {
        if e.world_idx != current_world {
            current_world = e.world_idx;
            eprintln!("\n=== World {} ({} entries) ===",
                current_world + 1,
                catalog.world(current_world).count(),
            );
        }

        let kind_str = match &e.kind {
            NodeKind::Level => "Level".to_string(),
            NodeKind::Fortress { boomboom_y_offset } =>
                format!("Fortress(bb=0x{boomboom_y_offset:05X})"),
            NodeKind::Pipe { dest_idx, .. } => format!("Pipe(dest={dest_idx})"),
            NodeKind::Airship => "Airship".to_string(),
            NodeKind::Bowser => "Bowser".to_string(),
            NodeKind::Start => "Start".to_string(),
            NodeKind::ToadHouse => "ToadHouse".to_string(),
            NodeKind::BonusGame => "BonusGame".to_string(),
            NodeKind::HammerBro => "HammerBro".to_string(),
            NodeKind::MapObject => "MapObject".to_string(),
        };

        let entry_str = if let Some(le) = &e.level_entry {
            let obj = (le.obj_hi as u16) << 8 | le.obj_lo as u16;
            let lay = (le.lay_hi as u16) << 8 | le.lay_lo as u16;
            format!("obj=${obj:04X} lay=${lay:04X} ts={}", le.tileset)
        } else {
            "—".to_string()
        };

        eprintln!(
            "  [{:2}] {:8} ({:2},{:2})  tile=${:02X}  {}  {}",
            e.entry_idx, e.name, e.grid_pos.0, e.grid_pos.1,
            e.tile, kind_str, entry_str,
        );
    }

    // Summary
    eprintln!("\n=== Summary ===");
    type KindPredicate = (&'static str, fn(&NodeKind) -> bool);
    let kind_names: &[KindPredicate] = &[
        ("Level", |k| matches!(k, NodeKind::Level)),
        ("Fortress", |k| matches!(k, NodeKind::Fortress { .. })),
        ("Pipe", |k| matches!(k, NodeKind::Pipe { .. })),
        ("Airship", |k| matches!(k, NodeKind::Airship)),
        ("Bowser", |k| matches!(k, NodeKind::Bowser)),
        ("Start", |k| matches!(k, NodeKind::Start)),
        ("ToadHouse", |k| matches!(k, NodeKind::ToadHouse)),
        ("BonusGame", |k| matches!(k, NodeKind::BonusGame)),
        ("HammerBro", |k| matches!(k, NodeKind::HammerBro)),
        ("MapObject", |k| matches!(k, NodeKind::MapObject)),
    ];
    for (name, pred) in kind_names {
        let c: usize = catalog.entries.iter().filter(|e| pred(&e.kind)).count();
        eprintln!("  {name:12} {c}");
    }
    eprintln!("  Total:       {}", catalog.entries.len());
}
