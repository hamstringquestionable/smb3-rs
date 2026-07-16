# Wild-Injection Rework

Level-centric redesign of the wild-injection pass (`enemies/injection.rs`),
replacing the old raw-enemy-pointer approach.

## What it does

For each seed, injects a level-wide chaser — Lakitu (`0x83`) or Angry Sun
(`0xAF`) — into a random subset of real action levels, replacing each chosen
level's first enemy with a CHR-compatible chaser.

**Boss Bass (`0x2D`) is excluded** from the pool: it's a `WATER_ENEMIES` member,
so the later walker pass would reshuffle an injected one into an ordinary water
enemy. Lakitu and Sun are in no class pool, so the walker leaves them alone.

## Why it was reworked

The old pass drove off `enemy_entry_points` (raw header pointers), which:

- targeted a frozen set of ~32 levels every seed (eligibility computed on
  pre-shuffle vanilla first-enemies);
- hit shared / nested / mid-segment enemy pointers, so one injection could
  land in two levels (e.g. `3-1` & `7-2` share an enemy set);
- could not reliably exclude boss rooms — a fortress boss lives in a *different*
  `0xFF` segment than the injection target, so a per-segment `0x4B/0x4C` scan
  never saw it (`4F2`, `7F2` slipped through);
- had **no guard against doubling** — a second Angry Sun could stack onto
  2-Quicksand's existing sun and break the level;
- used the raw pointer as a direct file index, which is **0x10 off** from the
  `enemy_ptr_to_file_offset` frame the rest of the codebase uses — writing to
  the wrong location.

## Design

Driven by `node_catalog`, which classifies every entry (`NodeKind::Level`,
`Fortress`, `Airship`, `Bowser`, …) and carries each level's `obj_ptr`.

```
collect_candidates(rom, data, opts):
    for entry in NodeCatalog::build(rom, include_beta_stages):
        keep only NodeKind::Level                       # boss types excluded here
        obj_ptr = entry.level_entry.obj_ptr
        first_idx = enemy_ptr_to_file_offset(obj_ptr) - ENEMY_DATA_START (+page byte)
        de-dupe by first_idx                            # shared enemy set = one candidate

inject_wild_chasers:
    shuffle(candidates)                                 # per-seed variety
    for cand in candidates:
        roll WILD_INJECTION_CHANCE
        require first enemy swappable + unprotected      # don't clobber critical
                                                         # objects / get reverted
        CHR pin-scan the enclosing $FF segment
        pick a chaser that is CHR-compatible, the level
          does NOT already have (has_enemy_id), and
          within the Big-Bertha per-segment cap
        replace first enemy; re-seed suns to screen 0 (0x02, 0x11)
```

### Guards
- **Boss exclusion** — by `NodeKind`, not enemy bytes. Reliable.
- **No-double** — `has_enemy_id(rom, obj_ptr, chosen)` skips any chaser the level
  already contains (the 2-Quicksand fix).
- **Shared-pointer de-dup** — one physical enemy set injects at most once.
- **Swappable + unprotected first enemy** — `find_class_pool` guards against
  destroying a non-enemy object; `entry_protection_at` avoids walker reverts.
- **CHR compatibility** — unchanged; the chaser's sprite page must fit the
  segment's committed slots.
- **Bertha cap** — `MAX_BERTHA_PER_SEGMENT`.

### Frame
Everything uses `obj_ptr` + `enemy_ptr_to_file_offset` (the same frame as
`has_enemy_id`), verified against 1-1 (obj `0xC527` → first enemy at file
`0xC538`).

### Sun placement
Kept as-is: injected suns are re-seeded to the vanilla 2-Quicksand spawn
(screen 0, `Y=0x11`). Confirmed necessary — deep suns idle in the background.

## Retained / removed
- `enemy_entry_points` retained (used by the `chr_stats` integration test).
- Removed: the old `inject_at_entry_points`, the per-segment `0x4B/0x4C` Boom-Boom
  scan, and `is_injection_blocked` (its only consumer was the old pass).

## Tests
- `wild_injection_rework_guarantees` (real ROM, 30 seeds): injections occur,
  every injected sun is on screen 0, no fortress/airship/Bowser receives a
  chaser, no level is doubled.
- `enemy_invariant_baseline` — its `injectable_offsets` oracle now mirrors the
  catalog-based candidate set.

## Known limitations / future work
- **Variety** comes from random selection over a fixed candidate *set* (level
  main areas with a swappable first enemy). The set is the same each seed; only
  the chosen subset varies. Making the set itself vary would require running
  injection after the enemy shuffle.
- Only the level's **main area** first enemy is targeted (no sub-area injection).
- **Rate** (`WILD_INJECTION_CHANCE = 102`, ~40%) now applies per candidate
  level; may warrant re-measuring against the (larger, cleaner) pool.
