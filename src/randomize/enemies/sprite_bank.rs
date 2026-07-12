//! Sprite CHR-bank model: which graphics page each enemy needs, and the
//! slot-commitment bookkeeping that keeps on-screen swaps from garbling tiles.

pub struct SpriteBank {
    pub chr_page: u8, // CHR ROM page number
    pub slot: u8,     // 4 or 5 (PatTable_BankSel index)
}

/// Look up the CHR sprite bank requirement for any object ID.
/// Returns `None` for objects that use NOCHANGE (no bank switch).
/// Covers ALL object IDs 0x00–0xB3 (from ROM PatTableSel tables) so that
/// the two-pass pre-scan can correctly pre-commit CHR pages from non-swappable
/// objects (platforms, rotodiscs, bosses, fire jets, etc.), plus the cannon
/// fire family 0xBC–0xD0 whose requirements come from in-routine bank writes
/// and spawned children rather than a PatTableSel entry (see the cfire arm).
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
        // Spike, Patooie, Nipper, NipperHopping, NipperFireBreather, BusterBeetle, PiranhaSpikeBall
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
        0x7A..=0x7F =>
            Some(SpriteBank { chr_page: 0x3D, slot: 4 }),
        // Bros (Hammer, Boomerang, Heavy, Fire)
        0x81 | 0x82 | 0x86 | 0x87 =>
            Some(SpriteBank { chr_page: 0x4E, slot: 4 }),
        0x89 => Some(SpriteBank { chr_page: 0x0A, slot: 4 }),
        // Thwomps (all variants)
        0x8A..=0x8F =>
            Some(SpriteBank { chr_page: 0x12, slot: 4 }),

        // === Group 5: PRG005 (IDs 0x90–0xB3) ===
        // Moving platforms
        0x90..=0x93 =>
            Some(SpriteBank { chr_page: 0x4F, slot: 5 }),
        // Big ? Blocks
        0x94..=0x9A =>
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

        // === Cannon fire family (IDs 0xBC–0xD0, CFire handlers in PRG007) ===
        // These spawners have no PatTableSel dispatch entry, but they have
        // real CHR needs (verified in southbird prg007.asm):
        // - Bullet/Missile Bill cannons spawn 0x78/0x79 and Goomba pipes
        //   spawn OBJ_GOOMBA (0x72) — the children's own PatTableSel demands
        //   $4F on slot +5.
        0xBC | 0xBD | 0xC0 | 0xC1 => Some(SpriteBank { chr_page: 0x4F, slot: 5 }),
        // - `CFire_Cannonball` (all cannonballs, BIG cannonballs, and the
        //   bob-omb launchers) and `CFire_4Way` write PatTable_BankSel+4=$36
        //   UNCONDITIONALLY every frame they're loaded — before their own
        //   timer/off-screen early-outs — and a spawned cfire slot survives
        //   until 8 newer cfires push it out of the FIFO, i.e. usually the
        //   rest of the level. Level-wide slot-4 pin (they're in CHASER_IDS
        //   for exactly that reason).
        // - The Rocky Wrench cfire (0xBE) spawns OBJ_ROCKYWRENCH (0xAD),
        //   which needs the same $36/+4 via its own PatTableSel.
        0xBE | 0xBF | 0xC2..=0xCF => Some(SpriteBank { chr_page: 0x36, slot: 4 }),

        // Everything else 0xB4+ (laser cfire 0xD0, autoscroll controllers,
        // etc.) — no bank switch and no spawned sprite bank need.
        _ => None,
    }
}

/// CHR slot state: Free (no commitment), Page (committed to a specific page),
/// or Conflicted (two non-swappable objects requested different pages — nothing
/// can safely use this slot).
#[derive(Clone, Copy, PartialEq)]
pub(super) enum ChrSlot {
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
pub(super) fn is_uniform_chr_class(class: &[u8]) -> bool {
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

/// CHR commitments a pick consults. `local` is the proximity group's slots —
/// what can share the screen with the entry under the one-screen model.
/// `segment` is every commitment anywhere in the segment (pinned pages from
/// all groups, plus every pick made so far): the bar a level-wide chaser must
/// clear, since it follows the player into every group (see `CHASER_IDS`).
#[derive(Clone, Copy)]
pub(super) struct ChrCtx {
    pub(super) local: (ChrSlot, ChrSlot),
    pub(super) segment: (ChrSlot, ChrSlot),
}

/// Check whether an enemy is compatible with the current CHR page commitments.
pub(super) fn is_chr_compatible(id: u8, slot4: ChrSlot, slot5: ChrSlot) -> bool {
    match sprite_bank(id) {
        None => true,
        Some(sb) => match sb.slot {
            4 => slot4.is_compatible(sb.chr_page),
            5 => slot5.is_compatible(sb.chr_page),
            _ => true,
        },
    }
}

/// Record a CHR page commitment for the given object's bank slot.
/// Detects conflicts: if two objects request different pages on the same slot,
/// the slot becomes Conflicted and no swappable enemy can use it.
pub(super) fn commit_chr_page(id: u8, slot4: &mut ChrSlot, slot5: &mut ChrSlot) {
    if let Some(sb) = sprite_bank(id) {
        match sb.slot {
            4 => slot4.commit(sb.chr_page),
            5 => slot5.commit(sb.chr_page),
            _ => {}
        }
    }
}
