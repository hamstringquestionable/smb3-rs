//! Per-class mode resolution (off / shuffle / wild) and class-pool lookup.

use super::*;

pub(super) struct ClassModes {
    pub(super) ground: EnemyMode,
    pub(super) shell: EnemyMode,
    pub(super) flying: EnemyMode,
    pub(super) piranhas: EnemyMode,
    pub(super) ghosts: EnemyMode,
    pub(super) thwomps: EnemyMode,
    pub(super) rotodiscs: EnemyMode,
    pub(super) cannons: EnemyMode,
    pub(super) water: EnemyMode,
    pub(super) bros: EnemyMode,
}

/// Return the wild swap pool that would be in effect for the given Options
/// (union of class pools where the class is in Wild mode). Exposed `pub`
/// so integration tests / analyzers can enumerate the pool to compute
/// per-pool distribution metrics.
pub fn wild_pool_for(opts: &Options) -> Vec<u8> {
    ClassModes::from_options(opts).build_wild_pool()
}

impl ClassModes {
    pub(super) fn from_options(opts: &Options) -> Self {
        Self {
            ground: opts.ground,
            shell: opts.shell,
            flying: opts.flying,
            piranhas: opts.piranhas,
            ghosts: opts.ghosts,
            thwomps: opts.thwomps,
            rotodiscs: opts.rotodiscs,
            cannons: opts.cannons,
            water: opts.water,
            bros: opts.bros,
        }
    }

    /// Build the dynamic wild pool: collect all IDs from classes set to Wild.
    pub(super) fn build_wild_pool(&self) -> Vec<u8> {
        let mut pool = Vec::new();
        if self.ground == EnemyMode::Wild { pool.extend_from_slice(GROUND_ENEMIES); }
        if self.shell == EnemyMode::Wild { pool.extend_from_slice(SHELL_ENEMIES); }
        if self.flying == EnemyMode::Wild { pool.extend_from_slice(FLYING_ENEMIES); }
        // Piranhas are intentionally NOT added to the global wild pool. Like
        // cfire, they are self-contained in Wild mode: a piranha slot swaps
        // only within piranha-kind (standard + Rocky Wrench, or ceiling), and
        // no other class can ever turn into a piranha. See find_class_pool.
        if self.ghosts == EnemyMode::Wild { pool.extend_from_slice(GHOST_ENEMIES); }
        if self.thwomps == EnemyMode::Wild { pool.extend_from_slice(THWOMPS); }
        if self.rotodiscs == EnemyMode::Wild {
            pool.extend_from_slice(ROTODISCS_SINGLE);
            pool.extend_from_slice(ROTODISCS_DUAL);
        }
        // ALL_CANNONS intentionally NOT added — cfire is self-contained in
        // Wild mode for TWO reasons, both load-bearing:
        //
        // 1. **Gameplay correctness (permanent).** cfire IDs are projectile
        //    emitters (bullet bill cannons, laser turrets, etc.). They fire
        //    blind across the screen from their X position. Spawning one
        //    where a player expects a stompable ground enemy means hits
        //    arrive out of nowhere with no telegraph — arguably unplayable.
        //    cfire must never appear as the random output of a non-cfire
        //    class swap.
        //
        // 2. **Distribution (legacy of the bucket-first picker).** cfire
        //    IDs share the NOCHANGE CHR slot, so they got per-bucket
        //    appended in PageBuckets; with the old bucket-first picker
        //    that over-weighted them ~K× per draw and flooded every level
        //    (observed: 49 → 213 bullet bill cannons before the fix).
        //    `PageBuckets::pick` is now uniform-among-compatibles, so this
        //    flooding mechanism no longer exists — but reason (1) alone
        //    is enough to keep cfire out.
        //
        // Net semantic: cfire can still transform INTO other wild enemies,
        // but other classes never swap TO cfire — total cfire count stays
        // ≤ vanilla and projectile emitters only appear where Nintendo put
        // them.
        if self.water == EnemyMode::Wild { pool.extend_from_slice(WATER_ENEMIES); }
        if self.bros == EnemyMode::Wild { pool.extend_from_slice(BRO_ENEMIES); }
        pool
    }
}

/// A classified enemy's swap pool. The kind is explicit because callers pick
/// differently per kind: `Wild` draws from the caller's wild pool with the
/// uniform CHR-aware pick, the two piranha kinds use their self-contained
/// category-equal strategies, and `Class` is a plain within-class pool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ClassPool {
    /// The global wild pool (union of Wild-mode classes). Its members are
    /// built per-Options at runtime, so callers supply the slice.
    Wild,
    /// Self-contained standard-piranha wild pool ([`PIRANHAS_WILD`]).
    PiranhaStd,
    /// Self-contained ceiling-piranha wild pool ([`PIRANHASC_WILD`]).
    PiranhaCeil,
    /// A plain class pool: Shuffle mode, or a self-contained Wild class
    /// (cannons).
    Class(&'static [u8]),
}

impl ClassPool {
    /// The pool members. `wild_pool` supplies the `Wild` variant's slice;
    /// the other variants ignore it.
    pub(super) fn slice(self, wild_pool: &[u8]) -> &[u8] {
        match self {
            ClassPool::Wild => wild_pool,
            ClassPool::PiranhaStd => PIRANHAS_WILD,
            ClassPool::PiranhaCeil => PIRANHASC_WILD,
            ClassPool::Class(class) => class,
        }
    }
}

/// Whether the walker's Pass 1 should pre-commit this object's CHR page
/// before replacements are picked: non-swappable objects and uniform-CHR
/// classes (every member shares the page/slot, so a swap can't change it).
/// Boom-Booms pre-commit too — their separate `BOOMBOOM_SWAP` is CHR-neutral
/// (0x4B↔0x4C share one page/slot; 0x4A never swaps), so the original id's
/// page is exact. Wild entries never pre-commit — their page is decided by
/// the pick itself.
pub(super) fn should_precommit(obj_id: u8, modes: &ClassModes) -> bool {
    match find_class_pool(obj_id, modes) {
        None => true,
        Some(ClassPool::Wild) => false,
        Some(ClassPool::PiranhaStd) => is_uniform_chr_class(PIRANHAS_WILD),
        Some(ClassPool::PiranhaCeil) => is_uniform_chr_class(PIRANHASC_WILD),
        Some(ClassPool::Class(class)) => is_uniform_chr_class(class),
    }
}

/// Whether this entry's CHR page is fixed before any pick is made: either the
/// enemy can't change (no class pool / class off / curated SkipSwap), or any
/// swap it can undergo keeps the same page (uniform-CHR class, Boom-Boom
/// self-swap). Pinned pages are pre-committed so picks route around them.
pub(super) fn is_pinned(obj_id: u8, file_offset: usize, modes: &ClassModes) -> bool {
    should_precommit(obj_id, modes)
        || entry_protection_at(file_offset) == Some(EntryProtection::SkipSwap)
}

/// Identify which class an enemy ID belongs to, and return the swap pool
/// based on that class's mode. Returns None if the class is Off or unknown.
pub(super) fn find_class_pool(id: u8, modes: &ClassModes) -> Option<ClassPool> {
    // Macro to check class membership and return appropriate pool
    macro_rules! check {
        ($ids:expr, $mode:expr) => {
            if $ids.contains(&id) {
                return match $mode {
                    EnemyMode::Off => None,
                    EnemyMode::Shuffle => Some(ClassPool::Class($ids)),
                    EnemyMode::Wild => Some(ClassPool::Wild),
                };
            }
        };
    }
    check!(GROUND_ENEMIES, modes.ground);
    check!(SHELL_ENEMIES, modes.shell);
    check!(FLYING_ENEMIES, modes.flying);

    // Piranhas are self-contained (never the global wild pool, either direction).
    // Standard plants + Rocky Wrench + the upward fire jet swap among each other;
    // ceiling plants + the downward fire jet swap among themselves. Rocky Wrench
    // (0xAD) and the fire jets (0x9D up / 0xB2 down) join ONLY in Wild mode — they
    // belong to no class otherwise, so in Shuffle/Off they're left untouched.
    if PIRANHAS.contains(&id) || id == ROCKY_WRENCH || id == FIREJET_UP {
        return match modes.piranhas {
            EnemyMode::Off => None,
            EnemyMode::Shuffle => {
                if PIRANHAS.contains(&id) { Some(ClassPool::Class(PIRANHAS)) } else { None }
            }
            EnemyMode::Wild => Some(ClassPool::PiranhaStd),
        };
    }
    if PIRANHASC.contains(&id) || id == FIREJET_DOWN {
        return match modes.piranhas {
            EnemyMode::Off => None,
            EnemyMode::Shuffle => {
                if PIRANHASC.contains(&id) { Some(ClassPool::Class(PIRANHASC)) } else { None }
            }
            EnemyMode::Wild => Some(ClassPool::PiranhaCeil),
        };
    }

    check!(GHOST_ENEMIES, modes.ghosts);
    check!(THWOMPS, modes.thwomps);
    check!(ROTODISCS_SINGLE, modes.rotodiscs);
    check!(ROTODISCS_DUAL, modes.rotodiscs);
    check!(WATER_ENEMIES, modes.water);
    check!(BRO_ENEMIES, modes.bros);

    // Cannons: 3 sub-classes (LEFT, RIGHT, BILLS). In Wild, all cfire merges
    // into ALL_CANNONS — self-contained, never pulls from wild_pool so cfire
    // count can't inflate (see build_wild_pool comment for why).
    if modes.cannons != EnemyMode::Off {
        for sub in [CFIRE_LEFT, CFIRE_RIGHT, CFIRE_BILLS] {
            if sub.contains(&id) {
                return match modes.cannons {
                    EnemyMode::Off => None,
                    // stay within sub-class
                    EnemyMode::Shuffle => Some(ClassPool::Class(sub)),
                    // any cfire → any cfire
                    EnemyMode::Wild => Some(ClassPool::Class(ALL_CANNONS)),
                };
            }
        }
    }

    None
}

/// Build a ClassModes for HB encounter segments.
/// In HB segments, the `hb_encounters` mode is the sole authority.
pub(super) fn hb_class_modes(hb_mode: EnemyMode) -> ClassModes {
    ClassModes {
        ground: hb_mode,
        shell: hb_mode,
        flying: hb_mode,
        piranhas: hb_mode,
        ghosts: hb_mode,
        thwomps: hb_mode,
        rotodiscs: hb_mode,
        cannons: hb_mode,
        water: hb_mode,
        bros: hb_mode,
    }
}
