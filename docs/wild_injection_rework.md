# Wild-Injection Rework

Level-centric redesign of the wild-injection pass (`enemies/injection.rs`),
replacing the old raw-enemy-pointer approach.

## What it does

For each seed, injects a level-wide chaser — Lakitu (`0x83`), Angry Sun
(`0xAF`) or Boss Bass (`0x2D`) — into a random subset of real action levels,
replacing each chosen level's first enemy with a CHR-compatible chaser. The
player picks which of the three are allowed (`Vec<WildChaser>`; empty = off).

**Boss Bass was excluded until injection moved inside the walker.** It's a
`WATER_ENEMIES` member, so while injection ran as a pass *before* the walker,
the walker reshuffled an injected Bertha into an ordinary water enemy whenever
water was Shuffle/Wild — a silent no-op. Deciding the injection in the walker's
own segment prologue and marking the entry settled closes that: nothing after
it gets a chance to swap it. Lakitu and the Sun never needed the protection,
being in no class pool.

## Where it runs

Not a pass. `collect_injection_sites` runs up front (level geography, no RNG,
no CHR), and `inject_segment_chasers` is called from the walker's segment
prologue in `randomize_object_data` — after the entries are parsed, before
anything is committed or picked. An injected chaser is therefore just an enemy
the segment contains: the Bertha tally counts it, `segment_pins` commits its
page, and every subsequent pick is chosen around it.

That ordering matters. Injecting *after* the walk would also work and would
also unblock Boss Bass, but the chaser could then only land where the finished
level happened to accommodate it, instead of the level being built around the
chaser. Injecting *before* the walk (the old design) required a second copy of
the walker's CHR model, which is the drift that hid the Boss Bass bug.

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

inject_segment_chasers:                                 # in the walker prologue
    for entry in this segment, in data order:
        skip unless collect_injection_sites knows it    # = some level's first enemy
        roll WILD_INJECTION_CHANCE
        require first enemy swappable + unprotected     # don't clobber critical objects
        pins = segment_pins(entries, skip=this entry)   # the walker's own model
        pick a chaser that is in the player's set
          (Vec<WildChaser>: sun / lakitu / bass),
          CHR-compatible, the level does NOT already
          have (has_enemy_id), and within the
          Big-Bertha per-segment cap
        replace first enemy; re-seed suns to screen 0 (0x02, 0x11)
        mark the entry settled                          # walker pass 2 skips it
```

### Guards
- **Boss exclusion** — by `NodeKind`, not enemy bytes. Reliable.
- **No-double** — `has_enemy_id(rom, obj_ptr, chosen)` skips any chaser the level
  already contains (the 2-Quicksand fix).
- **Shared-pointer de-dup** — one physical enemy set injects at most once.
- **Swappable + unprotected first enemy** — `find_class_pool` guards against
  destroying a non-enemy object; `entry_protection_at` keeps the protection
  registry authoritative.
- **CHR compatibility** — unchanged; the chaser's sprite page must fit the
  segment's committed slots.
- **Bertha cap** — `MAX_BERTHA_PER_SEGMENT`, counted by the injection itself.
  The walker enforces it across swaps but can't see an injection that already
  happened, so an injected Boss Bass checks the budget on its own.

### Frame
Everything uses `obj_ptr` + `enemy_ptr_to_file_offset` (the same frame as
`has_enemy_id`), verified against 1-1 (obj `0xC527` → first enemy at file
`0xC538`).

### Sun placement
Kept as-is: injected suns are re-seeded to the vanilla 2-Quicksand spawn
(screen 0, `Y=0x11`). Confirmed necessary — deep suns idle in the background.

### Pick weighting
When both chasers are eligible for a level, the pick is weighted toward the sun
(`SUN_INJECTION_WEIGHT = 2`, i.e. ~1/3 Lakitu / 2/3 sun) — Lakitu is much harder
to deal with, so it's kept rarer. Tune the constant to shift the mix.

### Lakitu height
Lakitu works at any spawn height, but inheriting the replaced first enemy's Y
(usually a low ground-enemy spot) makes it too consistently punishing — the
spinies land closer. So an injected Lakitu's Y is a coin-flip between the
inherited height and `LAKITU_ALT_Y` (`0x12`, the common vanilla Lakitu height):
half stay low, half lift up. X is always inherited.

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
- **A one-chaser pool is not, in practice, a sparser pool.** `Sun` / `Lakitu`
  filter the pool *before* the CHR-compatibility and no-doubling checks, and a
  candidate that fails them injects nothing rather than falling back to the
  other chaser — so the single modes ought to land on fewer levels. Measured
  over 20 seeds they don't: 8.2–9.2 injections per seed in every mode, with the
  between-mode spread no larger than the spread between two runs of one mode
  (each mode draws a different RNG stream). Few levels are CHR-restricted to
  exactly one chaser, so the effect exists but is far below the noise.
  `WILD_INJECTION_CHANCE` is therefore left alone. Numbers from the
  `print_injection_counts` diagnostic in `enemies/tests.rs`.
