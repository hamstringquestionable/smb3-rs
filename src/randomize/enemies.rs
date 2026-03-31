use rand::Rng;
use rand::seq::IndexedRandom;

use crate::rom::Rom;

/// Flags controlling which enemy classes are eligible for randomization.
pub struct EnemyFlags {
    pub enemies: bool,
    pub bullet_bills: bool,
    pub wild_thwomps: bool,
    pub wild_cannons: bool,
    pub wild_rotodiscs: bool,
}

impl Default for EnemyFlags {
    fn default() -> Self {
        EnemyFlags {
            enemies: true,
            bullet_bills: true,
            wild_thwomps: false,
            wild_cannons: false,
            wild_rotodiscs: false,
        }
    }
}

/// Enemy/object data block: 0x0BFD8–0x0E00D.
///
/// Format: each level's enemy set is a sequence of segments separated by 0xFF.
/// Each segment starts with a 1-byte page flag, then zero or more 3-byte
/// entries: [object_id, x_pos, y_pos], terminated by 0xFF.
///
/// We parse this structure properly and only randomize the object_id byte
/// of entries whose ID is in our explicit allowlist of swappable enemies.
const ENEMY_DATA_START: usize = 0x0BFD8;
const ENEMY_DATA_END: usize = 0x0E00D;

/// Boom-Boom boss IDs — excluded from CHR pre-commit because they occupy
/// far-right boss rooms, screens away from corridor enemies. The player
/// never sees both on-screen simultaneously, so different CHR pages on
/// the same slot won't cause visible sprite garbling.
const BOOMBOOM_IDS: &[u8] = &[0x4A, 0x4B, 0x4C];

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
    0x33, // OBJ_NIPPER (stationary)
    0x39, // OBJ_NIPPERHOPPING
    0x3F, // OBJ_DRYBONES
    0x40, // OBJ_BUSTERBEATLE
    0x55, // OBJ_BOBOMB
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
    0x61, // OBJ_BLOOPERWITHKIDS
    0x62, // OBJ_BLOOPER
    0x63, // OBJ_BIGBERTHABIRTHER
    0x64, // OBJ_CHEEPCHEEPHOPPER
    0x6A, // OBJ_BLOOPERCHILDSHOOT
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

/// Cheep cheep variants (overworld jumping types).
const CHEEPS: &[u8] = &[
    0x77, // OBJ_GREENCHEEP
    0x88, // OBJ_ORANGECHEEP
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

/// Cannon fire variants (OBJ_CFIRE_*) — 21 types covering cannons, pipe launchers,
/// and projectile emitters in various directions. Behind the `wild_cannons` flag
/// (off by default) because swapping fire directions creates chaotic gameplay.
const CANNONS: &[u8] = &[
    0xBC, 0xBD, 0xBE, 0xBF, 0xC0, 0xC1, 0xC2, 0xC3,
    0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xCB,
    0xCC, 0xCD, 0xCE, 0xCF, 0xD0,
];

/// Bullet Bill variants — standard and homing. Behind the `bullet_bills` flag
/// (on by default) because both are airborne projectiles with similar placement.
const BULLET_BILLS: &[u8] = &[
    0x78, // OBJ_BULLETBILL
    0x79, // OBJ_BULLETBILLHOMING
];

/// Rotodisc variants — single and dual, various rotation directions.
/// Behind the `wild_rotodiscs` flag (off by default) because rotation
/// direction matters for designed fortress corridors.
/// Does NOT include Podoboo from ceiling (0x53) — different behavior entirely.
const ROTODISCS: &[u8] = &[
    0x51, // OBJ_ROTODISCDUAL (CW sync)
    0x5A, // OBJ_ROTODISCCLOCKWISE
    0x5B, // OBJ_ROTODISCCCLOCKWISE
    0x5E, // OBJ_ROTODISCDUALOPPOSE (opposed H)
    0x5F, // OBJ_ROTODISCDUALOPPOSE2 (opposed V)
    0x60, // OBJ_ROTODISCDUALCCLOCK (CCW sync)
];

/// Ghost house enemies — Boo and Hot Foot variants. All use CHR page $12/+4.
/// NOT Stretch Boos (0x31/0x32) — attached to platforms, position-critical.
const GHOST_ENEMIES: &[u8] = &[
    0x2F, // OBJ_BOO (Boo Diddly)
    0x30, // OBJ_HOTFOOT_SHY (Hot Foot, shy variant)
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
struct SpriteBank {
    chr_page: u8, // CHR ROM page number
    slot: u8,     // 4 or 5 (PatTable_BankSel index)
}

/// Look up the CHR sprite bank requirement for any object ID.
/// Returns `None` for objects that use NOCHANGE (no bank switch).
/// Covers ALL object IDs 0x00–0xB3 (from ROM PatTableSel tables) so that
/// the two-pass pre-scan can correctly pre-commit CHR pages from non-swappable
/// objects (platforms, rotodiscs, bosses, fire jets, etc.).
fn sprite_bank(id: u8) -> Option<SpriteBank> {
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
        // Spike, Patooie, Nipper, NipperHopping, BusterBeetle, Airship anchor, WonderWing
        0x29 | 0x2A | 0x33 | 0x39 | 0x3D | 0x40 | 0x46 =>
            Some(SpriteBank { chr_page: 0x0A, slot: 4 }),
        // Goomba in Shoe
        0x2B => Some(SpriteBank { chr_page: 0x0B, slot: 4 }),
        // Chain Chomp
        0x2C => Some(SpriteBank { chr_page: 0x0E, slot: 4 }),
        // Chain Chomp (strained), Platform ULDR
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
        // Koopas, Paratroopas, Goomba, Paragoomba, FlyingParatroopa, OrangeCheep
        0x6C | 0x6D | 0x6E | 0x6F | 0x72 | 0x73 | 0x74 | 0x76 | 0x78 | 0x79 | 0x80 | 0x88 =>
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

/// Returns true if all members of a class share the same CHR page and slot.
/// Uniform classes can be pre-committed in Pass 1 because swapping within the
/// class can never change the CHR page.
fn is_uniform_chr_class(class: &[u8]) -> bool {
    let first = match sprite_bank(class[0]) {
        Some(sb) => (sb.chr_page, sb.slot),
        None => return false,
    };
    class[1..].iter().all(|&id| {
        sprite_bank(id).is_some_and(|sb| sb.chr_page == first.0 && sb.slot == first.1)
    })
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

use super::rom_data::PROTECTED_ENEMY_SEGMENTS;

/// All swap classes collected for lookup.
const ALL_CLASSES: &[&[u8]] = &[
    GROUND_ENEMIES,
    SHELL_ENEMIES,
    FLYING_ENEMIES,
    WATER_ENEMIES,
    BRO_ENEMIES,
    PIRANHAS,
    PIRANHASC,
    CHEEPS,
    GHOST_ENEMIES,
];

/// Find which class an enemy ID belongs to, if any.
/// Core classes require the `enemies` flag. Bullet Bills, Thwomps, Cannons,
/// and Rotodiscs have their own flags.
fn find_class(id: u8, flags: &EnemyFlags) -> Option<&'static [u8]> {
    if flags.enemies {
        for class in ALL_CLASSES {
            if class.contains(&id) {
                return Some(class);
            }
        }
    }
    if flags.bullet_bills && BULLET_BILLS.contains(&id) {
        return Some(BULLET_BILLS);
    }
    if flags.wild_thwomps && THWOMPS.contains(&id) {
        return Some(THWOMPS);
    }
    if flags.wild_cannons && CANNONS.contains(&id) {
        return Some(CANNONS);
    }
    if flags.wild_rotodiscs && ROTODISCS.contains(&id) {
        return Some(ROTODISCS);
    }
    None
}

/// Randomize enemies by parsing the structured object data and only swapping
/// object IDs that belong to a known enemy class. Position bytes and all
/// special objects (end-level cards, pipes, platforms, bosses, powerups,
/// autoscroll triggers, cannons, etc.) are never modified.
pub fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R, flags: &EnemyFlags) {
    randomize_object_data(rom, rng, false, flags);
}

/// Randomize Big ? Blocks by swapping their IDs among the set of Big ? Block
/// types. The Tanooki block in World 7-F1 is protected because flying is
/// required to beat that level.
pub fn randomize_big_q_blocks<R: Rng>(rom: &mut Rom, rng: &mut R) {
    let no_flags = EnemyFlags {
        enemies: false, bullet_bills: false,
        wild_thwomps: false, wild_cannons: false, wild_rotodiscs: false,
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

/// A parsed 3-byte entry from the enemy data block.
struct SegmentEntry {
    /// Index into the segment data buffer (points to the obj_id byte)
    data_index: usize,
    /// The object ID
    obj_id: u8,
}

fn randomize_object_data<R: Rng>(rom: &mut Rom, rng: &mut R, big_q_only: bool, flags: &EnemyFlags) {
    let len = ENEMY_DATA_END - ENEMY_DATA_START;
    let mut data = rom.read_range(ENEMY_DATA_START, len).to_vec();

    let mut i = 0;
    while i < data.len() {
        // 0xFF = segment boundary
        if data[i] == 0xFF {
            i += 1;
            continue;
        }

        // First non-FF byte after a terminator is the page/flag byte
        let seg_start = i;
        i += 1;

        // Skip entire segment if it's in the protected list
        let skip_segment = PROTECTED_ENEMY_SEGMENTS.contains(&(ENEMY_DATA_START + seg_start));
        if skip_segment {
            while i + 2 < data.len() && data[i] != 0xFF {
                i += 3;
            }
            continue;
        }

        // Collect all entries in this segment
        let mut entries: Vec<SegmentEntry> = Vec::new();
        while i + 2 < data.len() && data[i] != 0xFF {
            entries.push(SegmentEntry {
                data_index: i,
                obj_id: data[i],
            });
            i += 3;
        }

        // Two-pass approach:
        // Pass 1: pre-commit CHR pages from non-swappable objects AND uniform-CHR
        // classes (all members share the same page/slot, so swapping can't change it).
        // This prevents the ordering bug where a swappable enemy earlier in
        // the segment commits a CHR page that conflicts with a later fixed object.
        let mut committed_slot4 = ChrSlot::Free;
        let mut committed_slot5 = ChrSlot::Free;

        if !big_q_only {
            for entry in &entries {
                let should_precommit = match find_class(entry.obj_id, flags) {
                    None => !BOOMBOOM_IDS.contains(&entry.obj_id), // non-swappable, but skip Boom-Boom
                    Some(class) => is_uniform_chr_class(class),  // uniform: page invariant
                };
                if should_precommit {
                    commit_chr_page(entry.obj_id, &mut committed_slot4, &mut committed_slot5);
                }
            }
        }

        // Pass 2: randomize swappable entries respecting pre-commitments
        for entry in &entries {
            let file_offset = ENEMY_DATA_START + entry.data_index;

            if big_q_only {
                if BIG_Q_BLOCKS.contains(&entry.obj_id)
                    && file_offset != W7F1_TANOOKI_OFFSET
                {
                    data[entry.data_index] = *BIG_Q_BLOCKS.choose(rng).unwrap();
                }
            } else if let Some(class) = find_class(entry.obj_id, flags) {
                let compatible: Vec<u8> = class
                    .iter()
                    .copied()
                    .filter(|&c| is_chr_compatible(c, committed_slot4, committed_slot5))
                    .collect();

                if !compatible.is_empty() {
                    let chosen = *compatible.choose(rng).unwrap();
                    data[entry.data_index] = chosen;
                    commit_chr_page(chosen, &mut committed_slot4, &mut committed_slot5);
                }
                // else: no compatible candidates, keep original (safe fallback)
            }
            // Non-swappable entries already pre-committed in pass 1
        }
    }

    rom.write_range(ENEMY_DATA_START, &data);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

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
        randomize(&mut rom, &mut rng, &EnemyFlags::default());

        // Read back the segment (skip FF + page flag = offset 2)
        let base = ENEMY_DATA_START + 2;
        let result = rom.read_range(base, 15);

        // Goomba should be replaced with a ground enemy
        assert!(
            GROUND_ENEMIES.contains(&result[0]),
            "Goomba replaced with non-ground: 0x{:02X}",
            result[0]
        );
        // Position bytes must be unchanged
        assert_eq!(result[1], 0x0E);
        assert_eq!(result[2], 0x19);

        // Green Troopa should be replaced with a shell enemy
        assert!(
            SHELL_ENEMIES.contains(&result[3]),
            "Green Troopa replaced with non-shell enemy: 0x{:02X}",
            result[3]
        );
        assert_eq!(result[4], 0x24);
        assert_eq!(result[5], 0x16);

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

        randomize(&mut rom1, &mut rng1, &EnemyFlags::default());
        randomize(&mut rom2, &mut rng2, &EnemyFlags::default());

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
            randomize(&mut rom_copy, &mut rng, &EnemyFlags::default());

            let base = ENEMY_DATA_START + 2;
            let result = rom_copy.read_range(base, 12);
            let enemy1 = result[0]; // was Goomba
            let enemy2 = result[3]; // was Dry Bones
            let enemy3 = result[6]; // was Spike
            let enemy4 = result[9]; // was Spiny

            // All must still be ground enemies
            assert!(GROUND_ENEMIES.contains(&enemy1), "seed {seed}: enemy1 0x{enemy1:02X}");
            assert!(GROUND_ENEMIES.contains(&enemy2), "seed {seed}: enemy2 0x{enemy2:02X}");
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
        // Two segments: first has a Goomba ($4F/+5), second has a Dry Bones ($13/+5).
        // They should be able to choose independently since they're in different segments.
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        let seg = &[
            0xFF,
            0x01,             // page flag
            0x72, 0x10, 0x19, // Goomba (slot +5, page $4F)
            0xFF,             // segment boundary
            0x01,             // page flag
            0x3F, 0x20, 0x19, // Dry Bones (slot +5, page $13)
            0xFF,
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);
        let rom = Rom::from_bytes(&data).unwrap();

        // Run many times — Dry Bones in second segment should freely choose
        // any ground enemy, not be constrained by first segment's Goomba.
        let mut saw_slot5_4f_in_seg2 = false;
        let mut saw_slot5_13_in_seg2 = false;
        for seed in 0..200u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &EnemyFlags::default());

            // Second segment's enemy is at offset: FF(1) + page(1) + entry(3) + FF(1) + page(1) = 7
            let enemy2 = rom_copy.read_byte(ENEMY_DATA_START + 7);
            assert!(GROUND_ENEMIES.contains(&enemy2), "seed {seed}: 0x{enemy2:02X}");

            if let Some(sb) = sprite_bank(enemy2) {
                if sb.slot == 5 && sb.chr_page == 0x4F {
                    saw_slot5_4f_in_seg2 = true;
                }
                if sb.slot == 5 && sb.chr_page == 0x13 {
                    saw_slot5_13_in_seg2 = true;
                }
            }
        }
        // Over 200 seeds, we should see both CHR page variants in segment 2
        assert!(
            saw_slot5_4f_in_seg2 && saw_slot5_13_in_seg2,
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
            randomize(&mut rom_copy, &mut rng, &EnemyFlags::default());

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
            randomize(&mut rom_copy, &mut rng, &EnemyFlags::default());

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
            randomize(&mut rom_copy, &mut rng, &EnemyFlags::default());

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
            randomize(&mut rom_copy, &mut rng, &EnemyFlags::default());

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
            randomize(&mut rom_copy, &mut rng, &EnemyFlags::default());

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
        // Verify 0x2B (Goomba in Shoe) is in the ground enemy class
        assert!(GROUND_ENEMIES.contains(&0x2B), "Kuribo's Shoe Goomba missing from ground class");
        assert!(find_class(0x2B, &EnemyFlags::default()) == Some(GROUND_ENEMIES));
    }
}
