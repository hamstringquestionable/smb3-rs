//! Gather CHR page slot statistics across many seeds vs vanilla.
//! Run with: cargo test --test chr_stats -- --nocapture

use std::collections::BTreeMap;

use smb3_rs::randomize::enemies::sprite_bank;
use smb3_rs::randomize::rom_data::{ENEMY_DATA_END, ENEMY_DATA_START};
use smb3_rs::randomizer::{self, EnemyMode, Options};
use smb3_rs::rom::Rom;

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
        wild_injections: true,
        ..Options::default()
    }
}

type PageCounts = BTreeMap<u8, u64>;

#[derive(Default)]
struct ScanStats {
    slot4: PageCounts,
    slot5: PageCounts,
    ids: PageCounts,
    total_segments: u64,
}

/// Walk the enemy/object data block, invoking `f(segment_index, object_id)`
/// for each entry. Segment boundaries are 0xFF; each segment begins with a
/// page-flag byte followed by 3-byte (id, x, y) entries.
fn for_each_obj<F: FnMut(u64, u8)>(data: &[u8], mut f: F) {
    let mut i = 0;
    let mut seg: u64 = 0;
    while i < data.len() {
        if data[i] == 0xFF {
            i += 1;
            continue;
        }
        i += 1;
        let mut had_entry = false;
        while i + 2 < data.len() && data[i] != 0xFF {
            f(seg, data[i]);
            had_entry = true;
            i += 3;
        }
        if had_entry {
            seg += 1;
        }
    }
}

fn scan(data: &[u8]) -> ScanStats {
    let mut stats = ScanStats::default();
    let mut cur_seg: Option<u64> = None;
    let mut seg_slot4: Option<u8> = None;
    let mut seg_slot5: Option<u8> = None;

    let commit = |s: &mut ScanStats, p4: Option<u8>, p5: Option<u8>| {
        if p4.is_some() || p5.is_some() {
            s.total_segments += 1;
        }
        if let Some(p) = p4 {
            *s.slot4.entry(p).or_insert(0) += 1;
        }
        if let Some(p) = p5 {
            *s.slot5.entry(p).or_insert(0) += 1;
        }
    };

    for_each_obj(data, |seg, id| {
        if cur_seg != Some(seg) {
            commit(&mut stats, seg_slot4, seg_slot5);
            cur_seg = Some(seg);
            seg_slot4 = None;
            seg_slot5 = None;
        }
        *stats.ids.entry(id).or_insert(0) += 1;
        if let Some(bank) = sprite_bank(id) {
            match bank.slot {
                4 if seg_slot4.is_none() => seg_slot4 = Some(bank.chr_page),
                5 if seg_slot5.is_none() => seg_slot5 = Some(bank.chr_page),
                _ => {}
            }
        }
    });
    commit(&mut stats, seg_slot4, seg_slot5);
    stats
}

fn print_slot(label: &str, counts: &PageCounts) {
    let total: u64 = counts.values().sum();
    println!("{label} ({total} commitments):");
    for (&page, &count) in counts {
        println!("  CHR ${page:02X}: {count:6} ({:5.1}%)", count as f64 / total as f64 * 100.0);
    }
}

fn load_rom() -> Option<Rom> {
    let path = "Super Mario Bros. 3 (USA) (Rev 1).nes";
    let data = std::fs::read(path).ok()?;
    Rom::from_bytes(&data).ok()
}

#[test]
fn chr_page_stats() {
    let Some(rom) = load_rom() else {
        eprintln!("ROM file not present — skipping chr_page_stats (run locally with the ROM in repo root)");
        return;
    };
    let opts = max_enemy_opts();

    let vanilla = scan(rom.read_range(ENEMY_DATA_START, ENEMY_DATA_END - ENEMY_DATA_START));

    println!("\n============================================================");
    println!("=== VANILLA ===");
    println!("Total segments: {}", vanilla.total_segments);
    println!("Unique enemy IDs: {}\n", vanilla.ids.len());
    print_slot("Slot 4", &vanilla.slot4);
    println!();
    print_slot("Slot 5", &vanilla.slot5);

    let mut rando = ScanStats::default();
    for seed in 0..NUM_SEEDS {
        let mut rom_copy = rom.clone();
        randomizer::randomize(&mut rom_copy, seed, &opts);
        let s = scan(rom_copy.read_range(ENEMY_DATA_START, ENEMY_DATA_END - ENEMY_DATA_START));
        for (&page, &count) in &s.slot4 { *rando.slot4.entry(page).or_insert(0) += count; }
        for (&page, &count) in &s.slot5 { *rando.slot5.entry(page).or_insert(0) += count; }
        for (&id, &count) in &s.ids { *rando.ids.entry(id).or_insert(0) += count; }
        rando.total_segments += s.total_segments;
    }

    println!("\n============================================================");
    println!("=== RANDOMIZED ({NUM_SEEDS} seeds) ===");
    println!("Flags: {}", opts.to_flag_key());
    println!("Unique enemy IDs seen: {}", rando.ids.len());
    println!("Total segments (avg): {:.0}\n", rando.total_segments as f64 / NUM_SEEDS as f64);
    print_slot("Slot 4", &rando.slot4);
    println!();
    print_slot("Slot 5", &rando.slot5);

    println!("\n============================================================");
    println!("=== DIFF (rando avg per seed vs vanilla) ===\n");
    let all_pages: std::collections::BTreeSet<u8> = vanilla.slot4.keys()
        .chain(vanilla.slot5.keys())
        .chain(rando.slot4.keys())
        .chain(rando.slot5.keys())
        .copied().collect();

    println!("  Page   | Slot 4 vanilla | Slot 4 rando avg | Slot 5 vanilla | Slot 5 rando avg");
    println!("  -------+----------------+------------------+----------------+-----------------");
    for &page in &all_pages {
        let v4 = *vanilla.slot4.get(&page).unwrap_or(&0) as f64;
        let r4 = *rando.slot4.get(&page).unwrap_or(&0) as f64 / NUM_SEEDS as f64;
        let v5 = *vanilla.slot5.get(&page).unwrap_or(&0) as f64;
        let r5 = *rando.slot5.get(&page).unwrap_or(&0) as f64 / NUM_SEEDS as f64;
        println!(
            "  ${page:02X}    | {v4:>10.0}     | {r4:>10.1} ({:>+6.1}) | {v5:>10.0}     | {r5:>10.1} ({:>+6.1})",
            r4 - v4, r5 - v5,
        );
    }

    println!("\n============================================================");
    println!("=== TOP 20 ENEMY IDS (rando, total across {NUM_SEEDS} seeds) ===\n");
    let mut sorted_ids: Vec<_> = rando.ids.iter().collect();
    sorted_ids.sort_by(|a, b| b.1.cmp(a.1));
    let total_entries: u64 = rando.ids.values().sum();
    println!("  ID    | Count  | Avg/seed | % of all | Vanilla count");
    println!("  ------+--------+----------+----------+--------------");
    for &(&id, &count) in sorted_ids.iter().take(20) {
        let avg = count as f64 / NUM_SEEDS as f64;
        let pct = count as f64 / total_entries as f64 * 100.0;
        let vcount = *vanilla.ids.get(&id).unwrap_or(&0);
        println!("  0x{id:02X}  | {count:>6} | {avg:>8.1} | {pct:>7.1}% | {vcount:>6}");
    }
}
