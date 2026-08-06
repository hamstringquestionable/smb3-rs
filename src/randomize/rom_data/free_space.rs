//! Central registry of ROM free-space allocations (assembled code + data
//! tables). The overlap test guards against collisions between rows;
//! `audit_free_space` guards against the registry drifting from what the
//! randomizer actually writes, and `free_space_map` measures what is left.
//! Both are printed by `--write-log`, which is where the per-bank numbers
//! below and the `reserved / used` comments should be regenerated from.
//!
//! **Free space is the scarcest resource in this project — size every patch
//! accordingly.** See the "ROM Free Space Is Scarce" section in `CLAUDE.md`
//! for the per-bank budget and the size techniques that have paid off.
//!
//! The aggregate free-space number is misleading: space is per-bank, and a
//! routine must live in a bank mapped when it runs. The always-mapped banks
//! are effectively full — PRG031 (`$E000–$FFFF`) has 81 bytes left but a
//! largest contiguous gap of only 30, and PRG030 (`$8000–$9FFF`) has 88 with
//! a largest gap of 42. Swapped banks are roomier (PRG010 896, PRG026 2603,
//! and the in-level banks PRG004 426 / PRG006 1392 in one run each).
//!
//! Reserving some headroom in an allocation is fine and encouraged — a routine
//! that has to grow later is far safer to extend in place than to relocate
//! (several allocations here are origin-locked by self-referential absolute
//! addresses). Note both numbers when they differ, e.g. "112 reserved, 97
//! used". What is *not* fine is letting the code itself be sloppy: reserve
//! room, but write the bytes as tight as they will go.

use crate::rom::Rom;

/// One reserved region of ROM free space.
///
/// `owners` lists the write-log tags allowed to write here. Each matches as a
/// run of whole `/`-separated components, so the short name `fx_screen_check`
/// owns writes tagged `overworld_writer/fx_screen_check`, and `big_q_blocks`
/// owns both `qol/big_q_blocks` and `enemies/big_q_blocks`. The auditor
/// (`audit_free_space`) uses this to flag a module writing into free space it
/// does not own.
///
/// Most regions have exactly one owner. A second entry means the region is
/// shared on purpose — the cloned enemy streams are built by one module and
/// then have their item byte randomized by another — and is the place to say
/// so, not a licence to let an unrelated pass wander in.
pub struct FreeSpaceAlloc {
    pub offset: usize,
    pub size: usize,
    pub owners: &'static [&'static str],
    pub label: &'static str,
}

/// Shorthand so registry rows stay one line each.
const fn fs(
    offset: usize,
    size: usize,
    owners: &'static [&'static str],
    label: &'static str,
) -> FreeSpaceAlloc {
    FreeSpaceAlloc { offset, size, owners, label }
}

/// Every region of ROM free space this project has claimed.
///
/// `test_free_space_no_overlap` checks that no two rows share a byte;
/// `audit_free_space` checks the rows against what a real run actually wrote.
///
/// # Why this is a static map and not an allocator
///
/// Evaluated 2026-08-04 and deferred, so it does not get re-proposed blind.
///
/// The obvious objection to a static registry is that a flag-gated patch
/// reserves its bytes whether or not the feature is on — and that is a real
/// waste in the bank where it hurts most: under default options, 102 of
/// PRG031's 206 reserved bytes go unwritten (`start_airship_swap` 69,
/// `starting_items` 33), in a bank whose largest free gap is 30.
///
/// It is still the right trade, for two reasons the audit can now quantify:
///
/// * **Packing tighter buys almost nothing.** The ROM is generated per seed, so
///   an unwritten reservation is already `$FF` in the output — nobody pays for
///   it at runtime. Assigning offsets dynamically would only recover the
///   *headroom* inside allocations, and `audit_free_space` measures that at
///   about 39 bytes ROM-wide (`fx_screen_check` 30, `fire_flower` 6, three
///   others 1 each), an upper bound at that.
/// * **The only real win is overcommit, and it costs a guarantee.** Letting the
///   sum of reservations exceed a bank means some flag combinations stop
///   generating. A static map that fits always works; that property is worth
///   more than 40 bytes to a randomizer whose users tick arbitrary boxes.
///
/// On top of which the routines are not relocatable: hooks already are
/// (`jsr_into_bank` derives its operand from the allocation), but internal
/// absolute references are not — `FS_CANOE_SUMMON` is origin-locked by its own
/// `JMP $DEB7` and `ADC $DF2D` table reads, and the `FS_SAS_*` tables are read
/// from another bank. Moving patches at generation time means carrying
/// relocations per patch: a small linker, paid for by all 36 existing patches
/// and every future one.
///
/// **Reach for placement first.** Scarcity here is per-bank, so the cheap fix
/// when a tight bank fills is to move a flag-gated patch to a roomy one behind
/// a trampoline — PRG004 (426 bytes) and PRG006 (1392) are both empty.
///
/// **Revisit when** an always-mapped bank cannot hold a patch that genuinely
/// must run there (no trampoline available), *or* two features are provably
/// never on together. The second case does not need an allocator at all: a
/// registry row can name both owners, and the audit already knows how to check
/// that only one of them wrote.
pub const FREE_SPACE_ALLOCATIONS: &[FreeSpaceAlloc] = &[
    // PRG030 (fixed bank, always mapped $8000–$9FFF, file 0x3C010)
    fs(0x3DF20, 28, &["world_order"], "routine + tables"),
    fs(0x3DF3C, 20, &["big_q_blocks"], "big_q_block: save obj_ptr trampoline"),
    fs(0x3DFC6, 32, &["stomp_fairness"], "stomp_rise: rise-aware stomp height (32 reserved, 26 used)"),
    // PRG031 (always mapped $E000–$FFFF, file 0x3E010)
    fs(0x3E924, 25, &["title_screen"], "sprite copy routine"),
    fs(0x3E93D, 40, &["title_screen"], "sprite data table"),
    fs(0x3E260, 33, &["starting_items"], "lives + intro skip + menu music + inventory init trampoline"),
    fs(0x3E281, 69, &["start_airship_swap"], "4 tables (X/XHi/ScrL/ScrH × 8) + Map_Init seed helper"),
    fs(0x3E965, 13, &["title_screen"], "intro skip + menu music routine"),
    fs(0x3FFF0, 26, &["card_speed_clear"], "XOR trampoline"),
    // PRG026 (file 0x34010, CPU $A000–$BFFF)
    fs(0x35572, 13, &["mystery_anchor"], "item redirect trampoline"),
    fs(0x3557F, 50, &["hammer_breaks_tiles"], "hammer_locks: tile check subroutine + tables"),
    fs(0x355B1, 12, &["anchor_visuals"], "items-vs-cards index guard trampoline"),
    fs(0x355BD, 106, &["big_q_blocks"], "big_q_block: two-pass lookup routine + 13-entry tables (current-area then frozen entry)"),
    // PRG027 (file 0x36010, CPU $A000–$BFFF)
    fs(0x379D9, 894, &["king_quotes"], "7 quotes + hook (7×120 + 54)"),
    // PRG010 (file 0x14010, CPU $C000–$DFFF during map)
    fs(0x15554, 112, &["fx_screen_check"], "cross-screen lock patch (Fred's algorithm + darkness gate)"),
    fs(0x15DF0, 35, &["fix_canoe_softlock"], "canoe_fix: death respawn position save"),
    fs(0x15E13, 162, &["map_warp"], "2P Start+Select warp-to-partner routine"),
    fs(0x15EB5, 151, &["canoe_summon"], "A-on-dock call-the-boat routine + offset tables"),
    // PRG011 (file 0x16010, CPU $A000–$BFFF during map)
    fs(0x17C87, 36, &["start_airship_swap"], "game-over twirl finalize helper"),
    fs(0x17D00, 66, &["fix_canoe_softlock"], "canoe_fix: backup/restore subroutines (CANOE_BACKUP_ROUTINE)"),
    fs(0x17D50, 26, &["no_game_over_penalty"], "MaCobra NGO routine (macobra.rs NGO_ROUTINE_OFFSET)"),
    fs(0x17D70, 107, &["march_veto"], "landing-veto trampoline + per-world coord registry"),
    // PRG001 (file 0x02010, CPU $A000–$BFFF)
    fs(0x0382A, 23, &["koopalings"], "koopa_hits: subroutine + defeat JMP + threshold table"),
    fs(0x03841, 13, &["koopalings"], "koopa_collision_guard: skip collision bitmap during invuln"),
    fs(0x0384E, 16, &["koopalings"], "koopa_vram_clear: clear VRAM buffer on defeat"),
    fs(0x0385E, 12, &["koopalings"], "koopa_fire_preset: set stomp counter from threshold table for fireball defeat"),
    fs(0x03FD0, 22, &["koopalings"], "koopa_y_clamp: clamp Koopaling Y position to screen"),
    fs(0x03FE6, 36, &["fire_flower"], "position-hash suit routine + pool table"),
    fs(0x02713, 17, &["poison_mushrooms"], "object $0A init+hit stubs (Norm reuses 1-Up; reclaimed dead Obj0A code)"),
    fs(0x02724, 31, &["poison_mushrooms"], "1-up block-spawn hook (position-hash $0B->$0A)"),
    // PRG003 (file 0x06010, CPU $A000–$BFFF) — object AI bank (Boom-Boom lives here)
    fs(0x06609, 8, &["macobra"], "tail_stay_dead: MaCobra respawn-suppress routine (CPU $A5F9 gap)"),
    fs(0x07FCF, 16, &["boomboom"], "boomboom_hits: per-fortress threshold table"),
    fs(0x07FDF, 44, &["boomboom"], "boomboom_hits: decoupled stomp-count subroutine"),
    // PRG006 (file 0x0C010, CPU $C000–$DFFF) — level enemy data bank
    // `items` co-owns both clone blocks: the cloning module builds the stream,
    // then item randomization rewrites the OBJ_TREASURESET byte inside it
    // (items.rs reads the offsets from HAND_ROOM_CLONE_*_ITEM / P*_ROOM_ITEM).
    fs(0x0DA74, 22, &["hand_rooms", "items"], "2 cloned enemy streams for unique 8-Hnd treasure rooms"),
    fs(0x0DA8A, 22, &["piranha_rooms", "items"], "2 cloned chest-room streams with OBJ_TREASURESET for 7-P1/7-P2"),
    // PRG029 (file 0x3A010, CPU $C000–$DFFF) — swim physics bank
    fs(0x3A600, 24, &["faster_frog"], "Frog-Suit swim-speed boost routine"),
    // PRG000 (file 0x00010) — dead code at CPU $C918 (bytes skipped by the
    // vanilla `JMP $C927` at $C915), reused for MaCobra's hold-left fix helper.
    fs(0x00928, 7, &["macobra"], "hold_left_fix: scroll-commit helper (STA $FD; STA $0780; RTS)"),
];

// PRG030
pub(crate) const FS_WORLD_ORDER: usize       = 0x3DF20; // 28 bytes

/// CPU address of the world-order routine ($9F10). PRG030 is the MMC3 fixed
/// bank at $8000-$9FFF (file 0x3C010), so `prg_bank_file_to_cpu` (which assumes
/// the $A000 window) does not apply — `prg030_file_to_cpu` is its counterpart.
pub(crate) const WORLD_ORDER_CPU: u16        = super::prg030_file_to_cpu(FS_WORLD_ORDER);

pub(crate) const FS_BIG_Q_SAVE: usize        = 0x3DF3C; // 20 bytes

// PRG031
pub(crate) const FS_SEED_HASH_ROUTINE: usize = 0x3E924; // 25 bytes

pub(crate) const FS_SEED_HASH_DATA: usize    = 0x3E93D; // 40 bytes

pub(crate) const FS_INTRO_SKIP: usize        = 0x3E965; // 13 bytes

pub(crate) const FS_CARD_CLEAR: usize        = 0x3FFF0; // 26 bytes

pub(crate) const FS_STARTING_ITEMS: usize    = 0x3E260; // 33 bytes

// PRG031 — start_airship_swap engine scaffolding. One ~69-byte block: 4 × 8-byte
// per-world tables followed by a single assembled seed subroutine. PRG031 is
// always-mapped at $E000-$FFFF so Map_Init / GameOver_TwirlToStart (PRG011) can
// read the tables regardless of which bank is at $A000. NOTE: the PRG031 free run
// at 0x3E281 ends at 0x3E2D0 (real code follows) — only 79 bytes; do not grow this
// block past that ceiling.
pub(crate) const FS_SAS_BLOCK: usize             = 0x3E281;       // 69 bytes used (79 max)

pub(crate) const FS_SAS_X_TABLE: usize           = FS_SAS_BLOCK;       // 8 bytes — Mario X-low pixel per world

pub(crate) const FS_SAS_XHI_TABLE: usize         = FS_SAS_BLOCK + 8;   // 8 bytes — Mario screen index per world

pub(crate) const FS_SAS_SCRL_TABLE: usize        = FS_SAS_BLOCK + 16;  // 8 bytes — camera scroll low per world ($0722 / $7986)

pub(crate) const FS_SAS_SCRH_TABLE: usize        = FS_SAS_BLOCK + 24;  // 8 bytes — camera scroll high per world ($0724 / $7988)

// Single Map_Init seed subroutine: writes Mario's start position plus the primary
// AND secondary scroll backups from the four tables (replaces the former x/xhi
// helper pair). Reached via `JSR` from the Map_Init scroll-store site.
pub(crate) const FS_SAS_SEED_HELPER: usize       = FS_SAS_BLOCK + 32;  // 37 bytes

// The game-over twirl finalize helper lives in PRG011 free space (not FS_SAS_BLOCK
// — that PRG031 run has no room for it). PRG011 is the hook's own bank, so the JSR
// is bank-local; the helper still reads the FS_SAS_* tables in always-resident
// PRG031.
pub(crate) const FS_SAS_GAMEOVER_FINALIZE: usize = 0x17C87;  // PRG011, 36 bytes — stamps World_Map_X/XHi + primary/secondary scroll backup + live Horz_Scroll/Hi at twirl finalize (clean gap before FS_CANOE_BACKUP)

// Vanilla 8-byte Map_Y_Starts table (per-world Mario spawn Y-pixel). Lives in
// PRG030's world-enter routine. The start_airship_swap module rewrites this
// in place so swapped worlds spawn Mario at the airship row instead of the
// vanilla start row.
pub(crate) const MAP_Y_STARTS_OFF: usize  = 0x3C39A;

// Map_Init inline patch site in PRG011 (CPU $A237). The start_airship_swap module
// replaces the vanilla `STA $0724,X` scroll-store with `JSR seed_helper`, which
// overwrites the whole start position + scroll block from the FS_SAS_* tables. The
// earlier vanilla `LDA #$20 / STA $797A,X / STA $7982,X` X-low store at 0x16257 is
// left intact — the seed helper re-stamps $797A/$7982 later in the same loop
// iteration (before any draw), so the vanilla value is harmlessly overwritten.
pub(crate) const MAP_INIT_SCROLL_SITE: usize = 0x1627E;   // 3 bytes — `STA $0724,X`

// GameOver_TwirlToStart finalize hook in PRG011 (CPU $A6AA). The twirl is a
// delta-animation: it spirals Mario back to the start by a per-frame X/Y delta,
// then at finalize copies World_Map_X/XHi/Y into Map_Previous_*. The delta is
// low-byte/within-screen only and has a second hardcoded column-2 ($20) for the
// skid direction, so a swapped start on a different column/screen is unreachable
// by patching the delta. Instead we let the vanilla animation play and STAMP the
// correct start position at finalize: replace `STA Map_Prev_XHi2,X` (the last
// store before the World_Map → Map_Previous copies) with `JSR finalize helper`,
// which overwrites World_Map_X ($79,X) / World_Map_XHi ($77,X), the camera scroll
// ($0722/$0724,X) and both secondary backups ($7986/$7988) from the FS_SAS_*
// tables. The displaced `STA $7988` (A=0) is intentionally dropped: the helper now
// stamps $7988 with the start screen instead of zeroing it (nothing between the
// hook and the following copies reads it).
pub(crate) const GAMEOVER_FINALIZE_SITE: usize = 0x166BA;  // 3 bytes — `STA $7988,X` (Map_Prev_XHi2,X)

// Map_Object slot 1 == the airship sprite per southbird's disassembly:
// "NOTE: Assumes Index 1 is the Airship!"
pub(crate) const AIRSHIP_OBJ_SLOT: usize = 1;

// PRG026 — two-pass Big ? Block lookup (qol/big_q.rs), relocated off the old
// 0x35530 slot into tail free space to hold the two-pass routine + 12 entries.
pub(crate) const FS_BIG_Q_LOOKUP: usize      = 0x355BD; // 106 bytes (CPU $B5AD)

// PRG027
pub(crate) const FS_KING_QUOTES: usize       = 0x379D9; // 894 bytes

// PRG010
// Fred's visibility algorithm plus the busted / darkness gates (issue #131).
// The $FF run this sits in continues to 0x15810, so there is room to grow.
pub(crate) const FS_FX_SCREEN_CHECK: usize   = 0x15554; // 112 reserved, 82 used

pub(crate) const FS_CANOE_RESPAWN: usize     = 0x15DF0; // 35 bytes
pub(crate) const FS_MAP_WARP: usize          = 0x15E13; // 162 bytes (CPU $DE03)

// Canoe "call the boat" routine (CPU $DEA5). ORIGIN-LOCKED: the assembled bytes
// contain self-referential absolute addresses (JMP $DEB7, and the ADC $DF2D/31/35
// offset-table reads), so this MUST live at exactly 0x15EB5. See canoe_summon.rs.
pub(crate) const FS_CANOE_SUMMON: usize      = 0x15EB5; // 151 bytes (CPU $DEA5)

// PRG011
pub(crate) const FS_CANOE_BACKUP: usize      = 0x17D00; // 66 bytes

// March landing-veto trampoline + per-world coordinate registry (CPU $BD60).
// 59-byte routine + 8-byte per-world offset table + 40-byte address list.
// Hooked from Map_MarchValidateTravel's landing-zone PickTravel call at
// $B3FD; keeps wandering map objects off plant/army nodes and hand traps.
// Sits just past the 26-byte NGO routine (macobra.rs NGO_ROUTINE_OFFSET).
pub(crate) const FS_MARCH_VETO: usize        = 0x17D70; // 107 bytes (CPU $BD60)

// PRG026 (cont.)
pub(crate) const FS_MYSTERY_ANCHOR: usize    = 0x35572; // 13 bytes

pub(crate) const FS_HAMMER_LOCKS: usize      = 0x3557F; // 50 bytes

pub(crate) const FS_ANCHOR_ITEM_GUARD: usize = 0x355B1; // 12 bytes (CPU $B5A1)

// Stomp fairness — rise-aware stomp height, hooked from PRG000 at CPU $D22E.
// PRG030 is mapped at $8000–$9FFF at all times, so a JSR out of PRG000 reaches
// it whatever the $C000 window holds.
//
// The 74-byte run at 0x3DFC6 is the tail of PRG030 and is $FF in vanilla. It is
// unreferenced: a PRG-wide scan of every absolute-addressing opcode finds no
// operand in $9FB6-$9FD5. (The same scan reports two `JSR $9FF4` further up the
// run, at file 0x15D1F and 0x19D1F — those are byte-identical map data in
// PRG010 and PRG012, disassembled as data by southbird, not code. They sit
// outside this reservation either way, but anyone taking the remaining 42 bytes
// should confirm that before trusting the top of the gap.)
pub(crate) const FS_STOMP_RISE: usize        = 0x3DFC6; // 32 reserved, 26 used

/// CPU address of the rise-aware stomp-height routine ($9FB6).
pub(crate) const STOMP_RISE_CPU: u16         = super::prg030_file_to_cpu(FS_STOMP_RISE);

// PRG001 (file 0x02010, CPU $A000–$BFFF)
// Koopaling stomp handler is ObjHit_Koopaling in prg001.asm (southbird disassembly).
pub(crate) const FS_KOOPA_HITS_SUB: usize    = 0x0382A; // 13 code + 3 JMP + 7 table = 23 bytes

pub(crate) const FS_KOOPA_HITS_TABLE: usize  = 0x0383A; // 7 bytes (sub + 16)

/// CPU address of the subroutine: $A000 + (0x0382A - 0x02010) = $B81A
pub(crate) const KOOPA_HITS_SUB_CPU: u16     = 0xB81A;

/// CPU address of the threshold table: $A000 + (0x0383A - 0x02010) = $B82A
pub(crate) const KOOPA_HITS_TABLE_CPU: u16   = 0xB82A;

// Koopaling collision guard — skip collision bitmap update during invulnerability.
// Source: Fred's Koopaling fixes.
pub(crate) const FS_KOOPA_COLLISION_GUARD: usize = 0x03841; // 13 bytes

pub(crate) const KOOPA_COLLISION_GUARD_CPU: u16  = 0xB831;  // $A000 + (0x03841 - 0x02010)

// Koopaling defeat VRAM buffer clear — zero $0300/$0301 on defeat to prevent
// stale PPU writes during wand/king transition in non-native worlds.
// Source: Fred's Koopaling fixes.
pub(crate) const FS_KOOPA_VRAM_CLEAR: usize = 0x0384E; // 16 bytes

pub(crate) const KOOPA_VRAM_CLEAR_CPU: u16  = 0xB83E;  // $A000 + (0x0384E - 0x02010)

// Koopaling Y-position clamp — keep bouncing Koopalings on screen in non-native rooms.
// Source: Fred's Koopaling fixes.
pub(crate) const FS_KOOPA_Y_CLAMP: usize = 0x03FD0; // 22 bytes

pub(crate) const KOOPA_Y_CLAMP_CPU: u16  = 0xBFC0;  // $A000 + (0x03FD0 - 0x02010)

// Random Fire Flower (issue #22) — injected routine that derives the granted
// power state from a seed-derived salt (the shuffled starting world) + the
// current World_Num + the level layout pointer + the flower's screen number,
// instead of the vanilla hardcoded Fire. Sits in the PRG001 bank-end gap right
// after koopa_y_clamp (which ends at 0x3FE6). Up to 36 bytes: 26-byte routine +
// a 4- or 6-byte pool table. ObjHit_FireFlower runs with PRG001 banked at
// $A000, so the JSR from the hook is bank-local.
pub(crate) const FS_FIRE_FLOWER: usize     = 0x03FE6;

pub(crate) const FIRE_FLOWER_SUB_CPU: u16  = 0xBFD6; // $A000 + (0x03FE6 - 0x02010)

// Poison Mushroom object (ID $0A) — Init + Hit override stubs written over the
// dead vanilla Obj0A handler region ($A703-$A77D, never spawned by any level or
// code; see "Unused Group-0 Objects" in docs/smb3_rom_reference.md). Norm is not
// here: its table slot reuses the 1-Up's ObjNorm_PUp1UpMush ($A77E). Only 17
// bytes used; the rest of the dead region stays free for future group-0 objects.
pub(crate) const FS_POISON_MUSHROOM: usize = 0x02713; // CPU $A703

// Poison Mushroom 1-up spawn hook — patched into both block-spawn sites
// (PRG001 $A587 / $AADD) where `STA Level_ObjectID,Y` commits the object a hit
// block hands out. The routine replicates that store, and if the object is a
// 1-Up ($0B) position-hashes (World + LayPtr + block X/XHi/Y + seed salt) to
// keep it or swap it to the poison object ($0A). Sits right after the $0A
// stubs in the same reclaimed Obj0A region.
pub(crate) const FS_POISON_HOOK: usize = 0x02724; // CPU $A714

// Fireball defeat preset — load per-world stomp threshold from table so the
// fireball→stomp handoff always triggers defeat after INC.
pub(crate) const FS_KOOPA_FIRE_PRESET: usize = 0x0385E; // 12 bytes

pub(crate) const KOOPA_FIRE_PRESET_CPU: u16  = 0xB84E;  // $A000 + (0x0385E - 0x02010)

// PRG003 (file 0x06010, CPU $A000–$BFFF) — Boom-Boom stomp-count randomization.
// The Boom-Boom boss AI (ObjInit_BoomBoom, BoomBoom_HitTest, the DynJump state
// machine) lives in this bank, so the stomp handler's JMP into these routines is
// bank-local. Both allocations sit in the bank-end filler gap ($BFBF–$BFFF).
//
// Layout: 16-byte threshold table first, then the 44-byte subroutine.
// PRG003 8-byte dead gap between an RTS (file 0x06608) and the routine that
// starts at file 0x06611. MaCobra's "Tail Enemies don't respawn" fix drops an
// 8-byte respawn-suppress routine here, reached by a bank-local `JMP $A5F9`
// (see macobra.rs TAIL_STAY_DEAD_*).
pub(crate) const FS_TAIL_STAY_DEAD: usize = 0x06609; // 8 bytes (CPU $A5F9)

pub(crate) const FS_BOOMBOOM_HITS_TABLE: usize = 0x07FCF; // 16 bytes (CPU $BFBF)

pub(crate) const BOOMBOOM_HITS_TABLE_CPU: u16  = 0xBFBF;  // $A000 + (0x07FCF - 0x06010)

pub(crate) const FS_BOOMBOOM_HITS_SUB: usize   = 0x07FDF; // 44 bytes (CPU $BFCF)

pub(crate) const BOOMBOOM_HITS_SUB_CPU: u16    = 0xBFCF;  // $A000 + (0x07FDF - 0x06010)

// PRG006 — duplicated enemy streams for the W8 Hand sub-areas. Each clone is
// 11 bytes (page byte + 3 enemy entries + 0xFF terminator); two clones give
// the three Hand levels independent OBJ_TREASURESET item bytes.
//
// This starts one byte inside vanilla data, not in pure filler: 0x0DA74 is the
// 0xFF terminator of the object stream southbird labels `_DA60`
// (0x0DA70..0x0DA74). That stream is dead — nothing in the disassembly
// references it and no world pointer-table entry reaches past 0x0D9F6 — so
// overwriting its terminator is safe. It is also why the free-space audit
// reports this allocation as "sparse".
pub(crate) const FS_HAND_ROOMS: usize = 0x0DA74; // 22 bytes (2 × 11)

// PRG006 — piranha-shuffle chest-room clones (7-P1/7-P2), each 11 bytes:
// page byte + OBJ_TREASURESET + treasure box + box-appear trigger + 0xFF.
// Sits right after FS_HAND_ROOMS; the bank-end 0xFF filler runs to 0xE00F.
pub(crate) const FS_PIRANHA_ROOMS: usize = 0x0DA8A; // 22 bytes (2 × 11, CPU $DA7A)

// PRG029 (file 0x3A010, CPU $C000–$DFFF) — Frog-Suit swim-speed boost routine
// reached by a bank-local JSR $C5F0 from the swim-physics code. 24 bytes.
pub(crate) const FS_FASTER_FROG: usize = 0x3A600; // CPU $C5F0

// PRG000 dead code at CPU $C918 (file 0x928 = 0xC000 + (0x928 - 0x10)). The
// vanilla `JMP $C927` at $C915 skips these bytes and nothing else references
// them (verified: no `JSR/JMP $C918` anywhere in the ROM). MaCobra's hold-left
// fix drops a 7-byte scroll-commit helper here, reached by `JSR $C918` from the
// PRG008 in-level scroll tail.
pub(crate) const FS_HOLD_LEFT_HELPER: usize = 0x00928; // 7 bytes (CPU $C918)

// ---------------------------------------------------------------------------
// Auditor — the registry checked against what a run actually wrote
// ---------------------------------------------------------------------------
//
// The overlap and constants tests above check the registry against *itself*.
// This checks it against reality: after a randomization run, every byte the
// write log recorded inside a registered region is matched to the allocation
// that owns it. That catches a patch overrunning its reservation, a module
// writing into space it does not own, and `// N reserved, M used` comments
// that have drifted from the code.
//
// `used` counts bytes *covered by a logged write*, which is what a patch
// occupies — not the narrower "bytes that differ from vanilla". The two come
// apart because `write_range` logs the whole range whenever any byte in it
// differs, so a routine emitted as one range is measured exactly even where its
// bytes happen to match the filler underneath. A patch built byte-at-a-time
// with `write_byte` is the exception: there, no-op bytes are skipped and `used`
// under-counts.
//
// What the audit cannot see is a feature that was off for the run. Flag-gated
// code writes nothing, so an allocation showing 0 bytes means "not exercised",
// not "unused" — never read absence as evidence. (Overrun detection is
// unaffected by any of this: a write past the end that changes nothing changes
// nothing.)

/// Size of one PRG bank, and the file offset the PRG region starts at
/// (after the 16-byte iNES header).
const PRG_BANK_SIZE: usize = 0x2000;
const PRG_START: usize = 0x10;
const PRG_BANKS: usize = 32;

/// PRG bank containing a file offset. Meaningless for CHR offsets.
pub fn prg_bank_of(offset: usize) -> usize {
    (offset - PRG_START) / PRG_BANK_SIZE
}

/// One past the last file offset of the PRG bank containing `offset`.
///
/// A routine that runs past this executes whatever is paged in next, so it is
/// the hard bound on how far an allocation near the end of a bank may grow.
#[cfg(test)]
pub(crate) fn prg_bank_end(offset: usize) -> usize {
    PRG_START + (prg_bank_of(offset) + 1) * PRG_BANK_SIZE
}

/// True when write-log tag `tag` is covered by allocation owner `owner`.
///
/// Matches whole `/`-separated components, so `koopalings` owns
/// `koopalings/random_hits` but not `koopalings_extra`, and a full path like
/// `qol/starting_items` can be used when the short name would be ambiguous.
fn tag_owns(tag: &str, owner: &str) -> bool {
    format!("/{tag}/").contains(&format!("/{owner}/"))
}

/// What one allocation actually received during a run.
pub struct AllocUsage {
    pub alloc: &'static FreeSpaceAlloc,
    /// Bytes inside the region that the run changed.
    pub changed: usize,
    /// Region start through the last changed byte — the number that belongs in
    /// a `// N reserved, M used` comment. Zero when nothing was written.
    pub used: usize,
    /// Tags that wrote here without owning the region, and how many bytes each
    /// wrote. Any entry is a bug: either the owner is wrong or the module is.
    pub foreign: Vec<(String, usize)>,
    /// Writes crossing the region boundary: (offset, len, tag). A write
    /// starting inside and ending past the end is an overrun; one starting
    /// before is an encroachment from outside.
    pub overruns: Vec<(usize, usize, String)>,
}

impl AllocUsage {
    /// True when this allocation is in a state the registry does not describe.
    pub fn is_problem(&self) -> bool {
        !self.foreign.is_empty() || !self.overruns.is_empty()
    }
}

/// Cross-reference [`FREE_SPACE_ALLOCATIONS`] against `rom`'s write log.
/// Returns one entry per allocation, in registry order.
pub fn audit_free_space(rom: &Rom) -> Vec<AllocUsage> {
    use std::collections::BTreeMap;

    FREE_SPACE_ALLOCATIONS
        .iter()
        .map(|alloc| {
            let end = alloc.offset + alloc.size;
            let mut touched = vec![false; alloc.size];
            let mut foreign: BTreeMap<&str, usize> = BTreeMap::new();
            let mut overruns = Vec::new();

            for rec in rom.writes_in_range(alloc.offset, end) {
                if rec.offset < alloc.offset || rec.offset + rec.len > end {
                    overruns.push((rec.offset, rec.len, rec.tag.clone()));
                }
                let lo = rec.offset.max(alloc.offset);
                let hi = (rec.offset + rec.len).min(end);
                for b in lo..hi {
                    touched[b - alloc.offset] = true;
                }
                if !alloc.owners.iter().any(|o| tag_owns(&rec.tag, o)) {
                    *foreign.entry(&rec.tag).or_default() += hi - lo;
                }
            }

            AllocUsage {
                alloc,
                changed: touched.iter().filter(|t| **t).count(),
                used: touched.iter().rposition(|t| *t).map_or(0, |i| i + 1),
                foreign: foreign.into_iter().map(|(t, n)| (t.to_string(), n)).collect(),
                overruns,
            }
        })
        .collect()
}

/// Shortest run of identical filler bytes counted as free space. Small enough
/// to catch the scraps left in the always-mapped banks, large enough that
/// incidental byte pairs inside real data don't register.
const MIN_FILLER_RUN: usize = 8;

/// One unclaimed run of `$FF` filler: a place a patch could go.
#[derive(Clone, Copy)]
pub struct Gap {
    pub offset: usize,
    pub len: usize,
}

/// Unclaimed filler in one PRG bank, measured against the vanilla ROM.
///
/// **`gaps` is the field to make decisions from.** A patch needs one run of N
/// contiguous bytes in a bank that is mapped when it runs; no total answers
/// that, and the totals below carry caveats a gap does not (see
/// [`free_space_map`]).
pub struct BankFree {
    pub bank: usize,
    /// Bytes reserved by [`FREE_SPACE_ALLOCATIONS`] in this bank.
    pub allocated: usize,
    /// Unclaimed `$FF` runs, largest first. Candidates, not confirmations:
    /// filler that nothing has *claimed* may still be data something *reads*.
    /// Verify a gap against the disassembly once, then record it as a registry
    /// row and it never needs checking again.
    pub gaps: Vec<Gap>,
    /// `$FF` filler outside every allocation. An aggregate — useful for "is
    /// this bank roomy", never for "does my patch fit".
    pub free_ff: usize,
    /// `$00` runs outside every allocation. Reported apart from `free_ff`
    /// because zeroed *data* looks identical to zero padding — treat this
    /// column as a candidate list, not as available space.
    pub free_00: usize,
}

impl BankFree {
    /// Largest single unclaimed `$FF` run, or 0.
    pub fn largest_gap(&self) -> usize {
        self.gaps.first().map_or(0, |g| g.len)
    }

    /// Where the largest run starts, or 0 when there is none.
    pub fn largest_gap_at(&self) -> usize {
        self.gaps.first().map_or(0, |g| g.offset)
    }
}

/// Scan the vanilla PRG for filler runs no allocation has claimed.
///
/// Takes the *original* bytes, not the randomized ones: the question is how
/// much room the ROM has, and a run that this build has already written into
/// is still owned by whoever wrote it.
pub fn free_space_map(rom: &Rom) -> Vec<BankFree> {
    let mut claimed = vec![false; PRG_BANKS * PRG_BANK_SIZE];
    for a in FREE_SPACE_ALLOCATIONS {
        for b in a.offset..a.offset + a.size {
            claimed[b - PRG_START] = true;
        }
    }

    let prg = &rom.original[PRG_START..PRG_START + PRG_BANKS * PRG_BANK_SIZE];
    let mut banks: Vec<BankFree> = (0..PRG_BANKS)
        .map(|bank| BankFree { bank, allocated: 0, gaps: Vec::new(), free_ff: 0, free_00: 0 })
        .collect();

    for a in FREE_SPACE_ALLOCATIONS {
        banks[prg_bank_of(a.offset)].allocated += a.size;
    }

    // Walk each bank separately: a filler run spanning a bank boundary is two
    // gaps, not one, since a routine can only live in a bank that is mapped
    // when it runs.
    for (bank, entry) in banks.iter_mut().enumerate() {
        let base = bank * PRG_BANK_SIZE;
        let mut i = 0;
        while i < PRG_BANK_SIZE {
            let val = prg[base + i];
            if val != 0xFF && val != 0x00 {
                i += 1;
                continue;
            }
            let start = i;
            while i < PRG_BANK_SIZE && prg[base + i] == val {
                i += 1;
            }
            if i - start < MIN_FILLER_RUN {
                continue;
            }
            // Subtract the claimed parts, leaving the unclaimed sub-runs.
            let mut sub = start;
            while sub < i {
                if claimed[base + sub] {
                    sub += 1;
                    continue;
                }
                let sub_start = sub;
                while sub < i && !claimed[base + sub] {
                    sub += 1;
                }
                let len = sub - sub_start;
                if val == 0xFF {
                    entry.free_ff += len;
                    entry.gaps.push(Gap { offset: PRG_START + base + sub_start, len });
                } else {
                    entry.free_00 += len;
                }
            }
        }
    }

    for b in banks.iter_mut() {
        b.gaps.sort_by_key(|g| std::cmp::Reverse(g.len));
    }
    banks
}

/// Every unclaimed `$FF` gap of at least `need` bytes, largest first — the
/// query behind `--free-space --fit N`, and the only free-space question that
/// bears on where a new patch goes.
///
/// Ignore the totals when siting a patch: a bank with 200 free bytes in
/// ten scraps holds nothing. These are candidates — confirm the one you pick is
/// unreferenced (the disassembly is the oracle), then record it as a registry
/// row and the checking is done for good.
pub fn gaps_fitting(rom: &Rom, need: usize) -> Vec<(usize, Gap)> {
    let mut out: Vec<(usize, Gap)> = free_space_map(rom)
        .iter()
        .flat_map(|b| b.gaps.iter().filter(|g| g.len >= need).map(|g| (b.bank, *g)))
        .collect();
    out.sort_by_key(|(_, g)| std::cmp::Reverse(g.len));
    out
}

/// Human-readable free-space section for the `--write-log` dump: per-allocation
/// usage first (problems called out), then the per-bank budget.
pub fn format_free_space_report(rom: &Rom) -> String {
    format_alloc_audit(rom) + &format_bank_budget(rom)
}

/// Per-allocation half of the report. Reads the write log, so it describes the
/// run that produced `rom` — an allocation whose feature was off reads 0.
pub fn format_alloc_audit(rom: &Rom) -> String {
    use std::fmt::Write;

    let usage = audit_free_space(rom);
    let mut out = String::new();

    let problems: Vec<&AllocUsage> = usage.iter().filter(|u| u.is_problem()).collect();
    let _ = writeln!(out, "\n--- Free space: {} allocations, {} problem(s) ---", usage.len(), problems.len());
    for u in &problems {
        let _ = writeln!(
            out,
            "  !! 0x{:05X} {} (owner {})",
            u.alloc.offset, u.alloc.label, u.alloc.owners.join(" + "),
        );
        for (tag, n) in &u.foreign {
            let _ = writeln!(out, "     foreign write: {n} byte(s) tagged {tag}");
        }
        for (off, len, tag) in &u.overruns {
            let _ = writeln!(out, "     crosses boundary: 0x{off:05X}+{len} tagged {tag}");
        }
    }

    let _ = writeln!(out, "\n  bank   offset   used/reserved  owner");
    for u in &usage {
        let note = if u.used == 0 {
            "  (not written this run)"
        } else if u.changed != u.used {
            "  (sparse)"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "  PRG{:03} 0x{:05X}  {:>4}/{:<4}      {}{}",
            prg_bank_of(u.alloc.offset), u.alloc.offset, u.used, u.alloc.size,
            u.alloc.owners.join(" + "), note,
        );
    }
    let _ = writeln!(
        out,
        "\n  `used` is bytes covered by a logged write. Exact for a routine written as one\n  \
         write_range; an under-count only where a patch writes byte-at-a-time and some\n  \
         bytes already match. 0 means the feature was off, not that the space is unused."
    );

    out
}

/// Per-bank half of the report: what the ROM has left. Derived from the
/// *vanilla* bytes and the registry only — no write log, so it is identical for
/// every seed and every option set, and needs no randomization run to produce.
pub fn format_bank_budget(rom: &Rom) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    let _ = writeln!(out, "\n  bank   reserved  free($FF)  largest gap        zero runs");
    for b in free_space_map(rom) {
        if b.allocated == 0 && b.free_ff == 0 && b.free_00 == 0 {
            continue;
        }
        let gap = if b.gaps.is_empty() {
            String::from("-")
        } else {
            format!("{:>4} @ 0x{:05X}", b.largest_gap(), b.largest_gap_at())
        };
        let _ = writeln!(
            out,
            "  PRG{:03} {:>9} {:>10}  {:<18} {:>9}",
            b.bank, b.allocated, b.free_ff, gap, b.free_00,
        );
    }
    let _ = writeln!(
        out,
        "\n  Free space is per-bank: a routine must live in a bank mapped when it runs,\n  \
         so the largest-gap column decides what fits — never the free($FF) total, which\n  \
         can be ten unusable scraps. `--fit N` lists every gap that would hold N bytes.\n  \
         Runs shorter than {MIN_FILLER_RUN} bytes are not counted, and the zero-run column may be\n  \
         live data rather than padding."
    );

    out
}

/// Answer to "where can a patch of N bytes go" — the candidate list, printed by
/// `--free-space --fit N`.
pub fn format_gaps_fitting(rom: &Rom, need: usize) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    let fits = gaps_fitting(rom, need);
    if fits.is_empty() {
        let _ = writeln!(out, "\n  No unclaimed gap holds {need} bytes.");
        let _ = writeln!(
            out,
            "  A trampoline into a roomier bank is the usual answer; see the bank budget.",
        );
        return out;
    }

    let _ = writeln!(out, "\n  {} gap(s) hold {need} bytes:\n", fits.len());
    let _ = writeln!(out, "  bank    offset     size");
    for (bank, g) in &fits {
        let _ = writeln!(out, "  PRG{:03}  0x{:05X}  {:>7}", bank, g.offset, g.len);
    }
    let _ = writeln!(
        out,
        "\n  These are candidates, not confirmations. Unclaimed filler can still be data\n  \
         something reads: check the gap you pick against the disassembly\n  \
         (tools/southbird-smb3/PRG/prgNNN.asm) before claiming it, then add a\n  \
         FREE_SPACE_ALLOCATIONS row so it never needs checking again.\n  \
         The bank must also be mapped when your code runs — that is the first filter,\n  \
         not the last."
    );
    out
}

#[cfg(test)]
mod free_space_tests {
    use super::super::*;

    #[test]
    fn test_free_space_no_overlap() {
        for (i, a) in FREE_SPACE_ALLOCATIONS.iter().enumerate() {
            let a_end = a.offset + a.size;
            for b in &FREE_SPACE_ALLOCATIONS[i + 1..] {
                let b_end = b.offset + b.size;
                assert!(
                    a_end <= b.offset || b_end <= a.offset,
                    "free space overlap: '{}' (0x{:05X}..0x{:05X}) vs '{}' (0x{:05X}..0x{:05X})",
                    a.label, a.offset, a_end, b.label, b.offset, b_end,
                );
            }
        }
    }

    #[test]
    fn test_free_space_constants_match_registry() {
        let offsets: Vec<usize> = FREE_SPACE_ALLOCATIONS.iter().map(|a| a.offset).collect();
        // Every FS_* constant that names a whole allocation must be a
        // registry row. This list is exhaustive — add new FS_* consts here.
        for &(off, name) in &[
            (FS_WORLD_ORDER, "FS_WORLD_ORDER"),
            (FS_BIG_Q_SAVE, "FS_BIG_Q_SAVE"),
            (FS_SEED_HASH_ROUTINE, "FS_SEED_HASH_ROUTINE"),
            (FS_SEED_HASH_DATA, "FS_SEED_HASH_DATA"),
            (FS_INTRO_SKIP, "FS_INTRO_SKIP"),
            (FS_CARD_CLEAR, "FS_CARD_CLEAR"),
            (FS_STARTING_ITEMS, "FS_STARTING_ITEMS"),
            (FS_SAS_BLOCK, "FS_SAS_BLOCK"),
            (FS_SAS_GAMEOVER_FINALIZE, "FS_SAS_GAMEOVER_FINALIZE"),
            (FS_BIG_Q_LOOKUP, "FS_BIG_Q_LOOKUP"),
            (FS_MYSTERY_ANCHOR, "FS_MYSTERY_ANCHOR"),
            (FS_HAMMER_LOCKS, "FS_HAMMER_LOCKS"),
            (FS_ANCHOR_ITEM_GUARD, "FS_ANCHOR_ITEM_GUARD"),
            (FS_KING_QUOTES, "FS_KING_QUOTES"),
            (FS_FX_SCREEN_CHECK, "FS_FX_SCREEN_CHECK"),
            (FS_CANOE_RESPAWN, "FS_CANOE_RESPAWN"),
            (FS_CANOE_SUMMON, "FS_CANOE_SUMMON"),
            (FS_CANOE_BACKUP, "FS_CANOE_BACKUP"),
            (FS_MARCH_VETO, "FS_MARCH_VETO"),
            (FS_KOOPA_HITS_SUB, "FS_KOOPA_HITS_SUB"),
            (FS_KOOPA_COLLISION_GUARD, "FS_KOOPA_COLLISION_GUARD"),
            (FS_KOOPA_VRAM_CLEAR, "FS_KOOPA_VRAM_CLEAR"),
            (FS_KOOPA_FIRE_PRESET, "FS_KOOPA_FIRE_PRESET"),
            (FS_KOOPA_Y_CLAMP, "FS_KOOPA_Y_CLAMP"),
            (FS_FIRE_FLOWER, "FS_FIRE_FLOWER"),
            (FS_POISON_MUSHROOM, "FS_POISON_MUSHROOM"),
            (FS_POISON_HOOK, "FS_POISON_HOOK"),
            (FS_TAIL_STAY_DEAD, "FS_TAIL_STAY_DEAD"),
            (FS_BOOMBOOM_HITS_TABLE, "FS_BOOMBOOM_HITS_TABLE"),
            (FS_BOOMBOOM_HITS_SUB, "FS_BOOMBOOM_HITS_SUB"),
            (FS_HAND_ROOMS, "FS_HAND_ROOMS"),
            (FS_PIRANHA_ROOMS, "FS_PIRANHA_ROOMS"),
            (FS_FASTER_FROG, "FS_FASTER_FROG"),
            (FS_HOLD_LEFT_HELPER, "FS_HOLD_LEFT_HELPER"),
        ] {
            assert!(
                offsets.contains(&off),
                "{name} (0x{off:05X}) missing from FREE_SPACE_ALLOCATIONS"
            );
        }
        // Interior offsets (sub-tables inside a parent allocation) must fall
        // within some registry row.
        let covered = |o: usize| {
            FREE_SPACE_ALLOCATIONS.iter().any(|a| o >= a.offset && o < a.offset + a.size)
        };
        for &(off, name) in &[
            (FS_SAS_X_TABLE, "FS_SAS_X_TABLE"),
            (FS_SAS_XHI_TABLE, "FS_SAS_XHI_TABLE"),
            (FS_SAS_SCRL_TABLE, "FS_SAS_SCRL_TABLE"),
            (FS_SAS_SCRH_TABLE, "FS_SAS_SCRH_TABLE"),
            (FS_SAS_SEED_HELPER, "FS_SAS_SEED_HELPER"),
            (FS_KOOPA_HITS_TABLE, "FS_KOOPA_HITS_TABLE"),
        ] {
            assert!(
                covered(off),
                "{name} (0x{off:05X}) not covered by any FREE_SPACE_ALLOCATIONS row"
            );
        }
    }

    /// CLAUDE.md's per-bank budget table is derived data — the same numbers
    /// `free_space_map` computes — kept in the file because that is what gets
    /// read when a patch is being sized. This fails the moment a row drifts
    /// (adding an allocation shrinks whatever bank it lands in) and prints the
    /// replacement rows, so updating the table is copy-paste rather than
    /// arithmetic.
    ///
    /// Only rows already present are checked: the table lists the banks worth
    /// caring about, not all 32, and requiring every bank with spare filler to
    /// appear would bloat it for no gain.
    ///
    /// Skips without the ROM, like every other test that needs it.
    #[test]
    fn free_space_doc_table_is_current() {
        let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        let rom = crate::rom::Rom::from_bytes(&bytes).expect("valid ROM");
        let doc = std::fs::read_to_string("CLAUDE.md").expect("CLAUDE.md");
        let measured = free_space_map(&rom);

        let mut rows = 0;
        let mut wrong = Vec::new();
        for line in doc.lines() {
            // | PRG010 | `$C000–$DFFF`, map | 896 | 588 |
            let cells: Vec<&str> = line.split('|').map(|c| c.trim()).collect();
            if cells.len() < 5 || !cells[1].starts_with("PRG") {
                continue;
            }
            let strip = |c: &str| c.trim_matches('*').to_string();
            let (Ok(bank), Ok(free), Ok(gap)) = (
                cells[1][3..].parse::<usize>(),
                strip(cells[3]).parse::<usize>(),
                strip(cells[4]).parse::<usize>(),
            ) else {
                continue;
            };
            rows += 1;
            let m = &measured[bank];
            if free != m.free_ff || gap != m.largest_gap() {
                wrong.push(format!(
                    "  PRG{bank:03}: table says {free} free / {gap} gap, measured {} / {}",
                    m.free_ff, m.largest_gap(),
                ));
            }
        }

        assert!(rows >= 8, "parsed only {rows} bank rows from CLAUDE.md — did the table's shape change?");
        assert!(
            wrong.is_empty(),
            "CLAUDE.md's per-bank free-space table is stale:\n{}\n\n\
             Regenerate with `smb3-rs <rom> --free-space` and replace the rows.",
            wrong.join("\n"),
        );
    }

    // Ground-truth pins for the PRG bank ↔ file-offset mapping. Each pair is a
    // *known-correct* (bank, cpu, file) triple taken from a shipped patch, so
    // these tests catch drift in the mapping itself — including dropping the
    // 0x10 iNES header, the issue #14 root cause. Patch code derives operands
    // from these helpers (e.g. `jsr_into_bank`) rather than transcribing
    // address bytes, so getting the helpers right protects every hook.
    #[test]
    fn test_prg_bank_mapping_known_pairs() {
        // PRG011 canoe backup routine: file 0x17D00 ↔ CPU $BCF0 (JSR $BCF0).
        assert_eq!(prg_bank_cpu_to_file(11, 0xBCF0), 0x17D00);
        assert_eq!(prg_bank_file_to_cpu(11, 0x17D00), 0xBCF0);
        // Bank start maps to the window base, header included.
        assert_eq!(prg_bank_cpu_to_file(11, 0xA000), 0x16010);
        assert_eq!(prg_bank_file_to_cpu(11, 0x16010), 0xA000);
        // Round-trips across banks and the whole window.
        for bank in [9usize, 11, 13, 26] {
            for cpu in [0xA000u16, 0xA001, 0xB425, 0xBD32, 0xBFFF] {
                assert_eq!(prg_bank_file_to_cpu(bank, prg_bank_cpu_to_file(bank, cpu)), cpu);
            }
        }

        // The two fixed banks live outside the $A000 window and need their own
        // converters. These are the values the shipped patches assemble with,
        // so a regression here would silently retarget every PRG030/031 hook.
        assert_eq!(prg030_file_to_cpu(0x3C010), 0x8000);
        assert_eq!(prg030_file_to_cpu(FS_WORLD_ORDER), 0x9F10);
        assert_eq!(prg030_file_to_cpu(FS_STOMP_RISE), 0x9FB6);
        assert_eq!(prg031_file_to_cpu(0x3E010), 0xE000);
    }

    #[test]
    fn test_jsr_into_bank_builds_correct_operand() {
        // `JSR <file 0x17D00 in bank 11>` must encode as 20 F0 BC (JSR $BCF0).
        assert_eq!(jsr_into_bank(11, 0x17D00), [0x20, 0xF0, 0xBC]);
        // Opcode is always JSR; operand is little-endian CPU address.
        let j = jsr_into_bank(11, FS_MARCH_VETO);
        assert_eq!(j[0], 0x20);
        let cpu = u16::from_le_bytes([j[1], j[2]]);
        assert_eq!(prg_bank_cpu_to_file(11, cpu), FS_MARCH_VETO);
    }
}
