//! Enemy-pick mechanics: CHR-compatible random selection, category-equal
//! bucket picking, the per-entry replacement decision, and the
//! position-correcting swap writer.

use super::*;

/// Y offset (rows) that seats a piranha-pool member correctly relative to the
/// piranha "reference" position. Piranhas and Rocky Wrench sit at the reference
/// (0); fire jets self-position differently, so they carry an offset: the upward
/// jet sits `FIREJET_UP_Y_RISE` rows higher (−), the downward jet
/// `FIREJET_DOWN_Y_DROP` rows lower (+). See `swap_enemy`.
pub(super) fn piranha_pool_y_offset(id: u8) -> i8 {
    match id {
        FIREJET_UP => -(FIREJET_UP_Y_RISE as i8),
        FIREJET_DOWN => FIREJET_DOWN_Y_DROP as i8,
        _ => 0,
    }
}

/// Write `new_id` into the enemy slot at `id_index` and nudge Y so the
/// replacement lines up with the slot. Bundles the write + adjustment so call
/// sites can't forget one. Adjustments:
/// - Tall replacements get Y−1 to avoid floor clipping. Note this keys off the
///   *new* id only — a tall→tall swap (e.g. bro↔bro) also rises one row,
///   unlike the delta-based jet shift below.
/// - Fire jets self-position differently than a piranha/wrench, so Y shifts by
///   `offset(new) − offset(old)` (see `piranha_pool_y_offset`). This is
///   symmetric: a jet replacing a piranha/wrench rises/drops, and a piranha or
///   wrench replacing a jet gets the exact reverse. Non-jet ↔ non-jet swaps
///   (piranha↔piranha, piranha↔wrench, and every other class) shift nothing.
pub(super) fn swap_enemy(data: &mut [u8], id_index: usize, new_id: u8) {
    let old_id = data[id_index];
    data[id_index] = new_id;
    if TALL_ENEMIES.contains(&new_id) {
        data[id_index + 2] = data[id_index + 2].wrapping_sub(1);
    }
    let dy = piranha_pool_y_offset(new_id) - piranha_pool_y_offset(old_id);
    data[id_index + 2] = data[id_index + 2].wrapping_add_signed(dy);
}

/// Pick a random CHR-compatible enemy from `pool`, or `None` if nothing fits.
pub(super) fn pick_compatible<R: Rng>(
    pool: &[u8], slot4: ChrSlot, slot5: ChrSlot, rng: &mut R,
) -> Option<u8> {
    let compatible: Vec<u8> = pool
        .iter()
        .copied()
        .filter(|&c| is_chr_compatible(c, slot4, slot5))
        .collect();
    compatible.choose(rng).copied()
}

/// Bucket-first pick: weight each bucket equally rather than each member. Choose
/// uniformly among the buckets that have ≥1 CHR-compatible member, then a member
/// uniformly from that bucket. Gives a lone-member bucket (e.g. a single flame or
/// wrench) a full category share instead of `1/N_pool`. CHR is handled naturally:
/// a bucket with no compatible member under the current slot commitments is
/// skipped for this draw. Returns `None` only if no bucket has any fit.
pub(super) fn pick_bucket_first<R: Rng>(
    buckets: &[&[u8]], slot4: ChrSlot, slot5: ChrSlot, rng: &mut R,
) -> Option<u8> {
    let eligible: Vec<&[u8]> = buckets
        .iter()
        .copied()
        .filter(|b| b.iter().any(|&id| is_chr_compatible(id, slot4, slot5)))
        .collect();
    let &bucket = eligible.choose(rng)?;
    pick_compatible(bucket, slot4, slot5, rng)
}

/// Decide the replacement enemy for one swappable walker entry, or `None` to
/// leave it unchanged. Callers handle Big ? blocks, Boom-Booms, and SkipSwap
/// before calling; bertha bookkeeping and the actual write stay with them too.
///
/// Every entry funnels through one shape: choose a base pool + a primary
/// (CHR-aware) pick, then filter through the placement constraints before
/// committing. Applying the constraints uniformly — instead of only in the
/// wild-swap branch — is what makes the bertha cap (and the giant-red /
/// piranha-hazard guards) cover the Force*/ExcludeHazards paths too.
pub(super) fn pick_replacement<R: Rng>(
    entry: &SegmentEntry,
    protection: Option<EntryProtection>,
    modes: &ClassModes,
    wild_pool: &[u8],
    (slot4, slot5): (ChrSlot, ChrSlot),
    cap_full: bool,
    rng: &mut R,
) -> Option<u8> {
    // Base pool + primary pick. A pool-replacing protection
    // (ForceShell/TankBro/Stompable/ExcludeHazards) chooses the pool;
    // otherwise it's the normal class pool, picked via the
    // wild/piranha/plain strategy. `None` => no swap for this entry.
    let picked: Option<(Option<u8>, Cow<[u8]>)> = match protection {
        Some(EntryProtection::ForceShell) if modes.shell != EnemyMode::Off => Some((
            pick_compatible(SHELL_ENEMIES, slot4, slot5, rng),
            Cow::Borrowed(SHELL_ENEMIES),
        )),
        Some(EntryProtection::ForceTankBro) if modes.bros != EnemyMode::Off => Some((
            pick_compatible(TANK_BRO_POOL, slot4, slot5, rng),
            Cow::Borrowed(TANK_BRO_POOL),
        )),
        Some(EntryProtection::ForceStompable) => {
            find_class_pool(entry.obj_id, modes).map(|pool| {
                let sp: Vec<u8> = pool.slice(wild_pool).iter().copied()
                    .filter(|id| STOMPABLE_ENEMIES.contains(id)).collect();
                let pick = pick_compatible(&sp, slot4, slot5, rng);
                (pick, Cow::Owned(sp))
            })
        }
        Some(EntryProtection::ExcludeHazards) => {
            find_class_pool(entry.obj_id, modes).map(|pool| {
                // Drop hazards, but keep any of the same category as the
                // vanilla enemy here (additive-only: don't strip a
                // designed-in hazard, only block introducing a new one).
                let fp: Vec<u8> = pool.slice(wild_pool).iter().copied()
                    .filter(|&id| !hazard_excluded(id, entry.obj_id)).collect();
                let pick = pick_compatible(&fp, slot4, slot5, rng);
                (pick, Cow::Owned(fp))
            })
        }
        _ => find_class_pool(entry.obj_id, modes).map(|pool| {
            let pick = match pool {
                ClassPool::Wild => pick_compatible(wild_pool, slot4, slot5, rng),
                ClassPool::PiranhaStd => {
                    // Category-equal: piranha / upward jet / wrench each
                    // get a uniform turn. Giant red (0x7F) only when this
                    // slot already held one (keep filter covers the rest).
                    let bucket: &[u8] = if entry.obj_id == GIANT_RED_PIRANHA {
                        PIRANHAS
                    } else {
                        PIRANHAS_NO_RED
                    };
                    pick_bucket_first(&[bucket, BUCKET_UP_JET, BUCKET_WRENCH],
                        slot4, slot5, rng)
                }
                ClassPool::PiranhaCeil => {
                    pick_bucket_first(&[PIRANHASC, BUCKET_DOWN_JET], slot4, slot5, rng)
                }
                ClassPool::Class(class) => pick_compatible(class, slot4, slot5, rng),
            };
            (pick, Cow::Borrowed(pool.slice(wild_pool)))
        }),
    };
    let (primary, base_pool) = picked?;

    // Placement constraints, applied to every pick. `keep(id)` is
    // true when `id` is allowed in this slot.
    let keep = |id: u8| -> bool {
        // Big Bertha cap: no new bertha once the segment is full.
        let over_cap = cap_full && BERTHA_IDS.contains(&id);
        // Giant red piranha (off-center hitbox) only where one was.
        let bad_giant = id == GIANT_RED_PIRANHA && entry.obj_id != GIANT_RED_PIRANHA;
        !(over_cap || bad_giant)
    };
    // (A piranha slot can't become a hazard: the piranha pools are
    // self-contained and contain none — verified by the harness's
    // piranha-hazard invariant, so no explicit guard is needed.)

    // Accept the primary pick if it satisfies every constraint;
    // otherwise re-pick once from the base pool filtered by all of
    // them, so the constraints compose instead of undoing each other.
    match primary {
        Some(id) if keep(id) => Some(id),
        _ => {
            let filtered: Vec<u8> =
                base_pool.iter().copied().filter(|&id| keep(id)).collect();
            pick_compatible(&filtered, slot4, slot5, rng)
        }
    }
}
