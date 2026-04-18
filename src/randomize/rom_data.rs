/// Shared ROM constants, data structures, and helpers for SMB3 randomization.
///
/// This module holds all the shared knowledge about the ROM layout — constants,
/// lookup tables, data structures, and low-level read/write helpers — used by
/// multiple randomization modules. The BFS map walker lives in `map_walker.rs`.


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
    (0x3E260, 28, "starting_items: lives + intro skip + inventory init trampoline"),
    (0x3E965,  8, "title_screen: intro skip routine"),
    (0x3FFF0, 26, "card_speed_clear: XOR trampoline"),
    // PRG026 (file 0x34010, CPU $A000–$BFFF)
    (0x35530, 66, "big_q_block: lookup routine + tables"),
    // PRG027 (file 0x36010, CPU $A000–$BFFF)
    (0x379D9, 894, "king_quotes: 7 quotes + hook (7×120 + 54)"),
    // PRG010 (file 0x14010, CPU $A000–$BFFF during map)
    (0x15554, 67, "fx_screen_check: cross-screen lock patch"),
    (0x15DF0, 35, "canoe_fix: death respawn position save"),
    // PRG011 (file 0x16010, CPU $A000–$BFFF during map)
    (0x17D00, 59, "canoe_fix: backup/restore subroutines"),
    // PRG001 (file 0x02010, CPU $A000–$BFFF)
    (0x0382A, 23, "koopa_hits: subroutine + defeat JMP + threshold table"),
    (0x03841, 13, "koopa_collision_guard: skip collision bitmap during invuln"),
    (0x0384E, 16, "koopa_vram_clear: clear VRAM buffer on defeat"),
    (0x0385E, 12, "koopa_fire_preset: set stomp counter from threshold table for fireball defeat"),
    (0x03FD0, 22, "koopa_y_clamp: clamp Koopaling Y position to screen"),
];

// Individual constants for use by each module.

// PRG030
pub(super) const FS_WORLD_ORDER: usize       = 0x3DF20; // 28 bytes
pub(super) const FS_BIG_Q_SAVE: usize        = 0x3DF3C; // 20 bytes

// PRG031
pub(super) const FS_SEED_HASH_ROUTINE: usize = 0x3E924; // 25 bytes
pub(super) const FS_SEED_HASH_DATA: usize    = 0x3E93D; // 40 bytes
pub(super) const FS_INTRO_SKIP: usize        = 0x3E965; //  8 bytes
pub(super) const FS_CARD_CLEAR: usize        = 0x3FFF0; // 26 bytes

// PRG026
pub(super) const FS_BIG_Q_LOOKUP: usize      = 0x35530; // 66 bytes

// PRG027
pub(super) const FS_KING_QUOTES: usize       = 0x379D9; // 894 bytes

// PRG010
pub(super) const FS_FX_SCREEN_CHECK: usize   = 0x15554; // 67 bytes
pub(super) const FS_CANOE_RESPAWN: usize     = 0x15DF0; // 35 bytes

// PRG011
pub(super) const FS_CANOE_BACKUP: usize      = 0x17D00; // 59 bytes

// PRG026 (cont.)
pub(super) const FS_MYSTERY_ANCHOR: usize    = 0x35572; // 13 bytes
pub(super) const FS_HAMMER_LOCKS: usize      = 0x3557F; // 50 bytes
pub(super) const FS_STARTING_ITEMS: usize    = 0x3E260; // 28 bytes

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


// ---------------------------------------------------------------------------
// Tile constants
// ---------------------------------------------------------------------------

/// Valid horizontal path tiles (Map_Object_Valid_Left/Right in PRG010).
pub(super) const VALID_HORZ: &[u8] = &[0x45, 0x49, 0xB2, 0xB3, 0xAC, 0xB7, 0xB8, 0xDA, 0xB9, 0xE6];

/// Valid vertical path tiles (Map_Object_Valid_Down/Up in PRG010).
pub(super) const VALID_VERT: &[u8] = &[0x46, 0xB1, 0xAA, 0xAB, 0xB0, 0xDB, 0xBA];

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
pub(super) const CANOE_EDGES: &[(usize, ((usize, usize), (usize, usize)))] = &[
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

/// W5 Spiral Tower tile ID (functionally a pipe connecting screen 0 ↔ screen 1).
pub(super) const TILE_SPIRAL: u8 = 0x5F;

/// Fortress map tile ID (used in test code across multiple modules).
#[allow(dead_code)]
pub(super) const TILE_FORTRESS: u8 = 0x67;

/// Airship dock tile ID.
pub(super) const TILE_AIRSHIP: u8 = 0xC9;

/// Bowser's castle tile ID.
pub(super) const TILE_BOWSER: u8 = 0xCC;

/// Bonus game (spade/N-Spade) tile ID.
pub(super) const TILE_BONUS_GAME: u8 = 0xE8;

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

/// File offsets of the enemy data segments for each Hammer Bro encounter obj_ptr.
/// Computed as: 0x0C010 + (obj_ptr - 0xC000).
/// Used by the enemy randomizer to detect HB encounter segments.
pub(super) const HAMMER_BRO_SEGMENT_OFFSETS: &[usize] = &[
    0x0C73B, // 0xC72B — W1
    0x0D15D, // 0xD14D — W2
    0x0D152, // 0xD142 — W2 (variant)
    0x0C650, // 0xC640 — W3, W5, W6, W7
    0x0D0FA, // 0xD0EA — W4
    0x0C04D, // 0xC03D — W8 (uses 7-7 layout)
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
    // Bullet Bills
    0x78, // BulletBill
    0x79, // BulletBillHoming
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

/// Map transition entries.
pub(super) const MAP_TRANSITIONS: &[(usize, usize)] = &[];

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
    // β1 (ts1 $A74C) — 3 patches
    (0x1E785, 0x48), (0x1E787, 0x05), (0x1E916, 0x08),
    // β2 (ts1 $A9AC) — 7 patches (header: alt_layout/alt_objects redirect + command fixes)
    (0x1E9BC, 0x48), (0x1E9BD, 0xBE), (0x1E9BE, 0x84),
    (0x1E9E5, 0x71), (0x1E9E6, 0x80),
    // NOTE: (0x1EA02, 0x49) removed — crashes; moves cmd 20 to screen 4 which is
    // past course end page. Copied from a working ROM but crashes here for unknown reasons.
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
const ENEMY_DATA_FILE_BASE: usize = 0x0C010;

/// Enemy/object data block: 0x0BFD8..0x0E00D (exclusive end).
/// Each level's enemy set is a sequence of segments separated by 0xFF.
/// Each segment starts with a 1-byte page flag, then zero or more 3-byte
/// entries [object_id, x_pos, y_pos], terminated by 0xFF.
pub const ENEMY_DATA_START: usize = 0x0BFD8;
pub const ENEMY_DATA_END: usize = 0x0E00D;

/// Individual enemy offsets excluded from randomization.
/// These specific enemies are required for gameplay (e.g., needed as platforms
/// to make jumps, or positioned to enable required routes).
pub(super) const PROTECTED_ENEMY_OFFSETS: &[usize] = &[
    0x0C465, // 8-1 FlyingRedParatroopa (scr=6, col=14) — needed to reach upper platform
    0x0CAB1, // 6-3 FlyingRedParatroopa (scr=6, col=13) — needed for jump
    0x0CAB4, // 6-3 FlyingRedParatroopa (scr=7, col=1) — needed for jump
];

/// Shell-class enemies at these offsets are forced to shuffle within SHELL_ENEMIES
/// regardless of the shell mode (Shuffle or Wild). When shell is Off, these are
/// untouched. Levels where shells are required to break bricks for progression.
pub(super) const SHELL_PROTECTED_OFFSETS: &[usize] = &[
    // 2-Pyr sub-area (enemy_ptr 0xC5BC): 10 Buzzy Beetles + 1 extra
    0x0C5CD, // BuzzyBeetle scr=1 col=0 row=15
    0x0C5D0, // BuzzyBeetle scr=1 col=3 row=2
    0x0C5D3, // BuzzyBeetle scr=2 col=3 row=15
    0x0C5D6, // BuzzyBeetle scr=2 col=5 row=9
    0x0C5DC, // BuzzyBeetle scr=3 col=2 row=10
    0x0C5DF, // BuzzyBeetle scr=3 col=4 row=9
    0x0C5E2, // BuzzyBeetle scr=3 col=11 row=4
    0x0C5E5, // BuzzyBeetle scr=4 col=0 row=15
    0x0C5E8, // BuzzyBeetle scr=4 col=11 row=3
    0x0C5EB, // BuzzyBeetle scr=4 col=14 row=6
    0x0C5F1, // BuzzyBeetle scr=6 col=7 row=15
    // 2-3 (obj 0xD1F0): shells needed to break bricks at end
    0x0D22B, // GreenTroopa scr=8 col=11 row=3
    0x0D22E, // GreenTroopa scr=8 col=13 row=3
    // 6-5 sub-area (enemy_ptr 0xC5EB): shell needed for progression
    0x0C60E, // GreenTroopa scr=4 col=10 row=8
];

/// 8-Tank sub-area bro: HammerBro (0x81) doesn't spawn in tileset 10.
/// Must always shuffle within the non-HammerBro bro pool (0x82/0x86/0x87).
pub(super) const TANK_BRO_PROTECTED_OFFSETS: &[usize] = &[
    0x0DA3A, // BoomerangBro scr=0 col=12 row=7 (8-Tank sub-area, enemy_ptr 0xDA29)
];

/// Bro enemies that work in tileset 10 (8-Tank sub-area).
/// Excludes HammerBro (0x81) which fails to spawn in ts=10.
pub(super) const TANK_BRO_POOL: &[u8] = &[
    0x82, // OBJ_BOOMERANGBRO
    0x86, // OBJ_HEAVYBRO
    0x87, // OBJ_FIREBRO
];

/// Enemy segments (by file offset of page flag byte) excluded from randomization.
/// These levels rely on specific enemy types/counts for gameplay (e.g., enemies
/// used as platforms in speedtech, where wrong types cause sprite overload).
pub(super) const PROTECTED_ENEMY_SEGMENTS: &[usize] = &[
    0x0CA33, // 3-2 (obj 0xCA23): enemies used as platforms, sprite overload risk
];

/// One gauntlet of memorizable projectile enemies whose positions get
/// per-seed X/Y jitter when `jitter_enemy_positions` is enabled.
///
/// Each entry describes an enemy-data segment and which obj_ids within it
/// should be jittered. Non-matching entries in the segment are skipped so
/// level-critical non-projectile enemies (DryBones, Boos) stay put.
///
/// Y byte encoding: low nibble = row within the current vertical page
/// (0–15), high nibble = vertical page/flags. Jitter preserves the high
/// nibble so podoboos stay on the same page. X byte is the global tile
/// column across all screens.
pub(super) struct JitterSegment {
    /// File offset of the first 3-byte entry (after the page-flag byte).
    pub file_offset: usize,
    /// Number of 3-byte entries to scan in this segment.
    pub count: usize,
    /// Only entries whose obj_id is in this list get jittered.
    pub ids: &'static [u8],
    /// Inclusive max for the X byte (clamps jitter to stay within screen count).
    pub max_x: u8,
}

/// Gauntlets that get per-seed X/Y jitter when `jitter_enemy_positions` is on.
/// Extending the feature to more levels = adding a row here, no code changes.
pub(super) const JITTER_SEGMENTS: &[JitterSegment] = &[
    // 5F-2 sub-area 1 (CPU 0xD2B9, page flag at 0x0D2C9). 26 entries:
    // 16× 0x9E (Podoboo fire jet) + 6× 0x53 (ceiling podoboo) + 2× DryBones
    // + 2× Boo. Sub-area is 8 screens => max_x = 8*16 - 1 = 0x7F.
    JitterSegment {
        file_offset: 0x0D2CA,
        count: 26,
        ids: &[0x9E, 0x53],
        max_x: 0x7F,
    },
    // 8B sub-area 1 (CPU 0xD60B, page flag at 0x0D61B). 14 entries total,
    // including 9× 0x75 (OBJ_BOSSATTACK) fireballs in the pre-Bowser
    // corridor. Sub-area is 15 screens => max_x = 15*16 - 1 = 0xEF.
    JitterSegment {
        file_offset: 0x0D61C,
        count: 14,
        ids: &[0x75],
        max_x: 0xEF,
    },
];

/// Check whether the first enemy data segment at `obj_ptr` contains `target_id`.
///
/// Enemy data format: 1-byte page flag, then 3-byte entries `[id, x, y]`,
/// terminated by `0xFF`. Only the first segment is scanned.
pub(super) fn has_enemy_id(rom: &Rom, obj_ptr: u16, target_id: u8) -> bool {
    if obj_ptr < 0xC000 {
        return false;
    }
    let file_off = ENEMY_DATA_FILE_BASE + (obj_ptr as usize - 0xC000);
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

/// Read the screen count from a level's 9-byte header.
/// Header byte 4, bits 3-0 = (num_screens - 1).
pub(super) fn level_screen_count(rom: &Rom, layout_offset: usize) -> u8 {
    (rom.read_byte(layout_offset + 4) & 0x0F) + 1
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
pub(super) fn read_pipe_pairs(rom: &Rom) -> std::collections::HashMap<usize, Vec<((usize, usize), (usize, usize))>> {
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
    for wi in 0..8 {
        let fort_count = FORTRESS_ENTRIES.iter().filter(|&&(w, _)| w == wi).count();
        let base = FX_WORLD_TABLE + wi * 4;
        for i in 0..fort_count.min(4) {
            assignments[wi].push(rom.read_byte(base + i));
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
    }
}
