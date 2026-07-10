//! Static ROM data tables: tiles, FX, world/level/entry tables, enemy lists,
//! map-object masters, beta level data, and small pure predicates over them.

/// A grid coordinate on an overworld map, in `(row, col)` order.
pub(crate) type Pos = (usize, usize);

/// An ordered pair of grid positions that BFS treats as a single edge —
/// the path between them is skipped. Used for vanilla pipe pairs, W3
/// canoe edges, and any future traversal feature that connects two
/// positions without a walkable path between them.
pub(crate) type TeleportEdge = (Pos, Pos);

/// Valid horizontal path tiles (Map_Object_Valid_Left/Right in PRG010).
pub(crate) const VALID_HORZ: &[u8] = &[0x45, 0x49, 0xB2, 0xB3, 0xAC, 0xB7, 0xB8, 0xDA, 0xB9, 0xE6, 0xE8];

/// Valid vertical path tiles (Map_Object_Valid_Down/Up in PRG010).
pub(crate) const VALID_VERT: &[u8] = &[0x46, 0xB1, 0xAA, 0xAB, 0xB0, 0xDB, 0xBA, 0xE8];

/// Background / non-walkable tiles.
pub(crate) const BACKGROUND_TILES: &[u8] = &[0xB4, 0xFF, 0x02];

/// Valid blank node tiles — positions with these tiles are available for
/// level/fort/pipe/HB placement. Used by both pickup (Phase 2) and build
/// (Phase 3) to ensure consistent blank detection.
pub(crate) const VALID_BLANK_TILES: &[u8] = &[
    0x44, 0x47, 0x48, 0x4A,        // standard
    0xAE, 0xAF, 0xB5, 0xB6,        // island
    0xD9, 0xDC, 0xDD, 0xDE,        // sky
];

/// Start tile ID.
pub(crate) const TILE_START: u8 = 0xE5;

/// World index of the Dark World (W8). The W8 canoe edges below are gated
/// behind the `8s are Wild` option.
pub(crate) const W8_IDX: usize = 7;

/// Canoe teleport edges: (world_idx, (origin, destination)).
/// The canoe transports the player from a mainland dock to an island dock.
/// These are bidirectional teleport edges in BFS, like pipes. (W3's mainland
/// dock at (6,20) is only reachable when rocks are cleared.)
///
/// IMPORTANT: the coordinates are NOT world-unique — e.g. (6,20) exists in the
/// grids of W2/W4–W8 too, and the W8 edges only exist when `8s are Wild` is on.
/// This const is therefore PRIVATE: the only way to read it is
/// [`active_canoe_edges`], which applies both the world filter and the flag
/// gate. A new consumer that needs canoe edges cannot bypass that gate.
pub(crate) const CANOE_EDGES: &[(usize, TeleportEdge)] = &[
    // W3: mainland dock (6,20) → two island docks.
    (2, ((6, 20), (5, 24))),  // mainland dock → island 1
    (2, ((6, 20), (0, 32))),  // mainland dock → island 2
    // W8: mainland dock (5,6) → three island docks on screen 0. Only active
    // when `8s are Wild` is on (the docks/sprite aren't placed otherwise).
    // The canoe sprite floats at (5,7), immediately right of (5,6) — same
    // "sprite one tile beside the mainland dock" convention as W3.
    (W8_IDX, ((5, 6), (3, 8))),
    (W8_IDX, ((5, 6), (5, 10))),
    (W8_IDX, ((5, 6), (5, 12))),
];

/// The canoe edges active for `world_idx` under the current options, as plain
/// [`TeleportEdge`]s (world tag stripped). This is the SINGLE place the
/// world-scope filter and the `8s are Wild` gate live — every consumer
/// (reachability check, BFS walk, builder placement, progression debug) reads
/// canoe edges through here rather than touching `CANOE_EDGES` directly, so a
/// new consumer cannot accidentally bypass the gate.
///
/// When `eights_are_wild` is false the W8 docks/sprite are never written, so
/// the W8 edges must be excluded to keep the walker's view consistent with the
/// map. W3's canoe is unconditional.
pub(crate) fn active_canoe_edges(world_idx: usize, eights_are_wild: bool) -> Vec<TeleportEdge> {
    CANOE_EDGES
        .iter()
        .filter(|&&(w, _)| w == world_idx && (w != W8_IDX || eights_are_wild))
        .map(|&(_, edge)| edge)
        .collect()
}

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
pub(crate) struct LevelDataRegion {
    pub start: usize,
    pub end: usize,
    pub extra_byte_dispatches: &'static [u8],
    /// Whether group 2 fixed-size shapes 1-6 are note/wood powerups in this
    /// tileset. In most tilesets they are, but in TS2 (Dungeon) shapes 1-2 map
    /// to CCBridge and shapes 3-7 map to TopDecoBlocks — swapping them would
    /// corrupt level geometry.
    pub randomize_note_wood: bool,
}

impl LevelDataRegion {
    /// Size in bytes of the generator command whose first and third stream
    /// bytes are `b0`/`b2`: 3 normally, 4 when the tileset's variable-size
    /// dispatch reads an extra byte. Every level-stream walker (powerups,
    /// enemy entry points) must step with this — a re-derived copy of the
    /// formula is how parsers drift out of alignment.
    pub fn command_size(&self, b0: u8, b2: u8) -> usize {
        if (b2 & 0xF0) == 0 {
            return 3; // fixed-size generator
        }
        let dispatch = (b0 >> 5) as usize * 15 + ((b2 >> 4) as usize) - 1;
        if self.extra_byte_dispatches.contains(&(dispatch as u8)) { 4 } else { 3 }
    }
}

/// Level data regions by tileset (file offset ranges + extra-byte dispatch info).
pub(crate) const LEVEL_DATA_REGIONS: &[LevelDataRegion] = &[
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
        // End 0x2800A, NOT 0x28C05: this region's PRG bank ($A000 = file
        // 0x26010) ends at 0x28010, and the next bank opens with the desert
        // metatile quadrant table. Walking past the bank boundary misparses
        // that table as level data and the powerup pass then writes item IDs
        // into desert metatile quadrants (stray palm-leaf tiles in desert
        // levels and HB battle scenes). Real data ends with an empty stub
        // level at 0x28000-0x28009; 0x2800A is one past its terminator.
        start: 0x26A6F, end: 0x2800A,
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
pub(crate) const TILE_PIPE: u8 = 0xBC;

/// Fortress map tile ID (used in test code across multiple modules).
#[allow(dead_code)]
pub(crate) const TILE_FORTRESS: u8 = 0x67;

/// All map tiles the game treats as fortresses ($67, $EB, $6A —
/// Map_Removable_Tiles + completion-unsafe). $6A's CHR animation is frozen
/// by `patch_metatile_6a_freeze` so it can serve as a static variant.
pub(crate) const FORTRESS_TILES: [u8; 3] = [TILE_FORTRESS, 0xEB, 0x6A];

/// Airship dock tile ID.
pub(crate) const TILE_AIRSHIP: u8 = 0xC9;

/// Bowser's castle tile ID.
pub(crate) const TILE_BOWSER: u8 = 0xCC;

/// Bonus game (spade/N-Spade) tile ID.
pub(crate) const TILE_BONUS_GAME: u8 = 0xE8;

/// Toad House placeholder tile ID. Vanilla Toad Houses use either 0x50 or
/// 0xE0; the build phase stamps this constant when a HammerBro slot is
/// promoted to a Toad House. The writer later overwrites the cell with the
/// per-entry vanilla tile from the catalog.
pub(crate) const TILE_TOAD_HOUSE: u8 = 0x50;

/// Placeholder stamped on the BFS grid to mark a position as non-background.
/// The actual value is irrelevant — it just needs to be outside BACKGROUND_TILES
/// so walk_map treats the position as a reachable node.
pub(crate) const TILE_NODE: u8 = 0x47;

/// Number of rows in every overworld map.
pub(crate) const ROWS: usize = 9;

/// File offset where PRG012 begins. PRG012 is loaded at CPU $A000-$BFFF
/// during the map screen; file offset = 0x18010 + (cpu_addr - 0xA000).
/// Holds the map metatile quadrant tables and the InitIndex master table.
pub(crate) const PRG012_FILE_BASE: usize = 0x18010;

// Pipe destination tables (PRG002)
pub(crate) const PIPE_MAP_XHI: usize = 0x046AA;

pub(crate) const PIPE_MAP_X: usize = 0x046C2;

pub(crate) const PIPE_MAP_Y: usize = 0x046DA;

pub(crate) const PIPE_MAP_SCRL_XHI: usize = 0x046F2;

// FX table offsets (17 slots)
pub(crate) const FX_VADDR_H: usize = 0x147CD;

pub(crate) const FX_VADDR_L: usize = 0x147DE;

pub(crate) const FX_MAP_COMP_IDX: usize = 0x147EF; // 17 x 2 bytes

pub(crate) const FX_PATTERNS: usize = 0x14811;     // 17 x 4 bytes

pub(crate) const FX_MAP_LOC_ROW: usize = 0x14855;

pub(crate) const FX_MAP_LOC: usize = 0x14866;

pub(crate) const FX_MAP_TILE_REPLACE: usize = 0x14877;

pub(crate) const FX_WORLD_TABLE: usize = 0x14888;

/// Map_Complete_Bits lookup table: maps grid row to completion bit.
/// Row 0 = $80, row 1 = $40, ..., row 7 = $01.
pub(crate) const MAP_COMPLETE_BITS: [u8; 8] = [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01];

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
pub(crate) const CHEST_LEVELS: &[(usize, usize, &str)] = &[
    (0, 11, "1F (Warp Whistle)"),
    (2, 29, "3-7 (Cloud)"),
    (4, 5,  "5-1 (Music Box)"),
    (6, 11, "7-P1 (chest)"),
    (6, 45, "7-P2 (chest)"),
    (7, 5,  "8-Tank (Star)"),
    (7, 14, "8-Hnd1 (chest)"),
    (7, 15, "8-Hnd2 (chest)"),
    (7, 16, "8-Hnd3 (chest)"),
];

/// True if the given vanilla `(world_idx, entry_idx)` is in [`CHEST_LEVELS`].
pub(crate) fn is_chest_level(world_idx: usize, entry_idx: usize) -> bool {
    CHEST_LEVELS
        .iter()
        .any(|&(w, e, _)| w == world_idx && e == entry_idx)
}

/// True if the given vanilla `(world_idx, entry_idx)` is a W8 hand level
/// (8-Hnd1/2/3) — the short item-drop bonus rooms behind the hand traps.
pub(crate) fn is_hand_level(world_idx: usize, entry_idx: usize) -> bool {
    world_idx == 7 && (14..=16).contains(&entry_idx)
}

/// Destination byte → world index (0-based). Only paired pipe destinations.
pub(crate) const DEST_TO_WORLD: &[(u8, usize)] = &[
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
pub(crate) struct MapGridInfo {
    pub file_offset: usize,
    pub columns: usize,
    #[allow(dead_code)]
    pub screens: usize,
}

pub(crate) const MAP_TILE_GRIDS: [MapGridInfo; 8] = [
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
pub(crate) struct WorldTables {
    pub rowtype_offset: usize,
    pub entry_count: usize,
}

pub(crate) const WORLDS: [WorldTables; 8] = [
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
pub(crate) const FORTRESS_ENTRIES: &[(usize, usize)] = &[
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
pub(crate) const BOOMBOOM_Y_OFFSETS: [usize; 17] = [
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
pub(crate) const FORTRESS_1F_OBJ_PTR: u16 = 0xD32B;

/// Vanilla fortress obj_ptrs (same order as FORTRESS_ENTRIES).
/// The obj_ptr identifies the fortress level's enemy data stream in PRG006.
/// After level shuffle, the obj_ptr at a slot still points to the same enemy
/// data — only the pointer table entries move, not the data itself.
pub(crate) const VANILLA_FORTRESS_OBJ_PTRS: [u16; 17] = [
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
pub(crate) fn boomboom_y_offset_for_obj(obj_ptr: u16) -> Option<usize> {
    VANILLA_FORTRESS_OBJ_PTRS
        .iter()
        .zip(BOOMBOOM_Y_OFFSETS.iter())
        .find(|&(&op, _)| op == obj_ptr)
        .map(|(_, &y)| y)
}

/// Known airship entries (world_idx, entry_idx).
pub(crate) const AIRSHIP_ENTRIES: &[(usize, usize)] = &[
    (0, 17), (1, 36), (2, 49), (3, 6), (4, 35), (5, 53), (6, 43),
];

/// Bowser's castle entry.
pub(crate) const BOWSER_ENTRY: (usize, usize) = (7, 40);

/// Known toad house obj_ptrs. The standard format is $0700; the variant
/// formats ($0300-$0900) select different reward pools/game types but all
/// load a toad house screen. All share lay=$AD60.
pub(crate) const TOAD_HOUSE_OBJ_PTRS: &[u16] = &[
    0x0300, 0x0400, 0x0500, 0x0600, 0x0700, 0x0800, 0x0900,
];

/// Known hammer bro level obj_ptrs. Each world's hammer bro encounters point
/// to one of these object streams. Multiple pointer table entries share the
/// same obj_ptr (with varying layouts/tilesets).
/// W8's 0xC03D is included here despite using a full action level layout (7-7)
/// so the entry is classified as HammerBro and excluded from the level pool
/// (prevents 7-7 from appearing twice). It is filtered out of the HB cycling
/// pool by `unique_hammer_bro_levels()` via `HB_EXCLUDE_OBJ_PTRS`.
pub(crate) const HAMMER_BRO_OBJ_PTRS: &[u16] = &[
    0xC72B, // W1
    0xD14D, // W2
    0xD142, // W2 (variant)
    0xC640, // W3, W5, W6, W7
    0xD0EA, // W4
    0xC03D, // W8 (uses 7-7 layout — not a real HB battle)
];

/// Hammer bro obj_ptrs that should NOT appear in the HB cycling pool.
/// These are full action levels reused by HB entries, not short battles.
pub(crate) const HB_EXCLUDE_OBJ_PTRS: &[u16] = &[
    0xC03D, // W8 — 7-7 layout
];

/// Stompable enemies — safe for single-enemy HB Wild segments and the default
/// pool for 2-enemy segments. The player can always defeat these by jumping.
pub(crate) const STOMPABLE_ENEMIES: &[u8] = &[
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
pub(crate) const HB_NEEDS_SHELL_ENEMIES: &[u8] = &[
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
pub(crate) const HB_EXCLUDE_ENTRIES: &[(u16, u8)] = &[
    (0xC640, 3), // W3[41] — tileset 3 is wrong for lay 0xB3E7
];

/// Master pointer table for Map_List_Object_Ys (8 words, one per world).
pub(crate) const MAP_OBJ_YS_MASTER: usize = 0x16020;

/// Master pointer table for Map_List_Object_XHis.
pub(crate) const MAP_OBJ_XHIS_MASTER: usize = 0x16030;

/// Master pointer table for Map_List_Object_XLos.
pub(crate) const MAP_OBJ_XLOS_MASTER: usize = 0x16040;

/// Master pointer table for Map_List_Object_IDs.
pub(crate) const MAP_OBJ_IDS_MASTER: usize = 0x16050;

/// Map object → pointer table entry linkage.
/// (world_idx, object_slot, pointer_table_entry_idx)
/// W7 piranha plants: stationary overworld sprites whose positions must
/// stay in sync with their pointer table entries after pipe shuffling.
pub(crate) const MAP_OBJ_ENTRY_LINKS: &[(usize, usize, usize)] = &[
    (6, 2, 11), // W7 piranha plant 1
    (6, 3, 45), // W7 piranha plant 2
];

/// Data that travels with a level when shuffled.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LevelEntry {
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
pub(crate) struct BetaLevel {
    pub tileset: u8,
    pub obj_lo: u8,
    pub obj_hi: u8,
    pub lay_lo: u8,
    pub lay_hi: u8,
    pub name: &'static str,
}

/// Nine unreferenced beta levels found in the level data banks.
pub(crate) const BETA_LEVELS: &[BetaLevel] = &[
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
pub(crate) const BETA_PATCHES: &[(usize, u8)] = &[
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
pub(crate) struct FxSlot {
    pub grid_row: usize,
    pub grid_col: usize,
    pub replace_tile: u8,
}
