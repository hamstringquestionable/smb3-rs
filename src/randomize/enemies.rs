use rand::Rng;
use rand::seq::IndexedRandom;

use crate::randomize::rom_data::{ENEMY_DATA_END, ENEMY_DATA_START};
use crate::randomizer::{EnemyMode, Options};
use crate::rom::Rom;

/// Boom-Boom boss IDs — excluded from CHR pre-commit because they occupy
/// far-right boss rooms, screens away from corridor enemies. The player
/// never sees both on-screen simultaneously, so different CHR pages on
/// the same slot won't cause visible sprite garbling.
const BOOMBOOM_IDS: &[u8] = &[0x4A, 0x4B, 0x4C];

/// Boom-Boom variants that can be swapped with each other.
/// 0x4A is excluded — it's the stationary variant used in specific contexts.
const BOOMBOOM_SWAP: &[u8] = &[0x4B, 0x4C];

// Object IDs from the Southbird SMB3 disassembly (smb3.asm).
// Only IDs that are actual enemies safe to swap are included.
// Special objects (end-level card, pipes, platforms, bosses, powerups,
// autoscroll, event triggers, cannons, etc.) are NOT listed and will
// never be modified.

/// Ground-walking enemies (no shell). These can be freely swapped with each other.
const GROUND_ENEMIES: &[u8] = &[
    0x2B, // OBJ_GOOMBA_SHOE (Kuribo's Shoe)
    0x29, // OBJ_SPIKE
    0x2A, // OBJ_PATOOIE
    0x2D, // OBJ_CHAINCHOMP (strained on post)
    0x3D, // OBJ_CHAINCHOMPSTAKE (chained to stake, lunges)
    0x4F, // OBJ_CHAINCHOMPFREE (roams freely without post tile)
    0x33, // OBJ_NIPPER (stationary)
    0x39, // OBJ_NIPPERHOPPING
    0x40, // OBJ_BUSTERBEATLE
    0x46, // OBJ_LAKITU (level-placed variant, CHR $0A/+4)
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
const SHELL_ENEMIES: &[u8] = &[
    0x6C, // OBJ_GREENTROOPA
    0x6D, // OBJ_REDTROOPA
    0x70, // OBJ_BUZZYBEATLE
    0x7A, // OBJ_BIGGREENTROOPA
    0x7B, // OBJ_BIGREDTROOPA
];

/// Flying/hopping enemies that can be swapped with each other.
const FLYING_ENEMIES: &[u8] = &[
    0x6E, // OBJ_PARATROOPAGREENHOP
    0x6F, // OBJ_FLYINGREDPARATROOPA
    0x73, // OBJ_PARAGOOMBA
    0x74, // OBJ_PARAGOOMBAWITHMICROS
    0x7E, // OBJ_BIGGREENHOPPER
    0x80, // OBJ_FLYINGGREENPARATROOPA
];

/// Water enemies that can be swapped with each other.
const WATER_ENEMIES: &[u8] = &[
    0x48, // OBJ_BABYBLOOPER
    0x61, // OBJ_BLOOPERWITHKIDS
    0x62, // OBJ_BLOOPER
    0x63, // OBJ_BIGBERTHABIRTHER
    0x64, // OBJ_CHEEPCHEEPHOPPER
    0x6A, // OBJ_BLOOPERCHILDSHOOT
    0x76, // OBJ_GREENCHEEP (jumping)
    0x77, // OBJ_REDCHEEP
    0x88, // OBJ_ORANGECHEEP
];

/// Hammer/Boomerang/Fire Bros — swap among themselves.
const BRO_ENEMIES: &[u8] = &[
    0x81, // OBJ_HAMMERBRO
    0x82, // OBJ_BOOMERANGBRO
    0x86, // OBJ_HEAVYBRO
    0x87, // OBJ_FIREBRO
];

/// Piranha plant variants (including Giant World) — swap among themselves.
const PIRANHAS: &[u8] = &[
    0x7D, // OBJ_BIGGREENPIRANHA
    0x7F, // OBJ_BIGREDPIRANHA
    0xA0, // OBJ_GREENPIRANHA
    0xA2, // OBJ_REDPIRANHA
    0xA4, // OBJ_GREENPIRANHA_FIRE
    0xA6, // OBJ_VENUSFIRETRAP
];
/// Piranha Ceiling / Flipped variants
const PIRANHASC: &[u8] = &[
    0xA1, // OBJ_GREENPIRANHA_FLIPPED
    0xA3, // OBJ_REDPIRANHA_FLIPPED
    0xA5, // OBJ_GREENPIRANHA_FIREC
    0xA7, // OBJ_VENUSFIRETRAP_CEIL
];

/// Thwomp variants — all use CHR page $12/+4 and differ only in movement pattern.
/// Behind the `wild_thwomps` flag (off by default) because random movement
/// directions don't suit corridors designed for specific drop patterns.
const THWOMPS: &[u8] = &[
    0x8A, // OBJ_THWOMP (standard drop)
    0x8B, // OBJ_THWOMPLEFTSLIDE
    0x8C, // OBJ_THWOMPRIGHTSLIDE
    0x8D, // OBJ_THWOMPUPDOWN
    0x8E, // OBJ_THWOMPDIAGONALUL
    0x8F, // OBJ_THWOMPDIAGONALDL
];

/// Enemies whose sprites are taller than a standard 1-tile enemy.
/// When one of these is the replacement in a swap, Y is decremented by 1
/// to prevent the taller sprite from clipping into the floor.
const TALL_ENEMIES: &[u8] = &[
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

/// Cannon-spawned Bullet Bill and Missile Bill. Separate from standalone
/// BULLET_BILLS (0x78/0x79) because these require a cannon tile to spawn.
const CFIRE_BILLS: &[u8] = &[
    0xBC, // OBJ_CFIRE_BULLETBILL
    0xBD, // OBJ_CFIRE_MISSILEBILL
];

/// Cannon fire that travels RIGHT. Behind `wild_cannons`.
const CFIRE_RIGHT: &[u8] = &[
    0xC3, // OBJ_CFIRE_HCANNON_R
    0xCD, // OBJ_CFIRE_PPLANT_RIGHTFIRE
];

/// Cannon fire that travels LEFT. Behind `wild_cannons`.
const CFIRE_LEFT: &[u8] = &[
    0xC4, // OBJ_CFIRE_HCANNON_L
    0xCC, // OBJ_CFIRE_PPLANT_LEFTFIRE
];

/// Cannon fire that travels UP. Behind `wild_cannons`.
const CFIRE_UP: &[u8] = &[
    0xC5, // OBJ_CFIRE_VCANNON_U
    0xCA, // OBJ_CFIRE_PPLANT_UPFIRE
];

/// Cannon fire that travels DOWN. Behind `wild_cannons`.
const CFIRE_DOWN: &[u8] = &[
    0xC6, // OBJ_CFIRE_VCANNON_D
    0xCB, // OBJ_CFIRE_PPLANT_DOWNFIRE
];

/// Bullet Bill variants — standard and homing. Behind the `bullet_bills` flag
/// (on by default) because both are airborne projectiles with similar placement.
const BULLET_BILLS: &[u8] = &[
    0x78, // OBJ_BULLETBILL
    0x79, // OBJ_BULLETBILLHOMING
];

/// Single rotodisc variants — swap rotation direction.
/// Behind the `rotodiscs` flag (off by default).
const ROTODISCS_SINGLE: &[u8] = &[
    0x5A, // OBJ_ROTODISCCLOCKWISE
    0x5B, // OBJ_ROTODISCCCLOCKWISE
];

/// Dual rotodisc variants — swap rotation pattern.
/// Behind the `rotodiscs` flag (off by default).
/// Does NOT include Podoboo from ceiling (0x53) — different behavior entirely.
const ROTODISCS_DUAL: &[u8] = &[
    0x51, // OBJ_ROTODISCDUAL (CW sync)
    0x5E, // OBJ_ROTODISCDUALOPPOSE (opposed H)
    0x5F, // OBJ_ROTODISCDUALOPPOSE2 (opposed V)
    0x60, // OBJ_ROTODISCDUALCCLOCK (CCW sync)
];

/// Ghost house / fortress enemies. Boo and Hot Foot use CHR page $12/+4,
/// Dry Bones uses $13/+5 (compatible with all slot 4 pages).
/// NOT Stretch Boos (0x31/0x32) — attached to platforms, position-critical.
const GHOST_ENEMIES: &[u8] = &[
    0x2F, // OBJ_BOO (Boo Diddly)
    0x30, // OBJ_HOTFOOT_SHY (Hot Foot, shy variant)
    0x3F, // OBJ_DRYBONES
    0x45, // OBJ_HOTFOOT (Hot Foot, walks on floor)
];

// ---------------------------------------------------------------------------
// CHR sprite bank data — extracted from ROM PatTableSel tables
// (PRG001–PRG005, each at bank offset +0x144)
// ---------------------------------------------------------------------------
//
// Each object requests a 1KB CHR page be loaded into one of two sprite bank
// slots: PatTable_BankSel+4 (PPU $1800-$1BFF) or +5 (PPU $1C00-$1FFF).
// If two on-screen objects request different CHR pages for the same slot,
// one renders with garbled sprites (the last one rendered wins).
//
// We track CHR page commitments per enemy data segment (= one sub-area)
// and only allow swaps that are compatible with already-committed pages.
// The two-pass approach pre-commits CHR from ALL non-swappable objects
// before randomizing swappable enemies, preventing ordering-dependent bugs.

/// CHR sprite bank requirement for an object.
pub struct SpriteBank {
    pub chr_page: u8, // CHR ROM page number
    pub slot: u8,     // 4 or 5 (PatTable_BankSel index)
}

/// Look up the CHR sprite bank requirement for any object ID.
/// Returns `None` for objects that use NOCHANGE (no bank switch).
/// Covers ALL object IDs 0x00–0xB3 (from ROM PatTableSel tables) so that
/// the two-pass pre-scan can correctly pre-commit CHR pages from non-swappable
/// objects (platforms, rotodiscs, bosses, fire jets, etc.).
pub fn sprite_bank(id: u8) -> Option<SpriteBank> {
    match id {
        // === Group 1: PRG001 (IDs 0x00–0x23) ===
        // Boss fireball
        0x34 | 0x35 => Some(SpriteBank { chr_page: 0x05, slot: 4 }),
        // MicroGoomba, Poof, DVPlatform
        0x01 | 0x03 | 0x04 | 0x05 | 0x0A | 0x16 | 0x1D =>
            Some(SpriteBank { chr_page: 0x48, slot: 4 }),
        0x02 => Some(SpriteBank { chr_page: 0x4C, slot: 4 }),
        // FireChomp flames
        0x08 => Some(SpriteBank { chr_page: 0x13, slot: 5 }),
        0x09 => Some(SpriteBank { chr_page: 0x37, slot: 5 }),
        // Airship propeller
        0x17 => Some(SpriteBank { chr_page: 0x1A, slot: 4 }),
        // Bowser
        0x18 => Some(SpriteBank { chr_page: 0x3A, slot: 4 }),
        // DVPlatform_Drop3
        0x20 => Some(SpriteBank { chr_page: 0x0A, slot: 4 }),

        // === Group 2: PRG002 (IDs 0x24–0x47) ===
        // Platforms (various)
        0x24 | 0x26 | 0x27 | 0x28 | 0x36 | 0x37 | 0x38 | 0x3C | 0x44 =>
            Some(SpriteBank { chr_page: 0x0E, slot: 4 }),
        // Spike, Patooie, Nipper, NipperHopping, ChainChompStake, BusterBeetle, Lakitu
        0x29 | 0x2A | 0x33 | 0x39 | 0x3D | 0x40 | 0x46 =>
            Some(SpriteBank { chr_page: 0x0A, slot: 4 }),
        // Goomba in Shoe
        0x2B => Some(SpriteBank { chr_page: 0x0B, slot: 4 }),
        // Cloud platform
        0x2C => Some(SpriteBank { chr_page: 0x0E, slot: 4 }),
        // Chain Chomp (strained/staked), Platform ULDR
        0x2D | 0x3E => Some(SpriteBank { chr_page: 0x1A, slot: 4 }),
        // Wood block, Rocket sled
        0x2E | 0x3A => Some(SpriteBank { chr_page: 0x13, slot: 5 }),
        // Boo, Hot Foot shy, Stretch Boos, Hot Foot
        0x2F | 0x30 | 0x31 | 0x32 | 0x45 =>
            Some(SpriteBank { chr_page: 0x12, slot: 4 }),
        // Fire jet left
        0x3B => Some(SpriteBank { chr_page: 0x4F, slot: 5 }),
        // Dry Bones
        0x3F => Some(SpriteBank { chr_page: 0x13, slot: 5 }),
        // Object42, Object43
        0x42 | 0x43 => Some(SpriteBank { chr_page: 0x4F, slot: 5 }),

        // === Group 3: PRG003 (IDs 0x48–0x6B) ===
        0x48 => Some(SpriteBank { chr_page: 0x1A, slot: 4 }),
        0x49 | 0x50 => Some(SpriteBank { chr_page: 0x36, slot: 4 }),
        // Boom-Boom standard
        0x4A => Some(SpriteBank { chr_page: 0x13, slot: 4 }),
        // Boom-Boom fly/split
        0x4B | 0x4C => Some(SpriteBank { chr_page: 0x33, slot: 5 }),
        // Chain Chomp (freed, roaming)
        0x4F => Some(SpriteBank { chr_page: 0x0A, slot: 4 }),
        // Rotodiscs, Podoboo
        0x51 | 0x53 | 0x5A | 0x5B | 0x5E | 0x5F | 0x60 =>
            Some(SpriteBank { chr_page: 0x12, slot: 4 }),
        0x52 => Some(SpriteBank { chr_page: 0x05, slot: 4 }),
        // Missile Bill, Fire Chomp, Wandering Hammer
        0x54 | 0x58 | 0x59 => Some(SpriteBank { chr_page: 0x0E, slot: 4 }),
        // BobOmb
        0x55 => Some(SpriteBank { chr_page: 0x0B, slot: 4 }),
        // Toad House objects
        0x56 | 0x57 => Some(SpriteBank { chr_page: 0x5A, slot: 4 }),
        0x5C => Some(SpriteBank { chr_page: 0x4F, slot: 5 }),
        // Bloopers, Big Bertha, BlooperChildShoot
        0x61 | 0x62 | 0x63 | 0x6A => Some(SpriteBank { chr_page: 0x1A, slot: 4 }),
        // CheepCheep Hopper
        0x64 => Some(SpriteBank { chr_page: 0x4F, slot: 5 }),
        0x67 => Some(SpriteBank { chr_page: 0x1B, slot: 5 }),
        // Lava flotsam
        0x68 | 0x69 => Some(SpriteBank { chr_page: 0x0B, slot: 4 }),
        // Piledriver (micro goomba)
        0x6B => Some(SpriteBank { chr_page: 0x4F, slot: 5 }),

        // === Group 4: PRG004 (IDs 0x6C–0x8F) ===
        // Koopas, Paratroopas, Goomba, Paragoomba, FlyingParatroopa, Cheeps
        0x6C | 0x6D | 0x6E | 0x6F | 0x72 | 0x73 | 0x74 | 0x76 | 0x77 | 0x78 | 0x79 | 0x80 | 0x88 =>
            Some(SpriteBank { chr_page: 0x4F, slot: 5 }),
        // Buzzy Beetle, Spiny, Lakitu, Spiny Egg
        0x70 | 0x71 | 0x83 | 0x84 | 0x85 =>
            Some(SpriteBank { chr_page: 0x0B, slot: 4 }),
        // Big enemies (all variants including Big Piranhas)
        0x7A | 0x7B | 0x7C | 0x7D | 0x7E | 0x7F =>
            Some(SpriteBank { chr_page: 0x3D, slot: 4 }),
        // Bros (Hammer, Boomerang, Heavy, Fire)
        0x81 | 0x82 | 0x86 | 0x87 =>
            Some(SpriteBank { chr_page: 0x4E, slot: 4 }),
        0x89 => Some(SpriteBank { chr_page: 0x0A, slot: 4 }),
        // Thwomps (all variants)
        0x8A | 0x8B | 0x8C | 0x8D | 0x8E | 0x8F =>
            Some(SpriteBank { chr_page: 0x12, slot: 4 }),

        // === Group 5: PRG005 (IDs 0x90–0xB3) ===
        // Moving platforms
        0x90 | 0x91 | 0x92 | 0x93 =>
            Some(SpriteBank { chr_page: 0x4F, slot: 5 }),
        // Big ? Blocks
        0x94 | 0x95 | 0x96 | 0x97 | 0x98 | 0x99 | 0x9A =>
            Some(SpriteBank { chr_page: 0x4C, slot: 4 }),
        // Fire jets (Podoboo fire jet, upward, down, right)
        0x9D | 0xAC | 0xB1 | 0xB2 =>
            Some(SpriteBank { chr_page: 0x37, slot: 5 }),
        // Podoboo fire jet variant 2
        0x9E => Some(SpriteBank { chr_page: 0x12, slot: 4 }),
        0x9F => Some(SpriteBank { chr_page: 0x0E, slot: 4 }),
        // Piranhas (all 8 variants)
        0xA0..=0xA7 => Some(SpriteBank { chr_page: 0x4F, slot: 5 }),
        // Muncher
        0xA8 | 0xA9 => Some(SpriteBank { chr_page: 0x5A, slot: 4 }),
        0xAA | 0xAB | 0xAD | 0xAE | 0xB0 =>
            Some(SpriteBank { chr_page: 0x36, slot: 4 }),
        0xAF => Some(SpriteBank { chr_page: 0x32, slot: 4 }),
        0xB3 => Some(SpriteBank { chr_page: 0x0B, slot: 4 }),

        // IDs 0xB4+ (cannons, autoscroll, etc.) — handled by PRG007,
        // typically NOCHANGE or no visible sprites requiring bank switch
        _ => None,
    }
}

/// CHR slot state: Free (no commitment), Page (committed to a specific page),
/// or Conflicted (two non-swappable objects requested different pages — nothing
/// can safely use this slot).
#[derive(Clone, Copy, PartialEq)]
enum ChrSlot {
    Free,
    Page(u8),
    Conflicted,
}

impl ChrSlot {
    fn is_compatible(self, page: u8) -> bool {
        matches!(self, ChrSlot::Free) || self == ChrSlot::Page(page)
    }

    fn commit(&mut self, page: u8) {
        *self = match *self {
            ChrSlot::Free => ChrSlot::Page(page),
            ChrSlot::Page(p) if p == page => ChrSlot::Page(p),
            _ => ChrSlot::Conflicted,
        };
    }
}

/// Returns true if swapping within this class can never change the CHR page
/// on either slot. This means all slot-4 members share one page and all slot-5
/// members share one page (they can be different pages on different slots).
/// NOCHANGE members are always safe. Uniform classes can be pre-committed in
/// Pass 1 because swapping can't introduce a new page conflict.
fn is_uniform_chr_class(class: &[u8]) -> bool {
    let mut page4: Option<u8> = None;
    let mut page5: Option<u8> = None;
    for &id in class {
        if let Some(sb) = sprite_bank(id) {
            let slot_page = if sb.slot == 4 { &mut page4 } else { &mut page5 };
            match *slot_page {
                None => *slot_page = Some(sb.chr_page),
                Some(p) if p != sb.chr_page => return false,
                _ => {}
            }
        }
    }
    // At least one member must have a bank requirement
    page4.is_some() || page5.is_some()
}

/// Check whether an enemy is compatible with the current CHR page commitments.
fn is_chr_compatible(id: u8, slot4: ChrSlot, slot5: ChrSlot) -> bool {
    match sprite_bank(id) {
        None => true,
        Some(sb) => match sb.slot {
            4 => slot4.is_compatible(sb.chr_page),
            5 => slot5.is_compatible(sb.chr_page),
            _ => true,
        },
    }
}

/// Big ? Block IDs — these can be swapped with each other to randomize
/// which suit/powerup the player gets from Big ? Blocks.
const BIG_Q_BLOCKS: &[u8] = &[
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
const W7F1_TANOOKI_OFFSET: usize = 0x0C9B7;

use super::rom_data::{HAMMER_BRO_SEGMENT_OFFSETS, HB_NEEDS_SHELL_ENEMIES, PROTECTED_ENEMY_OFFSETS, PROTECTED_ENEMY_SEGMENTS, SHELL_PROTECTED_OFFSETS, STOMPABLE_ENEMIES, TANK_BRO_POOL, TANK_BRO_PROTECTED_OFFSETS};

/// Injection candidates for wild_injections mode: special enemies injected after
/// normal swaps. CHR compatibility checked via `sprite_bank()` at filter time.
const WILD_INJECTION_IDS: &[u8] = &[
    0x83, // Lakitu (enemy-spawning variant, CHR $0B/+4)
    0xAF, // Angry Sun
    0x63, // Boss Bass (Big Bertha)
];

/// Probability (out of 256) that a segment will receive an injection when wild_injections is on.
/// ~15% chance per segment.
const WILD_INJECTION_CHANCE: u8 = 38;

/// Maximum X-tile gap between consecutive enemies (sorted by X) before they
/// are split into separate CHR groups. Enemies more than one screen apart
/// can never be visible simultaneously, so they don't need compatible CHR pages.
const CHR_GROUP_GAP: u8 = 16;

/// All cannon sub-class IDs merged for Wild mode.
const ALL_CANNONS: &[u8] = &[
    0xBC, 0xBD, // CFIRE_BILLS
    0xC3, 0xCD, // CFIRE_RIGHT
    0xC4, 0xCC, // CFIRE_LEFT
    0xC5, 0xCA, // CFIRE_UP
    0xC6, 0xCB, // CFIRE_DOWN
];

/// Class-to-mode mapping, built from Options at the start of randomization.
struct ClassModes {
    ground: EnemyMode,
    shell: EnemyMode,
    flying: EnemyMode,
    bullet_bills: EnemyMode,
    piranhas: EnemyMode,
    ghosts: EnemyMode,
    thwomps: EnemyMode,
    rotodiscs: EnemyMode,
    cannons: EnemyMode,
    water: EnemyMode,
    bros: EnemyMode,
}

impl ClassModes {
    fn from_options(opts: &Options) -> Self {
        Self {
            ground: opts.ground,
            shell: opts.shell,
            flying: opts.flying,
            bullet_bills: opts.bullet_bills,
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
    fn build_wild_pool(&self) -> Vec<u8> {
        let mut pool = Vec::new();
        if self.ground == EnemyMode::Wild { pool.extend_from_slice(GROUND_ENEMIES); }
        if self.shell == EnemyMode::Wild { pool.extend_from_slice(SHELL_ENEMIES); }
        if self.flying == EnemyMode::Wild { pool.extend_from_slice(FLYING_ENEMIES); }
        if self.bullet_bills == EnemyMode::Wild { pool.extend_from_slice(BULLET_BILLS); }
        if self.piranhas == EnemyMode::Wild {
            pool.extend_from_slice(PIRANHAS);
            pool.extend_from_slice(PIRANHASC);
        }
        if self.ghosts == EnemyMode::Wild { pool.extend_from_slice(GHOST_ENEMIES); }
        if self.thwomps == EnemyMode::Wild { pool.extend_from_slice(THWOMPS); }
        if self.rotodiscs == EnemyMode::Wild {
            pool.extend_from_slice(ROTODISCS_SINGLE);
            pool.extend_from_slice(ROTODISCS_DUAL);
        }
        if self.cannons == EnemyMode::Wild { pool.extend_from_slice(ALL_CANNONS); }
        if self.water == EnemyMode::Wild { pool.extend_from_slice(WATER_ENEMIES); }
        if self.bros == EnemyMode::Wild { pool.extend_from_slice(BRO_ENEMIES); }
        pool
    }
}

/// Identify which class an enemy ID belongs to, and return the swap pool
/// based on that class's mode. Returns None if the class is Off or unknown.
fn find_class_pool<'a>(
    id: u8, modes: &ClassModes, wild_pool: &'a [u8],
) -> Option<&'a [u8]> {
    // Macro to check class membership and return appropriate pool
    macro_rules! check {
        ($ids:expr, $mode:expr) => {
            if $ids.contains(&id) {
                return match $mode {
                    EnemyMode::Off => None,
                    EnemyMode::Shuffle => Some($ids),
                    EnemyMode::Wild => Some(wild_pool),
                };
            }
        };
    }
    check!(GROUND_ENEMIES, modes.ground);
    check!(SHELL_ENEMIES, modes.shell);
    check!(FLYING_ENEMIES, modes.flying);
    check!(BULLET_BILLS, modes.bullet_bills);
    check!(PIRANHAS, modes.piranhas);
    check!(PIRANHASC, modes.piranhas); // ceiling piranhas share piranhas mode
    check!(GHOST_ENEMIES, modes.ghosts);
    check!(THWOMPS, modes.thwomps);
    check!(ROTODISCS_SINGLE, modes.rotodiscs);
    check!(ROTODISCS_DUAL, modes.rotodiscs);
    check!(WATER_ENEMIES, modes.water);
    check!(BRO_ENEMIES, modes.bros);

    // Cannons: 5 directional sub-classes
    if modes.cannons != EnemyMode::Off {
        for sub in [CFIRE_BILLS, CFIRE_RIGHT, CFIRE_LEFT, CFIRE_UP, CFIRE_DOWN] {
            if sub.contains(&id) {
                return match modes.cannons {
                    EnemyMode::Off => None,
                    EnemyMode::Shuffle => Some(sub), // stay within direction
                    EnemyMode::Wild if !wild_pool.is_empty() => Some(wild_pool),
                    EnemyMode::Wild => Some(ALL_CANNONS), // merge all cannon dirs
                };
            }
        }
    }

    None
}

/// Build a ClassModes for HB encounter segments.
/// In HB segments, the `hb_encounters` mode is the sole authority.
fn hb_class_modes(hb_mode: EnemyMode) -> ClassModes {
    ClassModes {
        ground: hb_mode,
        shell: hb_mode,
        flying: hb_mode,
        bullet_bills: hb_mode,
        piranhas: hb_mode,
        ghosts: hb_mode,
        thwomps: hb_mode,
        rotodiscs: hb_mode,
        cannons: hb_mode,
        water: hb_mode,
        bros: hb_mode,
    }
}

/// Randomize enemies by parsing the structured object data and only swapping
/// object IDs that belong to a known enemy class. Position bytes and all
/// special objects (end-level cards, pipes, platforms, bosses, powerups,
/// autoscroll triggers, cannons, etc.) are never modified.
pub fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R, opts: &Options) {
    randomize_object_data(rom, rng, false, opts);
}

/// Randomize Big ? Blocks by swapping their IDs among the set of Big ? Block
/// types. The Tanooki block in World 7-F1 is protected because flying is
/// required to beat that level.
pub fn randomize_big_q_blocks<R: Rng>(rom: &mut Rom, rng: &mut R) {
    // All enemy classes off — only Big ? Blocks get randomized
    let no_flags = Options {
        ground: EnemyMode::Off, shell: EnemyMode::Off, flying: EnemyMode::Off,
        bullet_bills: EnemyMode::Off, piranhas: EnemyMode::Off,
        ghosts: EnemyMode::Off, thwomps: EnemyMode::Off, rotodiscs: EnemyMode::Off,
        cannons: EnemyMode::Off, water: EnemyMode::Off, bros: EnemyMode::Off,
        hb_encounters: EnemyMode::Off, wild_injections: false,
        ..Options::default()
    };
    randomize_object_data(rom, rng, true, &no_flags);
}

/// Record a CHR page commitment for the given object's bank slot.
/// Detects conflicts: if two objects request different pages on the same slot,
/// the slot becomes Conflicted and no swappable enemy can use it.
fn commit_chr_page(id: u8, slot4: &mut ChrSlot, slot5: &mut ChrSlot) {
    if let Some(sb) = sprite_bank(id) {
        match sb.slot {
            4 => slot4.commit(sb.chr_page),
            5 => slot5.commit(sb.chr_page),
            _ => {}
        }
    }
}

/// Write `new_id` into the enemy slot at `id_index` and nudge X/Y so the
/// replacement sprite lines up with the slot. Bundles the write + adjustment
/// so call sites can't forget one. `_old_id` is captured for future pair-aware
/// adjustments (e.g. piranha→ground Y shift); current rules only use `new_id`:
/// - Tall replacements get Y−1 to avoid floor clipping.
fn swap_enemy(data: &mut [u8], id_index: usize, new_id: u8) {
    let _old_id = data[id_index];
    data[id_index] = new_id;
    if TALL_ENEMIES.contains(&new_id) {
        data[id_index + 2] = data[id_index + 2].wrapping_sub(1);
    }
}

/// Pick a random CHR-compatible enemy from `pool`, or `None` if nothing fits.
fn pick_compatible<R: Rng>(
    pool: &[u8], slot4: ChrSlot, slot5: ChrSlot, rng: &mut R,
) -> Option<u8> {
    let compatible: Vec<u8> = pool
        .iter()
        .copied()
        .filter(|&c| is_chr_compatible(c, slot4, slot5))
        .collect();
    compatible.choose(rng).copied()
}

/// Pre-built page buckets for page-first picking. Built once per segment,
/// reused for every Wild enemy in that segment.
struct PageBuckets {
    /// Each entry is (slot, page, enemy_ids). No-bank enemies are appended to every bucket.
    buckets: Vec<Vec<u8>>,
}

impl PageBuckets {
    /// Build buckets from the wild pool. Groups enemies by (slot, chr_page);
    /// no-bank enemies are added to every bucket so they don't get their own.
    fn build(pool: &[u8]) -> Self {
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

    /// Pick a page uniformly, then pick a CHR-compatible enemy from that page.
    fn pick<R: Rng>(&self, slot4: ChrSlot, slot5: ChrSlot, rng: &mut R) -> Option<u8> {
        // Filter to buckets that have at least one compatible enemy
        let compatible: Vec<&Vec<u8>> = self.buckets.iter()
            .filter(|b| b.iter().any(|&id| is_chr_compatible(id, slot4, slot5)))
            .collect();
        let bucket = *compatible.choose(rng)?;
        let candidates: Vec<u8> = bucket.iter()
            .copied()
            .filter(|&id| is_chr_compatible(id, slot4, slot5))
            .collect();
        candidates.choose(rng).copied()
    }
}

/// A parsed 3-byte entry from the enemy data block.
struct SegmentEntry {
    /// Index into the segment data buffer (points to the obj_id byte)
    data_index: usize,
    /// The object ID
    obj_id: u8,
    /// X tile position (byte 2 of the 3-byte entry)
    x_pos: u8,
}

/// Split entries into proximity groups based on X-position gaps.
/// Entries within `CHR_GROUP_GAP` tiles of their neighbors stay in the same group.
/// Returns groups of entry indices (sorted by X within each group).
fn chr_groups(entries: &[SegmentEntry]) -> Vec<Vec<usize>> {
    if entries.is_empty() {
        return Vec::new();
    }
    let mut sorted: Vec<usize> = (0..entries.len()).collect();
    sorted.sort_by_key(|&i| entries[i].x_pos);

    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = vec![sorted[0]];
    for &idx in &sorted[1..] {
        let last = *current.last().unwrap();
        if entries[idx].x_pos.saturating_sub(entries[last].x_pos) > CHR_GROUP_GAP {
            groups.push(std::mem::take(&mut current));
        }
        current.push(idx);
    }
    groups.push(current);
    groups
}

/// HB Wild segment randomization with stompability constraints.
/// 1-enemy segments: pick from STOMPABLE_ENEMIES only.
/// 2-enemy segments: 5/31 chance for non-stompable path (one from
/// HB_NEEDS_SHELL_ENEMIES + one from SHELL_ENEMIES), otherwise both stompable.
fn randomize_hb_wild_segment<R: Rng>(
    data: &mut [u8],
    entries: &[SegmentEntry],
    hb_modes: &ClassModes,
    hb_wild_pool: &[u8],
    rng: &mut R,
) {
    let swappable: Vec<usize> = entries.iter()
        .enumerate()
        .filter(|(_, e)| find_class_pool(e.obj_id, hb_modes, hb_wild_pool).is_some())
        .map(|(idx, _)| idx)
        .collect();

    // Pre-commit CHR from non-swappable entries
    let mut slot4 = ChrSlot::Free;
    let mut slot5 = ChrSlot::Free;
    for (idx, entry) in entries.iter().enumerate() {
        if !swappable.contains(&idx) {
            commit_chr_page(entry.obj_id, &mut slot4, &mut slot5);
        }
    }

    if swappable.len() == 1 {
        if let Some(chosen) = pick_compatible(STOMPABLE_ENEMIES, slot4, slot5, rng) {
            swap_enemy(data, entries[swappable[0]].data_index, chosen);
        }
    } else if swappable.len() == 2 {
        // Roll whether this segment gets a non-stompable enemy (5/31 ≈ 16%)
        if rng.random_range(..31u32) < 5 {
            // Pick non-stompable, then a shell partner
            if let Some(ns) = pick_compatible(HB_NEEDS_SHELL_ENEMIES, slot4, slot5, rng) {
                let mut s4 = slot4;
                let mut s5 = slot5;
                commit_chr_page(ns, &mut s4, &mut s5);
                if let Some(shell) = pick_compatible(SHELL_ENEMIES, s4, s5, rng) {
                    // Randomly assign which slot gets which
                    let (di0, di1) = (entries[swappable[0]].data_index, entries[swappable[1]].data_index);
                    if rng.random_range(..2u32) == 0 {
                        swap_enemy(data, di0, ns);
                        swap_enemy(data, di1, shell);
                    } else {
                        swap_enemy(data, di0, shell);
                        swap_enemy(data, di1, ns);
                    }
                }
            }
        } else {
            // Both from stompable pool
            if let Some(first) = pick_compatible(STOMPABLE_ENEMIES, slot4, slot5, rng) {
                swap_enemy(data, entries[swappable[0]].data_index, first);
                let mut s4 = slot4;
                let mut s5 = slot5;
                commit_chr_page(first, &mut s4, &mut s5);
                if let Some(second) = pick_compatible(STOMPABLE_ENEMIES, s4, s5, rng) {
                    swap_enemy(data, entries[swappable[1]].data_index, second);
                }
            }
        }
    }
}

fn randomize_object_data<R: Rng>(rom: &mut Rom, rng: &mut R, big_q_only: bool, opts: &Options) {
    let len = ENEMY_DATA_END - ENEMY_DATA_START;
    let mut data = rom.read_range(ENEMY_DATA_START, len).to_vec();

    // Build class modes, wild pool, and pre-bucketed page groups
    let normal_modes = ClassModes::from_options(opts);
    let normal_wild_pool = normal_modes.build_wild_pool();
    let normal_page_buckets = PageBuckets::build(&normal_wild_pool);
    let hb_modes = hb_class_modes(opts.hb_encounters);
    let hb_wild_pool = hb_modes.build_wild_pool();

    let mut i = 0;
    while i < data.len() {
        // 0xFF = segment boundary
        if data[i] == 0xFF {
            i += 1;
            continue;
        }

        // First non-FF byte after a terminator is the page/flag byte
        let seg_start = i;
        let seg_file_offset = ENEMY_DATA_START + seg_start;
        i += 1;

        // Skip entire segment if it's in the protected list
        let skip_segment = PROTECTED_ENEMY_SEGMENTS.contains(&seg_file_offset);
        if skip_segment {
            while i + 2 < data.len() && data[i] != 0xFF {
                i += 3;
            }
            continue;
        }

        // Determine if this is an HB encounter segment
        let is_hb_segment = HAMMER_BRO_SEGMENT_OFFSETS.contains(&seg_file_offset);
        let (modes, wild_pool, page_buckets) = if is_hb_segment {
            (&hb_modes, hb_wild_pool.as_slice(), &normal_page_buckets) // HB uses own wild path
        } else {
            (&normal_modes, normal_wild_pool.as_slice(), &normal_page_buckets)
        };

        // Collect all entries in this segment
        let mut entries: Vec<SegmentEntry> = Vec::new();
        while i + 2 < data.len() && data[i] != 0xFF {
            entries.push(SegmentEntry {
                data_index: i,
                obj_id: data[i],
                x_pos: data[i + 1],
            });
            i += 3;
        }

        // HB Wild: batch-assign enemies with stompability constraints.
        if is_hb_segment && opts.hb_encounters == EnemyMode::Wild && !big_q_only {
            randomize_hb_wild_segment(&mut data, &entries, &hb_modes, &hb_wild_pool, rng);
            continue;
        }

        // Split entries into proximity groups by X-position. Each group gets
        // independent CHR slot tracking — enemies more than CHR_GROUP_GAP tiles
        // apart can never be on-screen together, so they don't need compatible
        // CHR pages.
        let groups = chr_groups(&entries);

        for group in &groups {
            // Two-pass approach per CHR group:
            // Pass 1: pre-commit CHR pages from non-swappable objects AND uniform-CHR
            // classes (all members share the same page/slot, so swapping can't change it).
            let mut committed_slot4 = ChrSlot::Free;
            let mut committed_slot5 = ChrSlot::Free;

            if !big_q_only {
                for &idx in group {
                    let entry = &entries[idx];
                    let should_precommit = match find_class_pool(entry.obj_id, modes, wild_pool) {
                        None => !BOOMBOOM_IDS.contains(&entry.obj_id),
                        Some(pool) if std::ptr::eq(pool, wild_pool) => false,
                        Some(class) => is_uniform_chr_class(class),
                    };
                    if should_precommit {
                        commit_chr_page(entry.obj_id, &mut committed_slot4, &mut committed_slot5);
                    }
                }
            }

            // Pass 2: randomize swappable entries respecting pre-commitments
            for &idx in group {
                let entry = &entries[idx];
                let file_offset = ENEMY_DATA_START + entry.data_index;

                if big_q_only {
                    if BIG_Q_BLOCKS.contains(&entry.obj_id)
                        && file_offset != W7F1_TANOOKI_OFFSET
                    {
                        data[entry.data_index] = *BIG_Q_BLOCKS.choose(rng).unwrap();
                    }
                } else if BOOMBOOM_SWAP.contains(&data[entry.data_index]) {
                    data[entry.data_index] = *BOOMBOOM_SWAP.choose(rng).unwrap();
                } else if PROTECTED_ENEMY_OFFSETS.contains(&file_offset) {
                    commit_chr_page(entry.obj_id, &mut committed_slot4, &mut committed_slot5);
                } else if SHELL_PROTECTED_OFFSETS.contains(&file_offset) && modes.shell != EnemyMode::Off {
                    if let Some(chosen) = pick_compatible(SHELL_ENEMIES, committed_slot4, committed_slot5, rng) {
                        swap_enemy(&mut data, entry.data_index, chosen);
                        commit_chr_page(chosen, &mut committed_slot4, &mut committed_slot5);
                    }
                } else if TANK_BRO_PROTECTED_OFFSETS.contains(&file_offset) && modes.bros != EnemyMode::Off {
                    if let Some(chosen) = pick_compatible(TANK_BRO_POOL, committed_slot4, committed_slot5, rng) {
                        swap_enemy(&mut data, entry.data_index, chosen);
                        commit_chr_page(chosen, &mut committed_slot4, &mut committed_slot5);
                    }
                } else if let Some(pool) = find_class_pool(entry.obj_id, modes, wild_pool) {
                    let chosen = if std::ptr::eq(pool, wild_pool) {
                        page_buckets.pick(committed_slot4, committed_slot5, rng)
                    } else {
                        pick_compatible(pool, committed_slot4, committed_slot5, rng)
                    };
                    if let Some(chosen) = chosen {
                        swap_enemy(&mut data, entry.data_index, chosen);
                        commit_chr_page(chosen, &mut committed_slot4, &mut committed_slot5);
                    }
                }
            }

            // Pass 3 (wild_injections only): inject Lakitu/Angry Sun/Boss Bass.
            if opts.wild_injections && !big_q_only {
                let roll: u8 = rng.random_range(..=255);
                if roll < WILD_INJECTION_CHANCE {
                    let swappable_indices: Vec<usize> = group.iter()
                        .copied()
                        .filter(|&idx| {
                            let fo = ENEMY_DATA_START + entries[idx].data_index;
                            !PROTECTED_ENEMY_OFFSETS.contains(&fo)
                                && find_class_pool(data[entries[idx].data_index], modes, wild_pool).is_some()
                        })
                        .collect();

                    if let Some(&target_idx) = swappable_indices.choose(rng) {
                        if let Some(chosen) = pick_compatible(WILD_INJECTION_IDS, committed_slot4, committed_slot5, rng) {
                            let di = entries[target_idx].data_index;
                            swap_enemy(&mut data, di, chosen);
                            if chosen == 0xAF {
                                // Angry Sun: override to fixed sky position
                                data[di + 1] = 0x02;
                                data[di + 2] = 0x11;
                            } else if chosen == 0x83 {
                                // Lakitu: override to fixed sky position
                                data[di + 1] = 0x02;
                                data[di + 2] = 0x12;
                            }
                            commit_chr_page(chosen, &mut committed_slot4, &mut committed_slot5);
                        }
                    }
                }
            }
        }
    }

    rom.write_range(ENEMY_DATA_START, &data);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    /// Options with all default enemy classes enabled (Shuffle mode).
    fn enemy_opts() -> Options {
        Options::default()
    }

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        // iNES header
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16; // PRG pages
        data[5] = 16; // CHR pages
        data[6] = 0x40; // mapper flags

        // Set up a realistic enemy data segment at ENEMY_DATA_START:
        // FF terminator, then a segment with page flag + entries + FF
        let seg = &[
            0xFF, // leading terminator
            0x01, // page flag
            0x72, 0x0E, 0x19, // Goomba at (0x0E, 0x19)
            0x6C, 0x24, 0x16, // Green Troopa at (0x24, 0x16)
            0xA6, 0x16, 0x17, // Venus Fire Trap at (0x16, 0x17)
            0x41, 0xA8, 0x15, // End Level Card at (0xA8, 0x15) — must not change
            0xD3, 0x00, 0x50, // Autoscroll — must not change
            0xFF, // terminator
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);

        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_enemies_stay_in_class() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng, &enemy_opts());

        // Read back the segment (skip FF + page flag = offset 2)
        let base = ENEMY_DATA_START + 2;
        let result = rom.read_range(base, 15);

        // Goomba should be replaced with a ground enemy
        assert!(
            GROUND_ENEMIES.contains(&result[0]),
            "Goomba replaced with non-ground: 0x{:02X}",
            result[0]
        );
        // X must be unchanged; Y may be decremented by 1 for tall enemies
        assert_eq!(result[1], 0x0E);
        let expected_y = if TALL_ENEMIES.contains(&result[0]) { 0x18 } else { 0x19 };
        assert_eq!(result[2], expected_y,
            "Goomba slot Y: got 0x{:02X}, expected 0x{:02X} (replacement 0x{:02X})",
            result[2], expected_y, result[0]);

        // Green Troopa should be replaced with a shell enemy
        assert!(
            SHELL_ENEMIES.contains(&result[3]),
            "Green Troopa replaced with non-shell enemy: 0x{:02X}",
            result[3]
        );
        assert_eq!(result[4], 0x24);
        let expected_y = if TALL_ENEMIES.contains(&result[3]) { 0x15 } else { 0x16 };
        assert_eq!(result[5], expected_y,
            "Troopa slot Y: got 0x{:02X}, expected 0x{:02X} (replacement 0x{:02X})",
            result[5], expected_y, result[3]);

        // Venus Fire Trap should be replaced with a piranha
        assert!(
            PIRANHAS.contains(&result[6]),
            "Venus replaced with non-piranha: 0x{:02X}",
            result[6]
        );

        // End Level Card must NOT be changed
        assert_eq!(result[9], 0x41, "End Level Card was modified!");
        assert_eq!(result[10], 0xA8);
        assert_eq!(result[11], 0x15);

        // Autoscroll must NOT be changed
        assert_eq!(result[12], 0xD3, "Autoscroll was modified!");
    }

    #[test]
    fn test_deterministic() {
        let mut rom1 = make_test_rom();
        let mut rom2 = make_test_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(77);
        let mut rng2 = ChaCha8Rng::seed_from_u64(77);

        randomize(&mut rom1, &mut rng1, &enemy_opts());
        randomize(&mut rom2, &mut rng2, &enemy_opts());

        let len = ENEMY_DATA_END - ENEMY_DATA_START;
        assert_eq!(
            rom1.read_range(ENEMY_DATA_START, len),
            rom2.read_range(ENEMY_DATA_START, len),
        );
    }

    fn make_bigq_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        // Segment with a regular Big ? Block (should be randomized)
        let seg1_start = ENEMY_DATA_START;
        let seg1 = &[
            0xFF,
            0x01, // page flag
            0x94, 0x18, 0x05, // BIGQBLOCK_3UP
            0x98, 0x16, 0x14, // BIGQBLOCK_TANOOKI
            0x41, 0xA8, 0x15, // ENDLEVELCARD (must not change)
            0xFF,
        ];
        data[seg1_start..seg1_start + seg1.len()].copy_from_slice(seg1);

        // Place the protected W7 Big Q Tanooki at its exact file offset
        // W7F1_TANOOKI_OFFSET = 0x0C9B7, which is the ID byte of the entry.
        // We need: [FF] [page] [0x98, x, y] [0x41, x, y] [FF]
        // So page byte at 0x0C9B6, entry at 0x0C9B7
        let w7f1_seg_start = W7F1_TANOOKI_OFFSET - 2; // FF + page byte before the entry
        data[w7f1_seg_start] = 0xFF;
        data[w7f1_seg_start + 1] = 0x01; // page flag
        data[W7F1_TANOOKI_OFFSET] = 0x98; // BIGQBLOCK_TANOOKI
        data[W7F1_TANOOKI_OFFSET + 1] = 0x0A;
        data[W7F1_TANOOKI_OFFSET + 2] = 0x13;
        data[W7F1_TANOOKI_OFFSET + 3] = 0x41; // ENDLEVELCARD
        data[W7F1_TANOOKI_OFFSET + 4] = 0x48;
        data[W7F1_TANOOKI_OFFSET + 5] = 0x15;
        data[W7F1_TANOOKI_OFFSET + 6] = 0xFF;

        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_big_q_blocks_randomized() {
        let mut rom = make_bigq_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize_big_q_blocks(&mut rom, &mut rng);

        // Regular Big ? Blocks should be randomized to some Big ? Block ID
        let base = ENEMY_DATA_START + 2; // skip FF + page
        let result = rom.read_range(base, 9);
        assert!(
            BIG_Q_BLOCKS.contains(&result[0]),
            "Big Q block not replaced with Big Q: 0x{:02X}",
            result[0]
        );
        assert!(
            BIG_Q_BLOCKS.contains(&result[3]),
            "Big Q block not replaced with Big Q: 0x{:02X}",
            result[3]
        );
        // End level card must not change
        assert_eq!(result[6], 0x41);
    }

    #[test]
    fn test_chr_compatibility_enforced() {
        // Place a Goomba ($4F/+5) and Dry Bones ($13/+5) in the same segment.
        // After randomization, both must use compatible CHR pages on slot +5.
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        let seg = &[
            0xFF,
            0x01, // page flag
            0x72, 0x10, 0x19, // Goomba (slot +5, page $4F)
            0x3F, 0x20, 0x19, // Dry Bones (slot +5, page $13)
            0x29, 0x30, 0x19, // Spike (slot +4, page $0A)
            0x71, 0x40, 0x19, // Spiny (slot +4, page $0B)
            0xFF,
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);
        let rom = Rom::from_bytes(&data).unwrap();

        // Run many times to exercise different random paths
        for seed in 0..200u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            let base = ENEMY_DATA_START + 2;
            let result = rom_copy.read_range(base, 12);
            let enemy1 = result[0]; // was Goomba
            let enemy2 = result[3]; // was Dry Bones
            let enemy3 = result[6]; // was Spike
            let enemy4 = result[9]; // was Spiny

            // Each must stay in its class
            assert!(GROUND_ENEMIES.contains(&enemy1), "seed {seed}: enemy1 0x{enemy1:02X}");
            assert!(GHOST_ENEMIES.contains(&enemy2), "seed {seed}: enemy2 0x{enemy2:02X}");
            assert!(GROUND_ENEMIES.contains(&enemy3), "seed {seed}: enemy3 0x{enemy3:02X}");
            assert!(GROUND_ENEMIES.contains(&enemy4), "seed {seed}: enemy4 0x{enemy4:02X}");

            // Check CHR compatibility: no two enemies in the same segment
            // should request different CHR pages for the same bank slot.
            let enemies = [enemy1, enemy2, enemy3, enemy4];
            let mut seen_slot4: Option<u8> = None;
            let mut seen_slot5: Option<u8> = None;
            for &e in &enemies {
                if let Some(sb) = sprite_bank(e) {
                    match sb.slot {
                        4 => {
                            if let Some(prev) = seen_slot4 {
                                assert_eq!(
                                    prev, sb.chr_page,
                                    "seed {seed}: slot +4 conflict: 0x{prev:02X} vs 0x{:02X} (enemy 0x{e:02X})",
                                    sb.chr_page
                                );
                            }
                            seen_slot4 = Some(sb.chr_page);
                        }
                        5 => {
                            if let Some(prev) = seen_slot5 {
                                assert_eq!(
                                    prev, sb.chr_page,
                                    "seed {seed}: slot +5 conflict: 0x{prev:02X} vs 0x{:02X} (enemy 0x{e:02X})",
                                    sb.chr_page
                                );
                            }
                            seen_slot5 = Some(sb.chr_page);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    #[test]
    fn test_chr_resets_across_segments() {
        // Two segments: first has a Spike ($0A/+4), second has a Spiny ($0B/+4).
        // They should be able to choose independently since they're in different segments.
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        let seg = &[
            0xFF,
            0x01,             // page flag
            0x29, 0x10, 0x19, // Spike (slot +4, page $0A)
            0xFF,             // segment boundary
            0x01,             // page flag
            0x71, 0x20, 0x19, // Spiny (slot +4, page $0B)
            0xFF,
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);
        let rom = Rom::from_bytes(&data).unwrap();

        // Run many times — Spiny in second segment should freely choose
        // any ground enemy, not be constrained by first segment's Spike.
        let mut saw_slot4_0a_in_seg2 = false;
        let mut saw_slot4_0b_in_seg2 = false;
        for seed in 0..200u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            // Second segment's enemy is at offset: FF(1) + page(1) + entry(3) + FF(1) + page(1) = 7
            let enemy2 = rom_copy.read_byte(ENEMY_DATA_START + 7);
            assert!(GROUND_ENEMIES.contains(&enemy2), "seed {seed}: 0x{enemy2:02X}");

            if let Some(sb) = sprite_bank(enemy2) {
                if sb.slot == 4 && sb.chr_page == 0x0A {
                    saw_slot4_0a_in_seg2 = true;
                }
                if sb.slot == 4 && sb.chr_page == 0x0B {
                    saw_slot4_0b_in_seg2 = true;
                }
            }
        }
        // Over 200 seeds, we should see both CHR page variants in segment 2
        assert!(
            saw_slot4_0a_in_seg2 && saw_slot4_0b_in_seg2,
            "Segment 2 should not be constrained by segment 1's CHR choice"
        );
    }

    #[test]
    fn test_7f1_tanooki_protected() {
        let mut rom = make_bigq_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(99);
        randomize_big_q_blocks(&mut rom, &mut rng);

        // The 7-F1 Tanooki must remain 0x98
        let protected = rom.read_byte(W7F1_TANOOKI_OFFSET);
        assert_eq!(
            protected, 0x98,
            "7-F1 Tanooki was changed to 0x{:02X}!",
            protected
        );
    }

    #[test]
    fn test_ghost_enemies_stay_in_class() {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        let seg = &[
            0xFF,
            0x01,
            0x2F, 0x10, 0x08, // Boo
            0x45, 0x20, 0x18, // Hot Foot
            0xFF,
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);
        let rom = Rom::from_bytes(&data).unwrap();

        for seed in 0..100u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            let base = ENEMY_DATA_START + 2;
            let result = rom_copy.read_range(base, 6);
            assert!(GHOST_ENEMIES.contains(&result[0]), "seed {seed}: ghost1 0x{:02X}", result[0]);
            assert!(GHOST_ENEMIES.contains(&result[3]), "seed {seed}: ghost2 0x{:02X}", result[3]);
        }
    }

    #[test]
    fn test_big_enemies_in_regular_classes() {
        // Big enemies are merged into their regular-sized counterparts' classes:
        // BigGreenTroopa → SHELL_ENEMIES, BigGreenPiranha/BigRedPiranha → PIRANHAS
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        let seg = &[
            0xFF,
            0x01,
            0x7A, 0x10, 0x10, // BigGreenTroopa
            0x7D, 0x20, 0x10, // BigGreenPiranha
            0x7F, 0x30, 0x10, // BigRedPiranha
            0xFF,
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);
        let rom = Rom::from_bytes(&data).unwrap();

        for seed in 0..100u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            let base = ENEMY_DATA_START + 2;
            let result = rom_copy.read_range(base, 9);
            assert!(SHELL_ENEMIES.contains(&result[0]), "seed {seed}: big troopa 0x{:02X}", result[0]);
            assert!(PIRANHAS.contains(&result[3]), "seed {seed}: big piranha1 0x{:02X}", result[3]);
            assert!(PIRANHAS.contains(&result[6]), "seed {seed}: big piranha2 0x{:02X}", result[6]);
        }
    }

    #[test]
    fn test_two_pass_precommit() {
        // Regression test for the CHR ordering bug:
        // A swappable ground enemy (Spike, $0A/+4) appears BEFORE a Boo ($12/+4,
        // uniform ghost class — pre-committed in pass 1). Without two-pass, the
        // Spike could be swapped to something that commits a conflicting slot+4 page.
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        let seg = &[
            0xFF,
            0x01,
            // Swappable ground enemy BEFORE uniform-class ghost
            0x29, 0x10, 0x19, // Spike ($0A/+4) — swappable, mixed-CHR class
            0x2F, 0x20, 0x08, // Boo ($12/+4) — swappable, uniform-CHR class (pre-committed)
            0xFF,
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);
        let rom = Rom::from_bytes(&data).unwrap();

        for seed in 0..500u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            let base = ENEMY_DATA_START + 2;
            let result = rom_copy.read_range(base, 6);
            let enemy = result[0];
            let ghost = result[3];

            // Ghost must stay in ghost class
            assert!(GHOST_ENEMIES.contains(&ghost), "seed {seed}: ghost changed to 0x{ghost:02X}");

            // The swapped ground enemy must be CHR-compatible with Boo's $12/+4.
            assert!(GROUND_ENEMIES.contains(&enemy), "seed {seed}: enemy 0x{enemy:02X}");
            if let Some(sb) = sprite_bank(enemy) {
                if sb.slot == 4 {
                    assert_eq!(
                        sb.chr_page, 0x12,
                        "seed {seed}: enemy 0x{enemy:02X} has slot+4 page 0x{:02X}, \
                         conflicts with Boo's $12",
                        sb.chr_page
                    );
                }
            }
        }
    }

    #[test]
    fn test_uniform_class_precommit() {
        // Boo ($12/+4, uniform ghost class) + ground enemy in same segment.
        // The ground enemy must never commit a conflicting slot+4 page because
        // uniform classes are pre-committed in pass 1.
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        let seg = &[
            0xFF,
            0x01,
            0x72, 0x10, 0x19, // Goomba ($4F/+5) — ground, mixed-CHR
            0x2F, 0x20, 0x08, // Boo ($12/+4) — ghost, uniform-CHR
            0xFF,
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);
        let rom = Rom::from_bytes(&data).unwrap();

        for seed in 0..500u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            let base = ENEMY_DATA_START + 2;
            let result = rom_copy.read_range(base, 6);
            let ground = result[0];
            let ghost = result[3];

            assert!(GROUND_ENEMIES.contains(&ground), "seed {seed}: ground 0x{ground:02X}");
            assert!(GHOST_ENEMIES.contains(&ghost), "seed {seed}: ghost 0x{ghost:02X}");

            // No slot+4 conflict: ground enemy's slot+4 must match Boo's $12 or not use slot+4
            if let Some(sb) = sprite_bank(ground) {
                if sb.slot == 4 {
                    assert_eq!(
                        sb.chr_page, 0x12,
                        "seed {seed}: ground 0x{ground:02X} slot+4=0x{:02X} conflicts with Boo's $12",
                        sb.chr_page
                    );
                }
            }
        }
    }

    #[test]
    fn test_conflicted_slot_blocks_all() {
        // Two non-swappable objects with different +4 pages in the same segment.
        // Slot+4 becomes Conflicted, so any swappable enemy needing slot+4 gets
        // no compatible candidates and must keep its original ID.
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        let seg = &[
            0xFF,
            0x01,
            0x51, 0x10, 0x08, // Rotodisc CW ($12/+4) — non-swappable
            0x4A, 0x20, 0x18, // Boom-Boom std ($13/+4) — non-swappable
            0x29, 0x30, 0x19, // Spike ($0A/+4) — swappable ground enemy
            0xFF,
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);
        let rom = Rom::from_bytes(&data).unwrap();

        for seed in 0..100u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            let base = ENEMY_DATA_START + 2;
            let result = rom_copy.read_range(base, 9);

            // Non-swappable objects must not change
            assert_eq!(result[0], 0x51, "seed {seed}: rotodisc changed");
            assert_eq!(result[3], 0x4A, "seed {seed}: boom-boom changed");

            // Spike: slot+4 is conflicted ($12 vs $13), so only ground enemies
            // that don't use slot+4 (use slot+5 or NOCHANGE) can be chosen.
            // If all ground enemies need slot+4, Spike keeps original.
            let enemy = result[6];
            assert!(GROUND_ENEMIES.contains(&enemy), "seed {seed}: enemy 0x{enemy:02X}");
            if let Some(sb) = sprite_bank(enemy) {
                // Must NOT use slot+4 (it's conflicted)
                assert_ne!(sb.slot, 4,
                    "seed {seed}: enemy 0x{enemy:02X} uses conflicted slot+4 page 0x{:02X}",
                    sb.chr_page
                );
            }
        }
    }

    #[test]
    fn test_kuribo_shoe_in_ground_class() {
        assert!(GROUND_ENEMIES.contains(&0x2B), "Kuribo's Shoe Goomba missing from ground class");
        let modes = ClassModes::from_options(&enemy_opts());
        let wild_pool = modes.build_wild_pool();
        assert_eq!(find_class_pool(0x2B, &modes, &wild_pool), Some(GROUND_ENEMIES as &[u8]));
    }

    #[test]
    fn test_chain_chomp_fire_chomp_in_ground() {
        assert!(GROUND_ENEMIES.contains(&0x4F), "Chain Chomp (freed) missing from ground class");
        assert!(!GROUND_ENEMIES.contains(&0x2C), "0x2C (cloud platform) must NOT be in ground class");
        assert!(GROUND_ENEMIES.contains(&0x58), "Fire Chomp missing from ground class");
        let modes = ClassModes::from_options(&enemy_opts());
        let wild_pool = modes.build_wild_pool();
        assert_eq!(find_class_pool(0x4F, &modes, &wild_pool), Some(GROUND_ENEMIES as &[u8]));
        assert_eq!(find_class_pool(0x58, &modes, &wild_pool), Some(GROUND_ENEMIES as &[u8]));
    }

    #[test]
    fn test_wild_pool_merges_classes() {
        // With ground=Wild, shell=Wild, others=Shuffle: ground↔shell swaps happen
        let flags = Options {
            ground: EnemyMode::Wild,
            shell: EnemyMode::Wild,
            flying: EnemyMode::Shuffle,
            ..Options::default()
        };
        let modes = ClassModes::from_options(&flags);
        let wild_pool = modes.build_wild_pool();
        // Ground and shell IDs should be in the wild pool
        assert!(wild_pool.contains(&0x72)); // Goomba
        assert!(wild_pool.contains(&0x6C)); // GreenTroopa
        // Flying should NOT be in wild pool (it's Shuffle, not Wild)
        assert!(!wild_pool.contains(&0x6E)); // Paratroopa
        // Ground enemy → returns wild pool
        let pool = find_class_pool(0x72, &modes, &wild_pool).unwrap();
        assert!(pool.contains(&0x6C), "wild pool should contain shell enemies");
        // Flying → returns own class only
        let fly_pool = find_class_pool(0x6E, &modes, &wild_pool).unwrap();
        assert_eq!(fly_pool, FLYING_ENEMIES);

        // Run many seeds and confirm cross-class swaps happen
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16; data[5] = 16; data[6] = 0x40;
        let seg = &[
            0xFF, 0x01,
            0x72, 0x10, 0x19, // Goomba (ground)
            0xFF,
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);
        let rom = Rom::from_bytes(&data).unwrap();

        let mut saw_shell = false;
        for seed in 0..500u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &flags);
            let result_id = rom_copy.read_byte(ENEMY_DATA_START + 2);
            assert!(
                wild_pool.contains(&result_id),
                "seed {seed}: 0x{result_id:02X} not in wild pool"
            );
            if SHELL_ENEMIES.contains(&result_id) {
                saw_shell = true;
            }
        }
        assert!(saw_shell, "500 seeds and never saw a ground→shell swap");
    }

    #[test]
    fn test_off_mode_leaves_untouched() {
        // With ground=Off, ground enemies should stay vanilla
        let flags = Options {
            ground: EnemyMode::Off,
            shell: EnemyMode::Shuffle,
            ..Options::default()
        };
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16; data[5] = 16; data[6] = 0x40;
        let seg = &[
            0xFF, 0x01,
            0x72, 0x10, 0x19, // Goomba (ground - Off)
            0x6C, 0x20, 0x16, // GreenTroopa (shell - Shuffle)
            0xFF,
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);
        let rom = Rom::from_bytes(&data).unwrap();

        for seed in 0..100u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &flags);
            // Ground enemy stays vanilla (Off mode)
            assert_eq!(rom_copy.read_byte(ENEMY_DATA_START + 2), 0x72,
                "seed {seed}: ground enemy should stay vanilla in Off mode");
            // Shell enemy can change
            let shell = rom_copy.read_byte(ENEMY_DATA_START + 5);
            assert!(SHELL_ENEMIES.contains(&shell), "seed {seed}: shell 0x{shell:02X}");
        }
    }

    #[test]
    fn test_wild_fortress_tier_merges() {
        // With ghosts=Wild, thwomps=Wild, rotodiscs=Wild: they all share one pool
        let flags = Options {
            ghosts: EnemyMode::Wild,
            thwomps: EnemyMode::Wild,
            rotodiscs: EnemyMode::Wild,
            ..Options::default()
        };
        let modes = ClassModes::from_options(&flags);
        let wild_pool = modes.build_wild_pool();
        // Ghost, thwomp, rotodisc IDs should all be in the wild pool
        assert!(wild_pool.contains(&0x2F)); // Boo
        assert!(wild_pool.contains(&0x8A)); // Thwomp
        assert!(wild_pool.contains(&0x51)); // Rotodisc
        // All return the same wild pool
        assert_eq!(find_class_pool(0x2F, &modes, &wild_pool), Some(wild_pool.as_slice()));
        assert_eq!(find_class_pool(0x8A, &modes, &wild_pool), Some(wild_pool.as_slice()));
        assert_eq!(find_class_pool(0x51, &modes, &wild_pool), Some(wild_pool.as_slice()));
    }

    #[test]
    fn test_wild_injection_occurs() {
        // Run many seeds with wild_injections on, confirm at least one injection
        let flags = Options {
            wild_injections: true,
            ..Options::default()
        };
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16; data[5] = 16; data[6] = 0x40;
        let seg = &[
            0xFF, 0x01,
            0x72, 0x10, 0x19, // Goomba
            0x72, 0x20, 0x19, // Goomba
            0x72, 0x30, 0x19, // Goomba
            0x72, 0x40, 0x19, // Goomba
            0xFF,
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);
        let rom = Rom::from_bytes(&data).unwrap();

        let injection_ids: &[u8] = &[0x83, 0xAF, 0x63];
        let mut saw_injection = false;
        for seed in 0..2000u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &flags);
            for off in [2, 5, 8, 11] {
                let id = rom_copy.read_byte(ENEMY_DATA_START + off);
                if injection_ids.contains(&id) {
                    saw_injection = true;
                    break;
                }
            }
            if saw_injection { break; }
        }
        assert!(saw_injection, "2000 seeds and never saw an injection");
    }

    #[test]
    fn test_wild_injection_respects_chr() {
        // Pre-commit slot 4 to an incompatible page via a non-swappable object.
        let flags = Options {
            wild_injections: true,
            ..Options::default()
        };
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16; data[5] = 16; data[6] = 0x40;
        let seg = &[
            0xFF, 0x01,
            0x18, 0x05, 0x10, // Bowser (slot 4, page 0x3A — incompatible with all injections)
            0x72, 0x10, 0x19, // Goomba
            0x72, 0x20, 0x19, // Goomba
            0xFF,
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);
        let rom = Rom::from_bytes(&data).unwrap();

        let injection_ids: &[u8] = &[0x83, 0xAF, 0x63];
        for seed in 0..500u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &flags);
            assert_eq!(rom_copy.read_byte(ENEMY_DATA_START + 2), 0x18);
            for off in [5, 8] {
                let id = rom_copy.read_byte(ENEMY_DATA_START + off);
                assert!(
                    !injection_ids.contains(&id),
                    "seed {seed}: injection 0x{id:02X} despite slot 4 conflict"
                );
            }
        }
    }

    #[test]
    fn test_chr_groups_split_distant_enemies() {
        // Two enemies far apart (screen 0 vs screen 5) should get independent
        // CHR groups. A Boo ($12/+4) on screen 0 should NOT block a ground enemy
        // on screen 5 from picking a non-$12 slot+4 page.
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        let seg = &[
            0xFF,
            0x01,
            0x2F, 0x04, 0x08, // Boo ($12/+4) at x=4 (screen 0)
            0x29, 0x50, 0x19, // Spike ($0A/+4) at x=80 (screen 5)
            0xFF,
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);
        let rom = Rom::from_bytes(&data).unwrap();

        // Under segment-wide tracking, Spike would be locked to $12/+4 enemies only.
        // Under distance-based grouping, Spike should freely pick any ground enemy.
        let mut saw_non_12_slot4 = false;
        for seed in 0..500u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            let ghost = rom_copy.read_byte(ENEMY_DATA_START + 2);
            let ground = rom_copy.read_byte(ENEMY_DATA_START + 5);
            assert!(GHOST_ENEMIES.contains(&ghost), "seed {seed}: ghost 0x{ghost:02X}");
            assert!(GROUND_ENEMIES.contains(&ground), "seed {seed}: ground 0x{ground:02X}");

            if let Some(sb) = sprite_bank(ground) {
                if sb.slot == 4 && sb.chr_page != 0x12 {
                    saw_non_12_slot4 = true;
                }
            }
        }
        assert!(saw_non_12_slot4,
            "500 seeds: distant ground enemy never picked a non-$12 slot+4 page — grouping not working");
    }

    #[test]
    fn test_chr_groups_keep_close_together() {
        // Two enemies close together (10 tiles apart) should still share
        // CHR constraints — same behavior as before grouping.
        // Goomba ($4F/+5) won't conflict with Boo ($12/+4) on slot+4,
        // so we can verify that any slot+4 ground enemy picked must be $12.
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        let seg = &[
            0xFF,
            0x01,
            0x2F, 0x08, 0x08, // Boo ($12/+4) at x=8
            0x72, 0x12, 0x19, // Goomba ($4F/+5) at x=18 (10 tiles away, same group)
            0xFF,
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);
        let rom = Rom::from_bytes(&data).unwrap();

        // Boo pre-commits $12/+4 as uniform ghost class, so the ground enemy
        // must be compatible — any slot+4 pick must be $12 (or use slot+5 only).
        for seed in 0..500u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            let ground = rom_copy.read_byte(ENEMY_DATA_START + 5);
            assert!(GROUND_ENEMIES.contains(&ground), "seed {seed}: ground 0x{ground:02X}");
            if let Some(sb) = sprite_bank(ground) {
                if sb.slot == 4 {
                    assert_eq!(sb.chr_page, 0x12,
                        "seed {seed}: close enemy 0x{ground:02X} has slot+4 page 0x{:02X}, \
                         conflicts with Boo's $12", sb.chr_page);
                }
            }
        }
    }

    #[test]
    fn test_chr_groups_basic() {
        // Verify the grouping function itself
        let entries = vec![
            SegmentEntry { data_index: 0, obj_id: 0x72, x_pos: 5 },
            SegmentEntry { data_index: 3, obj_id: 0x72, x_pos: 10 },
            SegmentEntry { data_index: 6, obj_id: 0x72, x_pos: 80 },
            SegmentEntry { data_index: 9, obj_id: 0x72, x_pos: 85 },
        ];
        let groups = chr_groups(&entries);
        assert_eq!(groups.len(), 2, "should split into 2 groups");
        assert_eq!(groups[0].len(), 2, "first group: x=5, x=10");
        assert_eq!(groups[1].len(), 2, "second group: x=80, x=85");
    }

    #[test]
    fn test_chr_groups_single() {
        // All entries close together — one group
        let entries = vec![
            SegmentEntry { data_index: 0, obj_id: 0x72, x_pos: 5 },
            SegmentEntry { data_index: 3, obj_id: 0x72, x_pos: 10 },
            SegmentEntry { data_index: 6, obj_id: 0x72, x_pos: 20 },
        ];
        let groups = chr_groups(&entries);
        assert_eq!(groups.len(), 1, "all within gap — one group");
        assert_eq!(groups[0].len(), 3);
    }
}
