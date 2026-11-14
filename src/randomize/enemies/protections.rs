//! Per-level enemy protection registry.
//!
//! One table (`LEVEL_PROTECTIONS`) with a row per protected level area that
//! the enemy randomization passes must treat specially. Each row names the
//! area the way `rominfo --level` does (a level name, then the area number it
//! prints: 0 is the main area), gives the reason, and declares:
//!   - `walker_segment`: how the walker pass treats the containing $FF segment
//!     (`Default`, `Skip`, or `HammerBro`). Non-`Default` rows also block
//!     wild injection at this entry_ptr. The Hammer Bro arenas themselves
//!     need no row: every stream in `HAMMER_BRO_OBJ_PTRS` is `HammerBro`.
//!   - `entries`: per-entry rules keyed by absolute file offset.
//!
//! Adding a new protection is one block here. `enemy_ptr` and the entry
//! offsets are what the code reads; `every_row_names_the_area_its_offsets_are_in`
//! checks them against the named area on the vanilla ROM, so a row cannot
//! name one level and protect another's bytes.
//! The walker (`enemies::randomize_object_data`) and the wild injection it
//! runs per segment (`enemies::inject_first_enemy`) consume the registry
//! through the derived helpers (`entry_protection_at`, `walker_segment_rule_at`)
//! — never the table directly.

use super::tables::{BEHIND_BG, Group, HAZARDS, ROTODISCS};
use crate::randomize::rom_data::{HAMMER_BRO_OBJ_PTRS, enemy_ptr_to_file_offset};

/// One logical level or sub-area with protections that affect enemy
/// randomization.
struct LevelProtection {
    // Reason: `level` and `area` are read only by the test that checks them
    // against the level index; `why` is documentation in the table.
    #[allow(dead_code)]
    /// The level, as `rominfo --level` names it.
    level: &'static str,
    #[allow(dead_code)]
    /// Which of its areas, numbered as `rominfo --level` prints them.
    area: usize,
    #[allow(dead_code)]
    why: &'static str,
    /// The area's objects pointer.
    enemy_ptr: u16,
    walker_segment: WalkerSegmentRule,
    entries: &'static [EntryRule],
}

/// How the walker treats the $FF-bounded segment containing this level's
/// enemy_ptr.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum WalkerSegmentRule {
    /// Default class-based swaps.
    Default,
    /// Skip the area's whole object list (no swaps at all). Every area has a
    /// list of its own, so this never reaches another area.
    Skip,
    /// Hammer Bro encounter — walker uses HB-specific modes and pool.
    HammerBro,
}

/// Per-entry rule attached to a specific 3-byte enemy entry by its absolute
/// file offset.
struct EntryRule {
    offset: usize,
    /// The vanilla enemy at `offset`. Read only by
    /// `every_row_names_the_area_its_offsets_are_in`, so a row a few bytes
    /// off (on the next enemy in the same area) fails instead of protecting
    /// the wrong one.
    #[allow(dead_code)]
    id: u8,
    rule: EntryProtection,
}

/// Per-entry behavior. The walker applies these inline during its swap pass;
/// `inject_first_enemy` also skips injection at any position carrying one
/// of these (so wild injection can't bypass a per-entry safeguard).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum EntryProtection {
    /// Walker skips this entry (pre-commits CHR page only).
    SkipSwap,
    /// Walker forces a pick from SHELL_ENEMIES when shell mode is on.
    ForceShell,
    /// Walker forces a pick from the entry's natural pool ∩ STOMPABLE_ENEMIES.
    ForceStompable,
    /// Walker excludes a group of enemies from the chosen pool (additive-only:
    /// one sharing an ID list with the vanilla enemy here is kept). See
    /// `Group` in `enemies/tables.rs`.
    Exclude(Group),
}

const LEVEL_PROTECTIONS: &[LevelProtection] = &[
    // --- Whole-level skips ---
    LevelProtection {
        level: "3-2",
        area: 0,
        why: "enemies-as-platforms, sprite-overload risk",
        enemy_ptr: 0xCA23,
        walker_segment: WalkerSegmentRule::Skip,
        entries: &[],
    },
    // --- Individual gameplay-critical entries (walker skips, no whole-level block) ---
    LevelProtection {
        level: "8-1",
        area: 0,
        why: "FlyingRedParatroopa required for progression; Boo restricted from path-blocking hazards",
        enemy_ptr: 0xC424,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            // Boo is swap-safe, but a hazard here (Ptooie/nipper/lotus/etc.) would
            // block a narrow pathway the player must pass through — so swap it to
            // anything except a hazard rather than skipping it outright.
            EntryRule { offset: 0x0C456, id: 0x2F, rule: EntryProtection::Exclude(HAZARDS) }, // Boo scr=5 col=1
            EntryRule { offset: 0x0C465, id: 0x6F, rule: EntryProtection::SkipSwap }, // FlyingRedParatroopa scr=6 col=14
        ],
    },
    LevelProtection {
        level: "6-3",
        area: 0,
        why: "FlyingRedParatroopas required as platforms",
        enemy_ptr: 0xCA8E,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0CAB1, id: 0x6F, rule: EntryProtection::SkipSwap }, // scr=6 col=13
            EntryRule { offset: 0x0CAB4, id: 0x6F, rule: EntryProtection::SkipSwap }, // scr=7 col=1
        ],
    },
    // --- Shell-locked entries (shells needed to break bricks for progression) ---
    LevelProtection {
        level: "2-Pyr",
        area: 1,
        why: "Buzzy Beetles needed to break bricks",
        enemy_ptr: 0xC5BC,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0C5CD, id: 0x70, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5D0, id: 0x70, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5D3, id: 0x70, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5D6, id: 0x70, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5DC, id: 0x70, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5DF, id: 0x70, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5E2, id: 0x70, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5E5, id: 0x70, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5E8, id: 0x70, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5EB, id: 0x70, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5F1, id: 0x70, rule: EntryProtection::ForceShell },
        ],
    },
    LevelProtection {
        level: "2-3",
        area: 0,
        why: "shells needed to break end-of-level bricks",
        enemy_ptr: 0xD1F0,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0D22B, id: 0x6C, rule: EntryProtection::ForceShell }, // GreenTroopa scr=8 col=11
            EntryRule { offset: 0x0D22E, id: 0x6C, rule: EntryProtection::ForceShell }, // GreenTroopa scr=8 col=13
        ],
    },
    LevelProtection {
        level: "7-1",
        area: 1,
        why: "scr=0 GreenTroopas kept in the shell pool",
        enemy_ptr: 0xCD93,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0CDA7, id: 0x6C, rule: EntryProtection::ForceShell }, // GreenTroopa scr=0 col=2
            EntryRule { offset: 0x0CDAA, id: 0x6C, rule: EntryProtection::ForceShell }, // GreenTroopa scr=0 col=4
        ],
    },
    LevelProtection {
        level: "6-5",
        area: 1,
        why: "shell needed for progression",
        enemy_ptr: 0xC5EB,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0C60E, id: 0x6C, rule: EntryProtection::ForceShell }, // GreenTroopa scr=4 col=10
        ],
    },
    // --- Stompable-locked entries ---
    LevelProtection {
        level: "6-6",
        area: 1,
        why: "floor spikes — non-stompable swap would corner player; screen 8 floor Cheep must not become a hazard; screen 8 piranhas are underwater",
        enemy_ptr: 0xC64B,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0C698, id: 0x77, rule: EntryProtection::Exclude(HAZARDS) }, // GreenCheep scr=8 col=13 row=15
            // Underwater pipes: a Rocky Wrench or giant piranha here is
            // invisible behind the water tiles (see BEHIND_BG).
            EntryRule { offset: 0x0C69E, id: 0xA2, rule: EntryProtection::Exclude(BEHIND_BG) }, // RedPiranha scr=8 col=4
            EntryRule { offset: 0x0C6A4, id: 0xA2, rule: EntryProtection::Exclude(BEHIND_BG) }, // RedPiranha scr=8 col=12
            EntryRule { offset: 0x0C6A7, id: 0x29, rule: EntryProtection::ForceStompable }, // Spike scr=10 col=0
            EntryRule { offset: 0x0C6AA, id: 0x29, rule: EntryProtection::ForceStompable }, // Spike scr=10 col=6
            EntryRule { offset: 0x0C6AD, id: 0x29, rule: EntryProtection::ForceStompable }, // Spike scr=10 col=4
        ],
    },
    LevelProtection {
        level: "5-4",
        area: 0,
        why: "ParatroopaGreenHop guards a pipe; a hazard there would block it",
        enemy_ptr: 0xD039,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0D074, id: 0x6E, rule: EntryProtection::Exclude(HAZARDS) }, // ParatroopaGreenHop scr=7 col=12 row=6
        ],
    },
    // --- Bro-fight rooms reached by a non-bro map object ---
    // A treasure-box room is cleared to be left, so it needs the bro-fight
    // pool: every enemy in it must be permanently killable. These two are
    // fight-shaped rooms the player reaches through the Coin Ship / Tank map
    // sprite rather than a bro sprite, so nothing else classifies them.
    // A `$81` in one is rewritten into something unkillable (entered via
    // `MAPOBJ_TANK`, it resolves to the invisible `OBJ_WARPHIDE`); that half is
    // handled generically by `rewrites_hammer_bro`.
    LevelProtection {
        level: "8-Tank",
        area: 1,
        why: "bro fight via the Tank sprite",
        enemy_ptr: 0xDA29,
        walker_segment: WalkerSegmentRule::HammerBro,
        entries: &[],
    },
    // --- Hazard-excluded entries (no Patooie/Lavalotus on player walking path) ---
    LevelProtection {
        level: "7F2",
        area: 1,
        why: "tight boss arena",
        enemy_ptr: 0xD45C,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0D46D, id: 0x5A, rule: EntryProtection::Exclude(HAZARDS) }, // Rotodisc CW scr=1 col=0
            EntryRule { offset: 0x0D470, id: 0x3F, rule: EntryProtection::Exclude(HAZARDS) }, // DryBones    scr=1 col=1
            EntryRule { offset: 0x0D473, id: 0x3F, rule: EntryProtection::Exclude(HAZARDS) }, // DryBones    scr=1 col=3
            EntryRule { offset: 0x0D476, id: 0x5A, rule: EntryProtection::Exclude(HAZARDS) }, // Rotodisc CW scr=1 col=9
            EntryRule { offset: 0x0D479, id: 0x8A, rule: EntryProtection::Exclude(HAZARDS) }, // Thwomp      scr=1 col=10
        ],
    },
    LevelProtection {
        level: "7-5",
        area: 1,
        why: "open horizontal field — hazards at floor level unfair",
        enemy_ptr: 0xC171,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0C182, id: 0x6E, rule: EntryProtection::Exclude(HAZARDS) }, // ParatroopaGreenHop scr=0 col=12
            EntryRule { offset: 0x0C185, id: 0x6E, rule: EntryProtection::Exclude(HAZARDS) }, // ParatroopaGreenHop scr=1 col=2
            EntryRule { offset: 0x0C18E, id: 0x55, rule: EntryProtection::Exclude(HAZARDS) }, // BobOmb             scr=2 col=5
            EntryRule { offset: 0x0C191, id: 0x55, rule: EntryProtection::Exclude(HAZARDS) }, // BobOmb             scr=2 col=7
            EntryRule { offset: 0x0C194, id: 0x55, rule: EntryProtection::Exclude(HAZARDS) }, // BobOmb             scr=2 col=9
            EntryRule { offset: 0x0C1A0, id: 0x6E, rule: EntryProtection::Exclude(HAZARDS) }, // ParatroopaGreenHop scr=4 col=14
            EntryRule { offset: 0x0C1A3, id: 0x6E, rule: EntryProtection::Exclude(HAZARDS) }, // ParatroopaGreenHop scr=5 col=1
            EntryRule { offset: 0x0C1A6, id: 0x6E, rule: EntryProtection::Exclude(HAZARDS) }, // ParatroopaGreenHop scr=5 col=4
        ],
    },
    LevelProtection {
        level: "β4",
        area: 1,
        why: "narrow corridor — hazards on Buzzy Beetle path unfair",
        enemy_ptr: 0xC7A7,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0C7B8, id: 0x70, rule: EntryProtection::Exclude(HAZARDS) }, // BuzzyBeatle scr=1 col=2
            EntryRule { offset: 0x0C7BB, id: 0x70, rule: EntryProtection::Exclude(HAZARDS) }, // BuzzyBeatle scr=1 col=5
            EntryRule { offset: 0x0C7BE, id: 0x70, rule: EntryProtection::Exclude(HAZARDS) }, // BuzzyBeatle scr=1 col=9
            EntryRule { offset: 0x0C7CA, id: 0x70, rule: EntryProtection::Exclude(HAZARDS) }, // BuzzyBeatle scr=2 col=11
            EntryRule { offset: 0x0C7CD, id: 0x70, rule: EntryProtection::Exclude(HAZARDS) }, // BuzzyBeatle scr=3 col=2
            EntryRule { offset: 0x0C7D0, id: 0x70, rule: EntryProtection::Exclude(HAZARDS) }, // BuzzyBeatle scr=3 col=4
        ],
    },
    LevelProtection {
        level: "4F1",
        area: 0,
        why: "narrow-hallway fort — a hazard anywhere blocks the only path",
        enemy_ptr: 0xD528,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0D539, id: 0x30, rule: EntryProtection::Exclude(HAZARDS) }, // HotFootShy      scr=1 col=0
            EntryRule { offset: 0x0D53C, id: 0x30, rule: EntryProtection::Exclude(HAZARDS) }, // HotFootShy      scr=1 col=8
            EntryRule { offset: 0x0D53F, id: 0x30, rule: EntryProtection::Exclude(HAZARDS) }, // HotFootShy      scr=2 col=7
            EntryRule { offset: 0x0D542, id: 0x8B, rule: EntryProtection::Exclude(HAZARDS) }, // ThwompLeftSlide scr=2 col=2
            EntryRule { offset: 0x0D545, id: 0x8B, rule: EntryProtection::Exclude(HAZARDS) }, // ThwompLeftSlide scr=3 col=0
            EntryRule { offset: 0x0D548, id: 0x30, rule: EntryProtection::Exclude(HAZARDS) }, // HotFootShy      scr=3 col=2
            EntryRule { offset: 0x0D54B, id: 0x30, rule: EntryProtection::Exclude(HAZARDS) }, // HotFootShy      scr=3 col=10
            EntryRule { offset: 0x0D54E, id: 0x8C, rule: EntryProtection::Exclude(HAZARDS) }, // ThwompRightSlide scr=4 col=1
            EntryRule { offset: 0x0D551, id: 0x30, rule: EntryProtection::Exclude(HAZARDS) }, // HotFootShy      scr=4 col=12
            EntryRule { offset: 0x0D554, id: 0x8A, rule: EntryProtection::Exclude(HAZARDS) }, // Thwomp          scr=5 col=2
            EntryRule { offset: 0x0D557, id: 0x30, rule: EntryProtection::Exclude(HAZARDS) }, // HotFootShy      scr=5 col=3
            EntryRule { offset: 0x0D55A, id: 0x8C, rule: EntryProtection::Exclude(HAZARDS) }, // ThwompRightSlide scr=5 col=12
        ],
    },
    LevelProtection {
        level: "4F1",
        area: 1,
        why: "narrow-hallway fort",
        enemy_ptr: 0xC968,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0C979, id: 0x3F, rule: EntryProtection::Exclude(HAZARDS) }, // DryBones scr=0 col=8
            EntryRule { offset: 0x0C97C, id: 0x3F, rule: EntryProtection::Exclude(HAZARDS) }, // DryBones scr=1 col=4
            EntryRule { offset: 0x0C97F, id: 0x2F, rule: EntryProtection::Exclude(HAZARDS) }, // Boo      scr=1 col=13
            EntryRule { offset: 0x0C982, id: 0x3F, rule: EntryProtection::Exclude(HAZARDS) }, // DryBones scr=2 col=3
        ],
    },
    LevelProtection {
        level: "4F2",
        area: 0,
        why: "fort ground route — a hazard on the walking path is unavoidable",
        enemy_ptr: 0xD508,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0D519, id: 0x3F, rule: EntryProtection::Exclude(HAZARDS) }, // DryBones    scr=2 col=10
            EntryRule { offset: 0x0D51C, id: 0x3F, rule: EntryProtection::Exclude(HAZARDS) }, // DryBones    scr=3 col=9
            EntryRule { offset: 0x0D51F, id: 0x3F, rule: EntryProtection::Exclude(HAZARDS) }, // DryBones    scr=4 col=9
            EntryRule { offset: 0x0D522, id: 0x3F, rule: EntryProtection::Exclude(HAZARDS) }, // DryBones    scr=5 col=4
            EntryRule { offset: 0x0D525, id: 0x3F, rule: EntryProtection::Exclude(HAZARDS) }, // DryBones    scr=5 col=11
            EntryRule { offset: 0x0D528, id: 0x3F, rule: EntryProtection::Exclude(HAZARDS) }, // DryBones    scr=6 col=2
            EntryRule { offset: 0x0D52B, id: 0x3F, rule: EntryProtection::Exclude(HAZARDS) }, // DryBones    scr=6 col=3
            EntryRule { offset: 0x0D52E, id: 0x3F, rule: EntryProtection::Exclude(HAZARDS) }, // DryBones    scr=6 col=12
            EntryRule { offset: 0x0D531, id: 0x5B, rule: EntryProtection::Exclude(HAZARDS) }, // RotodiscCCW scr=6 col=14
        ],
    },
    LevelProtection {
        level: "4-1",
        area: 0,
        why: "each troopa sits on a small platform Mario must land on to progress; a hazard here forces an unavoidable hit; the high Venus pipe is in the reservoir",
        enemy_ptr: 0xCE97,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            // In the water up top: a Rocky Wrench or giant piranha here is
            // invisible behind the water tiles (see BEHIND_BG).
            EntryRule { offset: 0x0CEB1, id: 0xA6, rule: EntryProtection::Exclude(BEHIND_BG) }, // VenusFireTrap scr=2 col=6
            EntryRule { offset: 0x0CEBD, id: 0x7B, rule: EntryProtection::Exclude(HAZARDS) }, // BigRedTroopa scr=5 col=8
            EntryRule { offset: 0x0CEC0, id: 0x7B, rule: EntryProtection::Exclude(HAZARDS) }, // BigRedTroopa scr=5 col=15
            EntryRule { offset: 0x0CEC3, id: 0x7B, rule: EntryProtection::Exclude(HAZARDS) }, // BigRedTroopa scr=6 col=4
            EntryRule { offset: 0x0CEC9, id: 0x7A, rule: EntryProtection::Exclude(HAZARDS) }, // BigGreenTroopa scr=7 col=10
        ],
    },
    LevelProtection {
        level: "β2",
        area: 0,
        why: "the low pipe is underwater",
        enemy_ptr: 0xC221,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            // A Rocky Wrench or giant piranha here is invisible behind the
            // water tiles (see BEHIND_BG).
            EntryRule { offset: 0x0C23E, id: 0xA0, rule: EntryProtection::Exclude(BEHIND_BG) }, // GreenPiranha scr=1 col=10
        ],
    },
    LevelProtection {
        level: "8F1",
        area: 0,
        why: "roto-disc narrow area makes hazards unfair",
        enemy_ptr: 0xD551,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0D562, id: 0x5A, rule: EntryProtection::Exclude(HAZARDS) }, // Rotodisc
            EntryRule { offset: 0x0D568, id: 0x60, rule: EntryProtection::Exclude(HAZARDS) }, // Rotodisc
        ],
    },
    // --- Rotodisc-excluded entries ---
    LevelProtection {
        level: "3F1",
        area: 0,
        why: "screen 5 and 7 Dry Bones must not become Rotodiscs",
        enemy_ptr: 0xD393,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0D3BC, id: 0x3F, rule: EntryProtection::Exclude(ROTODISCS) }, // DryBones scr=5 col=12
            EntryRule { offset: 0x0D3BF, id: 0x3F, rule: EntryProtection::Exclude(ROTODISCS) }, // DryBones scr=5 col=5
            EntryRule { offset: 0x0D3C8, id: 0x3F, rule: EntryProtection::Exclude(ROTODISCS) }, // DryBones scr=7 col=2
            EntryRule { offset: 0x0D3CB, id: 0x3F, rule: EntryProtection::Exclude(ROTODISCS) }, // DryBones scr=7 col=4
            EntryRule { offset: 0x0D3CE, id: 0x3F, rule: EntryProtection::Exclude(ROTODISCS) }, // DryBones scr=7 col=13
        ],
    },
    // --- Thwomp-variant exclusions ---
    LevelProtection {
        level: "5F1",
        area: 0,
        why: "screen 1 Rotodisc must not become a standard dropping Thwomp",
        enemy_ptr: 0xD3D0,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            // OBJ_THWOMP (standard drop) only; the other five variants stay
            // allowed. Only reachable with Thwomps and Rotodiscs both Wild.
            EntryRule {
                offset: 0x0D3E7,
                id: 0x5B,
                rule: EntryProtection::Exclude(Group::Ids(&[0x8A])),
            }, // RotodiscCCW scr=1 col=1
        ],
    },
    LevelProtection {
        level: "CoinShip",
        area: 1,
        why: "end-pipe room: 2-BoomerangBro fight",
        enemy_ptr: 0xDA0F,
        walker_segment: WalkerSegmentRule::HammerBro,
        entries: &[],
    },
];

/// Every `Exclude(HAZARDS)` rule: `(entry offset, level, area, why)`. Read by
/// the hazard census, which scores a terrain-based detector against them.
#[cfg(test)]
pub(super) fn hazard_rules() -> Vec<(usize, &'static str, usize, &'static str)> {
    LEVEL_PROTECTIONS
        .iter()
        .flat_map(|l| {
            l.entries
                .iter()
                .filter(|e| e.rule == EntryProtection::Exclude(HAZARDS))
                .map(move |e| (e.offset, l.level, l.area, l.why))
        })
        .collect()
}

/// Per-entry rule for the entry at this absolute file offset, if any.
pub(super) fn entry_protection_at(file_offset: usize) -> Option<EntryProtection> {
    LEVEL_PROTECTIONS
        .iter()
        .flat_map(|l| l.entries)
        .find_map(|e| (e.offset == file_offset).then_some(e.rule))
}

// The coin-ship reward fight used to need a special case here — a
// `is_coinship_fight` predicate feeding a Dry Bones filter, because a Dry Bones
// revives forever in an enclosed room. Dry Bones now lives in
// `HB_NEEDS_SHELL_ENEMIES`, so it can only ever be dealt into a room that also
// has a shell to kill it with, in that room and every other. The coin ship is
// no longer a special case: it is one bro-fight room among several.

/// Whether this room rewrites a Hammer Bro into something else — in which case
/// it must never be given one.
///
/// In a room with `Level_Event = 7` (i.e. whose enemy data holds
/// [`TREASURE_BOX_APPEAR`](crate::randomize::rom_data::TREASURE_BOX_APPEAR)) a `$81` is not an enemy but a *placeholder* meaning
/// "whichever bro's map sprite the player walked in through", which is how one
/// arena serves all four bro battles. `ObjInit_HammerBro` (`prg004.asm:866`)
/// overwrites the object's own ID with `BattleEnemy_ByEnterID[Map_EnterViaID]`
/// and re-inits. That table (file `0x08478`) defines only indices 0-9 — the
/// disassembly says so outright: *"No definition for $0A-$10 map objects"* — so
/// any other entry path reads the routine's own machine code as an object ID.
/// The Coin Ship (`MAPOBJ_COINSHIP` = `$0B`) lands on `$66`, a water current;
/// the 8-Tank (`$0E`) on `$07` `OBJ_WARPHIDE`; an ordinary level tile on `$00`.
/// None can be killed, so the treasure box never appears and the room has no
/// exit. Full table in `docs/smb3_rom_reference.md`.
///
/// Both halves are load-bearing. The treasure box says the rewrite is *armed*;
/// [`HAMMER_BRO_OBJ_PTRS`] says whether it is *benign* — in a real bro battle
/// the lookup resolves correctly and `$81` is the vanilla value, so excluding
/// it there would mean a Hammer Bro could never appear as an HB encounter.
/// (W8's `0xC03D` is in that list but is the full 7-7 action level with no
/// treasure box, so it never reaches this check.)
pub(super) fn rewrites_hammer_bro(segment_file_offset: usize, has_treasure_box: bool) -> bool {
    has_treasure_box
        && !HAMMER_BRO_OBJ_PTRS.iter().any(|&p| enemy_ptr_to_file_offset(p) == segment_file_offset)
}

/// Walker rule for the segment whose page byte sits at this absolute file
/// offset. Returns `Default` for unprotected segments.
///
/// Every stream in [`HAMMER_BRO_OBJ_PTRS`] is a `HammerBro` segment; the table
/// adds only the bro-fight rooms reached some other way.
pub(super) fn walker_segment_rule_at(segment_file_offset: usize) -> WalkerSegmentRule {
    if HAMMER_BRO_OBJ_PTRS.iter().any(|&p| enemy_ptr_to_file_offset(p) == segment_file_offset) {
        return WalkerSegmentRule::HammerBro;
    }
    LEVEL_PROTECTIONS
        .iter()
        .find(|l| {
            l.walker_segment != WalkerSegmentRule::Default
                && enemy_ptr_to_file_offset(l.enemy_ptr) == segment_file_offset
        })
        .map_or(WalkerSegmentRule::Default, |l| l.walker_segment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::overworld::node_catalog::{NodeCatalog, level_name_key};
    use crate::randomize::rom_data::level_index::{self, read_objects};
    use crate::randomize::rom_data::{LevelEntry, UNLISTED_LEVELS};
    use crate::rom::Rom;
    use std::collections::HashSet;

    #[test]
    fn registry_entries_have_unique_offsets() {
        // An offset must not carry two conflicting rules — the walker would
        // take whichever rule entry_protection_at returns first, so duplicates
        // silently change behavior.
        let mut seen: HashSet<usize> = HashSet::new();
        for level in LEVEL_PROTECTIONS {
            for entry in level.entries {
                assert!(
                    seen.insert(entry.offset),
                    "offset 0x{:05X} appears in multiple entries",
                    entry.offset
                );
            }
        }
    }

    /// The 5F1 slot is the screen-1 Rotodisc in this level's stream, and only
    /// the standard drop Thwomp is kept out of it.
    #[test]
    fn the_5f1_rotodisc_excludes_only_the_drop_thwomp() {
        let Ok(rom) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        assert_eq!(enemy_ptr_to_file_offset(0xD3D0), 0x0D3E0, "5F1's stream starts here");
        let vanilla = rom[0x0D3E7];
        assert_eq!(vanilla, 0x5B, "RotodiscCCW at 0x0D3E7");

        let Some(EntryProtection::Exclude(group)) = entry_protection_at(0x0D3E7) else {
            panic!("0x0D3E7 carries no exclusion");
        };
        assert!(group.excludes(0x8A, vanilla), "the drop Thwomp must be excluded");
        for variant in 0x8B..=0x8F {
            assert!(!group.excludes(variant, vanilla), "{variant:#04X} must stay allowed");
        }
    }

    /// The underwater piranha pipes keep Rocky Wrench and the giant piranhas
    /// out, and every other piranha-pool member in.
    #[test]
    fn underwater_piranha_pipes_exclude_only_behind_bg_enemies() {
        let Ok(rom) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        // 4-1 high Venus, β2 low pipe, 6-6 screen-8 pair.
        for (offset, expected) in
            [(0x0CEB1, 0xA6), (0x0C23E, 0xA0), (0x0C69E, 0xA2), (0x0C6A4, 0xA2)]
        {
            let vanilla = rom[offset];
            assert_eq!(vanilla, expected, "piranha at {offset:#07X}");
            let Some(EntryProtection::Exclude(group)) = entry_protection_at(offset) else {
                panic!("{offset:#07X} carries no exclusion");
            };
            for id in [0x7D, 0x7F, 0xAD] {
                assert!(group.excludes(id, vanilla), "{id:#04X} must be excluded at {offset:#07X}");
            }
            for id in [0xA0, 0xA2, 0xA4, 0xA6, 0x9D] {
                assert!(
                    !group.excludes(id, vanilla),
                    "{id:#04X} must stay allowed at {offset:#07X}"
                );
            }
        }
    }

    #[test]
    fn registry_enemy_ptrs_are_unique() {
        let mut seen: HashSet<u16> = HashSet::new();
        for level in LEVEL_PROTECTIONS {
            assert!(
                seen.insert(level.enemy_ptr),
                "enemy_ptr 0x{:04X} appears in multiple LevelProtection rows",
                level.enemy_ptr
            );
        }
    }

    /// Each row's `level` and `area` name the object list its `enemy_ptr`
    /// reads, and every entry offset lies in that list. Skips without the ROM.
    #[test]
    fn every_row_names_the_area_its_offsets_are_in() {
        let Some(rom) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes")
            .ok()
            .and_then(|b| Rom::from_bytes(&b).ok())
        else {
            return;
        };
        let views = NodeCatalog::build(&rom, true).entry_views();
        let levels: Vec<(&str, &LevelEntry)> = views
            .iter()
            .filter_map(|v| v.level_entry.as_ref().map(|le| (v.name.as_str(), le)))
            .chain(UNLISTED_LEVELS.iter().map(|(name, le)| (*name, le)))
            .collect();
        for row in LEVEL_PROTECTIONS {
            let (_, entry) = levels
                .iter()
                .find(|(name, _)| level_name_key(name) == level_name_key(row.level))
                .unwrap_or_else(|| panic!("{}: no level by that name (rominfo --list)", row.level));
            let (areas, _) = level_index::trace(&rom, entry).expect("a protected level traces");
            let area = areas.get(row.area).unwrap_or_else(|| {
                panic!("{}: has {} areas, not area {}", row.level, areas.len(), row.area)
            });
            assert_eq!(
                area.objects_ptr, row.enemy_ptr,
                "{} area {}: its objects are at ${:04X}, the row says ${:04X}",
                row.level, row.area, area.objects_ptr, row.enemy_ptr
            );
            let objects = read_objects(&rom, area.objects_ptr, area.tileset).unwrap();
            for e in row.entries {
                let found = objects.iter().find(|o| o.offset == e.offset).unwrap_or_else(|| {
                    panic!(
                        "{} area {}: 0x{:05X} is not one of its objects",
                        row.level, row.area, e.offset
                    )
                });
                assert_eq!(
                    found.id, e.id,
                    "{} area {}: 0x{:05X} holds ${:02X}, the row says ${:02X}",
                    row.level, row.area, e.offset, found.id, e.id
                );
            }
        }
    }
}
