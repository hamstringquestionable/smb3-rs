//! CHR page statistics across many seeds vs vanilla. An instrument, not a
//! check: it prints and asserts nothing. Run with:
//!
//! ```sh
//! cargo test --release --lib chr_page_stats -- --ignored --nocapture
//! ```
//!
//! Every count reads the object lists the level index reaches on that ROM, so
//! bytes no level loads are not counted.

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::pipeline::{self, WildChaser};
use crate::randomize::rom_data::enemy_ptr_to_file_offset;
use crate::randomize::rom_data::level_index::{self, object_name};

/// Wild-injection-only obj_ids. Neither is a member of any class swap pool,
/// so any occurrence in a patched ROM where vanilla differed comes from wild
/// injection. (BossBass 0x63 is excluded because it's also in WATER_ENEMIES
/// and ambiguous under Wild water mode.)
const INJECTION_ONLY_IDS: &[u8] = &[0x83, 0xAF];

const NUM_SEEDS: u64 = 200;

/// All enemy classes Wild + hb_encounters Wild + wild_injections — the most
/// aggressive randomization config, used to surface CHR page imbalances.
fn max_enemy_opts() -> Options {
    Options {
        ground: EnemyMode::Wild,
        shell: EnemyMode::Wild,
        flying: EnemyMode::Wild,
        piranhas: EnemyMode::Wild,
        ghosts: EnemyMode::Wild,
        thwomps: EnemyMode::Wild,
        rotodiscs: EnemyMode::Wild,
        cannons: EnemyMode::Wild,
        water: EnemyMode::Wild,
        bros: EnemyMode::Wild,
        hb_encounters: EnemyMode::Wild,
        wild_injections: WildChaser::ALL.to_vec(),
        ..Options::default()
    }
}

/// Every object list a level loads on `rom`, keyed by its page byte's file
/// offset: each entry's (file offset of the ID byte, ID).
fn rooms(rom: &Rom) -> BTreeMap<usize, Vec<(usize, u8)>> {
    let mut rooms = BTreeMap::new();
    for area in level_index::every_level(rom).iter().flatten() {
        if let Some(objects) = level_index::read_objects(rom, area.objects_ptr, area.tileset) {
            rooms.insert(
                enemy_ptr_to_file_offset(area.objects_ptr),
                objects.iter().map(|o| (o.offset, o.id)).collect(),
            );
        }
    }
    rooms
}

type PageCounts = BTreeMap<u8, u64>;

#[derive(Default)]
struct ScanStats {
    slot4: PageCounts,
    slot5: PageCounts,
    ids: PageCounts,
    total_rooms: u64,
}

impl ScanStats {
    fn add(&mut self, other: &ScanStats) {
        for (dst, src) in [
            (&mut self.slot4, &other.slot4),
            (&mut self.slot5, &other.slot5),
            (&mut self.ids, &other.ids),
        ] {
            for (&k, &n) in src {
                *dst.entry(k).or_insert(0) += n;
            }
        }
        self.total_rooms += other.total_rooms;
    }
}

/// Per room: every ID, and the first CHR page each sprite slot commits to.
fn scan(rooms: &BTreeMap<usize, Vec<(usize, u8)>>) -> ScanStats {
    let mut stats = ScanStats::default();
    for objects in rooms.values().filter(|o| !o.is_empty()) {
        stats.total_rooms += 1;
        let (mut slot4, mut slot5) = (None, None);
        for &(_, id) in objects {
            *stats.ids.entry(id).or_insert(0) += 1;
            if let Some(bank) = sprite_bank(id) {
                match bank.slot {
                    4 if slot4.is_none() => slot4 = Some(bank.chr_page),
                    5 if slot5.is_none() => slot5 = Some(bank.chr_page),
                    _ => {}
                }
            }
        }
        for (counts, page) in [(&mut stats.slot4, slot4), (&mut stats.slot5, slot5)] {
            if let Some(p) = page {
                *counts.entry(p).or_insert(0) += 1;
            }
        }
    }
    stats
}

fn print_slot(label: &str, counts: &PageCounts) {
    let total: u64 = counts.values().sum();
    println!("{label} ({total} commitments):");
    for (&page, &count) in counts {
        println!("  CHR ${page:02X}: {count:6} ({:5.1}%)", count as f64 / total as f64 * 100.0);
    }
}

fn name(id: u8) -> &'static str {
    object_name(id).unwrap_or("?")
}

#[test]
#[ignore = "instrument: prints CHR page and wild-pool statistics over 200 seeds"]
fn chr_page_stats() {
    let Some(rom) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes")
        .ok()
        .and_then(|b| Rom::from_bytes(&b).ok())
    else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    let opts = max_enemy_opts();

    let vanilla_rooms = rooms(&rom);
    let vanilla = scan(&vanilla_rooms);

    println!("\n============================================================");
    println!("=== VANILLA ===");
    println!("Rooms with objects: {}", vanilla.total_rooms);
    println!("Unique enemy IDs: {}\n", vanilla.ids.len());
    print_slot("Slot 4", &vanilla.slot4);
    println!();
    print_slot("Slot 5", &vanilla.slot5);

    // Wild injection replaces a level's first enemy, so an injection-only ID
    // anywhere else in a room means it landed somewhere the design didn't
    // put it.
    let mut inj_first: u64 = 0;
    let mut inj_elsewhere: u64 = 0;

    let mut rando = ScanStats::default();
    for seed in 0..NUM_SEEDS {
        let mut rom_copy = rom.clone();
        pipeline::randomize(&mut rom_copy, seed, &opts);
        let finished = rooms(&rom_copy);
        rando.add(&scan(&finished));

        for objects in finished.values() {
            for (i, &(off, id)) in objects.iter().enumerate() {
                if !INJECTION_ONLY_IDS.contains(&id) || rom.read_byte(off) == id {
                    continue; // not an injection-only ID, or vanilla already had it
                }
                if i == 0 {
                    inj_first += 1;
                } else {
                    inj_elsewhere += 1;
                }
            }
        }
    }

    println!("\n============================================================");
    println!("=== RANDOMIZED ({NUM_SEEDS} seeds) ===");
    println!("Flags: {}", opts.to_flag_key());
    println!("Unique enemy IDs seen: {}", rando.ids.len());
    println!("Rooms with objects (avg): {:.0}\n", rando.total_rooms as f64 / NUM_SEEDS as f64);
    print_slot("Slot 4", &rando.slot4);
    println!();
    print_slot("Slot 5", &rando.slot5);

    println!("\n============================================================");
    println!("=== DIFF (rando avg per seed vs vanilla) ===\n");
    let all_pages: BTreeSet<u8> = vanilla
        .slot4
        .keys()
        .chain(vanilla.slot5.keys())
        .chain(rando.slot4.keys())
        .chain(rando.slot5.keys())
        .copied()
        .collect();

    println!("  Page   | Slot 4 vanilla | Slot 4 rando avg | Slot 5 vanilla | Slot 5 rando avg");
    println!("  -------+----------------+------------------+----------------+-----------------");
    for &page in &all_pages {
        let v4 = *vanilla.slot4.get(&page).unwrap_or(&0) as f64;
        let r4 = *rando.slot4.get(&page).unwrap_or(&0) as f64 / NUM_SEEDS as f64;
        let v5 = *vanilla.slot5.get(&page).unwrap_or(&0) as f64;
        let r5 = *rando.slot5.get(&page).unwrap_or(&0) as f64 / NUM_SEEDS as f64;
        println!(
            "  ${page:02X}    | {v4:>10.0}     | {r4:>10.1} ({:>+6.1}) | {v5:>10.0}     | {r5:>10.1} ({:>+6.1})",
            r4 - v4,
            r5 - v5,
        );
    }

    println!("\n============================================================");
    println!("=== WILD POOL DISTRIBUTION (rando, total across {NUM_SEEDS} seeds) ===\n");
    let total_entries: u64 = rando.ids.values().sum();
    // With max_enemy_opts() every class is Wild, so this is the union of all
    // class pools.
    let wild_set: BTreeSet<u8> =
        ClassModes::from_options(&opts).build_wild_pool().into_iter().collect();
    let mut wild_sorted: Vec<u8> = wild_set.iter().copied().collect();
    // Sort by rando count descending; tie-break by id ascending for stability.
    wild_sorted.sort_by(|a, b| {
        let ca = rando.ids.get(a).copied().unwrap_or(0);
        let cb = rando.ids.get(b).copied().unwrap_or(0);
        cb.cmp(&ca).then(a.cmp(b))
    });
    println!("  ID    | Name                       | Count  | Avg/seed | % of all | Vanilla count");
    println!("  ------+----------------------------+--------+----------+----------+--------------");
    let mut zero_count = 0u32;
    for &id in &wild_sorted {
        let count = rando.ids.get(&id).copied().unwrap_or(0);
        let avg = count as f64 / NUM_SEEDS as f64;
        let pct = count as f64 / total_entries as f64 * 100.0;
        let vcount = *vanilla.ids.get(&id).unwrap_or(&0);
        let marker = if count == 0 { " *" } else { "  " };
        println!(
            "  0x{id:02X}{marker}| {:<26} | {count:>6} | {avg:>8.1} | {pct:>7.1}% | {vcount:>6}",
            name(id)
        );
        if count == 0 {
            zero_count += 1;
        }
    }
    println!(
        "\n  Wild-pool size: {} ids   ({zero_count} dropped to zero, marked *)",
        wild_sorted.len()
    );

    // Also show the top non-wild-pool IDs for context (structural / fixed
    // objects the picker doesn't touch: useful anchors for the numbers above).
    let mut non_wild: Vec<(&u8, &u64)> =
        rando.ids.iter().filter(|(id, _)| !wild_set.contains(id)).collect();
    non_wild.sort_by(|a, b| b.1.cmp(a.1));
    println!(
        "\n  ── Top 10 non-wild-pool IDs (structural / fixed objects, picker doesn't touch) ──"
    );
    for &(&id, &count) in non_wild.iter().take(10) {
        let avg = count as f64 / NUM_SEEDS as f64;
        let pct = count as f64 / total_entries as f64 * 100.0;
        let vcount = *vanilla.ids.get(&id).unwrap_or(&0);
        println!(
            "  0x{id:02X}  | {:<26} | {count:>6} | {avg:>8.1} | {pct:>7.1}% | {vcount:>6}",
            name(id)
        );
    }

    println!("\n============================================================");
    println!("=== WILD INJECTION PLACEMENT ({NUM_SEEDS} seeds) ===\n");
    let inj_total = inj_first + inj_elsewhere;
    let first_pct = if inj_total > 0 { inj_first as f64 / inj_total as f64 * 100.0 } else { 0.0 };
    println!("Lakitu (0x83) + AngrySun (0xAF) in loaded rooms where vanilla differed:");
    println!(
        "  On the room's first object : {inj_first:>5}  ({first_pct:>5.1}%)  ← where injection writes"
    );
    println!(
        "  Elsewhere in the room      : {inj_elsewhere:>5}  ({:>5.1}%)  ← should be 0",
        100.0 - first_pct
    );
    println!("  Total injection events     : {inj_total:>5}");
    println!("  Avg per seed               : {:.1}", inj_total as f64 / NUM_SEEDS as f64);
}
