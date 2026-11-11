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

use rand::SeedableRng;

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

// --- Live-window CHR clashes -------------------------------------------------
//
// Every frame each live object writes its PatTableSel page into slot 4 or 5
// (`Object_DoStateAction`, prg000.asm), so two objects that want different
// pages on one slot clash whenever both are *alive*, not only when both are on
// screen. In a horizontal level an object spawns 1 tile past the right edge
// (`LOSBS_LookAhead`) and is deleted 2 tiles past the right / 3 past the left
// (`PRG000_D3CC`), so two stationary objects up to ~20 tiles apart can both be
// live. A few routines use the wide `Object_DeleteOffScreen_N2` test (8 tiles
// either side), stretching that to ~25. `CHR_GROUP_GAP` is 20; the bands below
// keep the 16 boundary so runs before and after the change compare.
//
// Run with:
//   cargo test --release --lib chr_clash_stats -- --ignored --nocapture

/// Objects whose normal routine deletes through `Object_DeleteOffScreen_N2`
/// (8 tiles past either edge): RotoDiscs, the sliding/vertical Thwomps (all
/// Thwomps counted, an over-count for the plain dropper), Big Bertha's
/// birther, the Flying Green Paratroopa and the tilting platform.
fn is_wide_range(id: u8) -> bool {
    ROTODISCS_SINGLE.contains(&id)
        || ROTODISCS_DUAL.contains(&id)
        || THWOMPS.contains(&id)
        || matches!(id, 0x63 | 0x80 | 0x90)
}

/// One room as the clash count sees it: vertical flag + (offset, id, x).
struct ClashRoom {
    vertical: bool,
    objects: Vec<(usize, u8, u8)>,
}

fn clash_rooms(rom: &Rom) -> Vec<ClashRoom> {
    let mut seen = BTreeSet::new();
    let mut rooms = Vec::new();
    for area in level_index::every_level(rom).iter().flatten() {
        if !seen.insert(area.objects_ptr) {
            continue;
        }
        if let Some(objects) = level_index::read_objects(rom, area.objects_ptr, area.tileset) {
            rooms.push(ClashRoom {
                vertical: area.header.vertical(),
                objects: objects.iter().map(|o| (o.offset, o.id, o.x)).collect(),
            });
        }
    }
    rooms
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Band {
    /// A level-wide chaser (CHASER_IDS) clashing with anything in the room.
    Chaser,
    /// Within CHR_GROUP_GAP: the model claims to prevent these.
    Within16,
    /// 17-20 tiles: both live under the standard deletion test.
    Gap17To20,
    /// 21-25 tiles, one of them wide-range: both live under the N2 test.
    Gap21To25Wide,
    /// Vertical level, any distance (X grouping does not apply there).
    Vertical,
}

impl Band {
    const ALL: [Band; 5] =
        [Band::Chaser, Band::Within16, Band::Gap17To20, Band::Gap21To25Wide, Band::Vertical];
    fn label(self) -> &'static str {
        match self {
            Band::Chaser => "chaser vs anything (any gap)",
            Band::Within16 => "gap <= 16 (inside the model)",
            Band::Gap17To20 => "gap 17-20 (both live)",
            Band::Gap21To25Wide => "gap 21-25, wide-range object",
            Band::Vertical => "vertical level (any gap)",
        }
    }
}

fn band(vertical: bool, a: u8, b: u8, gap: u8) -> Option<Band> {
    if vertical {
        return Some(Band::Vertical);
    }
    if CHASER_IDS.contains(&a) || CHASER_IDS.contains(&b) {
        return Some(Band::Chaser);
    }
    match gap {
        0..=16 => Some(Band::Within16),
        17..=20 => Some(Band::Gap17To20),
        21..=25 if is_wide_range(a) || is_wide_range(b) => Some(Band::Gap21To25Wide),
        _ => None,
    }
}

#[derive(Default)]
struct ClashTally {
    /// Per band: (all clashing pairs, pairs where at least one side changed).
    counts: BTreeMap<Band, (u64, u64)>,
    /// Per band: changed pairs by (name, name), unordered.
    pairs: BTreeMap<Band, BTreeMap<(&'static str, &'static str), u64>>,
    rooms_with_new: u64,
}

/// Count every same-slot, different-page pair in each room. `vanilla` is the
/// unrandomized ROM, to tell a pair Nintendo shipped from one the pass made.
fn tally_clashes(rooms: &[ClashRoom], vanilla: &Rom, t: &mut ClashTally) {
    for room in rooms {
        let mut room_new = false;
        for (i, &(off_a, a, xa)) in room.objects.iter().enumerate() {
            for &(off_b, b, xb) in &room.objects[i + 1..] {
                let (Some(pa), Some(pb)) = (sprite_bank(a), sprite_bank(b)) else { continue };
                if pa.slot != pb.slot || pa.chr_page == pb.chr_page {
                    continue;
                }
                let Some(band) = band(room.vertical, a, b, xa.abs_diff(xb)) else { continue };
                let changed = vanilla.read_byte(off_a) != a || vanilla.read_byte(off_b) != b;
                let c = t.counts.entry(band).or_default();
                c.0 += 1;
                if changed {
                    c.1 += 1;
                    room_new = true;
                    let (na, nb) = (name(a), name(b));
                    let key = if na <= nb { (na, nb) } else { (nb, na) };
                    *t.pairs.entry(band).or_default().entry(key).or_insert(0) += 1;
                }
            }
        }
        t.rooms_with_new += room_new as u64;
    }
}

fn print_tally(title: &str, t: &ClashTally, seeds: u64) {
    println!("\n=== {title} ===");
    println!("  rooms with a changed clash: {:.1}/seed", t.rooms_with_new as f64 / seeds as f64);
    println!("  {:<40} {:>10} {:>14}", "band", "all/seed", "changed/seed");
    for b in Band::ALL {
        let (all, new) = t.counts.get(&b).copied().unwrap_or((0, 0));
        println!(
            "  {:<40} {:>10.2} {:>14.2}",
            b.label(),
            all as f64 / seeds as f64,
            new as f64 / seeds as f64
        );
    }
    for b in Band::ALL {
        let Some(pairs) = t.pairs.get(&b) else { continue };
        let mut v: Vec<_> = pairs.iter().collect();
        v.sort_by(|x, y| y.1.cmp(x.1).then(x.0.cmp(y.0)));
        println!("\n  top changed pairs, {}:", b.label());
        for ((na, nb), n) in v.iter().take(12) {
            println!("    {:>7.2}/seed  {na} + {nb}", **n as f64 / seeds as f64);
        }
    }
}

#[test]
#[ignore = "instrument: counts CHR page clashes between objects that can be live together"]
fn chr_clash_stats() {
    let Some(rom) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes")
        .ok()
        .and_then(|b| Rom::from_bytes(&b).ok())
    else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    let seeds: u64 = std::env::var("CLASH_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(200);

    let mut vanilla = ClashTally::default();
    tally_clashes(&clash_rooms(&rom), &rom, &mut vanilla);
    print_tally("VANILLA (Nintendo's own clashes; 'changed' is 0 by definition)", &vanilla, 1);

    for (label, opts) in
        [("DEFAULT FLAGS", Options::default()), ("ALL ENEMY CLASSES WILD", max_enemy_opts())]
    {
        let mut t = ClashTally::default();
        for seed in 0..seeds {
            let mut r = rom.clone();
            pipeline::randomize(&mut r, seed, &opts);
            tally_clashes(&clash_rooms(&r), &rom, &mut t);
        }
        print_tally(&format!("{label} ({seeds} seeds) {}", opts.to_flag_key()), &t, seeds);
    }

    // The enemy pass alone, on a vanilla ROM: no stage before or after it. A
    // clash here is the pass breaking its own rules; one only in the finished
    // ROM came from another stage.
    let opts = max_enemy_opts();
    let mut t = ClashTally::default();
    let mut pass_time = std::time::Duration::ZERO;
    for seed in 0..seeds {
        let mut r = rom.clone();
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
        let started = std::time::Instant::now();
        randomize(&mut r, &mut rng, &opts);
        pass_time += started.elapsed();
        tally_clashes(&clash_rooms(&r), &rom, &mut t);
    }
    print_tally(&format!("ENEMY PASS ALONE, ALL WILD ({seeds} seeds)"), &t, seeds);
    println!("\n  enemy pass time: {:.3} ms/seed", pass_time.as_secs_f64() * 1000.0 / seeds as f64);
}

/// The pass never adds a CHR clash between two enemies it groups together:
/// no new same-slot, different-page pair within [`CHR_GROUP_GAP`] in a
/// horizontal room, and none at all in a vertical one. Chasers are left out
/// on purpose: Wild injection can still place one beside an enemy that then
/// fails to change, which `chr_clash_stats` reports and this does not claim to
/// prevent. Runs the enemy pass alone, so a failure here is the pass's own.
#[test]
fn enemy_pass_adds_no_chr_clash_within_a_group() {
    let Some(rom) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes")
        .ok()
        .and_then(|b| Rom::from_bytes(&b).ok())
    else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    // The two bands checked below together cover gaps 0..=20.
    assert_eq!(CHR_GROUP_GAP, 20, "re-point the bands checked here at the new gap");
    for (label, opts) in [("default", Options::default()), ("all Wild", max_enemy_opts())] {
        let mut t = ClashTally::default();
        for seed in 0..50 {
            let mut r = rom.clone();
            randomize(&mut r, &mut rand_chacha::ChaCha8Rng::seed_from_u64(seed), &opts);
            tally_clashes(&clash_rooms(&r), &rom, &mut t);
        }
        for b in [Band::Within16, Band::Gap17To20, Band::Vertical] {
            let new = t.counts.get(&b).map_or(0, |c| c.1);
            assert_eq!(
                new,
                0,
                "{label}: {new} new clash(es) in band '{}': {:?}",
                b.label(),
                t.pairs.get(&b)
            );
        }
    }
}
