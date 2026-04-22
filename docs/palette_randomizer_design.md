# Palette Randomizer — Design Draft

**Status**: plains MVP shipped (as of 2026-04-21). Variant-swap approach is the
active design; earlier pool-based sections kept below for historical context.

## Current approach (shipped MVP)

**Variant-swap** — for each palette position where we have curated alternatives,
the randomizer picks one whole 4-byte variant. No per-byte picking, no color
pools, no risk of cross-palette clash.

Positions where we have no curated alternatives (or where even Recolored didn't
change anything) are left vanilla. For plains that's ~70% of the bytes in the
mapped palette region.

Current variant library (`src/randomize/palette_variants.rs`):
- **Plains slot 3** (0x36BE4 band 3): 7 positions × 2 variants (vanilla, Recolored)
- **Plains slice 4 band 3** (0x376D8): 1 position × 2 variants

2^8 = 256 combinations for plains. Every emitted palette is built from
4-byte groups that either shipped in the game or were hand-tuned by the
Recolored author, so individual groups are guaranteed aesthetically valid.

To expand: add more curated variants per position in
`palette_variants.rs`. Each added variant widens the seed space linearly
per-position and combinatorially across positions, without adding clash risk.

## Rejected approaches (why variant-swap won)

**Flat color pools** (initial attempt): pools of per-theme NES colors with
per-byte random draws. Problem: flattening destroys the structural role of each
byte within a 4-byte palette — colors that were designed to coexist get separated,
and random combinations produce clashes the original artists never saw.
Luminance-row preservation helps but doesn't fix this; ~100% byte replacement is
still too aggressive.

**Recolored-derived flat pools** (second attempt): same as above but pools
seeded from Recolored's observed byte choices per region. Better than
hand-picked pools but still suffers from the structural-flattening problem —
pulling any mix of colors Recolored used still produces combinations Recolored
never chose. User reported "colors are not particularly appealing."

**Per-byte hue shift** (considered, not implemented): replace each byte with
a color within ±2 columns on the same luminance row. Always stays close to
vanilla but produces thin variation. Kept as possible future `--subtle-palettes`
mode if needed.

This draft is the output of the reverse-engineering phase documented in
`docs/smb3_rom_reference.md` (section "Palette Data") and
`tools/palette_inspect.py`. It proposes how to turn that understanding into a
thoughtful in-game palette randomizer, replacing the current minimal
`src/randomize/palettes.rs` (Mario/Luigi power-up colors only).

Goals, in priority order:
1. Coherent per-tileset theming — plains still *feels* like grass-and-sky,
   fortress still feels like stone-and-shadow, even after randomization.
2. Seed-deterministic and replayable.
3. Never break the game (no soft-locks, no crashes, no unreadable HUD).
4. Pluggable colorblind-friendly modes (aspirational, later phase).
5. Fits the existing codebase's "decide then write" architecture.

---

## 1. What we're working with

### Two tables drive in-level palettes (both fire at level load)

| Table | File offset | Size | What it drives |
|---|---|---|---|
| **Themed slot table** | `0x36BE4-0x36DA6` | 450 B, 8 × ~56 B slots | Indexed by level header byte 5's `d` field. Writes to `$3F00` (universal BG) + sprite palette 0. Slot is *shared across tilesets* (multiple tilesets can pick the same slot). |
| **Per-tileset variant table** | `0x37000-0x37846` | ~2 KB, slices per tileset family | Writes `$3F01-$3F0F` BG palettes + sprite palettes 1-3. Each slice has 8 bands of ~64 B. |

Plus three isolated tables we already know:
- `0x10539-0x10554` — Mario/Luigi power-up palettes (currently randomized in `palettes.rs`)
- `0x36DAA-0x36DAD` — Lava/Rotodisc
- `0x36DFE-0x36E01` — Bowser/Donut-lift
- `0x36EE2-0x37000` — Water-sprite palette + note-block animation frames

### Confirmed slot/band → graphic mappings (from emulator probes)

**`0x36BE4` slots** (themed slot table):

| Slot | Range | Drives |
|---|---|---|
| 0 | 0x36BE4-0x36C1C | W6 sky overworld map + map HUD |
| 1 | 0x36C1C-0x36C54 | W7 (pipe) overworld map |
| 2 | 0x36C54-0x36C8C | Hammer Bro overworld sprites + "HELP" text + world labels |
| 3 | 0x36C8C-0x36CC4 | **Plains 1-1 BG + HUD** (writes $3F00) |
| 4 | 0x36CC4-0x36CFC | Giant tileset (W4) |
| 5 | 0x36CFC-0x36D34 | Plains enemies + W7-5 sub-area BG |
| 6 | 0x36D34-0x36D6C | W4-F1 / W8 fortress HUD |
| 7 | 0x36D6C-0x36DA6 | Fortress BG + W7-5 sub-area enemies |

**`0x37000+` slices** (per-tileset variants):

| Slice | Range | Tileset family |
|---|---|---|
| 1 | 0x37000-0x37200 | Water tileset (underwater BG, water enemies) |
| 2 | 0x37200-0x37400 | Desert + fortress + airship (3 airships share bands 5/6/7) |
| 3 | 0x37400-0x37600 | Giant tileset + water pipe/decoration accents |
| 4-A | 0x37600-0x377DF | Sky-Land + plains BG/enemy variants |
| **4-ptr** | **0x377E0-0x37807** | ⚠️ **Level layout pointer table — MUST NOT be touched; crashes level load** |
| 4-B | 0x37808-0x37846 | Tail palette data (safe) |

### Hard constraints the randomizer must respect

- **`0x377E0-0x37807`**: 40 bytes of CPU pointers in `$ABD2-$B412`. Leave vanilla.
- **Outline color `0x0F`**: always byte-2 or byte-3 of a palette quartet depending on table alignment. The "raw painter" rule (don't touch any byte whose vanilla value is `0x00` or `0x0F`) is safe across every probe run so far.
- **Universal BG color `$3F00`**: when written via byte-0 of a palette quartet, changes the HUD background for all levels using that slot. If HUD readability matters, constrain `$3F00` writes to dark colors (≤ `0x1C`).
- **Color `0x0D`**: black variant that behaves weirdly on some NES hardware. Already excluded from `SAFE_COLORS` in `palettes.rs`.
- **Character palette slot count**: existing `src/randomize/palettes.rs` handles byte-1/byte-2 only. Keep that rule when extending.

---

## 2. Three design options

### Option A — "Safe extension" (minimal risk)
Extend `palettes.rs` to add lava + Bowser back (reverting commit `f99e705`'s
over-correction), plus Mario/Luigi already there. Total scope: ~36 bytes patched.

- **Pros**: ~10 lines of code, zero chance of breaking anything, deterministic today.
- **Cons**: doesn't remotely achieve the "massive recolor" the user asked for.
  Plains still look vanilla.

### Option B — "Themed per-slot/per-slice" (recommended)
Randomize both tables with themed color pools, coordinated per theme:
- 0x36BE4 slots → one themed pool per slot (with meta-rules for HUD slot safety).
- 0x37000+ slices → same themed pool as the 0x36BE4 slot that shares the same use
  (e.g., plains slot 3 + slice 4 plains variant both draw from a "plains" pool).
- Skip pointer table and structural bytes.

- **Pros**: delivers the visual impact, respects what we learned empirically,
  stays inside the existing "decide then write" architecture.
- **Cons**: modest complexity (~400-800 lines), some slots/slices still
  un-probed so first pass will likely need follow-up tweaks.

### Option C — "Full engine replacement"
Clone Recolored's approach: relocate the jump engine, install a new palette
engine, wholesale rewrite.

- **Pros**: maximum flexibility (could even support runtime palette switching).
- **Cons**: huge undertaking, conflicts with every other randomizer feature
  that writes in that bank, and unnecessary given Option B already solves the goal.

**Recommendation: B.**

---

## 3. Proposed design (Option B)

### 3.1 Themed color pools

Each pool is a curated set of NES color indices that "read as" a theme. Concrete
starting sketch (not final — expected to be tweaked iteratively):

```rust
// Each pool: 6-10 NES color bytes. Byte 0 always safe for $3F00 (≤ 0x1C to keep HUD dark).
// Order roughly "dark → light" so we can pick structured gradients.
const POOL_PLAINS:   &[u8] = &[0x0B, 0x1A, 0x2A, 0x3A, 0x08, 0x18, 0x28, 0x38]; // greens + warm accents
const POOL_WATER:    &[u8] = &[0x01, 0x11, 0x21, 0x31, 0x02, 0x12, 0x22, 0x32]; // blues
const POOL_DESERT:   &[u8] = &[0x07, 0x17, 0x27, 0x37, 0x08, 0x18, 0x28, 0x38]; // tan/orange
const POOL_ICE:      &[u8] = &[0x01, 0x11, 0x21, 0x31, 0x30, 0x3C, 0x20, 0x10]; // pale blues + whites
const POOL_SKY:      &[u8] = &[0x12, 0x22, 0x32, 0x35, 0x36, 0x26, 0x16, 0x06]; // sky blues + sunset
const POOL_FORTRESS: &[u8] = &[0x00, 0x06, 0x16, 0x26, 0x07, 0x17, 0x27, 0x37]; // gray/red-brown
const POOL_AIRSHIP:  &[u8] = &[0x00, 0x04, 0x14, 0x24, 0x07, 0x17, 0x27, 0x37]; // dark gray/purple
const POOL_GIANT:    &[u8] = &[0x06, 0x16, 0x26, 0x36, 0x09, 0x19, 0x29, 0x39]; // saturated warm+green
const POOL_HUD_SAFE: &[u8] = &[0x00, 0x01, 0x02, 0x05, 0x06, 0x0C, 0x11, 0x12]; // always dark BG / readable
```

**Tweakability principle**: pool definitions and slot→pool assignment (§3.2)
are expected to need visual iteration over many seeds. They should live in a
single data module (e.g., `src/randomize/palette_pools.rs`) organized as
data, not hard-coded inline — so changes don't ripple through the rest of
the code.

**Preview tool**: `tools/preview_palette_pools.py` renders each pool as a
static HTML file (`palette_pools_preview.html`) with color swatches + sample
palettes. Open in a browser to judge pools before baking them into the Rust
randomizer. Workflow:

1. Edit the `POOLS` dict in the Python script.
2. Re-run to regenerate the HTML.
3. Visually evaluate; iterate.
4. Once happy, port the dict verbatim into `palette_pools.rs`.

The Python preview keeps the Rust codebase stable while pool choices are
still being refined.

### 3.2 Slot ↔ pool assignment

```rust
enum Theme { Plains, Water, Desert, Ice, Sky, Fortress, Airship, Giant, Overworld, HudSafe }

// Map 0x36BE4 slots to themes based on what we proved they drive.
const SLOT_THEMES: [Theme; 8] = [
    Theme::Overworld,  // slot 0 → sky-overworld map (band 0)
    Theme::Overworld,  // slot 1 → pipe-overworld map (band 1)
    Theme::HudSafe,    // slot 2 → HELP text + map sprites (keep readable)
    Theme::Plains,     // slot 3 → plains BG (drives $3F00, needs HUD-safe byte 0)
    Theme::Giant,      // slot 4 → giant BG
    Theme::Plains,     // slot 5 → plains enemies (shared theme with slot 3)
    Theme::Fortress,   // slot 6 → fortress HUD
    Theme::Fortress,   // slot 7 → fortress BG (shared theme with slot 6)
];

// Slice themes (0x37000+).
const SLICE_THEMES: &[(u32, u32, Theme)] = &[
    (0x37000, 0x37200, Theme::Water),
    (0x37200, 0x37400, Theme::Desert),  // and fortress variants
    (0x37400, 0x37600, Theme::Giant),
    (0x37600, 0x377DF, Theme::Sky),     // includes plains variants (bands 3/5)
    // 0x377E0-0x37807 pointer table — SKIP
    (0x37808, 0x37846, Theme::Sky),
];
```

### 3.3 Randomization rule per byte

For every byte in every target range:
1. If offset is in the pointer table (`0x377E0-0x37807`) → skip.
2. If vanilla byte is `0x00` or `0x0F` → skip (structural).
3. If this is the universal-BG byte (detected by context: byte-0 of a quartet
   that writes to `$3F00`) and HUD-safety is on → pick from `POOL_HUD_SAFE`.
4. Otherwise → pick a random color from the slot/slice's theme pool.

The existing `SAFE_COLORS` constant in `palettes.rs` stays as a global filter
layered over pool picks (keeps us away from `0x0D` etc.).

### 3.4 RNG independence (NOT seed-bound)

**Palette randomization must stay fully decoupled from the main seed.** Players
must be able to change palette mode (off / characters / themed / chaos) or
re-roll palettes *without* changing anything else in the ROM — level layout,
enemy placement, powerups, etc. all stay identical.

The existing wiring in `randomizer.rs:619` already does this correctly:

```rust
if options.palettes {
    rom.set_tag("palettes");
    let mut palette_rng = ChaCha8Rng::from_os_rng();  // fresh OS RNG
    randomize::palettes::randomize(rom, &mut palette_rng);
}
```

And the flag-key encoder already excludes palettes:

```rust
palettes: true, // cosmetic — not encoded in flag key
```

The new `randomize_themed()` must preserve this: take an independent RNG,
not consume from the main seed stream, not leak into the flag key. Two
beneficial consequences:

1. Re-generating a ROM with the same seed but different palette mode yields
   the same level content with different colors. Players can A/B palettes
   freely.
2. Palette mode changes can't shift any other module's RNG draws downstream,
   so "my seed" is always shorthand for a specific level layout / enemy set
   / powerup arrangement regardless of palette taste.

**Optional power-user knob** (not in MVP): a separate `--palette-seed N` flag
for players who find a palette they like and want to share it. Defaults to
OS-random like today. Still never encoded in the flag key.

### 3.5 User-facing options

- `--palettes none` (new) — skip palette randomization entirely.
- `--palettes characters` **(default)** — current behavior (Mario/Luigi body+highlight only).
- `--palettes themed` — opt-in, the full Option B pipeline above. **Off by default** until pools and assignments are visually dialed in.
- `--palettes chaos` — ignore themes, pick from the union of all pools (for users who want wild).

Also a future `--palettes colorblind=deuter|protan|mono` flag (see §5).

Default-off for `themed` means we can land the code without forcing it on
random seeds until the pool tuning is confident. Seeds generated before
enablement are unaffected.

### 3.6 Integration with existing randomizer

In `randomizer.rs`, the call site becomes:

```rust
if opts.palettes != PaletteMode::None {
    palettes::randomize(rom, rng, opts.palettes, opts.colorblind);
}
```

No changes to `levels.rs`, `powerups.rs`, etc. Palette randomization still
doesn't depend on ordering — can run at any point in the pipeline, just
before the final IPS diff.

---

## 4. Concrete first increment (prove the design)

Before writing all of Option B, ship a **smallest-possible themed slice** to
validate the model end-to-end:

1. Add a new CLI/option `--palettes themed` (default off behind a flag).
2. Only randomize `0x36BE4 slot 3` (plains BG) and `0x37600 slice 4 band 3`
   (plains BG variant) — both drawn from `POOL_PLAINS`.
3. Leave every other palette table vanilla.
4. Manual test: generate a few seeds, boot, verify:
   - 1-1 BG looks different each seed but plains-like.
   - HUD is still readable.
   - Other levels look vanilla.

If that works, expand to giant, fortress, water, etc. slot by slot.

---

## 5. Colorblind modes (deferred, aspirational)

Later phase, once Option B is solid. Approach:

- **Pool overlay**: each theme pool is filtered through a colorblind-safe
  subset of NES colors. Same slot/slice assignments, different pools.
- Starting points from color-vision research on NES:
  - **Deuteranopia** (common red-green): greens and reds compress to
    indistinguishable olive/brown. Safe pool emphasizes blue/yellow axis.
  - **Protanopia** (rarer red-green): similar; emphasize blues.
  - **Tritanopia** (rare blue-yellow): emphasize red-green axis.
  - **Monochrome**: grayscale NES ramp (0x00, 0x10, 0x20, 0x30).
- Implementation: add a `colorblind: Option<ColorblindMode>` to the Options
  struct; inside each pool, filter to the safe subset before picking.

Not scoped in first implementation pass.

---

## 6. Open questions / work items

1. **Slice 4 sub-structure**: we know band 0 = Sky-Land enemies, band 3 = plains BG,
   band 5 = plains enemies. Bands 1/2/4/6/7 unmapped — a second rainbow pass
   (safe version) in more tilesets (underground, hilly, ice) would fill these in.
   Fine to ship an MVP before finishing this; just means those bands stay vanilla.
2. **Slot sub-structure inside 0x36BE4**: each slot is ~56 B, but a full NES
   palette upload is only 32 B. What's the extra 24 B? Could be a second
   palette variant, or fade frames, or indexing data. Worth a targeted
   sub-rainbow of one slot before randomizing its full range.
3. **Inventory reload**: we observed that opening the inventory screen triggers
   a palette re-upload that reverts to vanilla mid-frame. If randomized
   palettes *also* apply to the inventory load, this is free; if they don't,
   inventory will look jarringly different from gameplay. Needs a quick check.
4. **CHR coordination**: Recolored also edits CHR pixels (~160 B in page 0x0F)
   to keep tiles legible under its color choices. Our pool-based approach
   avoids extreme shifts, so hopefully no CHR edits needed — but a
   human-in-the-loop review on a handful of seeds will tell.
5. **Determinism with ROM map changes**: if a future refactor changes which
   level/tileset uses which slot, the themed assignment above may need to
   adapt. Keep the slot-theme table data-driven in one place.

---

## 7. Suggested next step

Write a ~80-line Rust prototype in a branch — `palettes::randomize_themed()`
that implements §4 (plains-only) — and generate 3 seeds, boot each in an
emulator, and visually confirm:

1. Plains BG is different per seed.
2. Plains still looks like "a green-blue level," not a color garbage fire.
3. Every other palette in the game is pixel-identical to vanilla.

If all three, expand slot by slot until every mapped table is randomized.
