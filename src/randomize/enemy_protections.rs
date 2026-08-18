//! Per-level enemy protection registry.
//!
//! One labeled table (`LEVEL_PROTECTIONS`) per protected level or sub-area
//! that the enemy randomization passes must treat specially. Each row
//! identifies the level by `enemy_ptr` and declares:
//!   - `walker_segment`: how the walker pass treats the containing $FF segment
//!     (`Default`, `Skip`, or `HammerBro`). Non-`Default` rows also block
//!     wild injection at this entry_ptr.
//!   - `entries`: per-entry rules keyed by absolute file offset.
//!
//! Adding a new protection is one block here with a label and a reason.
//! Both passes (the walker in `enemies::randomize_object_data` and
//! `enemies::inject_wild_chasers`) consume the registry through the
//! derived helpers (`entry_protection_at`, `walker_segment_rule_at`)
//! — never the table directly.

use super::rom_data::{enemy_ptr_to_file_offset, HAMMER_BRO_OBJ_PTRS};

/// One logical level or sub-area with protections that affect enemy
/// randomization.
pub(super) struct LevelProtection {
    // Reason: `label` is documentation embedded in the table — its value is
    // grep-ability when investigating a protected offset.
    #[allow(dead_code)]
    pub label: &'static str,
    pub enemy_ptr: u16,
    pub walker_segment: WalkerSegmentRule,
    pub entries: &'static [EntryRule],
}

/// How the walker treats the $FF-bounded segment containing this level's
/// enemy_ptr.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum WalkerSegmentRule {
    /// Default class-based swaps.
    Default,
    /// Skip the entire segment (no swaps at all). Only valid for top-level
    /// entries whose enemy_ptr lands at the segment's page byte — NOT for
    /// sub-area entries that share a segment with a parent level (skipping
    /// would also block the parent's swaps).
    Skip,
    /// Hammer Bro encounter — walker uses HB-specific modes and pool.
    HammerBro,
}

/// Per-entry rule attached to a specific 3-byte enemy entry by its absolute
/// file offset.
pub(super) struct EntryRule {
    pub offset: usize,
    pub rule: EntryProtection,
}

/// Per-entry behavior. The walker applies these inline during its swap pass;
/// `inject_at_entry_points` also skips injection at any position carrying one
/// of these (so wild injection can't bypass a per-entry safeguard).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum EntryProtection {
    /// Walker skips this entry (pre-commits CHR page only).
    SkipSwap,
    /// Walker forces a pick from SHELL_ENEMIES when shell mode is on.
    ForceShell,
    /// Walker forces a pick from the entry's natural pool ∩ STOMPABLE_ENEMIES.
    ForceStompable,
    /// Walker excludes hazard-category enemies from the chosen pool (additive-only:
    /// a hazard of the same category as the vanilla enemy here is kept). See
    /// `hazard_excluded` in enemies.rs.
    ExcludeHazards,
}

pub(super) const LEVEL_PROTECTIONS: &[LevelProtection] = &[
    // --- Whole-level skips ---
    LevelProtection {
        label: "3-2 (enemies-as-platforms, sprite-overload risk)",
        enemy_ptr: 0xCA23,
        walker_segment: WalkerSegmentRule::Skip,
        entries: &[],
    },

    // --- Individual gameplay-critical entries (walker skips, no whole-level block) ---
    LevelProtection {
        label: "8-1 (FlyingRedParatroopa required for progression; Boo restricted from path-blocking hazards)",
        enemy_ptr: 0xC424,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            // Boo is swap-safe, but a hazard here (Ptooie/nipper/lotus/etc.) would
            // block a narrow pathway the player must pass through — so swap it to
            // anything except a hazard rather than skipping it outright.
            EntryRule { offset: 0x0C456, rule: EntryProtection::ExcludeHazards }, // Boo scr=5 col=1
            EntryRule { offset: 0x0C465, rule: EntryProtection::SkipSwap }, // FlyingRedParatroopa scr=6 col=14
        ],
    },
    LevelProtection {
        label: "6-3 (FlyingRedParatroopas required as platforms)",
        enemy_ptr: 0xCA8E,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0CAB1, rule: EntryProtection::SkipSwap }, // scr=6 col=13
            EntryRule { offset: 0x0CAB4, rule: EntryProtection::SkipSwap }, // scr=7 col=1
        ],
    },

    // --- Shell-locked entries (shells needed to break bricks for progression) ---
    LevelProtection {
        label: "2-Pyr sub-area (Buzzy Beetles needed to break bricks)",
        enemy_ptr: 0xC5BC,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0C5CD, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5D0, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5D3, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5D6, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5DC, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5DF, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5E2, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5E5, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5E8, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5EB, rule: EntryProtection::ForceShell },
            EntryRule { offset: 0x0C5F1, rule: EntryProtection::ForceShell },
        ],
    },
    LevelProtection {
        label: "2-3 (shells needed to break end-of-level bricks)",
        enemy_ptr: 0xD1F0,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0D22B, rule: EntryProtection::ForceShell }, // GreenTroopa scr=8 col=11
            EntryRule { offset: 0x0D22E, rule: EntryProtection::ForceShell }, // GreenTroopa scr=8 col=13
        ],
    },
    LevelProtection {
        label: "7-1 sub-area (scr=0 GreenTroopas kept in the shell pool)",
        enemy_ptr: 0xCD93,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0CDA7, rule: EntryProtection::ForceShell }, // GreenTroopa scr=0 col=2
            EntryRule { offset: 0x0CDAA, rule: EntryProtection::ForceShell }, // GreenTroopa scr=0 col=4
        ],
    },
    LevelProtection {
        label: "6-5 sub-area (shell needed for progression)",
        enemy_ptr: 0xC5EB,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0C60E, rule: EntryProtection::ForceShell }, // GreenTroopa scr=4 col=10
        ],
    },

    // --- Stompable-locked entries ---
    LevelProtection {
        label: "6-6 sub-area (floor spikes — non-stompable swap would corner player)",
        enemy_ptr: 0xC64B,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0C6A7, rule: EntryProtection::ForceStompable }, // Spike scr=10 col=0
            EntryRule { offset: 0x0C6AA, rule: EntryProtection::ForceStompable }, // Spike scr=10 col=6
            EntryRule { offset: 0x0C6AD, rule: EntryProtection::ForceStompable }, // Spike scr=10 col=4
        ],
    },

    // --- Bro-fight rooms reached by a non-bro map object ---
    // A treasure-box room is cleared to be left, so it needs the bro-fight
    // pool: every enemy in it must be permanently killable. These two are
    // fight-shaped rooms the player reaches through the Coin Ship / Tank map
    // sprite rather than a bro sprite, so nothing else classifies them.
    //
    // This replaced a `ForceTankBro` row on $DA29 labelled "HammerBro fails to
    // spawn in ts=10". The tileset was never the cause: it is a
    // `Level_Event = 7` room entered via `MAPOBJ_TANK` ($0E), so a $81 there
    // resolved through `BattleEnemy_ByEnterID[14]` to $07 `OBJ_WARPHIDE`, which
    // is invisible — that is what "fails to spawn" actually was. The ID half of
    // it is now handled generically by `rewrites_hammer_bro`.
    LevelProtection {
        label: "8-Tank sub-area treasure room (bro fight via the Tank sprite)",
        enemy_ptr: 0xDA29,
        walker_segment: WalkerSegmentRule::HammerBro,
        entries: &[],
    },

    // --- Hazard-excluded entries (no Patooie/Lavalotus on player walking path) ---
    LevelProtection {
        label: "7F2 Boom-Boom sub-area (tight boss arena)",
        enemy_ptr: 0xD45C,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0D46D, rule: EntryProtection::ExcludeHazards }, // Rotodisc CW scr=1 col=0
            EntryRule { offset: 0x0D470, rule: EntryProtection::ExcludeHazards }, // DryBones    scr=1 col=1
            EntryRule { offset: 0x0D473, rule: EntryProtection::ExcludeHazards }, // DryBones    scr=1 col=3
            EntryRule { offset: 0x0D476, rule: EntryProtection::ExcludeHazards }, // Rotodisc CW scr=1 col=9
            EntryRule { offset: 0x0D479, rule: EntryProtection::ExcludeHazards }, // Thwomp      scr=1 col=10
        ],
    },
    LevelProtection {
        label: "7-5 sub-area (open horizontal field — hazards at floor level unfair)",
        enemy_ptr: 0xC171,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0C182, rule: EntryProtection::ExcludeHazards }, // ParatroopaGreenHop scr=0 col=12
            EntryRule { offset: 0x0C185, rule: EntryProtection::ExcludeHazards }, // ParatroopaGreenHop scr=1 col=2
            EntryRule { offset: 0x0C18E, rule: EntryProtection::ExcludeHazards }, // BobOmb             scr=2 col=5
            EntryRule { offset: 0x0C191, rule: EntryProtection::ExcludeHazards }, // BobOmb             scr=2 col=7
            EntryRule { offset: 0x0C194, rule: EntryProtection::ExcludeHazards }, // BobOmb             scr=2 col=9
            EntryRule { offset: 0x0C1A0, rule: EntryProtection::ExcludeHazards }, // ParatroopaGreenHop scr=4 col=14
            EntryRule { offset: 0x0C1A3, rule: EntryProtection::ExcludeHazards }, // ParatroopaGreenHop scr=5 col=1
            EntryRule { offset: 0x0C1A6, rule: EntryProtection::ExcludeHazards }, // ParatroopaGreenHop scr=5 col=4
        ],
    },

    LevelProtection {
        label: "β4 sub-area (narrow corridor — hazards on Buzzy Beetle path unfair)",
        enemy_ptr: 0xC7A7,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0C7B8, rule: EntryProtection::ExcludeHazards }, // BuzzyBeatle scr=1 col=2
            EntryRule { offset: 0x0C7BB, rule: EntryProtection::ExcludeHazards }, // BuzzyBeatle scr=1 col=5
            EntryRule { offset: 0x0C7BE, rule: EntryProtection::ExcludeHazards }, // BuzzyBeatle scr=1 col=9
            EntryRule { offset: 0x0C7CA, rule: EntryProtection::ExcludeHazards }, // BuzzyBeatle scr=2 col=11
            EntryRule { offset: 0x0C7CD, rule: EntryProtection::ExcludeHazards }, // BuzzyBeatle scr=3 col=2
            EntryRule { offset: 0x0C7D0, rule: EntryProtection::ExcludeHazards }, // BuzzyBeatle scr=3 col=4
        ],
    },

    LevelProtection {
        label: "4F1 (narrow-hallway fort — a hazard anywhere blocks the only path)",
        enemy_ptr: 0xD528,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0D539, rule: EntryProtection::ExcludeHazards }, // HotFootShy      scr=1 col=0
            EntryRule { offset: 0x0D53C, rule: EntryProtection::ExcludeHazards }, // HotFootShy      scr=1 col=8
            EntryRule { offset: 0x0D53F, rule: EntryProtection::ExcludeHazards }, // HotFootShy      scr=2 col=7
            EntryRule { offset: 0x0D542, rule: EntryProtection::ExcludeHazards }, // ThwompLeftSlide scr=2 col=2
            EntryRule { offset: 0x0D545, rule: EntryProtection::ExcludeHazards }, // ThwompLeftSlide scr=3 col=0
            EntryRule { offset: 0x0D548, rule: EntryProtection::ExcludeHazards }, // HotFootShy      scr=3 col=2
            EntryRule { offset: 0x0D54B, rule: EntryProtection::ExcludeHazards }, // HotFootShy      scr=3 col=10
            EntryRule { offset: 0x0D54E, rule: EntryProtection::ExcludeHazards }, // ThwompRightSlide scr=4 col=1
            EntryRule { offset: 0x0D551, rule: EntryProtection::ExcludeHazards }, // HotFootShy      scr=4 col=12
            EntryRule { offset: 0x0D554, rule: EntryProtection::ExcludeHazards }, // Thwomp          scr=5 col=2
            EntryRule { offset: 0x0D557, rule: EntryProtection::ExcludeHazards }, // HotFootShy      scr=5 col=3
            EntryRule { offset: 0x0D55A, rule: EntryProtection::ExcludeHazards }, // ThwompRightSlide scr=5 col=12
        ],
    },
    LevelProtection {
        label: "4F1 sub-area 1 (narrow-hallway fort)",
        enemy_ptr: 0xC968,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0C979, rule: EntryProtection::ExcludeHazards }, // DryBones scr=0 col=8
            EntryRule { offset: 0x0C97C, rule: EntryProtection::ExcludeHazards }, // DryBones scr=1 col=4
            EntryRule { offset: 0x0C97F, rule: EntryProtection::ExcludeHazards }, // Boo      scr=1 col=13
            EntryRule { offset: 0x0C982, rule: EntryProtection::ExcludeHazards }, // DryBones scr=2 col=3
        ],
    },
    LevelProtection {
        label: "4F2 (fort ground route — a hazard on the walking path is unavoidable)",
        enemy_ptr: 0xD508,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0D519, rule: EntryProtection::ExcludeHazards }, // DryBones    scr=2 col=10
            EntryRule { offset: 0x0D51C, rule: EntryProtection::ExcludeHazards }, // DryBones    scr=3 col=9
            EntryRule { offset: 0x0D51F, rule: EntryProtection::ExcludeHazards }, // DryBones    scr=4 col=9
            EntryRule { offset: 0x0D522, rule: EntryProtection::ExcludeHazards }, // DryBones    scr=5 col=4
            EntryRule { offset: 0x0D525, rule: EntryProtection::ExcludeHazards }, // DryBones    scr=5 col=11
            EntryRule { offset: 0x0D528, rule: EntryProtection::ExcludeHazards }, // DryBones    scr=6 col=2
            EntryRule { offset: 0x0D52B, rule: EntryProtection::ExcludeHazards }, // DryBones    scr=6 col=3
            EntryRule { offset: 0x0D52E, rule: EntryProtection::ExcludeHazards }, // DryBones    scr=6 col=12
            EntryRule { offset: 0x0D531, rule: EntryProtection::ExcludeHazards }, // RotodiscCCW scr=6 col=14
        ],
    },
    LevelProtection {
        label: "4-1 (each troopa sits on a small platform Mario must land on to progress; a hazard here forces an unavoidable hit)",
        enemy_ptr: 0xCE97,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0CEBD, rule: EntryProtection::ExcludeHazards }, // BigRedTroopa scr=5 col=8
            EntryRule { offset: 0x0CEC0, rule: EntryProtection::ExcludeHazards }, // BigRedTroopa scr=5 col=15
            EntryRule { offset: 0x0CEC3, rule: EntryProtection::ExcludeHazards }, // BigRedTroopa scr=6 col=4
            EntryRule { offset: 0x0CEC9, rule: EntryProtection::ExcludeHazards }, // BigGreenTroopa scr=7 col=10
        ],
    },
    LevelProtection {
        label: "8F roto-disc narrow area makes hazards unfair",
        enemy_ptr: 0xD551,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0D562, rule: EntryProtection::ExcludeHazards }, // Rotodisc
            EntryRule { offset: 0x0D568, rule: EntryProtection::ExcludeHazards }, // Rotodisc
        ],
    },
    // --- Hammer Bro encounters (walker uses HB modes; injection skips) ---
    LevelProtection {
        label: "W1 Hammer Bro",
        enemy_ptr: 0xC72B,
        walker_segment: WalkerSegmentRule::HammerBro,
        entries: &[],
    },
    LevelProtection {
        label: "W2 Hammer Bro",
        enemy_ptr: 0xD14D,
        walker_segment: WalkerSegmentRule::HammerBro,
        entries: &[],
    },
    LevelProtection {
        label: "W2 Hammer Bro (variant)",
        enemy_ptr: 0xD142,
        walker_segment: WalkerSegmentRule::HammerBro,
        entries: &[],
    },
    LevelProtection {
        label: "W3/W5/W6/W7 Hammer Bro",
        enemy_ptr: 0xC640,
        walker_segment: WalkerSegmentRule::HammerBro,
        entries: &[],
    },
    LevelProtection {
        label: "W4 Hammer Bro",
        enemy_ptr: 0xD0EA,
        walker_segment: WalkerSegmentRule::HammerBro,
        entries: &[],
    },
    LevelProtection {
        label: "W8 Hammer Bro (uses 7-7 layout)",
        enemy_ptr: 0xC03D,
        walker_segment: WalkerSegmentRule::HammerBro,
        entries: &[],
    },
    LevelProtection {
        label: "Coin Ship end-pipe (2-BoomerangBro fight)",
        enemy_ptr: 0xDA0F,
        walker_segment: WalkerSegmentRule::HammerBro,
        entries: &[],
    },
];

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
/// [`TREASURE_BOX_APPEAR`]) a `$81` is not an enemy but a *placeholder* meaning
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
///
/// Nothing here depends on the tileset — an earlier `ForceTankBro` row on the
/// 8-Tank sub-area blamed tileset 10, which was a misdiagnosis of this.
pub(super) fn rewrites_hammer_bro(segment_file_offset: usize, has_treasure_box: bool) -> bool {
    has_treasure_box
        && !HAMMER_BRO_OBJ_PTRS
            .iter()
            .any(|&p| enemy_ptr_to_file_offset(p) == segment_file_offset)
}

/// Walker rule for the segment whose page byte sits at this absolute file
/// offset. Returns `Default` for unprotected segments.
pub(super) fn walker_segment_rule_at(segment_file_offset: usize) -> WalkerSegmentRule {
    LEVEL_PROTECTIONS
        .iter()
        .find(|l| l.walker_segment != WalkerSegmentRule::Default
            && enemy_ptr_to_file_offset(l.enemy_ptr) == segment_file_offset)
        .map_or(WalkerSegmentRule::Default, |l| l.walker_segment)
}

#[cfg(test)]
mod tests {
    use super::*;
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
                    "offset 0x{:05X} appears in multiple entries", entry.offset
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
                "enemy_ptr 0x{:04X} appears in multiple LevelProtection rows", level.enemy_ptr
            );
        }
    }

    #[test]
    fn skip_segments_use_top_level_eps() {
        // WalkerSegmentRule::Skip only makes sense at top-level enemy_ptrs
        // (where the ep's file offset = a segment's page byte). Sub-area
        // entry_ptrs land mid-segment, so "skip the segment" would also block
        // the parent level's swaps. There's no cheap way to detect this without
        // walking the ROM, but we can at least sanity-check that a Skip row's
        // ep doesn't collide with any other row's per-entry offsets — if it
        // did, that's a strong hint the Skip target is mid-segment.
        let skip_eps: Vec<usize> = LEVEL_PROTECTIONS
            .iter()
            .filter(|l| l.walker_segment == WalkerSegmentRule::Skip)
            .map(|l| enemy_ptr_to_file_offset(l.enemy_ptr))
            .collect();
        for level in LEVEL_PROTECTIONS {
            for entry in level.entries {
                assert!(
                    !skip_eps.contains(&entry.offset),
                    "per-entry offset 0x{:05X} collides with a Skip segment's ep — likely a sub-area mislabeled as top-level skip",
                    entry.offset,
                );
            }
        }
    }
}
