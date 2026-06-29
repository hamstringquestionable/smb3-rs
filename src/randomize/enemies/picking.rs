//! Enemy-pick mechanics: CHR-compatible random selection, the per-segment
//! page buckets, and the position-correcting swap writer.

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
/// - Tall replacements get Y−1 to avoid floor clipping.
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

/// Pre-built page buckets for page-first picking. Built once per segment,
/// reused for every Wild enemy in that segment.
pub(super) struct PageBuckets {
    /// Each entry is (slot, page, enemy_ids). No-bank enemies are appended to every bucket.
    buckets: Vec<Vec<u8>>,
}

impl PageBuckets {
    /// Build buckets from the wild pool. Groups enemies by (slot, chr_page);
    /// no-bank enemies are added to every bucket so they don't get their own.
    pub(super) fn build(pool: &[u8]) -> Self {
        let mut map: Vec<((u8, u8), Vec<u8>)> = Vec::new();
        let mut no_bank: Vec<u8> = Vec::new();
        for &id in pool {
            match sprite_bank(id) {
                Some(sb) => {
                    let key = (sb.slot, sb.chr_page);
                    if let Some(entry) = map.iter_mut().find(|(k, _)| *k == key) {
                        entry.1.push(id);
                    } else {
                        map.push((key, vec![id]));
                    }
                }
                None => no_bank.push(id),
            }
        }
        if !no_bank.is_empty() {
            for (_, bucket) in &mut map {
                bucket.extend_from_slice(&no_bank);
            }
        }
        PageBuckets { buckets: map.into_iter().map(|(_, v)| v).collect() }
    }

    /// Pick a CHR-compatible enemy uniformly from the union of all buckets.
    ///
    /// Previously this picked a bucket uniformly *then* a member uniformly,
    /// which gave any enemy alone in its (slot, chr_page) bucket a full
    /// 1/N_buckets share of all draws — e.g. LavaLotus and DryBones each
    /// occupied ~8% of the wild pool (chr_stats baseline at beta.5),
    /// dominating every Wild seed.
    ///
    /// Flattening compatible members across all buckets and picking
    /// uniformly gives each compatible enemy equal weight per draw.
    /// Singletons drop to ~1/N_pool share. CHR-popular pages still see
    /// more total picks (because more members live in them), but no one
    /// enemy can outsize the pool.
    pub(super) fn pick<R: Rng>(&self, slot4: ChrSlot, slot5: ChrSlot, rng: &mut R) -> Option<u8> {
        let candidates: Vec<u8> = self.buckets.iter()
            .flat_map(|b| b.iter().copied())
            .filter(|&id| is_chr_compatible(id, slot4, slot5))
            .collect();
        candidates.choose(rng).copied()
    }
}
