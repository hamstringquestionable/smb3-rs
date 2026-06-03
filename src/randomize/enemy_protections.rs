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
//! `enemies::inject_at_entry_points`) consume the registry through the
//! derived helpers (`entry_protection_at`, `is_injection_blocked`,
//! `walker_segment_rule_at`) — never the table directly.

use super::rom_data::enemy_ptr_to_file_offset;

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
    /// Walker forces a pick from TANK_BRO_POOL when bros mode is on.
    ForceTankBro,
    /// Walker excludes HAZARD_PROJECTILE_IDS from the chosen pool.
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
        label: "8-1 (Boo + FlyingRedParatroopa required for progression)",
        enemy_ptr: 0xC424,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0C456, rule: EntryProtection::SkipSwap }, // Boo scr=5 col=1
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

    // --- Bros-pool restriction (HammerBro fails to spawn in tileset 10) ---
    LevelProtection {
        label: "8-Tank sub-area (HammerBro fails to spawn in ts=10)",
        enemy_ptr: 0xDA29,
        walker_segment: WalkerSegmentRule::Default,
        entries: &[
            EntryRule { offset: 0x0DA3A, rule: EntryProtection::ForceTankBro }, // BoomerangBro scr=0 col=12
        ],
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

/// True if `wild_injection` should not fire at this entry_ptr. Any row with a
/// non-`Default` walker rule blocks injection too — `Skip` and `HammerBro`
/// segments both override or replace the player-visible enemy choice.
pub(super) fn is_injection_blocked(enemy_ptr: u16) -> bool {
    LEVEL_PROTECTIONS
        .iter()
        .any(|l| l.enemy_ptr == enemy_ptr && l.walker_segment != WalkerSegmentRule::Default)
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
