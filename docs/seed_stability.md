# Seed stability — when output may change, and what has to be proved instead

**A seed is stable within a version and is not promised across versions.** The
same seed and flag key on the same release must always produce the same ROM;
a version bump may change every ROM in the game. That is the whole policy, and
the rest of this document is what follows from it.

This exists because the question keeps coming up in the same shape: *"this
change makes the builder faster / simpler / better, but it moves the output —
is that allowed?"* The answer is yes, at a version bump, **provided the
distribution is shown not to have moved.** The bar is not byte identity. It is
census equivalence, and it is a higher bar than it sounds.

## Why byte identity is not the standard

Byte identity is an easy thing to check and the wrong thing to require.

It forbids changes that provably cannot make a map worse: swapping a hasher
(iteration order moves, so tie-breaks move), replacing a binary heap with a
bucket queue (equal-cost states pop in a different order), reordering two
independent decisions. Every one of those changes *which* map a seed produces
without changing *what kind* of map the builder makes — and the second is the
thing players experience.

Held as a hard rule it also costs real quality. The July 2026 speed work
recorded A\* as forbidden for exactly this reason: it changes which goal state
represents a level set on ties, so paths differ, so placements differ. That is a
correct reading of a byte-identity requirement and a bad reason to leave
performance on the table.

**So byte identity is kept as an instrument and retired as a policy.** It
answers one question extremely well — *"I intended to change nothing; prove
it"* — and `tests/rom_identity.rs` exists for precisely that. It is the right
gate for a pure refactor and the wrong gate for anything else.

## What replaces it: the distribution must not move

A change that moves output has to show that the *shape* of what the builder
produces is unchanged. Concretely: run the censuses before and after, and
account for every figure that moved by more than sampling noise.

The censuses are in `src/randomize/overworld_build/builder_tests.rs` and
`src/randomize/maze/tests.rs`; all take `CENSUS_SEEDS`. The one that matters
most is the route census, because route choice *is* the builder's product:

```sh
CENSUS_SEEDS=1000 cargo test --release --lib test_route_census -- --ignored --nocapture
CENSUS_SEEDS=500  cargo test --release --lib all_world_targets_reachable
```

### The baseline, captured 2026-09-09 at 1000 seeds

`test_route_census`, on `feature/maze-content-floor` (the maze is off in this
census, so the content floor does not touch it):

```
=== Route choice over 1000 seeds (slack 3) ===
         mean  linear%  choice%   max      C1   <floor%
  W1     2.38       4%      96%    10    18.1      2.1%
  W2     2.82       0%     100%    12    17.8      0.0%
  W3     2.61       7%      93%    10    18.5      0.0%
  W4     2.16      12%      88%     6    17.1      0.6%
  W5     3.01       2%      98%    11    17.0      0.0%
  W6     3.42       0%     100%    14    20.2      0.0%
  W7     2.03      19%      81%     7    18.5      0.2%
  W8     2.28       4%      96%     7    26.4      0.0%
  overall: mean 2.587 routes/world; 6.19% linear; C1 19.2,
           0.36% below its own dealt floor (n=8000)

  C1 floor probe: min 11  p10 14  mean 19.2; <14 5.1%; goal-open 3.3%
  per-seed total C1: mean 153.6 (floor total 112)
  by dealt floor: 11 -> C1 18.0 / 2.50 routes;  14 -> 19.1 / 2.60;
                  17 -> 20.7 / 2.65
```

**Read the per-world column, never the overall alone.** W1's linear rate is
load-bearing and has regressed under changes that looked neutral globally — the
2026-07-27 lock-pool trim moved W1 from 67% to 72% linear while the global
histogram barely twitched. A global mean can hide a world going flat.

### What counts as "did not move"

- **Per-world `linear%`**: within a couple of points, and *no world may go up*
  more than that on its own. W1 and W7 are the sensitive ones.
- **`mean` routes/world**: the historical band for the rebuilt builder is
  narrow; treat a move of more than ~0.05 as something to explain.
- **`C1` and `<floor%`**: the floor is a guarantee, so `<floor%` rising is a
  regression regardless of what else improved.
- **`goal-open%`**: should stay near zero.

Sampling noise at 1000 seeds is small but not zero. If a figure moves and you
believe it is noise, **re-run with a different seed range rather than asserting
it** — the censuses take seeds from 0, so `CENSUS_SEEDS` alone does not give an
independent sample.

## The three instruments, and the question each answers

| Instrument | Question | When |
|---|---|---|
| `tests/rom_identity.rs` | "I changed no behaviour — prove it." Whole-ROM bytes, captured from two trees, no committed baseline. | Pure refactors, module splits, extracted helpers. |
| `tests/overworld_baseline.rs` | "Did the overworld move without anyone noticing?" A committed hash of the overworld alone, 20 seeds, guarding the suite. | Always. Regenerating it requires an entry in [overworld_baseline_log.md](overworld_baseline_log.md). |
| The censuses | "The output moved on purpose — is it the same *kind* of output?" | Any intentional-output change. |

The first two are cheap and mechanical. The third is the one that takes
judgement, and it is the one this document exists for.

## The rules

1. **Never regenerate a baseline to make a red test green.** The recapture log
   exists to make an unattributed recapture visible, and an unattributed
   recapture is indistinguishable from papering over a bug.
2. **Say in the commit message that output was intended to move, and why.** A
   silent output change is the failure mode; a declared one is ordinary work.
3. **A version bump is the landing place** for accumulated output-moving
   changes, since that is where the promise renews. Landing several together is
   fine and often better — one census comparison instead of five.
4. **The flag key is a separate promise.** Its *meaning* is versioned
   independently (`FLAG_KEY_VERSION`); a key that decodes must keep decoding to
   the same options. Output may move; what the options mean may not, except by
   a deliberate key version bump.
5. **Correctness invariants are never negotiable**, whatever the censuses say:
   every world completable, the target never stranded, no content sealed out of
   the game.

## The case this was written for

Performance. `generate_patch` in the browser measured 97.7 ms with the maze off
and 122.9 ms with it on (2026-09-09, 40 seeds), against a 75 ms figure from July
and a <100 ms budget — and a player's machine may be several times slower than
the one those numbers came from. The profile puts 47% of the run in route
enumeration and 23% in the walk BFS, and the cheapest wins there are exactly the
tie-break-moving kind: a bucket queue for the Dijkstra, the existing `FastHasher`
in place of SipHash across the builder.

Under a byte-identity rule none of that is reachable. Under this one it is, at
the price of running the census and accounting for what moved. See issue #233.
