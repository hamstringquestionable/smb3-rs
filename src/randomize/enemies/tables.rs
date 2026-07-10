//! Enemy / hazard class tables and the hazard-category predicates.
//! Pure data + small pure helpers shared across the enemies submodules.

pub(super) const BOOMBOOM_IDS: &[u8] = &[0x4A, 0x4B, 0x4C];

/// Boom-Boom variants that can be swapped with each other.
/// 0x4A is excluded — it's the stationary variant used in specific contexts.
pub(super) const BOOMBOOM_SWAP: &[u8] = &[0x4B, 0x4C];

// Object IDs from the Southbird SMB3 disassembly (smb3.asm).
// Only IDs that are actual enemies safe to swap are included.
// Special objects (end-level card, pipes, platforms, bosses, powerups,
// autoscroll, event triggers, cannons, etc.) are NOT listed and will
// never be modified.

/// Ground-walking enemies (no shell). These can be freely swapped with each other.
pub(super) const GROUND_ENEMIES: &[u8] = &[
    0x2B, // OBJ_GOOMBA_SHOE (Kuribo's Shoe)
    0x29, // OBJ_SPIKE
    0x2A, // OBJ_PATOOIE
    0x3D, // OBJ_NIPPERFIREBREATHER (stationary fire-spitting nipper)
    0x4F, // OBJ_CHAINCHOMPFREE (roams freely without post tile)
    0x33, // OBJ_NIPPER (stationary)
    0x39, // OBJ_NIPPERHOPPING
    0x40, // OBJ_BUSTERBEATLE
    0x46, // OBJ_PIRANHASPIKEBALL (tall plant with spike ball)
    0x55, // OBJ_BOBOMB
    0x58, // OBJ_FIRECHOMP (floats and chases)
    0x59, // OBJ_FIRESNAKE
    0x6B, // OBJ_PILEDRIVER (micro goomba)
    0x71, // OBJ_SPINY
    0x72, // OBJ_GOOMBA
    0x7C, // OBJ_BIGGOOMBA
];

/// Shell-producing enemies — kept in their own class because some levels require
/// shells to progress. Swapping these with non-shell enemies could make levels unbeatable.
pub(super) const SHELL_ENEMIES: &[u8] = &[
    0x6C, // OBJ_GREENTROOPA
    0x6D, // OBJ_REDTROOPA
    0x70, // OBJ_BUZZYBEATLE
    0x7A, // OBJ_BIGGREENTROOPA
    0x7B, // OBJ_BIGREDTROOPA
];

/// Flying/hopping enemies that can be swapped with each other.
pub(super) const FLYING_ENEMIES: &[u8] = &[
    0x6E, // OBJ_PARATROOPAGREENHOP
    0x6F, // OBJ_FLYINGREDPARATROOPA
    0x73, // OBJ_PARAGOOMBA
    0x74, // OBJ_PARAGOOMBAWITHMICROS
    0x7E, // OBJ_BIGGREENHOPPER
    0x80, // OBJ_FLYINGGREENPARATROOPA
];

/// Water enemies that can be swapped with each other.
pub(super) const WATER_ENEMIES: &[u8] = &[
    0x2D, // OBJ_BIGBERTHA (leaping eater — the "Boss Bass")
    0x48, // OBJ_BABYBLOOPER
    0x61, // OBJ_BLOOPERWITHKIDS
    0x62, // OBJ_BLOOPER
    0x63, // OBJ_BIGBERTHABIRTHER (swims, spits a baby Cheep Cheep)
    0x64, // OBJ_CHEEPCHEEPHOPPER
    0x67, // OBJ_LAVALOTUS (southbird: "underwater lava plant")
    0x6A, // OBJ_BLOOPERCHILDSHOOT
    0x76, // OBJ_GREENCHEEP (jumping)
    0x77, // OBJ_REDCHEEP
    0x88, // OBJ_ORANGECHEEP
];

/// Hammer/Boomerang/Fire Bros — swap among themselves.
pub(super) const BRO_ENEMIES: &[u8] = &[
    0x81, // OBJ_HAMMERBRO
    0x82, // OBJ_BOOMERANGBRO
    0x86, // OBJ_HEAVYBRO
    0x87, // OBJ_FIREBRO
];

/// Standard (non-ceiling) piranha plant variants (including Giant World) —
/// the Shuffle-mode pool; they swap among themselves.
pub(super) const PIRANHAS: &[u8] = &[
    0x7D, // OBJ_BIGGREENPIRANHA
    0x7F, // OBJ_BIGREDPIRANHA
    0xA0, // OBJ_GREENPIRANHA
    0xA2, // OBJ_REDPIRANHA
    0xA4, // OBJ_GREENPIRANHA_FIRE
    0xA6, // OBJ_VENUSFIRETRAP
];

/// Rocky Wrench — the mole (`OBJ_ROCKYWRENCH`) that pops out of the ground and
/// throws wrenches. NOT the flying-wrench cannon-fire variant (`0xBE`,
/// `CFIRE_ROCKYWRENCH`), which is a projectile spawner and stays untouched.
/// Carries CHR page `0x36`/slot 4, so it flows through the normal CHR-compat
/// system like any other enemy (no garbled-tile risk).
pub(super) const ROCKY_WRENCH: u8 = 0xAD;

/// Upward-shooting fire jet (`OBJ_FIREJET_UPWARD`). Joins the *standard* piranha
/// pool in Wild — its flame erupts upward like a piranha emerging from a pipe.
/// CHR page `$37`/slot 5, so it rides the normal CHR-compat system.
pub(super) const FIREJET_UP: u8 = 0x9D;

/// Downward-shooting fire jet (`OBJ_FIREJET_UPSIDEDOWN`). Joins the *ceiling*
/// piranha pool in Wild — its flame shoots down like a ceiling piranha. Same
/// CHR page `$37`/slot 5.
pub(super) const FIREJET_DOWN: u8 = 0xB2;

/// Rows the upward fire jet is raised (Y−) when it replaces a standard piranha,
/// and rows the downward jet is lowered (Y+) when it replaces a ceiling piranha.
/// Playtest-tuned so the flame base lines up with the former pipe mouth.
pub(super) const FIREJET_UP_Y_RISE: u8 = 3;
pub(super) const FIREJET_DOWN_Y_DROP: u8 = 1;

/// Wild-mode standard pool: the standard piranhas plus Rocky Wrench and the
/// upward fire jet. In Wild mode piranhas are *self-contained* (parallel to the
/// cannons model) — they never merge into the global wild pool in either
/// direction. Removing an extra member is a one-line edit on this list.
pub(super) const PIRANHAS_WILD: &[u8] = &[
    0x7D, // OBJ_BIGGREENPIRANHA
    0x7F, // OBJ_BIGREDPIRANHA
    0xA0, // OBJ_GREENPIRANHA
    0xA2, // OBJ_REDPIRANHA
    0xA4, // OBJ_GREENPIRANHA_FIRE
    0xA6, // OBJ_VENUSFIRETRAP
    ROCKY_WRENCH, // 0xAD — the mole; see ROCKY_WRENCH doc
    FIREJET_UP,   // 0x9D — upward fire jet; see FIREJET_UP doc
];

/// Piranha Ceiling / Flipped variants — the Shuffle-mode ceiling pool.
pub(super) const PIRANHASC: &[u8] = &[
    0xA1, // OBJ_GREENPIRANHA_FLIPPED
    0xA3, // OBJ_REDPIRANHA_FLIPPED
    0xA5, // OBJ_GREENPIRANHA_FIREC
    0xA7, // OBJ_VENUSFIRETRAP_CEIL
];

/// Wild-mode ceiling pool: the ceiling piranhas plus the downward fire jet.
/// Self-contained (no crossover to the standard pool).
pub(super) const PIRANHASC_WILD: &[u8] = &[
    0xA1, // OBJ_GREENPIRANHA_FLIPPED
    0xA3, // OBJ_REDPIRANHA_FLIPPED
    0xA5, // OBJ_GREENPIRANHA_FIREC
    0xA7, // OBJ_VENUSFIRETRAP_CEIL
    FIREJET_DOWN, // 0xB2 — downward fire jet; see FIREJET_DOWN doc
];

// --- Category buckets for bucket-first (category-equal) Wild picking ---
// A Wild piranha slot picks a *category* uniformly (piranha / flame / wrench),
// then a member, so the lone flame and the lone wrench each get a full category
// share instead of being two members lost among the many piranhas. CHR still
// applies — a bucket with no compatible member is skipped that draw. See
// `pick_bucket_first` and the swap site.

/// Standard piranhas excluding the giant red (0x7F): the piranha bucket for a
/// slot that wasn't already a giant red (0x7F may only stay where one was).
pub(super) const PIRANHAS_NO_RED: &[u8] = &[0x7D, 0xA0, 0xA2, 0xA4, 0xA6];

pub(super) const BUCKET_UP_JET: &[u8] = &[FIREJET_UP];
pub(super) const BUCKET_DOWN_JET: &[u8] = &[FIREJET_DOWN];
pub(super) const BUCKET_WRENCH: &[u8] = &[ROCKY_WRENCH];

/// Giant red piranha. Its hitbox is built off-center for *giant* pipes; placing
/// one in a slot sized for a regular pipe leaves the hitbox outside the pipe
/// (unfair). So `0x7F` may only be an output where a `0x7F` already was — never
/// as a replacement for anything else. Enforced in both Shuffle and Wild.
/// (Giant green `0x7D` was designed to fit regular pipes, so it's unconstrained.)
pub(super) const GIANT_RED_PIRANHA: u8 = 0x7F;

/// Thwomp variants — all use CHR page $12/+4 and differ only in movement pattern.
/// Behind the `wild_thwomps` flag (off by default) because random movement
/// directions don't suit corridors designed for specific drop patterns.
pub(super) const THWOMPS: &[u8] = &[
    0x8A, // OBJ_THWOMP (standard drop)
    0x8B, // OBJ_THWOMPLEFTSLIDE
    0x8C, // OBJ_THWOMPRIGHTSLIDE
    0x8D, // OBJ_THWOMPUPDOWN
    0x8E, // OBJ_THWOMPDIAGONALUL
    0x8F, // OBJ_THWOMPDIAGONALDL
];

// --- Hazard taxonomy ---
//
// Enemies that are unfair to *introduce* at a curated `ExcludeHazards` spot:
// unstompable/continuous threats (drops, fire, spike balls, projectile bros)
// that a player can't avoid in a tight or forced-transit position. Grouped by
// category so the placement filter can honor the "vanilla exception" — a hazard
// is allowed when the slot's vanilla enemy was the *same category*, so the level
// keeps the threat it was designed with (and within-category shuffle, e.g.
// thwomp variants, still works). The filter is additive-only: it blocks
// introducing a new hazard category, never strips an existing one.

pub(super) const HAZARD_LAVA_LOTUS: &[u8] = &[0x67]; // OBJ_LAVALOTUS (fire arcs)
pub(super) const HAZARD_PATOOIE: &[u8] = &[
    0x2A, // OBJ_PATOOIE (spits a spike ball up)
    0x46, // OBJ_PIRANHASPIKEBALL (Ptooie-style spike-ball launcher)
];
pub(super) const HAZARD_NIPPER: &[u8] = &[
    0x33, // OBJ_NIPPER
    0x39, // OBJ_NIPPERHOPPING
    0x3D, // OBJ_NIPPERFIREBREATHER
];
pub(super) const HAZARD_HOTFOOT: &[u8] = &[
    0x30, // OBJ_HOTFOOT_SHY
    0x45, // OBJ_HOTFOOT
];

/// All hazard categories. THWOMPS and BRO_ENEMIES are reused as-is (the bros
/// throw continuous projectiles, unavoidable in a forced spot).
pub(super) const HAZARD_CATEGORIES: &[&[u8]] = &[
    THWOMPS,
    HAZARD_LAVA_LOTUS,
    HAZARD_PATOOIE,
    HAZARD_NIPPER,
    HAZARD_HOTFOOT,
    BRO_ENEMIES,
];

/// The hazard category `id` belongs to (its index in [`HAZARD_CATEGORIES`]), or
/// `None` if `id` isn't a hazard.
pub(super) fn hazard_category(id: u8) -> Option<usize> {
    HAZARD_CATEGORIES.iter().position(|cat| cat.contains(&id))
}

/// Whether `candidate` must be excluded at a protected spot whose vanilla enemy
/// was `vanilla`. A hazard is excluded unless it shares the vanilla enemy's
/// category (the additive-only vanilla exception); a non-hazard is never
/// excluded.
pub(super) fn hazard_excluded(candidate: u8, vanilla: u8) -> bool {
    match hazard_category(candidate) {
        None => false,
        Some(c) => hazard_category(vanilla) != Some(c),
    }
}

/// Enemies whose sprites are taller than a standard 1-tile enemy.
/// When one of these is the replacement in a swap, Y is decremented by 1
/// to prevent the taller sprite from clipping into the floor.
pub(super) const TALL_ENEMIES: &[u8] = &[
    0x3F, // OBJ_DRYBONES
    0x7A, // OBJ_BIGGREENTROOPA
    0x7B, // OBJ_BIGREDTROOPA
    0x7C, // OBJ_BIGGOOMBA
    0x7E, // OBJ_BIGGREENHOPPER
    0x81, // OBJ_HAMMERBRO
    0x82, // OBJ_BOOMERANGBRO
    0x86, // OBJ_HEAVYBRO
    0x87, // OBJ_FIREBRO
];

// Cannon-fire object IDs sit in 0xBC..=0xD0 and are dispatched by the cannon
// code in PRG007 via index = OBJ_ID - $BC + 1 (see prg007.asm:5485-5505 and
// the CFIRE_* constants in smb3.asm:2539-2559 of the southbird disassembly).
// Each ID's actual fire direction is read off the CannonPoof_XOffs /
// CannonPoof_YOffs tables in prg007.asm:5858. The groupings below merge
// diagonals into the corresponding horizontal direction (UL+LL → LEFT,
// UR+LR → RIGHT) so a Shuffle within a sub-class never reverses a cannon.

/// Cannon-fire IDs that fire LEFT-ward (horizontal left, diagonal upper-left,
/// diagonal lower-left, goomba pipe left, bob-omb launcher left).
pub(super) const CFIRE_LEFT: &[u8] = &[
    0xC0, // OBJ_CFIRE_GOOMBAPIPE_L
    0xC2, // OBJ_CFIRE_HLCANNON
    0xC3, // OBJ_CFIRE_HLBIGCANNON
    0xC4, // OBJ_CFIRE_ULCANNON
    0xC6, // OBJ_CFIRE_LLCANNON
    0xC8, // OBJ_CFIRE_HLCANNON2
    0xC9, // OBJ_CFIRE_ULCANNON2
    0xCB, // OBJ_CFIRE_LLCANNON2
    0xCE, // OBJ_CFIRE_LBOBOMBS
];

/// Cannon-fire IDs that fire RIGHT-ward (horizontal right, diagonal upper-right,
/// diagonal lower-right, goomba pipe right, bob-omb launcher right).
pub(super) const CFIRE_RIGHT: &[u8] = &[
    0xC1, // OBJ_CFIRE_GOOMBAPIPE_R
    0xC5, // OBJ_CFIRE_URCANNON
    0xC7, // OBJ_CFIRE_LRCANNON
    0xCA, // OBJ_CFIRE_URCANNON2
    0xCC, // OBJ_CFIRE_HRCANNON
    0xCD, // OBJ_CFIRE_HRBIGCANNON
    0xCF, // OBJ_CFIRE_RBOBOMBS
];

/// Bullet Bill cannons — regular and missile (homing). Sub-class within
/// `cannons`. The actual projectile objects (0x78/0x79) are spawned by the
/// cannon at runtime via CFire_BulletBill, which sets up their XVel/Var3/Var4 —
/// placing 0x78/0x79 directly in level data leaves them uninitialized and
/// motionless, so they are NOT included here.
pub(super) const CFIRE_BILLS: &[u8] = &[
    0xBC, // OBJ_CFIRE_BULLETBILL
    0xBD, // OBJ_CFIRE_MISSILEBILL
];

/// Single rotodisc variants — swap rotation direction.
/// Behind the `rotodiscs` flag (off by default).
pub(super) const ROTODISCS_SINGLE: &[u8] = &[
    0x5A, // OBJ_ROTODISCCLOCKWISE
    0x5B, // OBJ_ROTODISCCCLOCKWISE
];

/// Dual rotodisc variants — swap rotation pattern.
/// Behind the `rotodiscs` flag (off by default).
/// Does NOT include Podoboo from ceiling (0x53) — different behavior entirely.
pub(super) const ROTODISCS_DUAL: &[u8] = &[
    0x51, // OBJ_ROTODISCDUAL (CW sync)
    0x5E, // OBJ_ROTODISCDUALOPPOSE (opposed H)
    0x5F, // OBJ_ROTODISCDUALOPPOSE2 (opposed V)
    0x60, // OBJ_ROTODISCDUALCCLOCK (CCW sync)
];

/// Ghost house / fortress enemies. Boo and Hot Foot use CHR page $12/+4,
/// Dry Bones uses $13/+5 (compatible with all slot 4 pages).
/// NOT Stretch Boos (0x31/0x32) — attached to platforms, position-critical.
pub(super) const GHOST_ENEMIES: &[u8] = &[
    0x2F, // OBJ_BOO (Boo Diddly)
    0x30, // OBJ_HOTFOOT_SHY (Hot Foot, shy variant)
    0x3F, // OBJ_DRYBONES
    0x45, // OBJ_HOTFOOT (Hot Foot, walks on floor)
];

/// Big ? Block IDs — these can be swapped with each other to randomize
/// which suit/powerup the player gets from Big ? Blocks.
pub(super) const BIG_Q_BLOCKS: &[u8] = &[
    0x94, // OBJ_BIGQBLOCK_3UP
    0x95, // OBJ_BIGQBLOCK_MUSHROOM
    0x96, // OBJ_BIGQBLOCK_FIREFLOWER
    0x97, // OBJ_BIGQBLOCK_SUPERLEAF
    0x98, // OBJ_BIGQBLOCK_TANOOKI
    0x99, // OBJ_BIGQBLOCK_FROG
    0x9A, // OBJ_BIGQBLOCK_HAMMER
];

/// File offset of the Tanooki Big ? Block in the World 7 Big ? Block room.
/// This block must NOT be randomized — flying/Tanooki is required to beat 7-F1.
/// The W7 room is at enemy_ptr 0xC9A3; the Tanooki is the second entry.
pub(super) const W7F1_TANOOKI_OFFSET: usize = 0x0C9B7;

/// Injection candidates for wild_injections mode: special enemies injected after
/// normal swaps. CHR compatibility checked via `sprite_bank()` at filter time.
pub(super) const WILD_INJECTION_IDS: &[u8] = &[
    0x83, // Lakitu (enemy-spawning variant, CHR $0B/+4)
    0xAF, // Angry Sun
    0x2D, // Boss Bass (Big Bertha — the leaping eater)
];

/// Probability (out of 256) that a segment will receive an injection when wild_injections is on.
/// ~15% chance per segment.
pub(super) const WILD_INJECTION_CHANCE: u8 = 38;

/// Odds (numerator, denominator) that a 2-enemy HB wild segment takes the
/// non-stompable path (one HB_NEEDS_SHELL enemy + one shell partner) instead
/// of two stompables. 5/31 ≈ 16%.
pub(super) const HB_NONSTOMPABLE_ODDS: (u32, u32) = (5, 31);

/// Large "Big Bertha" fish that exhaust sprite slots when stacked: the leaping
/// eater (0x2D, the injected "Boss Bass") and the cheep-spitting birther (0x63).
/// Both are sprite-heavy, so the per-segment cap counts them together.
pub(super) const BERTHA_IDS: &[u8] = &[0x2D, 0x63];

/// Maximum number of Big Bertha fish (see [`BERTHA_IDS`]) allowed in a single
/// enemy segment (= one obj_ptr / sub-area). More than this causes sprite slot
/// exhaustion that can prevent other objects (e.g. white blocks) from spawning —
/// this was observed in 3-9 where the white block became unreachable.
pub(super) const MAX_BERTHA_PER_SEGMENT: u8 = 2;

/// Maximum X-tile gap between consecutive enemies (sorted by X) before they
/// are split into separate CHR groups. Enemies more than one screen apart
/// can never be visible simultaneously, so they don't need compatible CHR pages.
pub(super) const CHR_GROUP_GAP: u8 = 16;

/// All cannon-fire IDs merged for Wild mode — every cfire ID can become every
/// other cfire ID (incl. cross-direction and cross-type swaps). Excludes
/// CFIRE_ROCKYWRENCH (0xBE), CFIRE_4WAY (0xBF), and CFIRE_LASER (0xD0)
/// because their level-design role (spawner / multi-direction / fortress wall
/// element) is too distinct to randomize generically.
pub(super) const ALL_CANNONS: &[u8] = &[
    // CFIRE_LEFT
    0xC0, 0xC2, 0xC3, 0xC4, 0xC6, 0xC8, 0xC9, 0xCB, 0xCE,
    // CFIRE_RIGHT
    0xC1, 0xC5, 0xC7, 0xCA, 0xCC, 0xCD, 0xCF,
    // CFIRE_BILLS
    0xBC, 0xBD,
];
