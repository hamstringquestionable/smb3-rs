//! Shared ROM constants, data structures, and helpers for SMB3 randomization.
//!
//! This module holds all the shared knowledge about the ROM layout — constants,
//! lookup tables, data structures, and low-level read/write helpers — used by
//! multiple randomization modules. The BFS map walker lives in `map_walker.rs`.

use crate::rom::Rom;


// ---------------------------------------------------------------------------
// Free space registry
// ---------------------------------------------------------------------------
//
// Central registry of all free space allocations where we write assembled code
// or data tables into the ROM. Every module that needs free space MUST define
// its allocation here — never use a local constant for a free space offset.
//
// Before adding a new allocation:
//   1. Check this list for the target bank
//   2. Pick an address after the last allocation in that bank
//   3. Run `cargo test` — the overlap test will catch mistakes

/// Free space allocation: (file_offset, size_bytes, label).
/// The overlap test checks that no two allocations in this list share any bytes.
#[cfg(test)]
const FREE_SPACE_ALLOCATIONS: &[(usize, usize, &str)] = &[
    // PRG030 (fixed bank, always mapped $8000–$9FFF, file 0x3C010)
    (0x3DF20, 28, "world_order: routine + tables"),
    (0x3DF3C, 20, "big_q_block: save obj_ptr trampoline"),
    // PRG031 (always mapped $E000–$FFFF, file 0x3E010)
    (0x3E924, 25, "title_screen: sprite copy routine"),
    (0x3E93D, 40, "title_screen: sprite data table"),
    (0x35572, 13, "mystery_anchor: item redirect trampoline"),
    (0x3557F, 50, "hammer_locks: tile check subroutine + tables"),
    (0x3E260, 33, "starting_items: lives + intro skip + menu music + inventory init trampoline"),
    (0x3E281, 69, "start_airship_swap: 4 tables (X/XHi/ScrL/ScrH × 8) + X helper + XHi helper + game-over X helper"),
    (0x3E965, 13, "title_screen: intro skip + menu music routine"),
    (0x3FFF0, 26, "card_speed_clear: XOR trampoline"),
    // PRG026 (file 0x34010, CPU $A000–$BFFF)
    (0x35530, 66, "big_q_block: lookup routine + tables"),
    (0x355B1, 12, "anchor_visuals: items-vs-cards index guard trampoline"),
    // PRG027 (file 0x36010, CPU $A000–$BFFF)
    (0x379D9, 894, "king_quotes: 7 quotes + hook (7×120 + 54)"),
    // PRG010 (file 0x14010, CPU $C000–$DFFF during map)
    (0x15554, 80, "fx_screen_check: cross-screen lock patch (Fred's algorithm verbatim)"),
    (0x15DF0, 35, "canoe_fix: death respawn position save"),
    // PRG011 (file 0x16010, CPU $A000–$BFFF during map)
    (0x17D00, 59, "canoe_fix: backup/restore subroutines"),
    // PRG001 (file 0x02010, CPU $A000–$BFFF)
    (0x0382A, 23, "koopa_hits: subroutine + defeat JMP + threshold table"),
    (0x03841, 13, "koopa_collision_guard: skip collision bitmap during invuln"),
    (0x0384E, 16, "koopa_vram_clear: clear VRAM buffer on defeat"),
    (0x0385E, 12, "koopa_fire_preset: set stomp counter from threshold table for fireball defeat"),
    (0x03FD0, 22, "koopa_y_clamp: clamp Koopaling Y position to screen"),
    // PRG006 (file 0x0C010, CPU $C000–$DFFF) — level enemy data bank
    (0x0DA74, 22, "hand_rooms: 2 cloned enemy streams for unique 8-Hnd treasure rooms"),
    // PRG004 (file 0x08010, CPU $A000–$BFFF) — big piranha bank
    (0x09E66,  8, "piranha_visibility: big piranha (0x7D/0x7F) init thunk"),
    // PRG005 (file 0x0A010, CPU $A000–$BFFF) — small piranha bank
    (0x0BFD6,  7, "piranha_visibility: small piranha (0xA0–0xA7) init thunk"),
    (0x0BFDD, 18, "piranha_visibility: small piranha (0xA0–0xA7) hit-skip thunk"),
];

// Individual constants for use by each module.

// PRG030
pub(super) const FS_WORLD_ORDER: usize       = 0x3DF20; // 28 bytes
pub(super) const FS_BIG_Q_SAVE: usize        = 0x3DF3C; // 20 bytes

// PRG031
pub(super) const FS_SEED_HASH_ROUTINE: usize = 0x3E924; // 25 bytes
pub(super) const FS_SEED_HASH_DATA: usize    = 0x3E93D; // 40 bytes
pub(super) const FS_INTRO_SKIP: usize        = 0x3E965; // 13 bytes
pub(super) const FS_CARD_CLEAR: usize        = 0x3FFF0; // 26 bytes

// PRG031 — start_airship_swap engine scaffolding. One 69-byte block split into
// 4 × 8-byte per-world tables followed by three assembled subroutines. PRG031
// is always-mapped at $E000-$FFFF so Map_Init / GameOver_TwirlToStart (PRG011)
// can JSR into it directly.
pub(super) const FS_SAS_BLOCK: usize             = 0x3E281;       // 69 bytes total
pub(super) const FS_SAS_X_TABLE: usize           = FS_SAS_BLOCK;       // 8 bytes — Mario X-low pixel per world
pub(super) const FS_SAS_XHI_TABLE: usize         = FS_SAS_BLOCK + 8;   // 8 bytes — Mario screen index per world
pub(super) const FS_SAS_SCRL_TABLE: usize        = FS_SAS_BLOCK + 16;  // 8 bytes — camera scroll low per world ($0722)
pub(super) const FS_SAS_SCRH_TABLE: usize        = FS_SAS_BLOCK + 24;  // 8 bytes — camera scroll high per world ($0724)
pub(super) const FS_SAS_X_HELPER: usize          = FS_SAS_BLOCK + 32;  // 10 bytes — writes Map_Entered_X / $7982
pub(super) const FS_SAS_XHI_HELPER: usize        = FS_SAS_BLOCK + 42;  // 22 bytes — writes Map_Entered_XHi + death-respawn XHi + scroll seeds
pub(super) const FS_SAS_GAMEOVER_X_HELPER: usize = FS_SAS_BLOCK + 64;  // 5 bytes — A := A - FS_SAS_X_TABLE[Y]; RTS

// Vanilla 8-byte Map_Y_Starts table (per-world Mario spawn Y-pixel). Lives in
// PRG030's world-enter routine. The start_airship_swap module rewrites this
// in place so swapped worlds spawn Mario at the airship row instead of the
// vanilla start row.
pub(super) const MAP_Y_STARTS_OFF: usize  = 0x3C39A;

// Map_Init inline patch sites in PRG011 (CPU $A237). The start_airship_swap
// module replaces these with `JSR helper` so the per-world spawn position
// loads from the FS_SAS_* tables instead of the vanilla inline immediates.
pub(super) const MAP_INIT_X_LOW_SITE: usize  = 0x16257;   // 8 bytes — `LDA #$20 / STA $797A,X / STA $7982,X`
pub(super) const MAP_INIT_SCROLL_SITE: usize = 0x1627E;   // 3 bytes — `STA $0724,X`

// GameOver_TwirlToStart inline patch site in PRG011 (CPU $A5F1). Vanilla
// hardcodes the spiral-back-to-start X target as column 2 (`SEC / SBC #$20`).
// SAS replaces these 3 bytes with `JSR FS_SAS_GAMEOVER_X_HELPER` so the
// delta is computed against the per-world X start instead. Y is preloaded
// with World_Num two instructions earlier (`LDY $0727`) and isn't modified
// before this site, so the helper can use `SBC FS_SAS_X_TABLE,Y` directly.
pub(super) const GAMEOVER_TWIRL_X_SUB_SITE: usize = 0x16601;  // 3 bytes — `SEC / SBC #$20`

// Map_Object slot 1 == the airship sprite per southbird's disassembly:
// "NOTE: Assumes Index 1 is the Airship!"
pub(super) const AIRSHIP_OBJ_SLOT: usize = 1;

// PRG026
pub(super) const FS_BIG_Q_LOOKUP: usize      = 0x35530; // 66 bytes

// PRG027
pub(super) const FS_KING_QUOTES: usize       = 0x379D9; // 894 bytes

// PRG010
pub(super) const FS_FX_SCREEN_CHECK: usize   = 0x15554; // 80 bytes (Fred's algorithm)
pub(super) const FS_CANOE_RESPAWN: usize     = 0x15DF0; // 35 bytes

// PRG011
pub(super) const FS_CANOE_BACKUP: usize      = 0x17D00; // 59 bytes

// PRG026 (cont.)
pub(super) const FS_MYSTERY_ANCHOR: usize    = 0x35572; // 13 bytes
pub(super) const FS_HAMMER_LOCKS: usize      = 0x3557F; // 50 bytes
pub(super) const FS_ANCHOR_ITEM_GUARD: usize = 0x355B1; // 12 bytes (CPU $B5A1)
pub(super) const FS_STARTING_ITEMS: usize    = 0x3E260; // 33 bytes

// PRG001 (file 0x02010, CPU $A000–$BFFF)
// Koopaling stomp handler is ObjHit_Koopaling in prg001.asm (southbird disassembly).
pub(super) const FS_KOOPA_HITS_SUB: usize    = 0x0382A; // 13 code + 3 JMP + 7 table = 23 bytes
pub(super) const FS_KOOPA_HITS_TABLE: usize  = 0x0383A; // 7 bytes (sub + 16)
/// CPU address of the subroutine: $A000 + (0x0382A - 0x02010) = $B81A
pub(super) const KOOPA_HITS_SUB_CPU: u16     = 0xB81A;
/// CPU address of the threshold table: $A000 + (0x0383A - 0x02010) = $B82A
pub(super) const KOOPA_HITS_TABLE_CPU: u16   = 0xB82A;

// Koopaling collision guard — skip collision bitmap update during invulnerability.
// Source: Fred's Koopaling fixes.
pub(super) const FS_KOOPA_COLLISION_GUARD: usize = 0x03841; // 13 bytes
pub(super) const KOOPA_COLLISION_GUARD_CPU: u16  = 0xB831;  // $A000 + (0x03841 - 0x02010)

// Koopaling defeat VRAM buffer clear — zero $0300/$0301 on defeat to prevent
// stale PPU writes during wand/king transition in non-native worlds.
// Source: Fred's Koopaling fixes.
pub(super) const FS_KOOPA_VRAM_CLEAR: usize = 0x0384E; // 16 bytes
pub(super) const KOOPA_VRAM_CLEAR_CPU: u16  = 0xB83E;  // $A000 + (0x0384E - 0x02010)

// Koopaling Y-position clamp — keep bouncing Koopalings on screen in non-native rooms.
// Source: Fred's Koopaling fixes.
pub(super) const FS_KOOPA_Y_CLAMP: usize = 0x03FD0; // 22 bytes
pub(super) const KOOPA_Y_CLAMP_CPU: u16  = 0xBFC0;  // $A000 + (0x03FD0 - 0x02010)

// Fireball defeat preset — load per-world stomp threshold from table so the
// fireball→stomp handoff always triggers defeat after INC.
pub(super) const FS_KOOPA_FIRE_PRESET: usize = 0x0385E; // 12 bytes
pub(super) const KOOPA_FIRE_PRESET_CPU: u16  = 0xB84E;  // $A000 + (0x0385E - 0x02010)

// PRG006 — duplicated enemy streams for the W8 Hand sub-areas. Each clone is
// 11 bytes (page byte + 3 enemy entries + 0xFF terminator); two clones give
// the three Hand levels independent OBJ_TREASURESET item bytes.
pub(super) const FS_HAND_ROOMS: usize = 0x0DA74; // 22 bytes (2 × 11)

// PRG004 — big piranha (0x7D / 0x7F) visibility thunk. Patched into the shared
// init tail so Var4 is primed to state 1 (Emerge) on spawn, skipping the
// invisible HideInPipe state when a wild-shuffle drops one in a non-pipe slot.
pub(super) const FS_PIRANHA_VIS_BIG: usize = 0x09E66; // 8 bytes
/// CPU address of the big-piranha thunk: $A000 + (0x09E66 - 0x08010) = $BE56
pub(super) const PIRANHA_VIS_BIG_CPU: u16  = 0xBE56;

// PRG005 — small piranha (0xA0–0xA7) visibility thunk. Same purpose as the
// big-piranha thunk but for the eight small-piranha IDs which share a single
// init tail in PRG005.
pub(super) const FS_PIRANHA_VIS_SMALL: usize = 0x0BFD6; // 7 bytes
/// CPU address of the small-piranha thunk: $A000 + (0x0BFD6 - 0x0A010) = $BFC6
pub(super) const PIRANHA_VIS_SMALL_CPU: u16  = 0xBFC6;

// PRG005 — small piranha (0xA0–0xA7) per-frame hit-skip thunk. Replaces the
// `JSR Player_HitEnemy` in `ObjNorm_Piranha` with a distance-based gate:
// skip the call whenever the piranha's current Y is within ±10 px of its
// hidden-position Var5. That covers the fully-hidden state and adds ~10
// frames of safety on either side of the transition (Retract end and
// Emerge start), and is orientation-agnostic — same metric works for both
// upright and ceiling variants because Var5 is the spawn/hidden Y in both
// cases. The big-piranha bank already short-circuits state 0 in
// `ObjNorm_BigPiranha` (`JMP $B79D` past the JSR) so no equivalent thunk is
// needed for 0x7D / 0x7F.
pub(super) const FS_PIRANHA_HIT_SMALL: usize = 0x0BFDD; // 18 bytes
/// CPU address of the small-piranha hit-skip thunk: $A000 + (0x0BFDD - 0x0A010) = $BFCD
pub(super) const PIRANHA_HIT_SMALL_CPU: u16  = 0xBFCD;


// ---------------------------------------------------------------------------
// Shared type aliases
// ---------------------------------------------------------------------------

/// A grid coordinate on an overworld map, in `(row, col)` order.
pub(super) type Pos = (usize, usize);

/// An ordered pair of grid positions that BFS treats as a single edge —
/// the path between them is skipped. Used for vanilla pipe pairs, W3
/// canoe edges, and any future traversal feature that connects two
/// positions without a walkable path between them.
pub(super) type TeleportEdge = (Pos, Pos);

// ---------------------------------------------------------------------------
// Tile constants
// ---------------------------------------------------------------------------

/// Valid horizontal path tiles (Map_Object_Valid_Left/Right in PRG010).
pub(super) const VALID_HORZ: &[u8] = &[0x45, 0x49, 0xB2, 0xB3, 0xAC, 0xB7, 0xB8, 0xDA, 0xB9, 0xE6, 0xE8];

/// Valid vertical path tiles (Map_Object_Valid_Down/Up in PRG010).
pub(super) const VALID_VERT: &[u8] = &[0x46, 0xB1, 0xAA, 0xAB, 0xB0, 0xDB, 0xBA, 0xE8];

/// Background / non-walkable tiles.
pub(super) const BACKGROUND_TILES: &[u8] = &[0xB4, 0xFF, 0x02];

/// Valid blank node tiles — positions with these tiles are available for
/// level/fort/pipe/HB placement. Used by both pickup (Phase 2) and build
/// (Phase 3) to ensure consistent blank detection.
pub(super) const VALID_BLANK_TILES: &[u8] = &[
    0x44, 0x47, 0x48, 0x4A,        // standard
    0xAE, 0xAF, 0xB5, 0xB6,        // island
    0xD9, 0xDC, 0xDD, 0xDE,        // sky
];

/// Start tile ID.
pub(super) const TILE_START: u8 = 0xE5;

/// W3 canoe teleport edges: (world_idx, (origin, destination)).
/// The canoe transports the player from the mainland dock at (6,20) to two
/// island docks. These are bidirectional teleport edges in BFS, like pipes.
/// The mainland dock is only reachable when rocks are cleared.
pub(super) const CANOE_EDGES: &[(usize, TeleportEdge)] = &[
    (2, ((6, 20), (5, 24))),  // mainland dock → island 1
    (2, ((6, 20), (0, 32))),  // mainland dock → island 2
];

// ---------------------------------------------------------------------------
// Level data regions and tile generator dispatch tables
// ---------------------------------------------------------------------------

/// A level data region: file offset range + tileset-specific extra-byte dispatches.
///
/// Most tile generator commands are 3 bytes, but some variable-size routines
/// read a 4th byte from the layout stream. `extra_byte_dispatches` lists the
/// variable-size dispatch indices that consume 4 bytes for this tileset.
///
/// Dispatch index = group * 15 + (byte2 >> 4) - 1, where group = (byte0 >> 5).
///
/// Verified against the Southbird SMB3 disassembly per-tileset dispatch tables
/// (LoadLevel_Generator_TSx in PRG013-023). Handlers that call
/// LoadLevel_GetLayoutByte, LL_GetLayoutByte_AndBackup, LL21_InitLongRun,
/// or equivalent are 4-byte.
pub(super) struct LevelDataRegion {
    pub start: usize,
    pub end: usize,
    pub extra_byte_dispatches: &'static [u8],
    /// Whether group 2 fixed-size shapes 1-6 are note/wood powerups in this
    /// tileset. In most tilesets they are, but in TS2 (Dungeon) shapes 1-2 map
    /// to CCBridge and shapes 3-7 map to TopDecoBlocks — swapping them would
    /// corrupt level geometry.
    pub randomize_note_wood: bool,
}

/// Level data regions by tileset (file offset ranges + extra-byte dispatch info).
pub(super) const LEVEL_DATA_REGIONS: &[LevelDataRegion] = &[
    LevelDataRegion { // Underground (TS14) — same dispatch table as TS3
        start: 0x1A587, end: 0x1C005,
        extra_byte_dispatches: &[
            35, 36, 37, 38, 39, 40, 41, 42, // TopDecoBlocks
            60, 61, 62,                       // BGOrWater
            63, 64, 65, 66, 67, 68,           // DecoGround
            69, 70, 71,                       // DecoCeiling
        ],
        randomize_note_wood: true,
    },
    LevelDataRegion { // Plains (TS1)
        start: 0x1E512, end: 0x20005,
        extra_byte_dispatches: &[
            11, 12,                            // GroundRun
            35, 36, 37, 38, 39, 40, 41, 42,   // TopDecoBlocks
        ],
        randomize_note_wood: true,
    },
    LevelDataRegion { // Hilly (TS3)
        start: 0x20587, end: 0x22005,
        extra_byte_dispatches: &[
            35, 36, 37, 38, 39, 40, 41, 42, // TopDecoBlocks
            60, 61, 62,                       // BGOrWater
            63, 64, 65, 66, 67, 68,           // DecoGround
            69, 70, 71,                       // DecoCeiling
        ],
        randomize_note_wood: true,
    },
    LevelDataRegion { // Ice / Sky (TS4/12)
        start: 0x227E0, end: 0x24005,
        extra_byte_dispatches: &[
            0,                                 // LongWoodBlock
            35, 36, 37, 38, 39, 40, 41, 42,   // TopDecoBlocks
            54,                                // Muncher17
            60,                                // Group 4 variable
            112,                               // Group 7 variable
        ],
        randomize_note_wood: true,
    },
    LevelDataRegion { // Pipe / Water (TS7)
        start: 0x24BA7, end: 0x26005,
        extra_byte_dispatches: &[
            35, 36, 37, 38, 39, 40, 41, 42,   // TopDecoBlocks
            49,                                // OrangeBlock
            57,                                // WaterFill
        ],
        randomize_note_wood: true,
    },
    LevelDataRegion { // Cloudy / Giant / Plant (TS5/11/13)
        start: 0x26A6F, end: 0x28C05,
        extra_byte_dispatches: &[
            13,                                // DoubleCloud
            35, 36, 37, 38, 39, 40, 41, 42,   // TopDecoBlocks
            45,                                // CloudGoal
            46,                                // RoundCloudTop
            48,                                // CloudSpace
            51,                                // Lava
        ],
        randomize_note_wood: true,
    },
    LevelDataRegion { // Desert (TS9)
        start: 0x28F36, end: 0x2A005,
        extra_byte_dispatches: &[
            10, 11, 12, 13,                    // DiagRect variants
            35, 36, 37, 38, 39, 40, 41, 42,   // TopDecoBlocks
        ],
        randomize_note_wood: false, // shapes 1-5 = desert decorations (palms, cacti) in TS9
    },
    LevelDataRegion { // Dungeon (TS2)
        start: 0x2A7F7, end: 0x2C005,
        extra_byte_dispatches: &[
            13, 14,                            // SolidBrick, BrightDiamondLong (LL21_InitLongRun)
            35, 36, 37, 38, 39, 40, 41, 42,   // TopDecoBlocks
            46, 47,                            // Background (LoadLevel21_Background)
            48,                                // Lava
            57,                                // BrightDiamond (4-byte like BrightDiamondLong)
            95, 96,                            // Group 6 handlers (empirically verified 4-byte)
        ],
        randomize_note_wood: false, // shapes 1-2 = CCBridge, 3-7 = TopDecoBlocks in TS2
    },
    LevelDataRegion { // Ship (TS10)
        start: 0x2EC07, end: 0x30005,
        extra_byte_dispatches: &[
            1, 2,                              // WoodBodyLong
            35, 36, 37, 38, 39, 40, 41, 42,   // TopDecoBlocks
            48,                                // MetalPlate
            // 49 (Crate) is 3-byte, NOT 4-byte — was causing W5 airship corruption
            51,                                // DoubleTipBodyWood
        ],
        randomize_note_wood: true,
    },
];

/// Pipe tile ID.
pub(super) const TILE_PIPE: u8 = 0xBC;

/// Fortress map tile ID (used in test code across multiple modules).
#[allow(dead_code)]
pub(super) const TILE_FORTRESS: u8 = 0x67;

/// Airship dock tile ID.
pub(super) const TILE_AIRSHIP: u8 = 0xC9;

/// Bowser's castle tile ID.
pub(super) const TILE_BOWSER: u8 = 0xCC;

/// Bonus game (spade/N-Spade) tile ID.
pub(super) const TILE_BONUS_GAME: u8 = 0xE8;

/// Toad House placeholder tile ID. Vanilla Toad Houses use either 0x50 or
/// 0xE0; the build phase stamps this constant when a HammerBro slot is
/// promoted to a Toad House. The writer later overwrites the cell with the
/// per-entry vanilla tile from the catalog.
pub(super) const TILE_TOAD_HOUSE: u8 = 0x50;

/// Placeholder stamped on the BFS grid to mark a position as non-background.
/// The actual value is irrelevant — it just needs to be outside BACKGROUND_TILES
/// so walk_map treats the position as a reachable node.
pub(super) const TILE_NODE: u8 = 0x47;

/// Number of rows in every overworld map.
pub(super) const ROWS: usize = 9;

// ---------------------------------------------------------------------------
// ROM offset constants
// ---------------------------------------------------------------------------

// Pipe destination tables (PRG002)
pub(super) const PIPE_MAP_XHI: usize = 0x046AA;
pub(super) const PIPE_MAP_X: usize = 0x046C2;
pub(super) const PIPE_MAP_Y: usize = 0x046DA;
pub(super) const PIPE_MAP_SCRL_XHI: usize = 0x046F2;

// FX table offsets (17 slots)
pub(super) const FX_VADDR_H: usize = 0x147CD;
pub(super) const FX_VADDR_L: usize = 0x147DE;
pub(super) const FX_MAP_COMP_IDX: usize = 0x147EF; // 17 x 2 bytes
pub(super) const FX_PATTERNS: usize = 0x14811;     // 17 x 4 bytes
pub(super) const FX_MAP_LOC_ROW: usize = 0x14855;
pub(super) const FX_MAP_LOC: usize = 0x14866;
pub(super) const FX_MAP_TILE_REPLACE: usize = 0x14877;
pub(super) const FX_WORLD_TABLE: usize = 0x14888;

/// Map_Complete_Bits lookup table: maps grid row to completion bit.
/// Row 0 = $80, row 1 = $40, ..., row 7 = $01.
pub(super) const MAP_COMPLETE_BITS: [u8; 8] = [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01];

/// Vanilla `(world_idx, entry_idx)` pairs identifying levels whose enemy
/// stream contains an in-level treasure chest (`OBJ_TREASURESET`, ID `$D6`).
/// Chests yield inventory items (Music Box, Cloud, Warp Whistle, Star,
/// P-Wing, Anchor, Hammer) as opposed to form-changing power-ups, which
/// makes them player-visible "rewards" rather than power-up rolls.
///
/// The chest sub-area travels with the level data when the level pool
/// shuffles, so this list is keyed by vanilla position and remains valid
/// after shuffling. `is_chest_level` resolves a `CatalogEntry`'s vanilla
/// position to a yes/no.
///
/// Membership notes:
/// - 1F is a fortress and 8-Hnd1..3 are hand levels — both already
///   excluded from the regular-level pool by other mechanisms. They are
///   listed here for completeness so future consumers can rely on the
///   list being a full enumeration of chest-bearing levels.
pub(super) const CHEST_LEVELS: &[(usize, usize, &str)] = &[
    (0, 11, "1F (Warp Whistle)"),
    (2, 29, "3-7 (Cloud)"),
    (4, 5,  "5-1 (Music Box)"),
    (7, 5,  "8-Tank (Star)"),
    (7, 14, "8-Hnd1 (chest)"),
    (7, 15, "8-Hnd2 (chest)"),
    (7, 16, "8-Hnd3 (chest)"),
];

/// True if the given vanilla `(world_idx, entry_idx)` is in [`CHEST_LEVELS`].
pub(super) fn is_chest_level(world_idx: usize, entry_idx: usize) -> bool {
    CHEST_LEVELS
        .iter()
        .any(|&(w, e, _)| w == world_idx && e == entry_idx)
}

// ---------------------------------------------------------------------------
// Entry lookup tables
// ---------------------------------------------------------------------------

/// Destination byte → world index (0-based). Only paired pipe destinations.
pub(super) const DEST_TO_WORLD: &[(u8, usize)] = &[
    (0x00, 4),  // W5 (spiral tower)
    (0x01, 1),  // W2
    (0x02, 5), (0x03, 5),  // W6
    (0x04, 6), (0x05, 6), (0x06, 6), (0x07, 6),  // W7
    (0x08, 6), (0x09, 6), (0x0A, 6), (0x0B, 6),  // W7
    (0x0C, 7), (0x0D, 7), (0x0E, 7), (0x0F, 7), (0x10, 7), (0x11, 7),  // W8
    (0x12, 2), (0x13, 2), (0x14, 2),  // W3
    (0x15, 3), (0x16, 3),  // W4
    (0x17, 4),  // W5
];

/// Per-world map tile grid info.
pub(super) struct MapGridInfo {
    pub file_offset: usize,
    pub columns: usize,
    #[allow(dead_code)]
    pub screens: usize,
}

pub(super) const MAP_TILE_GRIDS: [MapGridInfo; 8] = [
    MapGridInfo { file_offset: 0x185BA, columns: 16, screens: 1 },  // W1
    MapGridInfo { file_offset: 0x1864B, columns: 32, screens: 2 },  // W2
    MapGridInfo { file_offset: 0x1876C, columns: 48, screens: 3 },  // W3
    MapGridInfo { file_offset: 0x1891D, columns: 32, screens: 2 },  // W4
    MapGridInfo { file_offset: 0x18A3E, columns: 32, screens: 2 },  // W5
    MapGridInfo { file_offset: 0x18B5F, columns: 48, screens: 3 },  // W6
    MapGridInfo { file_offset: 0x18D10, columns: 32, screens: 2 },  // W7
    MapGridInfo { file_offset: 0x18E31, columns: 64, screens: 4 },  // W8
];

/// Pointer table locations per world.
pub(super) struct WorldTables {
    pub rowtype_offset: usize,
    pub entry_count: usize,
}

pub(super) const WORLDS: [WorldTables; 8] = [
    WorldTables { rowtype_offset: 0x19438, entry_count: 21 },
    WorldTables { rowtype_offset: 0x194BA, entry_count: 47 },
    WorldTables { rowtype_offset: 0x195D8, entry_count: 52 },
    WorldTables { rowtype_offset: 0x19714, entry_count: 34 },
    WorldTables { rowtype_offset: 0x197E4, entry_count: 42 },
    WorldTables { rowtype_offset: 0x198E4, entry_count: 57 },
    WorldTables { rowtype_offset: 0x19A3E, entry_count: 46 },
    WorldTables { rowtype_offset: 0x19B56, entry_count: 41 },
];

/// Known fortress entries (world_idx, entry_idx).
pub(super) const FORTRESS_ENTRIES: &[(usize, usize)] = &[
    (0, 11),
    (1, 13),
    (2, 13), (2, 34),
    (3, 9), (3, 16),
    (4, 12), (4, 31),
    (5, 9), (5, 27), (5, 48),
    (6, 5), (6, 40),
    (7, 7), (7, 10), (7, 26), (7, 36),
];

/// ROM file offset of the Boom-Boom Y-byte for each fortress (same order as
/// FORTRESS_ENTRIES). The Y-byte upper nibble encodes the fortress ordinal
/// (1-based Map_DoFortressFX value); the lower nibble is spawn Y position.
pub(super) const BOOMBOOM_Y_OFFSETS: [usize; 17] = [
    0x0D35F, // W1[11]
    0x0D262, // W2[13]
    0x0D3D3, // W3[13]
    0x0D3A1, // W3[34]
    0x0D536, // W4[ 9]
    0x0D55F, // W4[16]
    0x0D40F, // W5[12]
    0x0D2C7, // W5[31]
    0x0D4E1, // W6[ 9]
    0x0CAE1, // W6[27]
    0x0D4B0, // W6[48]
    0x0D4FA, // W7[ 5]
    0x0D47E, // W7[40]
    0x0DA32, // W8[ 7]
    0x0DA37, // W8[10]
    0x0D597, // W8[26]
    0x0DA2D, // W8[36]
];

/// The 1-F fortress obj_ptr. This fortress level has a secret exit that
/// bypasses the Boom-Boom boss (no crystal ball → no FX trigger → lock
/// stays closed). Must be placed in a slot whose lock is secret_exit_safe.
pub(super) const FORTRESS_1F_OBJ_PTR: u16 = 0xD32B;

/// Vanilla fortress obj_ptrs (same order as FORTRESS_ENTRIES).
/// The obj_ptr identifies the fortress level's enemy data stream in PRG006.
/// After level shuffle, the obj_ptr at a slot still points to the same enemy
/// data — only the pointer table entries move, not the data itself.
pub(super) const VANILLA_FORTRESS_OBJ_PTRS: [u16; 17] = [
    0xD32B, // W1[11]
    0xD222, // W2[13]
    0xD393, // W3[13]
    0xD362, // W3[34]
    0xD508, // W4[ 9]
    0xD528, // W4[16]
    0xD3D0, // W5[12]
    0xD2B4, // W5[31]
    0xD4B0, // W6[ 9]
    0xCAAB, // W6[27]
    0xD470, // W6[48]
    0xD4E4, // W7[ 5]
    0xD41B, // W7[40]
    0xD8CC, // W8[ 7]
    0xD867, // W8[10]
    0xD551, // W8[26]
    0xD91C, // W8[36]
];

/// Given an obj_ptr found at a fortress slot, return the Boom-Boom Y-byte
/// ROM file offset for that fortress's enemy data.
pub(super) fn boomboom_y_offset_for_obj(obj_ptr: u16) -> Option<usize> {
    VANILLA_FORTRESS_OBJ_PTRS
        .iter()
        .zip(BOOMBOOM_Y_OFFSETS.iter())
        .find(|&(&op, _)| op == obj_ptr)
        .map(|(_, &y)| y)
}

/// Known airship entries (world_idx, entry_idx).
pub(super) const AIRSHIP_ENTRIES: &[(usize, usize)] = &[
    (0, 17), (1, 36), (2, 49), (3, 6), (4, 35), (5, 53), (6, 43),
];

/// Bowser's castle entry.
pub(super) const BOWSER_ENTRY: (usize, usize) = (7, 40);

/// Known toad house obj_ptrs. The standard format is $0700; the variant
/// formats ($0300-$0900) select different reward pools/game types but all
/// load a toad house screen. All share lay=$AD60.
pub(super) const TOAD_HOUSE_OBJ_PTRS: &[u16] = &[
    0x0300, 0x0400, 0x0500, 0x0600, 0x0700, 0x0800, 0x0900,
];

/// Known hammer bro level obj_ptrs. Each world's hammer bro encounters point
/// to one of these object streams. Multiple pointer table entries share the
/// same obj_ptr (with varying layouts/tilesets).
/// W8's 0xC03D is included here despite using a full action level layout (7-7)
/// so the entry is classified as HammerBro and excluded from the level pool
/// (prevents 7-7 from appearing twice). It is filtered out of the HB cycling
/// pool by `unique_hammer_bro_levels()` via `HB_EXCLUDE_OBJ_PTRS`.
pub(super) const HAMMER_BRO_OBJ_PTRS: &[u16] = &[
    0xC72B, // W1
    0xD14D, // W2
    0xD142, // W2 (variant)
    0xC640, // W3, W5, W6, W7
    0xD0EA, // W4
    0xC03D, // W8 (uses 7-7 layout — not a real HB battle)
];

/// Hammer bro obj_ptrs that should NOT appear in the HB cycling pool.
/// These are full action levels reused by HB entries, not short battles.
pub(super) const HB_EXCLUDE_OBJ_PTRS: &[u16] = &[
    0xC03D, // W8 — 7-7 layout
];

/// Stompable enemies — safe for single-enemy HB Wild segments and the default
/// pool for 2-enemy segments. The player can always defeat these by jumping.
pub(super) const STOMPABLE_ENEMIES: &[u8] = &[
    // Ground (stompable)
    0x29, // Spike
    0x2B, // GoombShoe (Kuribo)
    0x3F, // DryBones
    0x40, // BusterBeetle
    0x55, // BobOmb
    0x58, // FireChomp
    0x6B, // PileDriver
    0x72, // Goomba
    0x7C, // BigGoomba
    // Shell
    0x6C, // GreenTroopa
    0x6D, // RedTroopa
    0x70, // BuzzyBeetle
    0x7A, // BigGreenTroopa
    0x7B, // BigRedTroopa
    // Flying
    0x6E, // ParatroopaGreenHop
    0x6F, // FlyingRedParatroopa
    0x73, // Paragoomba
    0x74, // ParagoombaMicros
    0x7E, // BigGreenHopper
    0x80, // FlyingGreenParatroopa
    // Bros
    0x81, // HammerBro
    0x82, // BoomerangBro
    0x86, // HeavyBro
    0x87, // FireBro
    // NOTE: Bullet Bills (0x78/0x79) intentionally excluded — they're
    // cannon-spawned projectiles. Placed directly in level data their XVel
    // stays 0 (standard) or they accelerate once and lock (homing). The
    // `cannons` class swaps the cannon IDs (0xBC/0xBD) via the BILLS sub-class
    // instead.
];

/// Non-stompable enemies allowed in 2-enemy HB Wild segments only.
/// If one of these is picked, the other enemy MUST be a shell so the
/// player can use it to kill the non-stompable.
pub(super) const HB_NEEDS_SHELL_ENEMIES: &[u8] = &[
    0x71, // Spiny
    0x2A, // Patooie
    0x33, // Nipper
    0x39, // NipperHopping
    0x63, // BigBertha
];

/// Specific (obj_ptr, tileset) pairs to exclude from the HB cycling pool.
/// W3[41] has lay=0xB3E7 with tileset 3, but the layout is designed for tileset 1
/// (17 other entries with the same layout use tileset 1). Loading it with tileset 3
/// causes garbled background graphics.
pub(super) const HB_EXCLUDE_ENTRIES: &[(u16, u8)] = &[
    (0xC640, 3), // W3[41] — tileset 3 is wrong for lay 0xB3E7
];

// ---------------------------------------------------------------------------
// Overworld map object tables (PRG011)
// ---------------------------------------------------------------------------

/// Master pointer table for Map_List_Object_Ys (8 words, one per world).
const MAP_OBJ_YS_MASTER: usize = 0x16020;
/// Master pointer table for Map_List_Object_XHis.
const MAP_OBJ_XHIS_MASTER: usize = 0x16030;
/// Master pointer table for Map_List_Object_XLos.
const MAP_OBJ_XLOS_MASTER: usize = 0x16040;

/// Map object → pointer table entry linkage.
/// (world_idx, object_slot, pointer_table_entry_idx)
/// W7 piranha plants: stationary overworld sprites whose positions must
/// stay in sync with their pointer table entries after pipe shuffling.
pub(super) const MAP_OBJ_ENTRY_LINKS: &[(usize, usize, usize)] = &[
    (6, 2, 11), // W7 piranha plant 1
    (6, 3, 45), // W7 piranha plant 2
];

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Mutable overworld tile grid.
#[derive(Clone, Debug)]
pub(crate) struct Grid {
    pub tiles: Vec<Vec<u8>>,
    pub rows: usize,
    pub cols: usize,
}

impl Grid {
    pub fn get(&self, row: usize, col: usize) -> u8 {
        self.tiles[row][col]
    }

    pub fn set(&mut self, row: usize, col: usize, tile: u8) {
        self.tiles[row][col] = tile;
    }

}

/// Data that travels with a level when shuffled.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct LevelEntry {
    pub tileset: u8,
    pub obj_lo: u8,
    pub obj_hi: u8,
    pub lay_lo: u8,
    pub lay_hi: u8,
}

/// A beta (unreferenced) level — layout data exists in the ROM but no vanilla
/// pointer table entry references it. Injected into the shuffle pool when
/// `include_beta_stages` is enabled. The `obj_ptr` is borrowed from a
/// compatible vanilla level (beta layouts have no canonical enemy pairing).
pub(super) struct BetaLevel {
    pub tileset: u8,
    pub obj_lo: u8,
    pub obj_hi: u8,
    pub lay_lo: u8,
    pub lay_hi: u8,
    pub name: &'static str,
}

/// Nine unreferenced beta levels found in the level data banks.
pub(super) const BETA_LEVELS: &[BetaLevel] = &[
    BetaLevel { tileset:  1, obj_lo: 0xAB, obj_hi: 0xC1, lay_lo: 0x4C, lay_hi: 0xA7, name: "\u{03B2}1" },
    BetaLevel { tileset:  1, obj_lo: 0x21, obj_hi: 0xC2, lay_lo: 0xAC, lay_hi: 0xA9, name: "\u{03B2}2" },
    BetaLevel { tileset:  3, obj_lo: 0xDC, obj_hi: 0xC6, lay_lo: 0xDD, lay_hi: 0xB0, name: "\u{03B2}3" },
    BetaLevel { tileset:  3, obj_lo: 0x06, obj_hi: 0xC0, lay_lo: 0x42, lay_hi: 0xB4, name: "\u{03B2}4" },
    BetaLevel { tileset:  4, obj_lo: 0xD8, obj_hi: 0xCA, lay_lo: 0xCD, lay_hi: 0xAD, name: "\u{03B2}5" },
    BetaLevel { tileset:  8, obj_lo: 0x06, obj_hi: 0xC0, lay_lo: 0x18, lay_hi: 0xAF, name: "\u{03B2}6" },
    BetaLevel { tileset: 12, obj_lo: 0xE5, obj_hi: 0xCB, lay_lo: 0xBF, lay_hi: 0xB2, name: "\u{03B2}7" },
    BetaLevel { tileset: 12, obj_lo: 0xA0, obj_hi: 0xCC, lay_lo: 0xBA, lay_hi: 0xB7, name: "\u{03B2}8" },
    BetaLevel { tileset: 13, obj_lo: 0xBD, obj_hi: 0xCE, lay_lo: 0xA9, lay_hi: 0xAC, name: "\u{03B2}9" },
];

/// Deterministic layout fixes for the 9 beta stages.
///
/// Each entry is `(file_offset, new_byte)`. These repair broken sub-area
/// pointers, wrong start positions, and misaligned tile commands that would
/// cause visual corruption or softlocks. β7 and β8 need no fixed patches.
pub(super) const BETA_PATCHES: &[(usize, u8)] = &[
    // β1 (ts1 $A74C) — 4 patches
    // Swap the row-7 wood block and row-8 wood-leaf in the 3-block stack at
    // scr=1 col=13 so the powerup sits on top of the stack. Without the swap,
    // a roll to flower spawns the flower trapped between two wood blocks.
    // byte0 0x27→0x28 sinks the top wood block to row 8; byte0 0x47 (instead
    // of 0x48) raises the leaf to row 7. fixed_idx stays 37 (wood-leaf, tile
    // $74) since the row bits don't enter the dispatch math.
    (0x1E782, 0x28), (0x1E785, 0x47), (0x1E787, 0x05), (0x1E916, 0x08),
    // β2 (ts1 $A9AC) — 9 patches (header: alt_layout/alt_objects redirect + command fixes)
    (0x1E9BC, 0x48), (0x1E9BD, 0xBE), (0x1E9BE, 0x84),
    (0x1E9E5, 0x71), (0x1E9E6, 0x80),
    // Convert the WOODBLOCKBOUNCE at scr=1 col=9 row=9 into a true wood-block
    // powerup (group 2 fixed, byte2=0x05 = wood-leaf, tile $74). byte0 flips
    // 0x29→0x49 (group 1→2, row=9 unchanged) and byte2 flips 0x70→0x05.
    // The byte0 change alone was tried previously and crashed — leaving byte2
    // at 0x70 reinterprets it as variable dispatch 36 in group 2, which is a
    // 4-byte command in Plains, mis-aligning the rest of the level.
    (0x1EA02, 0x49), (0x1EA04, 0x05),
    (0x1EA6F, 0x0D), (0x1EB34, 0x06),
    // β3 (ts3 $B0DD) — 2 patches
    (0x2113F, 0x00), (0x212BA, 0x00),
    // β4 (ts3 $B442) — 5 patches (header: byte5 X-start + command fixes,
    // and convert hidden 1-up brick to Q-block star so the powerup randomizer reaches it).
    (0x21457, 0x04), (0x214B2, 0x04),
    (0x214CF, 0x2F), (0x214D0, 0x24), (0x214D1, 0x02),
    // β5 (ts4 $ADCD) — 5 patches
    (0x22F05, 0x1A), (0x22F08, 0x19), (0x22F0B, 0x1A),
    (0x22F0F, 0x9F), (0x23004, 0x04),
    // β6 (ts8 $AF18) — 20 patches (header: alt_layout/alt_objects/alt_tileset + commands)
    (0x24F28, 0x32), (0x24F29, 0xB3), (0x24F2A, 0xD8),
    (0x24F2B, 0xC3), (0x24F2E, 0xB1),
    (0x25028, 0x00), (0x25029, 0x41), (0x2502B, 0x00),
    (0x2502C, 0x41), (0x2502D, 0x23), (0x25031, 0x00),
    (0x25032, 0x41), (0x2503A, 0x00), (0x2503B, 0x41),
    (0x25046, 0x00), (0x25047, 0x41), (0x25063, 0x21),
    (0x25067, 0x53), (0x25068, 0x32), (0x250D2, 0x01),
    // β9 (ts13 $ACA9) — 2 patches
    (0x26DB8, 0x63), (0x26DB9, 0x20),
];

/// An FX slot (lock/bridge position and replacement tile).
pub(super) struct FxSlot {
    pub grid_row: usize,
    pub grid_col: usize,
    pub replace_tile: u8,
}

// ---------------------------------------------------------------------------
// ROM helpers
// ---------------------------------------------------------------------------

/// Read a 16-bit little-endian word from ROM.
pub(super) fn read_word(rom: &Rom, offset: usize) -> u16 {
    let lo = rom.read_byte(offset) as u16;
    let hi = rom.read_byte(offset + 1) as u16;
    (hi << 8) | lo
}

/// Compute sub-table file offsets for a world's pointer tables.
/// Returns (scrcol_offset, objsets_offset, layouts_offset).
pub(super) fn table_offsets(world: &WorldTables) -> (usize, usize, usize) {
    let n = world.entry_count;
    let scrcol = world.rowtype_offset + n;
    let objsets = scrcol + n;
    let layouts = objsets + n * 2;
    (scrcol, objsets, layouts)
}

/// Get the (grid_row, grid_col) for a pointer table entry.
pub(super) fn entry_grid_position(rom: &Rom, world: &WorldTables, idx: usize) -> (usize, usize) {
    let row_nibble = (rom.read_byte(world.rowtype_offset + idx) >> 4) & 0x0F;
    let scrcol = rom.read_byte(world.rowtype_offset + world.entry_count + idx);
    let screen = (scrcol >> 4) & 0x0F;
    let column = scrcol & 0x0F;
    let grid_row = (row_nibble as usize).wrapping_sub(2);
    let grid_col = screen as usize * 16 + column as usize;
    (grid_row, grid_col)
}

/// Compute the ROM file offset of a map tile at (row, col).
pub(super) fn map_tile_offset(world_idx: usize, row: usize, col: usize) -> usize {
    let info = &MAP_TILE_GRIDS[world_idx];
    let screen = col / 16;
    let col_in_screen = col % 16;
    info.file_offset + screen * 144 + row * 16 + col_in_screen
}

// ---------------------------------------------------------------------------
// Level entry helpers
// ---------------------------------------------------------------------------

/// PRG bank loaded at CPU $A000-$BFFF for each tileset (0-18).
pub(super) const PAGE_A000_BY_TILESET: [usize; 19] = [
    11, 15, 21, 16, 17, 19, 18, 18, 18, 20, 23, 19, 17, 19, 13, 26, 26, 26, 9,
];

/// Returns true if this map entry has a real level pointer (not a toad house,
/// bonus game, hand trap, or pipe junction).
pub(super) fn is_level_pointer(obj_ptr: u16, lay_ptr: u16) -> bool {
    obj_ptr >= 0xC000 && lay_ptr != 0x0000
}

/// Convert a layout CPU address ($A000-$BFFF) + tileset to a ROM file offset.
pub(super) fn layout_file_offset(cpu_addr: u16, tileset: u8) -> Option<usize> {
    if tileset as usize >= PAGE_A000_BY_TILESET.len() || cpu_addr < 0xA000 {
        return None;
    }
    let bank = PAGE_A000_BY_TILESET[tileset as usize];
    Some(bank * 0x2000 + 0x10 + (cpu_addr as usize - 0xA000))
}

/// ROM file offset of PRG006 enemy/object data base (CPU $C000).
pub(super) const ENEMY_DATA_FILE_BASE: usize = 0x0C010;

/// Translate a CPU enemy-data pointer (`$C000..=$E00D`) to its absolute file
/// offset.
pub(super) fn enemy_ptr_to_file_offset(ep: u16) -> usize {
    ENEMY_DATA_FILE_BASE + (ep as usize - 0xC000)
}

/// Enemy/object data block: 0x0BFD8..0x0E00D (exclusive end).
/// Each level's enemy set is a sequence of segments separated by 0xFF.
/// Each segment starts with a 1-byte page flag, then zero or more 3-byte
/// entries [object_id, x_pos, y_pos], terminated by 0xFF.
pub const ENEMY_DATA_START: usize = 0x0BFD8;
pub const ENEMY_DATA_END: usize = 0x0E00D;

/// Unstompable hazards that are unfair to introduce into tight sub-areas
/// (especially boss arenas) or onto a player's walking path. Patooie/Lavalotus
/// fire continuously with no telegraph; Thwomps drop or slide unpredictably
/// when added on top of a designed encounter. The registry filters these
/// out of the chosen swap pool at `EntryProtection::ExcludeHazards`
/// positions, and they're also excluded from piranha-slot replacements
/// (a pipe-lip swap to one of these covers or blocks the only way through).
pub(super) const HAZARD_PROJECTILE_IDS: &[u8] = &[
    0x2A, // OBJ_PATOOIE (spits spikes upward)
    0x67, // OBJ_LAVALOTUS (spits fire in arcs)
    0x8A, // OBJ_THWOMP (standard drop)
    0x8B, // OBJ_THWOMPLEFTSLIDE
    0x8C, // OBJ_THWOMPRIGHTSLIDE
    0x8D, // OBJ_THWOMPUPDOWN
    0x8E, // OBJ_THWOMPDIAGONALUL
    0x8F, // OBJ_THWOMPDIAGONALDL
];

/// Bro enemies that work in tileset 10 (8-Tank sub-area).
/// Excludes HammerBro (0x81) which fails to spawn in ts=10.
pub(super) const TANK_BRO_POOL: &[u8] = &[
    0x82, // OBJ_BOOMERANGBRO
    0x86, // OBJ_HEAVYBRO
    0x87, // OBJ_FIREBRO
];

/// Check whether the first enemy data segment at `obj_ptr` contains `target_id`.
///
/// Enemy data format: 1-byte page flag, then 3-byte entries `[id, x, y]`,
/// terminated by `0xFF`. Only the first segment is scanned.
pub(super) fn has_enemy_id(rom: &Rom, obj_ptr: u16, target_id: u8) -> bool {
    if obj_ptr < 0xC000 {
        return false;
    }
    let file_off = enemy_ptr_to_file_offset(obj_ptr);
    if file_off + 1 >= rom.data.len() {
        return false;
    }
    let mut pos = file_off + 1; // skip page flag byte
    while pos + 2 < rom.data.len() {
        if rom.data[pos] == 0xFF {
            break;
        }
        if rom.data[pos] == target_id {
            return true;
        }
        pos += 3;
    }
    false
}

/// Read a LevelEntry from ROM for a given world and entry index.
pub(super) fn read_entry(rom: &Rom, world: &WorldTables, idx: usize) -> LevelEntry {
    let (_scrcol, objsets, layouts) = table_offsets(world);
    let obj_off = objsets + idx * 2;
    let lay_off = layouts + idx * 2;

    LevelEntry {
        tileset: rom.read_byte(world.rowtype_offset + idx) & 0x0F,
        obj_lo: rom.read_byte(obj_off),
        obj_hi: rom.read_byte(obj_off + 1),
        lay_lo: rom.read_byte(lay_off),
        lay_hi: rom.read_byte(lay_off + 1),
    }
}

/// Write a LevelEntry back to ROM for a given world and entry index.
/// Only the tileset (lower nibble of ByRowType) is updated — the upper
/// nibble (map row position) is preserved.
pub(super) fn write_entry(rom: &mut Rom, world: &WorldTables, idx: usize, entry: &LevelEntry) {
    let (_scrcol, objsets, layouts) = table_offsets(world);
    let obj_off = objsets + idx * 2;
    let lay_off = layouts + idx * 2;

    let old_brt = rom.read_byte(world.rowtype_offset + idx);
    let new_brt = (old_brt & 0xF0) | (entry.tileset & 0x0F);
    rom.write_byte(world.rowtype_offset + idx, new_brt);

    rom.write_byte(obj_off, entry.obj_lo);
    rom.write_byte(obj_off + 1, entry.obj_hi);
    rom.write_byte(lay_off, entry.lay_lo);
    rom.write_byte(lay_off + 1, entry.lay_hi);
}

// ---------------------------------------------------------------------------
// Grid reading
// ---------------------------------------------------------------------------

/// Read a world's tile grid from ROM as a mutable Grid.
pub(super) fn read_tile_grid(rom: &Rom, world_idx: usize) -> Grid {
    let info = &MAP_TILE_GRIDS[world_idx];
    let cols = info.columns;

    let mut tiles = Vec::with_capacity(ROWS);
    for r in 0..ROWS {
        let mut row = Vec::with_capacity(cols);
        for c in 0..cols {
            let screen = c / 16;
            let col_in_screen = c % 16;
            let offset = info.file_offset + screen * 144 + r * 16 + col_in_screen;
            row.push(rom.read_byte(offset));
        }
        tiles.push(row);
    }

    Grid { tiles, rows: ROWS, cols }
}

/// Find the START tile position in a grid.
pub(super) fn find_start(grid: &Grid) -> Option<(usize, usize)> {
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            if grid.get(r, c) == TILE_START {
                return Some((r, c));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Pipe data reading
// ---------------------------------------------------------------------------

/// Get destination table indices that belong to a given world.
pub(super) fn dest_indices_for_world(world_idx: usize) -> Vec<usize> {
    DEST_TO_WORLD
        .iter()
        .filter(|&&(_, w)| w == world_idx)
        .map(|&(d, _)| d as usize)
        .collect()
}


/// Read all pipe pairs from ROM destination tables, grouped by world.
/// Returns a map: world_idx → Vec of ((row_a, col_a), (row_b, col_b)).
#[cfg(test)]
pub(super) fn read_pipe_pairs(rom: &Rom) -> std::collections::HashMap<usize, Vec<TeleportEdge>> {
    let mut pipes_by_world: std::collections::HashMap<usize, Vec<_>> = std::collections::HashMap::new();

    for &(dest, world_idx) in DEST_TO_WORLD {
        let d = dest as usize;
        let xhi = rom.read_byte(PIPE_MAP_XHI + d);
        let x = rom.read_byte(PIPE_MAP_X + d);
        let y = rom.read_byte(PIPE_MAP_Y + d);

        let a_scr = ((xhi >> 4) & 0x0F) as usize;
        let b_scr = (xhi & 0x0F) as usize;
        let a_col = ((x >> 4) & 0x0F) as usize;
        let b_col = (x & 0x0F) as usize;
        let a_row_nib = ((y >> 4) & 0x0F) as usize;
        let b_row_nib = (y & 0x0F) as usize;

        let a_pos = (a_row_nib.wrapping_sub(2), a_scr * 16 + a_col);
        let b_pos = (b_row_nib.wrapping_sub(2), b_scr * 16 + b_col);

        pipes_by_world.entry(world_idx).or_default().push((a_pos, b_pos));
    }

    pipes_by_world
}

// ---------------------------------------------------------------------------
// FX helpers
// ---------------------------------------------------------------------------

/// Read all 17 FX slots from ROM.
pub(super) fn read_fx_slots(rom: &Rom) -> Vec<FxSlot> {
    let mut slots = Vec::with_capacity(17);
    for i in 0..17 {
        let loc_row = rom.read_byte(FX_MAP_LOC_ROW + i);
        let loc = rom.read_byte(FX_MAP_LOC + i);
        let replace_tile = rom.read_byte(FX_MAP_TILE_REPLACE + i);

        let grid_row = ((loc_row >> 4) as usize).wrapping_sub(2);
        let col_in_screen = ((loc >> 4) & 0x0F) as usize;
        let screen = (loc & 0x0F) as usize;

        slots.push(FxSlot {
            grid_row,
            grid_col: screen * 16 + col_in_screen,
            replace_tile,
        });
    }
    slots
}

/// Read FortressFX_W1-W8: which FX slots each world uses.
/// Returns array of 8 Vecs, one per world.
///
/// Each world has 4 bytes in the table, but only the first N are meaningful
/// where N = number of fortresses in that world. The rest are zero-padded.
/// We use the fortress count from FORTRESS_ENTRIES to know how many to read.
pub(super) fn read_world_fx_assignments(rom: &Rom) -> [Vec<u8>; 8] {
    let mut assignments: [Vec<u8>; 8] = Default::default();
    for (wi, assignment) in assignments.iter_mut().enumerate() {
        let fort_count = FORTRESS_ENTRIES.iter().filter(|&&(w, _)| w == wi).count();
        let base = FX_WORLD_TABLE + wi * 4;
        for i in 0..fort_count.min(4) {
            assignment.push(rom.read_byte(base + i));
        }
    }
    assignments
}

// ---------------------------------------------------------------------------
// Map object position sync
// ---------------------------------------------------------------------------

/// Resolve a master pointer table entry to a ROM file offset for a given slot.
/// The master table holds 8 CPU-address words ($A010 bank); each points to a
/// 9-byte per-world sub-table.
fn map_obj_slot_offset(rom: &Rom, master_table: usize, world_idx: usize, slot: usize) -> usize {
    let cpu = read_word(rom, master_table + world_idx * 2) as usize;
    // PRG011 is bank 11 → file offset = 11 * 0x2000 + 0x10 + (cpu - 0xA000)
    0x16010 + (cpu - 0xA000) + slot
}


/// Write a map object sprite's position to the map object tables.
///
/// Converts a grid position to pixel coordinates and writes to the Y/XHi/XLo
/// tables for the given world and slot.
pub(super) fn write_map_sprite_position(
    rom: &mut Rom,
    world_idx: usize,
    slot: usize,
    grid_row: usize,
    grid_col: usize,
) {
    let y = ((grid_row + 2) * 16) as u8;
    let xhi = (grid_col / 16) as u8;
    let xlo = ((grid_col % 16) * 16) as u8;

    let y_off = map_obj_slot_offset(rom, MAP_OBJ_YS_MASTER, world_idx, slot);
    let xhi_off = map_obj_slot_offset(rom, MAP_OBJ_XHIS_MASTER, world_idx, slot);
    let xlo_off = map_obj_slot_offset(rom, MAP_OBJ_XLOS_MASTER, world_idx, slot);

    rom.write_byte(y_off, y);
    rom.write_byte(xhi_off, xhi);
    rom.write_byte(xlo_off, xlo);
}

/// Read the grid positions of all active floating sprites for a world.
///
/// Each world has up to 9 map object slots. A slot with ID $FF is unused.
/// For active slots, we convert pixel coordinates back to grid positions.
/// These are the positions where floating sprites sit (hammer bros, piranhas,
/// W8 hand traps, etc.) and should not have level/fort tiles placed under them.
pub(super) fn read_map_sprite_positions(rom: &Rom, world_idx: usize) -> Vec<(usize, usize)> {
    const MAP_OBJ_IDS_MASTER: usize = 0x16050;
    let mut positions = Vec::new();

    for slot in 0..9 {
        let id_off = map_obj_slot_offset(rom, MAP_OBJ_IDS_MASTER, world_idx, slot);
        let id = rom.read_byte(id_off);
        if id == 0xFF {
            continue; // unused slot
        }

        let y_off = map_obj_slot_offset(rom, MAP_OBJ_YS_MASTER, world_idx, slot);
        let xhi_off = map_obj_slot_offset(rom, MAP_OBJ_XHIS_MASTER, world_idx, slot);
        let xlo_off = map_obj_slot_offset(rom, MAP_OBJ_XLOS_MASTER, world_idx, slot);

        let y = rom.read_byte(y_off) as usize;
        let xhi = rom.read_byte(xhi_off) as usize;
        let xlo = rom.read_byte(xlo_off) as usize;

        // Reverse of Grid→pixel: Y=(row+2)*16, XHi=col/16, XLo=(col%16)*16
        if y < 32 {
            continue; // invalid (row would be negative)
        }
        let row = (y / 16).saturating_sub(2);
        let col = xhi * 16 + xlo / 16;

        positions.push((row, col));
    }

    positions
}

/// Read grid positions of hammer bro sprites only (IDs 0x03–0x06).
///
/// These positions need HB level pointer entries even though they are excluded
/// from level/fort/pipe placement by `fixed_positions`.
pub(super) fn read_hb_sprite_positions(rom: &Rom, world_idx: usize) -> Vec<(usize, usize)> {
    const MAP_OBJ_IDS_MASTER: usize = 0x16050;
    let mut positions = Vec::new();

    for slot in 0..9 {
        let id_off = map_obj_slot_offset(rom, MAP_OBJ_IDS_MASTER, world_idx, slot);
        let id = rom.read_byte(id_off);
        if !(0x03..=0x06).contains(&id) {
            continue;
        }

        let y_off = map_obj_slot_offset(rom, MAP_OBJ_YS_MASTER, world_idx, slot);
        let xhi_off = map_obj_slot_offset(rom, MAP_OBJ_XHIS_MASTER, world_idx, slot);
        let xlo_off = map_obj_slot_offset(rom, MAP_OBJ_XLOS_MASTER, world_idx, slot);

        let y = rom.read_byte(y_off) as usize;
        let xhi = rom.read_byte(xhi_off) as usize;
        let xlo = rom.read_byte(xlo_off) as usize;

        if y < 32 {
            continue;
        }
        let row = (y / 16).saturating_sub(2);
        let col = xhi * 16 + xlo / 16;

        positions.push((row, col));
    }

    positions
}

// ---------------------------------------------------------------------------
// Free space overlap test
// ---------------------------------------------------------------------------

#[cfg(test)]
mod free_space_tests {
    use super::*;

    #[test]
    fn test_free_space_no_overlap() {
        for (i, &(a_off, a_sz, a_label)) in FREE_SPACE_ALLOCATIONS.iter().enumerate() {
            let a_end = a_off + a_sz;
            for &(b_off, b_sz, b_label) in &FREE_SPACE_ALLOCATIONS[i + 1..] {
                let b_end = b_off + b_sz;
                assert!(
                    a_end <= b_off || b_end <= a_off,
                    "free space overlap: '{}' (0x{:05X}..0x{:05X}) vs '{}' (0x{:05X}..0x{:05X})",
                    a_label, a_off, a_end, b_label, b_off, b_end,
                );
            }
        }
    }

    #[test]
    fn test_free_space_constants_match_registry() {
        let offsets: Vec<usize> = FREE_SPACE_ALLOCATIONS.iter().map(|&(o, _, _)| o).collect();
        assert!(offsets.contains(&FS_WORLD_ORDER));
        assert!(offsets.contains(&FS_BIG_Q_SAVE));
        assert!(offsets.contains(&FS_SEED_HASH_ROUTINE));
        assert!(offsets.contains(&FS_SEED_HASH_DATA));
        assert!(offsets.contains(&FS_INTRO_SKIP));
        assert!(offsets.contains(&FS_CARD_CLEAR));
        assert!(offsets.contains(&FS_BIG_Q_LOOKUP));
        assert!(offsets.contains(&FS_KING_QUOTES));
        assert!(offsets.contains(&FS_FX_SCREEN_CHECK));
        assert!(offsets.contains(&FS_CANOE_RESPAWN));
        assert!(offsets.contains(&FS_CANOE_BACKUP));
        assert!(offsets.contains(&FS_KOOPA_HITS_SUB));
        assert!(offsets.contains(&FS_STARTING_ITEMS));
        assert!(offsets.contains(&FS_MYSTERY_ANCHOR));
        assert!(offsets.contains(&FS_HAMMER_LOCKS));
        assert!(offsets.contains(&FS_PIRANHA_VIS_BIG));
        assert!(offsets.contains(&FS_PIRANHA_VIS_SMALL));
        assert!(offsets.contains(&FS_PIRANHA_HIT_SMALL));
    }
}
