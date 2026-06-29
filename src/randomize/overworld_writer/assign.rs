//! Step 1 — assign pool entries to grid slots (the writer's decision pass).

use super::*;

/// Reorder HB level entries so unique `obj_ptr` values are evenly interleaved.
///
/// Without this, the cycling pool is dominated by entries sharing `obj_ptr`
/// 0xC640 (8 of 13 entries), causing most HB encounters to have identical
/// enemies. Interleaving ensures each unique enemy set appears once before
/// any repeats: round-robin through obj_ptr groups, picking a random layout
/// variant from each group per round.
pub(super) fn interleave_hb_by_obj_ptr<R: Rng>(
    levels: Vec<rom_data::LevelEntry>,
    rng: &mut R,
) -> Vec<rom_data::LevelEntry> {
    if levels.is_empty() {
        return levels;
    }

    // Group by obj_ptr using BTreeMap for deterministic iteration order.
    let mut groups: std::collections::BTreeMap<u16, Vec<rom_data::LevelEntry>> =
        std::collections::BTreeMap::new();
    for le in levels {
        let obj = u16::from_le_bytes([le.obj_lo, le.obj_hi]);
        groups.entry(obj).or_default().push(le);
    }

    // Shuffle within each group and collect group keys in random order.
    let mut keys: Vec<u16> = groups.keys().copied().collect();
    keys.as_mut_slice().shuffle(rng);
    for group in groups.values_mut() {
        group.as_mut_slice().shuffle(rng);
    }

    // Round-robin: pick one from each group per round until all exhausted.
    let max_len = groups.values().map(|g| g.len()).max().unwrap_or(0);
    let mut result = Vec::new();
    for round in 0..max_len {
        for &key in &keys {
            let group = groups.get(&key).unwrap();
            if round < group.len() {
                result.push(group[round].clone());
            }
        }
    }

    result
}

pub(super) fn assign_pool<R: Rng>(
    rom: &Rom,
    build: &BuildResult,
    data: &OverworldData,
    rng: &mut R,
    flags: WriteFlags,
) -> Vec<WorldAssignments> {
    let WriteFlags { cross_world, shuffle_hammer_bros } = flags;
    let pickup = data.pickup;
    let catalog = data.catalog;
    // Partition pool by kind.
    let mut fort_pool: Vec<usize> = Vec::new();
    let mut level_pool: Vec<usize> = Vec::new();
    let mut airship_pool: Vec<usize> = Vec::new();
    let mut bonus_pool: Vec<usize> = Vec::new();
    let mut toad_pool: Vec<usize> = Vec::new();
    let mut bowser_idx: Option<usize> = None;
    // Pipe groups: world → dest_idx → Vec<(pool_idx, is_a_side)>.
    let mut pipe_groups: HashMap<usize, HashMap<usize, Vec<(usize, bool)>>> = HashMap::new();
    for (pi, pe) in pickup.pool.iter().enumerate() {
        let entry = &catalog.entries[pe.catalog_idx];
        match &entry.kind {
            NodeKind::Fortress { .. } => fort_pool.push(pi),
            NodeKind::Level => level_pool.push(pi),
            NodeKind::Airship => airship_pool.push(pi),
            NodeKind::Bowser => {
                debug_assert!(bowser_idx.is_none(), "duplicate Bowser in pickup pool");
                bowser_idx = Some(pi);
            }
            NodeKind::Pipe { dest_idx, is_a_side } => {
                pipe_groups
                    .entry(entry.world_idx)
                    .or_default()
                    .entry(*dest_idx)
                    .or_default()
                    .push((pi, *is_a_side));
            }
            NodeKind::BonusGame => bonus_pool.push(pi),
            NodeKind::ToadHouse => toad_pool.push(pi),
            _ => {} // HammerBro entries don't need a pool — see HB assignment below
        }
    }
    bonus_pool.as_mut_slice().shuffle(rng);
    let mut bonus_iter = bonus_pool.into_iter();

    // Build cycling hammer bro level pool, interleaved by obj_ptr so each
    // unique enemy set appears once before any repeats.
    let hb_levels = interleave_hb_by_obj_ptr(catalog.unique_hammer_bro_levels(), rng);
    let mut hb_level_iter = hb_levels.iter().cycle().cloned();

    // Build per-obj_ptr groups for sprite position round-robin assignment.
    // This ensures each HB sprite encounter in a world gets a different
    // enemy set (different obj_ptr = different enemies).
    let mut hb_obj_groups: std::collections::BTreeMap<u16, Vec<rom_data::LevelEntry>> =
        std::collections::BTreeMap::new();
    for le in &hb_levels {
        let obj = u16::from_le_bytes([le.obj_lo, le.obj_hi]);
        hb_obj_groups.entry(obj).or_default().push(le.clone());
    }
    let mut hb_group_keys: Vec<u16> = hb_obj_groups.keys().copied().collect();
    hb_group_keys.as_mut_slice().shuffle(rng);
    for group in hb_obj_groups.values_mut() {
        group.as_mut_slice().shuffle(rng);
    }

    // --- Pre-assign the 1-F fortress to a secret-exit-safe slot ---
    //
    // The 1-F fortress level has a secret exit that bypasses Boom-Boom
    // (no crystal ball → no FX trigger → lock stays closed). It must
    // land in a slot whose lock is marked secret_exit_safe to avoid
    // softlocking the player.

    // Find the 1-F pool entry.
    let fort_1f_pos = fort_pool.iter().position(|&pi| {
        let ce = &catalog.entries[pickup.pool[pi].catalog_idx];
        ce.level_entry.as_ref().is_some_and(|le| {
            u16::from_le_bytes([le.obj_lo, le.obj_hi]) == FORTRESS_1F_OBJ_PTR
        })
    }).expect("1-F fortress not found in pool");
    let fort_1f_pi = fort_pool.remove(fort_1f_pos);

    // Collect all safe (world_idx, section) slots. In intra-world mode,
    // 1-F can only go to a safe slot in its origin world.
    let fort_1f_origin = catalog.entries[pickup.pool[fort_1f_pi].catalog_idx].world_idx;
    let mut safe_slots: Vec<(usize, usize)> = Vec::new();
    for wi in 0..8 {
        if !cross_world && wi != fort_1f_origin {
            continue;
        }
        for lock in &build.worlds[wi].locks {
            if lock.secret_exit_safe {
                safe_slots.push((wi, lock.fort_section));
            }
        }
    }
    // Pre-assign 1-F to a safe slot if one exists. In intra-world mode,
    // W1 may have no safe lock — that's fine, the player must use the
    // normal exit (beat Boom-Boom) to open the lock.
    let mut preassigned_forts: HashMap<(usize, usize), usize> = HashMap::new();
    if let Some(&(safe_wi, safe_section)) = safe_slots.choose(rng) {
        preassigned_forts.insert((safe_wi, safe_section), fort_1f_pi);
    } else {
        // No safe slot available — return 1-F to the regular pool.
        fort_pool.push(fort_1f_pi);
    }

    // Shuffle remaining fortress and level pools.
    fort_pool.as_mut_slice().shuffle(rng);
    level_pool.as_mut_slice().shuffle(rng);
    airship_pool.as_mut_slice().shuffle(rng);
    // Toad House pool shuffled here (after level_pool) so adding this
    // shuffle doesn't shift the level pool's RNG sequence and break tests
    // that depend on specific level assignments per seed.
    toad_pool.as_mut_slice().shuffle(rng);
    let mut toad_iter = toad_pool.into_iter();

    // For intra-world mode, partition fort/level pools by origin world.
    let mut fort_by_world: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut level_by_world: HashMap<usize, Vec<usize>> = HashMap::new();
    if !cross_world {
        for &pi in &fort_pool {
            let wi = catalog.entries[pickup.pool[pi].catalog_idx].world_idx;
            fort_by_world.entry(wi).or_default().push(pi);
        }
        for &pi in &level_pool {
            let wi = catalog.entries[pickup.pool[pi].catalog_idx].world_idx;
            level_by_world.entry(wi).or_default().push(pi);
        }
    }

    let mut fort_iter = fort_pool.into_iter();
    let mut level_pool: VecDeque<usize> = level_pool.into();

    // Troll pipes don't clear when beaten, so a slot stamped as a troll pipe
    // can be replayed infinitely. We exclude two families of levels from the
    // troll-pipe assignment pool:
    //
    //  - W8 Hand levels (8-Hnd1/2/3): short bonus rooms that drop items, so
    //    re-entering the pipe would let the player farm items.
    //
    //  - Chest levels (rom_data::CHEST_LEVELS): the player needs to find these
    //    levels to collect the inventory item. Disguising them as pipes hides
    //    them from players who skip pipe-look tiles. Includes 3-7 (Cloud),
    //    5-1 (Music Box), 8-Tank (Star). 1F is also in the list but is a
    //    fortress, never a regular-level slot.
    let is_troll_pipe_ineligible = |pi: usize| -> bool {
        let ce = &catalog.entries[pickup.pool[pi].catalog_idx];
        (ce.world_idx == 7 && matches!(ce.entry_idx, 14..=16))
            || rom_data::is_chest_level(ce.world_idx, ce.entry_idx)
    };

    let mut assignments: Vec<WorldAssignments> = Vec::with_capacity(8);

    for wi in 0..8 {
        let built = &build.worlds[wi];

        // --- Fortress assignments (ordered by section for FX) ---
        let mut fortress = Vec::new();
        for section in 0..built.section_count {
            if let Some(slot) = built.slots.iter().find(|s| {
                s.kind == SlotKind::Fortress && s.section == section
            }) {
                // Check if this slot was pre-assigned (1-F safe placement).
                let pi = if let Some(pre) = preassigned_forts.remove(&(wi, section)) {
                    pre
                } else if cross_world {
                    fort_iter.next().expect("fortress pool exhausted")
                } else {
                    fort_by_world
                        .get_mut(&wi)
                        .and_then(|v| v.pop())
                        .expect("intra-world fortress pool exhausted")
                };
                fortress.push(Assignment { pool_idx: pi, pos: slot.pos });
            }
        }

        // --- Level assignments ---
        // Process troll-pipe slots before regular ones so the non-hand-level
        // constraint (troll pipes must NOT be hand levels — those are reserved
        // for the levels they front for) can always be satisfied while
        // non-hand entries remain in the pool. Iterating in `built.slots`
        // order would let regular slots drain non-hand levels first and then
        // strand troll pipes with only hand levels left.
        //
        // If even processing troll-pipe slots first can't find a non-hand
        // entry (pool genuinely under-supplies non-hand levels for the number
        // of troll pipes marked), demote the slot to a regular level tile
        // and track it in `demoted_troll_pipes`. The tile-stamping step
        // consults that set so a demoted slot shows as a level icon rather
        // than a pipe leading to the hand-trap behind it.
        let mut level = Vec::new();
        let mut demoted_troll_pipes: HashSet<(usize, usize)> = HashSet::new();
        let level_slots: Vec<&_> = built.slots.iter()
            .filter(|s| s.kind == SlotKind::Level)
            .collect();
        let mut ordered: Vec<&_> = level_slots.iter().copied()
            .filter(|s| s.is_troll_pipe)
            .collect();
        ordered.extend(level_slots.iter().copied().filter(|s| !s.is_troll_pipe));

        for slot in ordered {
            let pi = if cross_world {
                if slot.is_troll_pipe {
                    if let Some(pos) = level_pool.iter().position(|&pi| !is_troll_pipe_ineligible(pi)) {
                        level_pool.remove(pos).unwrap()
                    } else {
                        demoted_troll_pipes.insert(slot.pos);
                        level_pool.pop_front().expect("level pool exhausted")
                    }
                } else {
                    level_pool.pop_front().expect("level pool exhausted")
                }
            } else {
                let v = level_by_world
                    .get_mut(&wi)
                    .expect("intra-world level pool missing");
                if slot.is_troll_pipe {
                    if let Some(idx) = v.iter().rposition(|&pi| !is_troll_pipe_ineligible(pi)) {
                        v.remove(idx)
                    } else {
                        demoted_troll_pipes.insert(slot.pos);
                        v.pop().expect("intra-world level pool exhausted")
                    }
                } else {
                    v.pop().expect("intra-world level pool exhausted")
                }
            };
            level.push(Assignment { pool_idx: pi, pos: slot.pos });
        }

        // --- Pipe assignments ---
        // Each dest_idx has two pool entries: the A-side (left pipe in transit
        // level, layout byte5 bit 6 = 0) and the B-side (right pipe, bit 6 = 1).
        // The dest table upper nibble = A position, lower = B position.  The
        // game picks the nibble based on Mario's exit side in the transit level,
        // so pool_idx_a/pos_a must be the A-side entry or the pipe self-references.
        let mut pipes = Vec::new();
        if let Some(world_pipes) = pipe_groups.get_mut(&wi) {
            let mut groups: Vec<(usize, Vec<(usize, bool)>)> = world_pipes.drain().collect();
            groups.sort_by_key(|(dest_idx, _)| *dest_idx);
            groups.as_mut_slice().shuffle(rng);

            for (pair_idx, (dest_idx, group)) in groups.into_iter().enumerate() {
                if pair_idx >= built.pipe_pairs.len() || group.len() < 2 {
                    break;
                }
                let (pos_a, pos_b) = built.pipe_pairs[pair_idx];

                // Use the is_a_side flag precomputed during catalog building.
                let (idx_a, idx_b) = if group[0].1 {
                    (group[0].0, group[1].0)
                } else {
                    (group[1].0, group[0].0)
                };
                pipes.push(PipeAssignment {
                    pool_idx_a: idx_a,
                    pool_idx_b: idx_b,
                    dest_idx,
                    pos_a,
                    pos_b,
                });
            }
        }

        // --- Airship (W1-W7) ---
        let airship = if wi < 7 {
            let airship_pos = catalog.entries.iter()
                .find(|e| e.world_idx == wi && matches!(e.kind, NodeKind::Airship))
                .map(|e| e.grid_pos);
            airship_pos.and_then(|pos| {
                airship_pool.pop().map(|pi| Assignment { pool_idx: pi, pos })
            })
        } else {
            None
        };

        // --- Bowser (W8 only) ---
        let bowser = if wi == 7 {
            bowser_idx.map(|pi| {
                let pos = catalog.entries[pickup.pool[pi].catalog_idx].grid_pos;
                Assignment { pool_idx: pi, pos }
            })
        } else {
            None
        };

        // --- Bonus game (spade) assignments ---
        //
        // Each SlotKind::BonusGame position gets a picked-up BonusGame pool
        // entry. All BonusGame entries are functionally identical (obj=$0001,
        // lay=$0000), so any pool entry works for any slot.
        let mut bonus = Vec::new();
        for slot in &built.slots {
            if slot.kind != SlotKind::BonusGame {
                continue;
            }
            match bonus_iter.next() {
                Some(pi) => bonus.push(Assignment { pool_idx: pi, pos: slot.pos }),
                None => break, // pool exhausted (shouldn't happen — budget is capped)
            }
        }

        // --- Toad House assignments ---
        //
        // Each SlotKind::ToadHouse position gets a picked-up ToadHouse pool
        // entry. Each entry carries its vanilla obj_ptr (one of 7 reward
        // variants), so write_pointer_entries preserves reward identity by
        // routing through the per-entry rom_data::write_entry path.
        let mut toad = Vec::new();
        for slot in &built.slots {
            if slot.kind != SlotKind::ToadHouse {
                continue;
            }
            match toad_iter.next() {
                Some(pi) => toad.push(Assignment { pool_idx: pi, pos: slot.pos }),
                None => break, // pool exhausted (shouldn't happen — pool drains globally)
            }
        }

        // --- Hammer bro assignments (remaining blank slots) ---
        //
        // Every SlotKind::HammerBro position gets a cycling HB level, up to
        // the remaining pointer table capacity after level-like assignments.
        //
        // Sprite positions (actual encounters the player fights) get a
        // dedicated per-obj_ptr round-robin so each encounter in a world
        // has a different enemy set. Filler positions (blank tiles needing
        // valid pointer entries) use the normal cycling pool.
        let level_like_count = fortress.len() + level.len() + pipes.len() * 2 + bonus.len() + toad.len();
        let remaining_slots = pickup.worlds[wi].pool_indices.len().saturating_sub(level_like_count);

        // When Hammer Bros are redistributed, the sprite positions are the new
        // ones decided in the build phase; otherwise they're the vanilla ROM
        // positions. Either way these slots get the per-obj_ptr variety pass.
        let sprite_positions: HashSet<(usize, usize)> = if shuffle_hammer_bros {
            built.hb_sprites.iter().map(|s| s.grid_pos).collect()
        } else {
            rom_data::read_hb_sprite_positions(rom, wi).into_iter().collect()
        };

        let mut sprite_slots = Vec::new();
        let mut filler_slots = Vec::new();
        for slot in &built.slots {
            if slot.kind != SlotKind::HammerBro { continue; }
            if sprite_positions.contains(&slot.pos) {
                sprite_slots.push(slot.pos);
            } else {
                filler_slots.push(slot.pos);
            }
        }

        // Assign sprite slots from per-obj_ptr round-robin.
        let mut hammer_bro = Vec::new();
        for (sprite_obj_idx, pos) in sprite_slots.iter().enumerate() {
            if hammer_bro.len() >= remaining_slots { break; }
            let key = hb_group_keys[sprite_obj_idx % hb_group_keys.len()];
            let group = hb_obj_groups.get(&key).unwrap();
            let le = group[sprite_obj_idx / hb_group_keys.len() % group.len()].clone();
            hammer_bro.push(HammerBroAssignment { pos: *pos, level_entry: le });
        }

        // Assign filler slots from normal cycling pool.
        for pos in &filler_slots {
            if hammer_bro.len() >= remaining_slots { break; }
            hammer_bro.push(HammerBroAssignment {
                pos: *pos,
                level_entry: hb_level_iter.next().unwrap(),
            });
        }

        assignments.push(WorldAssignments {
            fortress,
            level,
            pipes,
            airship,
            bowser,
            bonus,
            toad,
            hammer_bro,
            demoted_troll_pipes,
        });
    }

    assignments
}
