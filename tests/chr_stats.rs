//! Gather CHR page slot statistics across many seeds vs vanilla.
//! Run with: cargo test --test chr_stats -- --nocapture

use std::collections::{BTreeMap, HashSet};

use smb3_rs::randomize::autoscroll::SPOILED_SEGMENT_RANGES;
use smb3_rs::randomize::enemies::{enemy_entry_points, sprite_bank, wild_pool_for};
use smb3_rs::randomize::rom_data::{ENEMY_DATA_END, ENEMY_DATA_START};
use smb3_rs::randomizer::{self, EnemyMode, Options};
use smb3_rs::rom::Rom;

/// Wild_injection-only obj_ids. Neither is a member of any class swap pool,
/// so any occurrence in a patched ROM where vanilla differed is guaranteed
/// to come from `inject_at_entry_points`. (BossBass 0x63 is excluded
/// because it's also in WATER_ENEMIES and ambiguous under Wild water mode.)
const INJECTION_ONLY_IDS: &[u8] = &[0x83, 0xAF];

/// Friendly name for an obj_id, sourced from enemies.rs comments,
/// tools/rom_map.py's ENEMY_NAMES, and docs/smb3_rom_reference.md.
/// Returns "?" for unknown / structural / control IDs.
fn obj_name(id: u8) -> &'static str {
    match id {
        // Ground / stompable
        0x29 => "Spike", 0x2A => "Patooie", 0x2B => "GoombaShoe",
        0x2D => "ChainChomp", 0x3D => "ChainChompStake", 0x4F => "ChainChompFree",
        0x33 => "Nipper", 0x39 => "NipperHopping", 0x40 => "BusterBeetle",
        0x55 => "BobOmb", 0x58 => "FireChomp", 0x59 => "FireSnake",
        0x6B => "PileDriver", 0x72 => "Goomba", 0x7C => "BigGoomba",
        0x3F => "DryBones",
        // Shell
        0x6C => "GreenTroopa", 0x6D => "RedTroopa", 0x70 => "BuzzyBeetle",
        0x71 => "Spiny", 0x7A => "BigGreenTroopa", 0x7B => "BigRedTroopa",
        // Flying
        0x46 => "Lakitu(LevelPlaced)",
        0x6E => "ParaTroopaGreenHop", 0x6F => "FlyingRedParaTroopa",
        0x73 => "ParaGoomba", 0x74 => "ParaGoombaMicros",
        0x7E => "BigGreenHopper", 0x80 => "FlyingGreenParaTroopa",
        // Water
        0x48 => "BabyBlooper", 0x61 => "BlooperWithKids", 0x62 => "Blooper",
        0x63 => "BigBertha", 0x64 => "CheepHopper", 0x67 => "LavaLotus",
        0x6A => "BlooperChildShoot", 0x76 => "GreenCheep(jumping)",
        0x77 => "RedCheep", 0x88 => "OrangeCheep",
        // Bros
        0x81 => "HammerBro", 0x82 => "BoomerangBro",
        0x86 => "HeavyBro", 0x87 => "FireBro",
        // Piranhas
        0x7D => "BigGreenPiranha", 0x7F => "BigRedPiranha",
        0xA0 => "GreenPiranha", 0xA1 => "GreenPiranhaFlipped",
        0xA2 => "RedPiranha", 0xA3 => "RedPiranhaFlipped",
        0xA4 => "GreenPiranhaFire", 0xA5 => "GreenPiranhaFireC",
        0xA6 => "VenusFireTrap", 0xA7 => "VenusFireTrapCeil",
        // Thwomps
        0x8A => "Thwomp", 0x8B => "ThwompLeftSlide", 0x8C => "ThwompRightSlide",
        0x8D => "ThwompUpDown", 0x8E => "ThwompDiagonalUL", 0x8F => "ThwompDiagonalDL",
        // Ghosts / hotfoot
        0x2F => "Boo", 0x30 => "HotFootShy", 0x45 => "HotFoot",
        // Rotodiscs / podoboo
        0x51 => "Rotodisc", 0x53 => "CeilingPodoboo", 0x9E => "Podoboo",
        0x5A => "RotodiscCW", 0x5B => "RotodiscCCW", 0x5E => "RotodiscDualOpposedH",
        0x5F => "RotodiscDualOpposedV", 0x60 => "RotodiscDualCCWSync",
        // Cannon fire (NOCHANGE CHR)
        0xBC => "CannonFire_BC", 0xBD => "CannonFire_BD",
        0xBE => "CannonFire_BE", 0xBF => "CannonFire_BF",
        0xC0 => "CannonFire_C0", 0xC1 => "CannonFire_C1",
        0xC2 => "CannonFire_C2", 0xC3 => "CannonFire_C3",
        0xC4 => "CannonFire_C4", 0xC5 => "CannonFire_C5",
        0xC6 => "CannonFire_C6", 0xC7 => "CannonFire_C7",
        0xC8 => "CannonFire_C8", 0xC9 => "CannonFire_C9",
        0xCA => "CannonFire_CA", 0xCB => "CannonFire_CB",
        0xCC => "CannonFire_CC", 0xCD => "CannonFire_CD",
        0xCE => "CannonFire_CE", 0xCF => "CannonFire_CF", 0xD0 => "CannonFireLaser",
        // Bosses + wild injection
        0x0E => "Koopaling", 0x18 => "Bowser",
        0x4A => "BoomBoomQBall", 0x4B => "BoomBoomJump", 0x4C => "BoomBoomFly",
        0x83 => "Lakitu", 0xAF => "AngrySun",
        // Structural / control
        0x25 => "PipeWayController", 0x34 => "Toad", 0x41 => "EndLevelCard",
        0x47 => "GiantBlockCtl", 0x50 => "BobOmbExplode", 0x52 => "TreasureBox",
        0x75 => "BossAttack(fireball)", 0x84 => "SpinyEgg", 0x85 => "SpinyEggDud",
        0xAD => "RockyWrench", 0xAE => "BoltLift", 0xB1 => "FireJetRight",
        0x3C => "WoodenPlatformFall", 0xD3 => "Autoscroll",
        0xB4 => "CheepCheepBeginEvent", 0xB5 => "GreenCheepBeginEvent",
        0xB6 => "LakituFleeEvent",
        // Powerups
        0x0B => "PowerUp1Up", 0x0C => "PowerUpStarman", 0x0D => "PowerUpMushroom",
        0x19 => "PowerUpFireFlower", 0x1E => "PowerUpSuperLeaf", 0x1F => "GrowingVine",
        _ => "?",
    }
}

/// Walk obj_id byte slots in the enemy data block, yielding
/// `(file_offset, obj_id)` for each entry. Honors autoscroll spoiled
/// ranges the same way `segment_writer::walk_segments` does so injection
/// false-positives from autoscroll-clobbered bytes don't show up.
fn for_each_obj_offset<F: FnMut(usize, u8)>(rom_data: &[u8], mut f: F) {
    let mut i = ENEMY_DATA_START;
    let in_spoiled = |idx: usize| -> Option<usize> {
        SPOILED_SEGMENT_RANGES
            .iter()
            .find(|r| r.contains(&idx))
            .map(|r| r.end)
    };
    while i < ENEMY_DATA_END {
        if let Some(end) = in_spoiled(i) { i = end; continue; }
        if rom_data[i] == 0xFF { i += 1; continue; }
        i += 1; // skip page byte
        while i + 2 < ENEMY_DATA_END && rom_data[i] != 0xFF && in_spoiled(i).is_none() {
            f(i, rom_data[i]);
            i += 3;
        }
    }
}

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
    let path = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";
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

    // Authoritative set of level enemy_ptrs — what `inject_at_entry_points`
    // uses to gate where it writes. For visibility scoring we accept either
    // `pos == ep` (entries-only header form) or `pos - 1 == ep` (page-byte
    // form where ep points at the 0x00/0x01 page byte and the first entry
    // is at ep+1).
    let entry_ptrs: HashSet<u16> = enemy_entry_points(&rom).into_iter().collect();
    let pos_visible = |pos: usize| -> bool {
        (pos as u16) <= u16::MAX
            && (entry_ptrs.contains(&(pos as u16))
                || (pos > 0 && entry_ptrs.contains(&((pos - 1) as u16))))
    };

    // Wild-injection visibility counters (across all NUM_SEEDS seeds).
    let mut inj_visible: u64 = 0;
    let mut inj_displaced: u64 = 0;
    // Vanilla bytes at every obj_id slot — needed to filter for actual
    // post-injection appearances of Lakitu/AngrySun (vs. vanilla already
    // having one there, which doesn't count as an injection event).
    let mut vanilla_at_slot: std::collections::HashMap<usize, u8> =
        std::collections::HashMap::new();
    for_each_obj_offset(&rom.data, |off, id| { vanilla_at_slot.insert(off, id); });

    let mut rando = ScanStats::default();
    for seed in 0..NUM_SEEDS {
        let mut rom_copy = rom.clone();
        randomizer::randomize(&mut rom_copy, seed, &opts);
        let s = scan(rom_copy.read_range(ENEMY_DATA_START, ENEMY_DATA_END - ENEMY_DATA_START));
        for (&page, &count) in &s.slot4 { *rando.slot4.entry(page).or_insert(0) += count; }
        for (&page, &count) in &s.slot5 { *rando.slot5.entry(page).or_insert(0) += count; }
        for (&id, &count) in &s.ids { *rando.ids.entry(id).or_insert(0) += count; }
        rando.total_segments += s.total_segments;

        // Walk this seed's patched ROM for injection events.
        for_each_obj_offset(&rom_copy.data, |off, id| {
            if !INJECTION_ONLY_IDS.contains(&id) { return; }
            if vanilla_at_slot.get(&off) == Some(&id) { return; } // unchanged → not an injection
            if pos_visible(off) { inj_visible += 1; } else { inj_displaced += 1; }
        });
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
    println!("=== WILD POOL DISTRIBUTION (rando, total across {NUM_SEEDS} seeds) ===\n");
    let total_entries: u64 = rando.ids.values().sum();
    // Build the set of wild-pool members for this Options config. With
    // max_enemy_opts() every class is Wild, so this is the union of all
    // class pools (minus cfire per the build_wild_pool workaround).
    let wild_pool = wild_pool_for(&opts);
    let wild_set: std::collections::BTreeSet<u8> = wild_pool.iter().copied().collect();
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
    for id in &wild_sorted {
        let count = rando.ids.get(id).copied().unwrap_or(0);
        let avg = count as f64 / NUM_SEEDS as f64;
        let pct = count as f64 / total_entries as f64 * 100.0;
        let vcount = *vanilla.ids.get(id).unwrap_or(&0);
        let name = obj_name(*id);
        let marker = if count == 0 { " *" } else { "  " };
        println!("  0x{id:02X}{marker}| {name:<26} | {count:>6} | {avg:>8.1} | {pct:>7.1}% | {vcount:>6}");
        if count == 0 { zero_count += 1; }
    }
    println!("\n  Wild-pool size: {} ids   ({zero_count} dropped to zero, marked *)", wild_sorted.len());

    // Also show any non-wild-pool IDs in the top 10 for context (these are
    // structural/fixed: EndLevelCard, Boom-Boom variants, etc — not subject
    // to the picker, but useful anchors when reading the wild-pool numbers).
    let mut non_wild: Vec<(&u8, &u64)> = rando.ids.iter()
        .filter(|(id, _)| !wild_set.contains(id))
        .collect();
    non_wild.sort_by(|a, b| b.1.cmp(a.1));
    println!("\n  ── Top 10 non-wild-pool IDs (structural / fixed objects, picker doesn't touch) ──");
    for &(&id, &count) in non_wild.iter().take(10) {
        let avg = count as f64 / NUM_SEEDS as f64;
        let pct = count as f64 / total_entries as f64 * 100.0;
        let vcount = *vanilla.ids.get(&id).unwrap_or(&0);
        let name = obj_name(id);
        println!("  0x{id:02X}  | {name:<26} | {count:>6} | {avg:>8.1} | {pct:>7.1}% | {vcount:>6}");
    }

    println!("\n============================================================");
    println!("=== WILD INJECTION VISIBILITY ({NUM_SEEDS} seeds) ===\n");
    let inj_total = inj_visible + inj_displaced;
    let visibility_pct = if inj_total > 0 {
        inj_visible as f64 / inj_total as f64 * 100.0
    } else { 0.0 };
    println!("Lakitu (0x83) + AngrySun (0xAF) occurrences at obj_id slots,");
    println!("excluding positions where vanilla already had that ID:");
    println!("  At known level entry_ptr : {inj_visible:>5}  ({visibility_pct:>5.1}%)  ← visible in-game");
    println!("  Not at entry_ptr         : {inj_displaced:>5}  ({:>5.1}%)  ← orphan / displaced", 100.0 - visibility_pct);
    println!("  Total injection events   : {inj_total:>5}");
    println!("  Avg per seed             : {:.1}", inj_total as f64 / NUM_SEEDS as f64);
    println!();
    println!("Post-entry_ptr refactor (0.5.12-beta.5) this should be ≈100% visible.");
    println!("Pre-refactor baseline was ~3% — most injections landed on orphan-prefix bytes.");
}
