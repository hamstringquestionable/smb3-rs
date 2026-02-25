#!/usr/bin/env python3
# pyright: basic
"""
SMB3 ROM Map — comprehensive ROM analysis, map walking, and visualization.

Modes:
  python3 tools/rom_map.py [rom]                    # generate tools/rom_map.json
  python3 tools/rom_map.py [rom] --json out.json    # custom JSON output path
  python3 tools/rom_map.py [rom] --walk             # BFS walk all worlds
  python3 tools/rom_map.py [rom] --walk --world 6   # BFS walk one world
  python3 tools/rom_map.py [rom] --progression      # fortress progression sim
  python3 tools/rom_map.py [rom] --numbered         # BFS-ordered level numbering
  python3 tools/rom_map.py [rom] --viz              # pointer entry overlay
  python3 tools/rom_map.py [rom] --viz --raw        # raw hex tile IDs

Default ROM: "Super Mario Bros. 3 (USA) (Rev 1).nes"
"""

import json
import os
import sys
from collections import defaultdict, deque

# --------------------------------------------------------------------------
# Constants
# --------------------------------------------------------------------------

ROM_SIZE = 393232

# PRG bank layout
PRG_BANK_SIZE = 0x2000  # 8 KB
PRG_OFFSET = 0x10       # after 16-byte iNES header

# Level data regions by tileset (file offset ranges + extra-byte dispatch info)
# From powerups.rs
LEVEL_DATA_REGIONS = [
    {
        "name": "Underground (TS14)",
        "tileset_ids": [14],
        "start": 0x1A587,
        "end": 0x1C005,
        "extra_byte_dispatches": {35, 36, 37, 38, 39, 40, 41, 42, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71},
    },
    {
        "name": "Plains (TS1)",
        "tileset_ids": [1],
        "start": 0x1E512,
        "end": 0x20005,
        "extra_byte_dispatches": {11, 12, 35, 36, 37, 38, 39, 40, 41, 42},
    },
    {
        "name": "Hilly (TS3)",
        "tileset_ids": [3],
        "start": 0x20587,
        "end": 0x22005,
        "extra_byte_dispatches": {35, 36, 37, 38, 39, 40, 41, 42, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71},
    },
    {
        "name": "Ice/Sky (TS4/12)",
        "tileset_ids": [4, 12],
        "start": 0x227E0,
        "end": 0x24005,
        "extra_byte_dispatches": {0, 35, 36, 37, 38, 39, 40, 41, 42, 60, 112},
    },
    {
        "name": "Pipe/Water (TS7)",
        "tileset_ids": [7],
        "start": 0x24BA7,
        "end": 0x26005,
        "extra_byte_dispatches": {35, 36, 37, 38, 39, 40, 41, 42, 57},
    },
    {
        "name": "Cloudy/Giant/Plant (TS5/11/13)",
        "tileset_ids": [5, 11, 13],
        "start": 0x26A6F,
        "end": 0x28C05,
        "extra_byte_dispatches": {13, 35, 36, 37, 38, 39, 40, 41, 42, 45, 46, 48, 51},
    },
    {
        "name": "Desert (TS9)",
        "tileset_ids": [9],
        "start": 0x28F3F,
        "end": 0x2A005,
        "extra_byte_dispatches": {10, 11, 12, 13, 35, 36, 37, 38, 39, 40, 41, 42},
    },
    {
        "name": "Dungeon (TS2)",
        "tileset_ids": [2],
        "start": 0x2A7F7,
        "end": 0x2C005,
        "extra_byte_dispatches": {35, 36, 37, 38, 39, 40, 41, 42, 46, 47, 48},
    },
    {
        "name": "Ship (TS10)",
        "tileset_ids": [10],
        "start": 0x2EC07,
        "end": 0x30005,
        "extra_byte_dispatches": {1, 2, 35, 36, 37, 38, 39, 40, 41, 42, 48, 51},
    },
]

# LL_PowerBlocks table (ROM offset 0x1CAD4, 24 bytes)
LL_POWER_BLOCKS = [
    0x60, 0x61, 0x62, 0x64, 0x65, 0x66, 0x68, 0x69,
    0x6A, 0x6C, 0x6D, 0x6E, 0x6F, 0x70, 0x44, 0x45,
    0x03, 0x2F, 0x30, 0x31, 0x73, 0x74, 0x75, 0x46,
]

# Powerup block names indexed by byte2 value (0-15 = randomizable powerups)
POWER_NAMES = {
    0x00: "QBLOCKFLOWER", 0x01: "QBLOCKLEAF", 0x02: "QBLOCKSTAR",
    0x03: "QBLOCKCOINSTAR", 0x04: "QBLOCKCOIN", 0x05: "MUNCHER",
    0x06: "BRICKFLOWER", 0x07: "BRICKLEAF", 0x08: "BRICKSTAR",
    0x09: "BRICKCOINSTAR", 0x0A: "BRICK10COIN", 0x0B: "BRICK1UP",
    0x0C: "BRICKVINE", 0x0D: "BRICKPSWITCH", 0x0E: "INVISCOIN",
    0x0F: "INVIS1UP",
}

# Variable-size base offsets per group
VAR_BASES = [0, 15, 30, 45, 60, 75, 90, 105]

# Variable-size block run names
BLOCK_RUN_NAMES = {
    15: "BRICK", 16: "QBLOCKCOIN", 17: "BRICKCOIN", 18: "WOODBLOCK",
    19: "GNOTE", 20: "NOTE", 21: "WOODBLOCKBOUNCE", 22: "COIN",
}

# Level pointer tables (PRG012)
MASTER_TABLES = {
    "InitIndex": 0x193DA,
    "ByRowType": 0x193EC,
    "ByScrCol": 0x193FE,
    "ObjSets": 0x19410,
    "LevelLayouts": 0x19422,
}

# Per-world sub-table info
WORLDS = [
    {"name": "World 1 (Grass Land)", "rowtype_offset": 0x19438, "entry_count": 21},
    {"name": "World 2 (Desert Land)", "rowtype_offset": 0x194BA, "entry_count": 47},
    {"name": "World 3 (Water Land)", "rowtype_offset": 0x195D8, "entry_count": 52},
    {"name": "World 4 (Giant Land)", "rowtype_offset": 0x19714, "entry_count": 34},
    {"name": "World 5 (Sky Land)", "rowtype_offset": 0x197E4, "entry_count": 42},
    {"name": "World 6 (Ice Land)", "rowtype_offset": 0x198E4, "entry_count": 57},
    {"name": "World 7 (Pipe Land)", "rowtype_offset": 0x19A3E, "entry_count": 46},
    {"name": "World 8 (Dark Land)", "rowtype_offset": 0x19B56, "entry_count": 41},
]

# Per-world map tile grid metadata (PRG012)
# Pointer table at 0x185A8: 9 x 2-byte LE CPU pointers (8 worlds + warp zone)
# Storage: column-major (each column = 9 bytes for rows 0-8), 0xFF terminator after each world
MAP_TILE_GRIDS = [
    {"name": "World 1", "cpu_addr": 0xA5AA, "file_offset": 0x185BA, "columns": 16, "screens": 1},
    {"name": "World 2", "cpu_addr": 0xA63B, "file_offset": 0x1864B, "columns": 32, "screens": 2},
    {"name": "World 3", "cpu_addr": 0xA75C, "file_offset": 0x1876C, "columns": 48, "screens": 3},
    {"name": "World 4", "cpu_addr": 0xA90D, "file_offset": 0x1891D, "columns": 32, "screens": 2},
    {"name": "World 5", "cpu_addr": 0xAA2E, "file_offset": 0x18A3E, "columns": 32, "screens": 2},
    {"name": "World 6", "cpu_addr": 0xAB4F, "file_offset": 0x18B5F, "columns": 48, "screens": 3},
    {"name": "World 7", "cpu_addr": 0xAD00, "file_offset": 0x18D10, "columns": 32, "screens": 2},
    {"name": "World 8", "cpu_addr": 0xAE21, "file_offset": 0x18E31, "columns": 64, "screens": 4},
    {"name": "Warp Zone", "cpu_addr": 0xB062, "file_offset": 0x19072, "columns": None, "screens": None},
]
MAP_TILE_GRID_ROWS = 9  # All worlds have 9 rows
MAP_TILE_GRID_PTR_TABLE = 0x185A8  # File offset of the 9-entry pointer table

# Tileset-to-PRG bank mapping (bank at CPU $A000)
PAGE_A000_BY_TILESET = [11, 15, 21, 16, 17, 19, 18, 18, 18, 20, 23, 19, 17, 19, 13, 26, 26, 26, 9]

# Enemy/object data
ENEMY_DATA_START = 0x0BFD8
ENEMY_DATA_END = 0x0E00D

# Enemy class definitions
ENEMY_CLASSES = {
    "ground": [0x29, 0x2A, 0x33, 0x39, 0x3F, 0x40, 0x55, 0x6B, 0x70, 0x71, 0x72],
    "koopa": [0x6C, 0x6D],
    "big": [0x7A, 0x7B, 0x7C, 0x7E],
    "flying": [0x6E, 0x6F, 0x73, 0x74, 0x80],
    "water": [0x61, 0x62, 0x63, 0x64, 0x6A],
    "bro": [0x81, 0x82, 0x86, 0x87],
    "piranha": [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7],
    "cheep": [0x77, 0x88],
    "big_q_block": [0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A],
}

ENEMY_NAMES = {
    0x29: "Spike", 0x2A: "Patooie", 0x33: "Nipper", 0x39: "NipperHopping",
    0x3F: "DryBones", 0x40: "BusterBeatle", 0x55: "BobOmb", 0x6B: "PileDriver",
    0x70: "BuzzyBeatle", 0x71: "Spiny", 0x72: "Goomba",
    0x6C: "GreenTroopa", 0x6D: "RedTroopa",
    0x7A: "BigGreenTroopa", 0x7B: "BigRedTroopa", 0x7C: "BigGoomba", 0x7E: "BigGreenHopper",
    0x6E: "ParatroopaGreenHop", 0x6F: "FlyingRedParatroopa", 0x73: "ParaGoomba",
    0x74: "ParaGoombaMicros", 0x80: "FlyingGreenParatroopa",
    0x61: "BlooperWithKids", 0x62: "Blooper", 0x63: "BigBertha", 0x64: "CheepHopper",
    0x6A: "BlooperChildShoot",
    0x81: "HammerBro", 0x82: "BoomerangBro", 0x86: "HeavyBro", 0x87: "FireBro",
    0xA0: "GreenPiranha", 0xA1: "GreenPiranhaFlipped", 0xA2: "RedPiranha",
    0xA3: "RedPiranhaFlipped", 0xA4: "GreenPiranhaFire", 0xA5: "GreenPiranhaFireC",
    0xA6: "VenusFireTrap", 0xA7: "VenusFireTrapCeil",
    0x77: "GreenCheep", 0x88: "OrangeCheep",
    0x94: "BigQ_3Up", 0x95: "BigQ_Mushroom", 0x96: "BigQ_FireFlower",
    0x97: "BigQ_SuperLeaf", 0x98: "BigQ_Tanooki", 0x99: "BigQ_Frog", 0x9A: "BigQ_Hammer",
}

# Protected offsets
PROTECTED_POWERUP_OFFSETS = [0x23DB0, 0x23E1F, 0x23EA0]  # 7-7 Q-stars
PROTECTED_ENEMY_OFFSET = 0x0C336  # 7-F1 Tanooki big Q block

# Key ROM tables
KEY_TABLES = {
    "LL_PowerBlocks": {"offset": 0x1CAD4, "size": 24, "desc": "Fixed-size group 1 index -> tile ID"},
    "LATP_QBlocks": {"offset": 0x1168D, "size": 17, "desc": "Tile ID ($60+index) -> item type"},
    "World_BGM": {"offset": 0x3C424, "size": 9, "desc": "Music track per world"},
    "Princess_Rewards": {"offset": 0x360DE, "size": 7, "desc": "Princess reward items per world"},
    "Debug_Mode": {"offset": 0x309D5, "size": 1, "desc": "Debug toggle (enable value uncertain, 0x35=off)"},
    # World map tile grid pointer table (PRG012): 9 x 2-byte LE CPU pointers (8 worlds + warp zone)
    "Map_TileGrid_Ptrs": {"offset": 0x185A8, "size": 18, "desc": "Per-world tile grid CPU pointers (9x2)"},
    # World map scroll limit table (PRG010)
    "World_Map_Max_PanR": {"offset": 0x14F44, "size": 8, "desc": "Max rightward scroll per world (0x10=1 screen)"},
    # Fortress Lock & Bridge FX tables (PRG010, 17 FX slots 0x00-0x10)
    "FortressFX_VAddrH": {"offset": 0x147CD, "size": 17, "desc": "VRAM high byte per FX slot"},
    "FortressFX_VAddrL": {"offset": 0x147DE, "size": 17, "desc": "VRAM low byte per FX slot"},
    "FortressFX_MapCompIdx": {"offset": 0x147EF, "size": 34, "desc": "Map_Completions col+bit per FX slot (17x2)"},
    "FortressFX_Patterns": {"offset": 0x14811, "size": 68, "desc": "Replacement 8x8 patterns per FX slot (17x4)"},
    "FortressFX_MapLocationRow": {"offset": 0x14855, "size": 17, "desc": "Map row for tile replacement per FX slot"},
    "FortressFX_MapLocation": {"offset": 0x14866, "size": 17, "desc": "Map screen+col for tile replacement per FX slot"},
    "FortressFX_MapTileReplace": {"offset": 0x14877, "size": 17, "desc": "Replacement map tile per FX slot"},
    "FortressFX_W1_W8": {"offset": 0x14888, "size": 32, "desc": "Per-world FX slot assignments (4 per world, 0-padded)"},
    "FortressFXBase_ByWorld": {"offset": 0x148A8, "size": 8, "desc": "Per-world base index into FortressFX_Wx"},
}

# Boss enemy IDs for fortress/boss detection
BOSS_ENEMY_IDS = {
    0x0E: "Koopaling",
    0x18: "Bowser",
    0x4A: "BoomBoomQBall",
    0x4B: "BoomBoomJump",
    0x4C: "BoomBoomFly",
}

BOOMBOOM_IDS = {0x4A, 0x4B, 0x4C}
KOOPALING_IDS = {0x0E}
BOWSER_IDS = {0x18}

# Pipe destination tables (PRG002)
PIPE_MAP_XHI = 0x046AA      # 24 bytes: packed screen numbers (upper=A, lower=B)
PIPE_MAP_X = 0x046C2         # 24 bytes: packed column positions
PIPE_MAP_Y = 0x046DA         # 24 bytes: packed row nibbles
PIPE_MAP_SCRL_XHI = 0x046F2  # 24 bytes: packed scroll X high
PIPE_DEST_COUNT = 24          # 24 destination slots

# Destination byte -> world index (0-based)
DEST_TO_WORLD = {
    0x01: 1,  # W2
    0x02: 5, 0x03: 5,  # W6
    0x04: 6, 0x05: 6, 0x06: 6, 0x07: 6,  # W7
    0x08: 6, 0x09: 6, 0x0A: 6, 0x0B: 6,  # W7
    0x0C: 7, 0x0D: 7, 0x0E: 7, 0x0F: 7, 0x10: 7, 0x11: 7,  # W8
    0x12: 2, 0x13: 2, 0x14: 2,  # W3
    0x15: 3, 0x16: 3,  # W4
    0x17: 4,  # W5
}

# Known fortress entries (world_idx, entry_idx) — detected by Boom-Boom enemies
FORTRESS_ENTRIES = {
    (0, 11),
    (1, 13),
    (2, 13), (2, 34),
    (3, 9), (3, 16),
    (4, 12), (4, 31),
    (5, 9), (5, 27), (5, 48),
    (6, 5), (6, 40),
    (7, 7), (7, 10), (7, 26), (7, 36),
}

# Known airship entries (world_idx, entry_idx)
AIRSHIP_ENTRIES_SET = {
    (0, 17), (1, 36), (2, 49), (3, 6), (4, 35), (5, 53), (6, 43),
}

# Bowser's castle
BOWSER_ENTRY_PAIR = (7, 40)

# Map transition entries
MAP_TRANSITIONS_SET = set()

# Special level-based transitions (acts like pipes but aren't in pipe dest tables)
# W5 spiral castle (idx 10 at (2,12)) drops you at pipe (idx 21 at (2,24)) on screen 1
SPECIAL_TRANSITIONS = [
    (4, 10, 21),  # (world_idx, from_entry_idx, to_entry_idx)
]

# Human-readable level names, keyed by (world_idx, entry_idx)
# Most names are auto-derived from map tiles (0x03-0x0F = level 1-13).
# This dict holds overrides for special tiles that can't be auto-named.
# Tile meanings: 0x67/0xEB/0xAF = fortress, 0xC9 = airship, 0xCC = bowser,
#   0x5F = spiral castle, 0x68 = quicksand, 0x69 = pyramid,
#   0xE6 = W8 dark-land level (unnumbered), 0x4A/0x47 = hammer bro (but some are levels)
LEVEL_NAME_OVERRIDES = {
    # W2: special desert tiles
    (1, 32): "2-QS",   # quicksand (tile 0x68)
    (1, 42): "2-Pyr",  # pyramid (tile 0x69)
    # W5: spiral castle
    (4, 10): "5-SC",   # spiral castle (tile 0x5F)
    # W7: levels on hammer-bro tiles (0x4A)
    (6, 11): "7-P1",   # Piranha level 1 (tile 0x4A, not a hammer bro)
    (6, 45): "7-P2",   # Piranha level 2 (tile 0x4A)
    # W8: special levels
    (7, 5): "8-Tank",  # tank level (tile 0x47)
    (7, 7): "8-Navy",  # navy/battleship level (fortress tile 0xAF)
    (7, 14): "8-Hnd3", # hand trap 3 (tile 0xE6)
    (7, 15): "8-Hnd2", # hand trap 2 (tile 0xE6)
    (7, 16): "8-Hnd1", # hand trap 1 (tile 0xE6)
    (7, 10): "8-Air",  # air force level (fortress tile 0x47)
    (7, 26): "8F",     # the one true fortress
    (7, 36): "8-STnk", # super tank (fortress tile 0x47)
}


def derive_level_name(world_idx, entry_idx, entry_type, tile):
    """Derive a human-readable name from tile and entry type.

    Returns name string or empty string if not a named entry.
    """
    w = world_idx + 1  # 1-indexed for display

    # Check overrides first
    override = LEVEL_NAME_OVERRIDES.get((world_idx, entry_idx))
    if override:
        return override

    # Fortress by entry type (tiles vary: 0x67, 0xEB, 0xAF, 0x47...)
    if entry_type == "fortress":
        return f"{w}F"  # caller adds F2/F3 suffix for duplicates

    # Airship by entry type
    if entry_type == "airship":
        return f"{w}A"

    # Bowser by entry type
    if entry_type == "bowser":
        return "8B"

    # Numbered level tiles: 0x03 = level 1, 0x04 = level 2, ...
    if 0x03 <= tile <= 0x0F:
        return f"{w}-{tile - 2}"

    return ""

# InitIndex master table (PRG012) — 9 x 2-byte LE pointers (8 worlds + warp zone)
INIT_INDEX_MASTER = 0x193DA

# Character palette offsets
PALETTE_OFFSETS = {
    "mario_normal": {"offset": 0x10539, "size": 4},
    "luigi_normal": {"offset": 0x1053D, "size": 4},
    "fire": {"offset": 0x10541, "size": 4},
    "frog": {"offset": 0x10549, "size": 4},
    "tanooki": {"offset": 0x1054D, "size": 4},
    "hammer": {"offset": 0x10551, "size": 4},
    "lava": {"offset": 0x36DAA, "size": 4},
    "bowser": {"offset": 0x36DFE, "size": 4},
}


# --------------------------------------------------------------------------
# Map walker constants (BFS traversal, fortress progression, visualization)
# --------------------------------------------------------------------------

# Per-direction valid path tiles (Map_Object_Valid_Left/Right/Down/Up in PRG010)
VALID_HORZ = {0x45, 0xB2, 0xB3, 0xAC, 0xB7, 0xB8, 0xDA, 0xB9, 0xE6}
VALID_VERT = {0x46, 0xB1, 0xAA, 0xAB, 0xB0, 0xDB, 0xBA}

# Directions: (delta_row, delta_col, valid_set)
DIRECTIONS = [
    (0, +1, VALID_HORZ),   # right
    (0, -1, VALID_HORZ),   # left
    (+1, 0, VALID_VERT),   # down
    (-1, 0, VALID_VERT),   # up
]

# Special tiles
TILE_START = 0xE5
TILE_PIPE = 0xBC
TILE_SPIRAL = 0x5F

# Tiles that are "background" / non-walkable
BACKGROUND_TILES = {0xB4, 0xFF, 0x02}

# FX table offsets (17 slots for fortress locks/bridges)
FX_MAP_LOC_ROW = 0x14855       # 17 bytes
FX_MAP_LOC = 0x14866           # 17 bytes
FX_MAP_TILE_REPLACE = 0x14877  # 17 bytes
FX_WORLD_TABLE = 0x14888       # 32 bytes (4 per world)

# ANSI color codes
RESET   = "\033[0m"
RED     = "\033[1;31m"
GREEN   = "\033[1;32m"
CYAN    = "\033[1;36m"
MAGENTA = "\033[1;35m"
YELLOW  = "\033[1;33m"
DIM     = "\033[2m"
WHITE   = "\033[1;37m"
BLUE    = "\033[1;34m"


# --------------------------------------------------------------------------
# Helper functions
# --------------------------------------------------------------------------

def read_word(rom, offset):
    """Read a 16-bit little-endian word."""
    return rom[offset] | (rom[offset + 1] << 8)


def layout_file_offset(cpu_addr, tileset):
    """Convert a layout CPU address ($A000-$BFFF) + tileset to ROM file offset."""
    if tileset >= len(PAGE_A000_BY_TILESET) or cpu_addr < 0xA000:
        return None
    bank = PAGE_A000_BY_TILESET[tileset]
    return bank * PRG_BANK_SIZE + PRG_OFFSET + (cpu_addr - 0xA000)


def obj_file_offset(cpu_addr):
    """Convert an object data CPU address ($C000-$DFFF) to ROM file offset.
    Object data lives in PRG006 (bank 6), mapped at $C000."""
    if cpu_addr < 0xC000:
        return None
    # PRG006 is always at $C000 for level objects
    return 6 * PRG_BANK_SIZE + PRG_OFFSET + (cpu_addr - 0xC000)


def find_enemy_class(obj_id):
    """Find which enemy class an object ID belongs to."""
    for cls_name, ids in ENEMY_CLASSES.items():
        if obj_id in ids:
            return cls_name
    return None


def scan_enemy_segment_bosses(rom, obj_cpu_ptr):
    """Scan the enemy data segment at obj_cpu_ptr for boss enemy IDs.
    Returns a dict with 'has_boomboom', 'has_koopaling', 'has_bowser' bools
    and 'boss_ids' list of found boss enemy IDs."""
    result = {"has_boomboom": False, "has_koopaling": False, "has_bowser": False, "boss_ids": []}
    # Enemy data lives in PRG006 mapped at $C000-$DFFF
    if obj_cpu_ptr < 0xC000 or obj_cpu_ptr > 0xDFFF:
        return result
    file_off = obj_file_offset(obj_cpu_ptr)
    if file_off is None or file_off >= len(rom):
        return result
    pos = file_off + 1  # skip page flag byte
    while pos + 2 < len(rom):
        oid = rom[pos]
        if oid == 0xFF:
            break
        if oid in BOSS_ENEMY_IDS:
            result["boss_ids"].append(oid)
            if oid in BOOMBOOM_IDS:
                result["has_boomboom"] = True
            if oid in KOOPALING_IDS:
                result["has_koopaling"] = True
            if oid in BOWSER_IDS:
                result["has_bowser"] = True
        pos += 3
    return result


# --------------------------------------------------------------------------
# Level data parsing
# --------------------------------------------------------------------------

def parse_level_commands(rom, offset, region):
    """Parse all generator commands in a single level starting at offset.
    Returns (commands_list, end_offset)."""
    commands = []
    i = offset

    while i + 2 < len(rom) and i < region["end"] and rom[i] != 0xFF:
        byte0 = rom[i]
        byte1 = rom[i + 1]
        byte2 = rom[i + 2]

        group = (byte0 & 0xE0) >> 5
        row = byte0 & 0x0F
        hi = (byte0 >> 4) & 1
        screen = (byte1 >> 4) & 0x0F
        col = byte1 & 0x0F
        is_fixed = (byte2 & 0xF0) == 0

        cmd = {
            "offset": i,
            "bytes": [byte0, byte1, byte2],
            "group": group,
            "row": row,
            "hi": hi,
            "screen": screen,
            "col": col,
            "size": 3,
        }

        if group == 7:
            cmd["type"] = "junction"
        elif is_fixed:
            cmd["type"] = "fixed"
            fixed_idx = ((byte0 & 0xE0) >> 1) + byte2
            cmd["fixed_idx"] = fixed_idx

            # Check if it's a powerup block (group 1, indices 16-39)
            if group == 1 and 16 <= fixed_idx < 16 + len(LL_POWER_BLOCKS):
                power_idx = fixed_idx - 16
                cmd["powerup"] = True
                cmd["power_name"] = POWER_NAMES.get(byte2, f"POWER_{byte2}")
                cmd["tile_id"] = LL_POWER_BLOCKS[power_idx]
                cmd["byte2_offset"] = i + 2
                cmd["protected"] = (i + 2) in PROTECTED_POWERUP_OFFSETS

                # Classify for randomization
                if byte2 in (0x00, 0x01, 0x02):
                    cmd["randomize_class"] = "qblock"
                elif byte2 in (0x06, 0x07, 0x08):
                    cmd["randomize_class"] = "brick"
        else:
            cmd["type"] = "variable"
            var_type = byte2 >> 4
            width = byte2 & 0x0F
            dispatch = VAR_BASES[group] + var_type - 1
            cmd["dispatch"] = dispatch
            cmd["width"] = width

            # Check for block runs
            if dispatch in BLOCK_RUN_NAMES:
                cmd["block_run"] = BLOCK_RUN_NAMES[dispatch]

            # Check for extra byte
            if dispatch in region["extra_byte_dispatches"]:
                if i + 3 < len(rom):
                    cmd["extra_byte"] = rom[i + 3]
                    cmd["bytes"].append(rom[i + 3])
                    cmd["size"] = 4

        commands.append(cmd)
        i += cmd["size"]

    return commands, i


def parse_level_header(rom, offset):
    """Parse a 9-byte level header and return structured info."""
    header = rom[offset:offset + 9]
    return {
        "offset": offset,
        "bytes": list(header),
        "screens": (header[4] & 0x0F) + 1,
        "bg_palette": header[5] & 0x07,
        "obj_palette": (header[5] >> 3) & 0x03,
        "music": header[8] & 0x0F,
        "timer": (header[8] >> 6) & 0x03,
    }


def scan_level_data_region(rom, region):
    """Scan all levels within a level data region.
    Returns a list of level dicts."""
    levels = []
    i = region["start"]

    # Compute CPU address base for this region's tileset
    ts = region["tileset_ids"][0] if region["tileset_ids"] else None
    bank = PAGE_A000_BY_TILESET[ts] if ts is not None and ts < len(PAGE_A000_BY_TILESET) else None
    bank_start = bank * PRG_BANK_SIZE + PRG_OFFSET if bank is not None else None

    while i + 9 < region["end"]:
        # Parse header
        header = parse_level_header(rom, i)
        i += 9

        # Extract enemy/object pointer from header bytes 2-3
        enemy_ptr = header["bytes"][2] | (header["bytes"][3] << 8)

        # Scan enemy data for boss enemies
        boss_info = scan_enemy_segment_bosses(rom, enemy_ptr)

        # Compute layout CPU address
        layout_cpu = None
        if bank_start is not None:
            layout_cpu = 0xA000 + (header["offset"] - bank_start)

        # Parse commands
        commands, end = parse_level_commands(rom, i, region)

        # Count junctions
        junction_count = sum(1 for cmd in commands if cmd.get("type") == "junction")

        # Collect powerup blocks from this level
        powerups = []
        for cmd in commands:
            if cmd.get("powerup"):
                powerups.append({
                    "offset": cmd["offset"],
                    "byte2_offset": cmd["byte2_offset"],
                    "byte2": cmd["bytes"][2],
                    "name": cmd["power_name"],
                    "tile_id": cmd["tile_id"],
                    "screen": cmd["screen"],
                    "row": cmd["row"],
                    "col": cmd["col"],
                    "protected": cmd["protected"],
                    "randomize_class": cmd.get("randomize_class"),
                })

        level = {
            "header_offset": header["offset"],
            "data_offset": header["offset"] + 9,
            "end_offset": end,
            "region": region["name"],
            "header": header,
            "enemy_ptr": enemy_ptr,
            "layout_cpu": layout_cpu,
            "command_count": len(commands),
            "junction_count": junction_count,
            "powerup_count": len(powerups),
            "powerups": powerups,
            "has_boomboom": boss_info["has_boomboom"],
            "has_koopaling": boss_info["has_koopaling"],
            "has_bowser": boss_info["has_bowser"],
        }
        levels.append(level)

        # Skip terminator
        if end < len(rom) and rom[end] == 0xFF:
            i = end + 1
        else:
            break

    return levels


# --------------------------------------------------------------------------
# Level grouping (entry point + sub-areas)
# --------------------------------------------------------------------------

def build_level_groups(rom, all_region_levels, worlds_data):
    """For each pointer table entry, find all sub-area headers reachable
    from its position in the layout data.

    Strategy: multiple pointer table entries can point into the same data
    segment (a block of commands between two 0xFF terminators). All entries
    within the same segment share the same sub-areas — the segments that
    follow in the data stream. We group entries by which pre-parsed data
    segment they fall into, then their sub-areas are the subsequent segments
    until the next entry-containing segment.

    Returns a list of level groups, each containing:
      - entry_layout_cpu: CPU address of the entry-point level
      - entry_obj_ptr: enemy data pointer from the entry header
      - world_refs: list of (world, index) pointer table entries
      - sub_areas: list of sub-area info dicts (entry segment + following)
      - has_boomboom/has_koopaling/has_bowser: aggregate boss flags
    """
    # Build region lookup: for each tileset, find the region info
    ts_to_region = {}
    for region in all_region_levels:
        for ts_id in region["tileset_ids"]:
            ts_to_region[ts_id] = region

    groups = []

    for region_data in all_region_levels:
        levels = region_data["levels"]
        if not levels:
            continue

        # Build a sorted list of (file_offset_start, file_offset_end, level_dict)
        # for each parsed level/segment in this region
        segments = []
        for lv in levels:
            segments.append((lv["header_offset"], lv["end_offset"], lv))

        # Collect all pointer table entries that point into this region
        # Map each to the segment it falls within
        # segment_idx -> [(world, index, lay_ptr, obj_ptr)]
        seg_entries = defaultdict(list)

        for wd in worlds_data:
            for entry in wd["entries"]:
                lay = entry["lay_ptr"]
                if lay == 0 or entry["type"] not in ("level", "fortress", "airship", "bowser", "pipe", "hammer_bro"):
                    continue
                tileset = entry["tileset"]
                rgn = ts_to_region.get(tileset)
                if rgn is None or rgn["region"] != region_data["region"]:
                    continue
                file_off = layout_file_offset(lay, tileset)
                if file_off is None:
                    continue
                # Find which segment this falls within
                for seg_idx, (seg_start, seg_end, _) in enumerate(segments):
                    if seg_start <= file_off < seg_end:
                        seg_entries[seg_idx].append(
                            (wd["world"], entry["index"], lay, entry["obj_ptr"]))
                        break

        # Identify which segments are entry-point segments (have pointer table refs)
        entry_seg_indices = sorted(seg_entries.keys())

        # For each entry-point segment, the sub-areas are the subsequent segments
        # until the next entry-point segment
        for i, seg_idx in enumerate(entry_seg_indices):
            # Determine range of sub-area segments
            if i + 1 < len(entry_seg_indices):
                next_entry_seg = entry_seg_indices[i + 1]
            else:
                next_entry_seg = len(segments)

            # Collect sub-area info from segments [seg_idx .. next_entry_seg)
            # Stop at clearly invalid levels (garbage data past real level data)
            sub_areas = []
            for j in range(seg_idx, next_entry_seg):
                _, _, lv = segments[j]
                # Skip garbage: 0 commands with invalid enemy ptr
                if j > seg_idx and lv["command_count"] == 0 and lv["enemy_ptr"] in (0x0000, 0xFFFF):
                    continue
                if lv["command_count"] > 700:
                    break  # Past real data (largest valid is ~641 cmds)
                sub_areas.append({
                    "header_offset": lv["header_offset"],
                    "layout_cpu": lv.get("layout_cpu"),
                    "enemy_ptr": lv["enemy_ptr"],
                    "screens": lv["header"]["screens"],
                    "command_count": lv["command_count"],
                    "junction_count": lv["junction_count"],
                    "has_boomboom": lv["has_boomboom"],
                    "has_koopaling": lv["has_koopaling"],
                    "has_bowser": lv["has_bowser"],
                })

            refs = seg_entries[seg_idx]
            world_refs = [(w, idx) for w, idx, _, _ in refs]
            obj_ptrs = list(set(obj for _, _, _, obj in refs))
            # Use the first ref's lay_ptr as the canonical entry
            _, _, lay_ptr, _ = refs[0]
            entry_enemy = sub_areas[0]["enemy_ptr"] if sub_areas else 0

            # Boss flags: check BOTH layout header enemy ptrs AND pointer table obj_ptrs
            # The obj_ptr is what the game uses for entry-point enemies;
            # layout header bytes 2-3 are for sub-area enemy data
            has_boomboom = any(sa["has_boomboom"] for sa in sub_areas)
            has_koopaling = any(sa["has_koopaling"] for sa in sub_areas)
            has_bowser = any(sa["has_bowser"] for sa in sub_areas)

            for obj_ptr in obj_ptrs:
                obj_boss = scan_enemy_segment_bosses(rom, obj_ptr)
                has_boomboom = has_boomboom or obj_boss["has_boomboom"]
                has_koopaling = has_koopaling or obj_boss["has_koopaling"]
                has_bowser = has_bowser or obj_boss["has_bowser"]

            groups.append({
                "region": region_data["region"],
                "entry_layout_cpu": lay_ptr,
                "entry_obj_ptrs": sorted(obj_ptrs),
                "entry_enemy_ptr": entry_enemy,
                "world_refs": world_refs,
                "level_count": len(sub_areas),
                "sub_area_count": len(sub_areas) - 1,
                "has_boomboom": has_boomboom,
                "has_koopaling": has_koopaling,
                "has_bowser": has_bowser,
                "sub_areas": sub_areas,
            })

    return groups


# --------------------------------------------------------------------------
# Level pointer table parsing
# --------------------------------------------------------------------------

def has_pipeway_controller(rom, obj_ptr):
    """Check if enemy data at obj_ptr contains PIPEWAYCONTROLLER (0x25)."""
    if obj_ptr < 0xC000 or obj_ptr > 0xDFFF:
        return False
    file_off = obj_file_offset(obj_ptr)
    if file_off is None or file_off + 1 >= len(rom):
        return False
    pos = file_off + 1  # skip page flag
    while pos + 2 < len(rom):
        if rom[pos] == 0xFF:
            break
        if rom[pos] == 0x25:
            return True
        pos += 3
    return False


def classify_entry(world_idx, entry_idx, obj_ptr, lay_ptr, tileset, rom=None):
    """Classify a level pointer table entry by type."""
    if (world_idx, entry_idx) in FORTRESS_ENTRIES:
        return "fortress"
    if (world_idx, entry_idx) in AIRSHIP_ENTRIES_SET:
        return "airship"
    if (world_idx, entry_idx) == BOWSER_ENTRY_PAIR:
        return "bowser"
    if (world_idx, entry_idx) in MAP_TRANSITIONS_SET:
        return "transition"
    if obj_ptr == 0x0700:
        return "toad_house"
    if obj_ptr == 0x0001 and lay_ptr == 0x0000:
        return "bonus_game"
    if rom is not None and obj_ptr >= 0xC000 and has_pipeway_controller(rom, obj_ptr):
        return "pipe"
    if obj_ptr >= 0xC000 and lay_ptr != 0x0000:
        return "level"
    if obj_ptr < 0x1000:
        return "special"
    return "unknown"


def parse_world_tables(rom, world_idx, world_info):
    """Parse a world's pointer tables and return structured entry data."""
    n = world_info["entry_count"]
    rt_off = world_info["rowtype_offset"]
    sc_off = rt_off + n
    obj_off = sc_off + n
    lay_off = obj_off + n * 2

    entries = []
    for i in range(n):
        rowtype = rom[rt_off + i]
        scrcol = rom[sc_off + i]
        obj_ptr = read_word(rom, obj_off + i * 2)
        lay_ptr = read_word(rom, lay_off + i * 2)

        tileset = rowtype & 0x0F
        row_nib = (rowtype >> 4) & 0x0F
        screen = (scrcol >> 4) & 0x0F
        col = scrcol & 0x0F
        grid_row = row_nib - 2
        grid_col = screen * 16 + col

        entry_type = classify_entry(world_idx, i, obj_ptr, lay_ptr, tileset, rom)

        entry = {
            "index": i,
            "rowtype": rowtype,
            "rowtype_offset": rt_off + i,
            "scrcol": scrcol,
            "scrcol_offset": sc_off + i,
            "obj_ptr": obj_ptr,
            "obj_offset": obj_off + i * 2,
            "lay_ptr": lay_ptr,
            "lay_offset": lay_off + i * 2,
            "tileset": tileset,
            "type": entry_type,
            "grid_row": grid_row,
            "grid_col": grid_col,
            "screen": screen,
            "col_in_screen": col,
            "row_nib": row_nib,
        }

        # Resolve file offsets for levels/fortresses/airships/pipes
        if entry_type in ("level", "fortress", "airship", "bowser", "pipe") and lay_ptr != 0:
            lay_file = layout_file_offset(lay_ptr, tileset)
            if lay_file is not None and lay_file + 9 < len(rom):
                entry["layout_file_offset"] = lay_file
                entry["screens"] = (rom[lay_file + 4] & 0x0F) + 1

        # Check shuffleability (same criteria as levels.rs)
        if entry_type == "level":
            entry["shuffleable"] = True
            if entry.get("screens", 0) < 3:
                entry["shuffleable"] = False
                entry["exclude_reason"] = "too_short"
        else:
            entry["shuffleable"] = False
            entry["exclude_reason"] = entry_type

        entries.append(entry)

    # Mark duplicate (obj, lay) pairs as hammer bros (non-shuffleable)
    pair_counts = defaultdict(int)
    for e in entries:
        if e["type"] == "level":
            pair_counts[(e["obj_ptr"], e["lay_ptr"])] += 1

    for e in entries:
        if e["type"] == "level" and pair_counts.get((e["obj_ptr"], e["lay_ptr"]), 0) > 1:
            e["type"] = "hammer_bro"
            e["shuffleable"] = False
            e["exclude_reason"] = "hammer_bro"

    return {
        "world": world_idx + 1,
        "name": world_info["name"],
        "entry_count": n,
        "rowtype_offset": rt_off,
        "scrcol_offset": sc_off,
        "objsets_offset": obj_off,
        "layouts_offset": lay_off,
        "table_end": lay_off + n * 2,
        "entries": entries,
        "shuffleable_count": sum(1 for e in entries if e.get("shuffleable")),
    }


# --------------------------------------------------------------------------
# Enemy data parsing
# --------------------------------------------------------------------------

def parse_enemy_data(rom):
    """Parse the enemy/object data block and catalog all entries."""
    data = rom[ENEMY_DATA_START:ENEMY_DATA_END]
    segments = []
    i = 0

    while i < len(data):
        # Skip terminators
        if data[i] == 0xFF:
            i += 1
            continue

        # Page flag byte
        page_flag = data[i]
        seg_start = ENEMY_DATA_START + i
        i += 1

        # Parse 3-byte entries
        entries = []
        while i + 2 < len(data) and data[i] != 0xFF:
            obj_id = data[i]
            x_pos = data[i + 1]
            y_pos = data[i + 2]
            file_offset = ENEMY_DATA_START + i

            entry = {
                "offset": file_offset,
                "obj_id": obj_id,
                "x": x_pos,
                "y": y_pos,
            }

            name = ENEMY_NAMES.get(obj_id)
            if name:
                entry["name"] = name

            cls = find_enemy_class(obj_id)
            if cls:
                entry["class"] = cls
                entry["randomizable"] = True
                if file_offset == PROTECTED_ENEMY_OFFSET:
                    entry["protected"] = True
                    entry["randomizable"] = False
            else:
                entry["randomizable"] = False

            entries.append(entry)
            i += 3

        if entries:
            segments.append({
                "start_offset": seg_start,
                "page_flag": page_flag,
                "entry_count": len(entries),
                "entries": entries,
            })

    return segments


# --------------------------------------------------------------------------
# Main map generation
# --------------------------------------------------------------------------

# --------------------------------------------------------------------------
# Pipe destination tables
# --------------------------------------------------------------------------

def parse_pipe_dest_tables(rom):
    """Parse the 4 pipe destination tables and decode all pipe pairs.

    Returns a dict with:
      - tables: raw table data (4 x 24 bytes)
      - destinations: per-dest-index decoded info
      - pairs_by_world: world_idx -> list of pipe pair dicts
    """
    tables = {
        "map_xhi": {"offset": PIPE_MAP_XHI, "bytes": list(rom[PIPE_MAP_XHI:PIPE_MAP_XHI + PIPE_DEST_COUNT])},
        "map_x": {"offset": PIPE_MAP_X, "bytes": list(rom[PIPE_MAP_X:PIPE_MAP_X + PIPE_DEST_COUNT])},
        "map_y": {"offset": PIPE_MAP_Y, "bytes": list(rom[PIPE_MAP_Y:PIPE_MAP_Y + PIPE_DEST_COUNT])},
        "map_scrl_xhi": {"offset": PIPE_MAP_SCRL_XHI, "bytes": list(rom[PIPE_MAP_SCRL_XHI:PIPE_MAP_SCRL_XHI + PIPE_DEST_COUNT])},
    }

    destinations = []
    pairs_by_world = {i: [] for i in range(8)}

    for dest in range(PIPE_DEST_COUNT):
        xhi = rom[PIPE_MAP_XHI + dest]
        x = rom[PIPE_MAP_X + dest]
        y = rom[PIPE_MAP_Y + dest]
        scrl = rom[PIPE_MAP_SCRL_XHI + dest]

        a_scr = (xhi >> 4) & 0x0F
        b_scr = xhi & 0x0F
        a_col = (x >> 4) & 0x0F
        b_col = x & 0x0F
        a_row_nib = (y >> 4) & 0x0F
        b_row_nib = y & 0x0F

        a_grid_row = a_row_nib - 2
        b_grid_row = b_row_nib - 2
        a_grid_col = a_scr * 16 + a_col
        b_grid_col = b_scr * 16 + b_col

        world_idx = DEST_TO_WORLD.get(dest)

        entry = {
            "dest_index": dest,
            "world": world_idx + 1 if world_idx is not None else None,
            "world_idx": world_idx,
            "raw_bytes": {"xhi": xhi, "x": x, "y": y, "scrl": scrl},
            "endpoint_a": {"grid_row": a_grid_row, "grid_col": a_grid_col, "screen": a_scr, "col": a_col, "row_nib": a_row_nib},
            "endpoint_b": {"grid_row": b_grid_row, "grid_col": b_grid_col, "screen": b_scr, "col": b_col, "row_nib": b_row_nib},
        }
        destinations.append(entry)

        if world_idx is not None:
            pairs_by_world[world_idx].append({
                "dest_index": dest,
                "a": {"grid_row": a_grid_row, "grid_col": a_grid_col},
                "b": {"grid_row": b_grid_row, "grid_col": b_grid_col},
            })

    return {
        "tables": tables,
        "dest_to_world": {str(k): v for k, v in sorted(DEST_TO_WORLD.items())},
        "destinations": destinations,
        "pairs_by_world": {str(k): v for k, v in pairs_by_world.items()},
        "summary": {
            "total_dest_slots": PIPE_DEST_COUNT,
            "active_slots": len(DEST_TO_WORLD),
            "pairs_per_world": {f"W{k+1}": len(v) for k, v in pairs_by_world.items()},
        },
    }


# --------------------------------------------------------------------------
# InitIndex table
# --------------------------------------------------------------------------

def parse_init_index(rom):
    """Parse the InitIndex master table and per-world sub-tables.

    The InitIndex table stores per-screen byte offsets used by the game
    to quickly find the first pointer table entry on each screen.

    Returns dict with master table pointers and per-world decoded data.
    """
    master = []
    for i in range(9):
        ptr = read_word(rom, INIT_INDEX_MASTER + i * 2)
        master.append(ptr)

    per_world = []
    for w_idx in range(8):
        world = WORLDS[w_idx]
        n = world["entry_count"]
        grid_info = MAP_TILE_GRIDS[w_idx]
        num_screens = grid_info["screens"]

        # InitIndex sub-table: CPU pointer from master table
        cpu_ptr = master[w_idx]
        # Convert to file offset (PRG012 at bank 12, CPU $8000)
        file_off = 12 * PRG_BANK_SIZE + PRG_OFFSET + (cpu_ptr - 0x8000)

        bytes_list = list(rom[file_off:file_off + num_screens])

        per_world.append({
            "world": w_idx + 1,
            "cpu_ptr": cpu_ptr,
            "file_offset": file_off,
            "screens": num_screens,
            "bytes": bytes_list,
        })

    return {
        "master_table_offset": INIT_INDEX_MASTER,
        "master_pointers": master,
        "per_world": per_world,
    }


def generate_rom_map(rom):
    """Generate the complete ROM map."""
    rom_map = {
        "_comment": "SMB3 (USA Rev 1) ROM Map - auto-generated by tools/rom_map.py",
        "rom_size": len(rom),
        "rom_sha256": None,  # could add if needed
    }

    # -- Key tables --
    tables = {}
    for name, info in KEY_TABLES.items():
        tables[name] = {
            "offset": info["offset"],
            "size": info["size"],
            "desc": info["desc"],
            "bytes": list(rom[info["offset"]:info["offset"] + info["size"]]),
        }
    rom_map["key_tables"] = tables

    # -- Palette offsets --
    palettes = {}
    for name, info in PALETTE_OFFSETS.items():
        palettes[name] = {
            "offset": info["offset"],
            "size": info["size"],
            "bytes": list(rom[info["offset"]:info["offset"] + info["size"]]),
        }
    rom_map["palettes"] = palettes

    # -- Level data regions (powerup scanning) --
    all_region_levels = []
    total_powerups = 0
    total_levels = 0

    for region in LEVEL_DATA_REGIONS:
        levels = scan_level_data_region(rom, region)
        total_levels += len(levels)
        for lev in levels:
            total_powerups += lev["powerup_count"]
        all_region_levels.append({
            "region": region["name"],
            "tileset_ids": region["tileset_ids"],
            "start": region["start"],
            "end": region["end"],
            "extra_byte_dispatches": sorted(region["extra_byte_dispatches"]),
            "level_count": len(levels),
            "levels": levels,
        })

    rom_map["level_data_regions"] = all_region_levels
    rom_map["level_data_summary"] = {
        "total_regions": len(LEVEL_DATA_REGIONS),
        "total_levels_in_regions": total_levels,
        "total_powerup_blocks": total_powerups,
    }

    # -- World pointer tables --
    worlds = []
    for w_idx, w_info in enumerate(WORLDS):
        world_data = parse_world_tables(rom, w_idx, w_info)
        worlds.append(world_data)

    rom_map["worlds"] = worlds
    rom_map["world_summary"] = {
        "total_entries": sum(w["entry_count"] for w in worlds),
        "total_shuffleable": sum(w["shuffleable_count"] for w in worlds),
    }

    # -- Level groups (entry point + sub-areas with boss tracking) --
    level_groups = build_level_groups(rom, all_region_levels, worlds)

    rom_map["level_groups"] = level_groups
    rom_map["level_groups_summary"] = {
        "total_groups": len(level_groups),
        "groups_with_boomboom": sum(1 for g in level_groups if g["has_boomboom"]),
        "groups_with_koopaling": sum(1 for g in level_groups if g["has_koopaling"]),
        "groups_with_bowser": sum(1 for g in level_groups if g["has_bowser"]),
    }

    # -- Enemy/object data --
    enemy_segments = parse_enemy_data(rom)
    total_enemies = sum(s["entry_count"] for s in enemy_segments)
    randomizable_enemies = sum(
        sum(1 for e in s["entries"] if e.get("randomizable"))
        for s in enemy_segments
    )

    rom_map["enemy_data"] = {
        "start": ENEMY_DATA_START,
        "end": ENEMY_DATA_END,
        "segment_count": len(enemy_segments),
        "total_entries": total_enemies,
        "randomizable_entries": randomizable_enemies,
        "segments": enemy_segments,
    }

    # -- Protected offsets --
    rom_map["protected_offsets"] = {
        "powerup_byte2": [
            {"offset": o, "reason": "7-7 Q-star (muncher level)"} for o in PROTECTED_POWERUP_OFFSETS
        ],
        "enemy_obj_id": [
            {"offset": PROTECTED_ENEMY_OFFSET, "reason": "7-F1 Tanooki big Q block (flying required)"},
        ],
    }

    # -- Pipe destination tables --
    rom_map["pipe_destinations"] = parse_pipe_dest_tables(rom)

    # -- InitIndex table --
    rom_map["init_index"] = parse_init_index(rom)

    return rom_map


def print_summary(rom_map):
    """Print a human-readable summary of the map."""
    print("=" * 70)
    print("SMB3 ROM Map Summary")
    print("=" * 70)

    # Level data regions
    summary = rom_map["level_data_summary"]
    print(f"\nLevel Data Regions: {summary['total_regions']}")
    print(f"  Total levels found: {summary['total_levels_in_regions']}")
    print(f"  Total powerup blocks: {summary['total_powerup_blocks']}")

    for region in rom_map["level_data_regions"]:
        lvl_count = region["level_count"]
        pw_count = sum(l["powerup_count"] for l in region["levels"])
        print(f"  {region['region']}: {lvl_count} levels, {pw_count} powerups "
              f"(0x{region['start']:05X}-0x{region['end']:05X})")

    # World pointer tables
    ws = rom_map["world_summary"]
    print(f"\nWorld Pointer Tables:")
    print(f"  Total entries: {ws['total_entries']}")
    print(f"  Shuffleable: {ws['total_shuffleable']}")

    for world in rom_map["worlds"]:
        types = defaultdict(int)
        for e in world["entries"]:
            types[e["type"]] += 1
        type_str = ", ".join(f"{k}={v}" for k, v in sorted(types.items()))
        print(f"  {world['name']}: {world['entry_count']} entries "
              f"({world['shuffleable_count']} shuffleable) [{type_str}]")

    # Level groups
    lgs = rom_map["level_groups_summary"]
    print(f"\nLevel Groups (entry point + sub-areas):")
    print(f"  Total groups: {lgs['total_groups']}")
    print(f"  With Boom-Boom: {lgs['groups_with_boomboom']}")
    print(f"  With Koopaling: {lgs['groups_with_koopaling']}")
    print(f"  With Bowser: {lgs['groups_with_bowser']}")

    # Show groups with boom-boom and their world refs
    for g in rom_map["level_groups"]:
        if g["has_boomboom"]:
            refs = ", ".join(f"W{w}[{i}]" for w, i in g["world_refs"])
            if not refs:
                refs = "(no ptr table ref)"
            print(f"    {refs}: lay=0x{g['entry_layout_cpu']:04X} "
                  f"enemy=0x{g['entry_enemy_ptr']:04X} "
                  f"{g['level_count']} levels "
                  f"({'+ koopaling' if g['has_koopaling'] else ''}{'+ bowser' if g['has_bowser'] else ''})")

    # Enemy data
    ed = rom_map["enemy_data"]
    print(f"\nEnemy/Object Data:")
    print(f"  Segments: {ed['segment_count']}")
    print(f"  Total entries: {ed['total_entries']}")
    print(f"  Randomizable: {ed['randomizable_entries']}")

    # Count by class
    class_counts = defaultdict(int)
    for seg in ed["segments"]:
        for e in seg["entries"]:
            cls = e.get("class")
            if cls:
                class_counts[cls] += 1
    if class_counts:
        print("  By class: " + ", ".join(f"{k}={v}" for k, v in sorted(class_counts.items())))

    # Pipe destinations
    pd = rom_map["pipe_destinations"]["summary"]
    print(f"\nPipe Destinations:")
    print(f"  Total slots: {pd['total_dest_slots']}, Active: {pd['active_slots']}")
    print(f"  Pairs per world: {', '.join(f'{k}={v}' for k, v in sorted(pd['pairs_per_world'].items()))}")

    # InitIndex
    ii = rom_map["init_index"]
    print(f"\nInitIndex Table:")
    for pw in ii["per_world"]:
        print(f"  W{pw['world']}: {pw['screens']} screens, bytes={pw['bytes']}")

    # Protected offsets
    prot = rom_map["protected_offsets"]
    print(f"\nProtected Offsets:")
    for p in prot["powerup_byte2"]:
        print(f"  0x{p['offset']:05X}: {p['reason']}")
    for p in prot["enemy_obj_id"]:
        print(f"  0x{p['offset']:05X}: {p['reason']}")

    print()


# --------------------------------------------------------------------------
# Map walker: tile grid, BFS, pipes, FX, fortress progression
# --------------------------------------------------------------------------

def read_tile_grid(rom, world_idx):
    """Read a world's tile grid as a 2D list [row][col]."""
    info = MAP_TILE_GRIDS[world_idx]
    start = info["file_offset"]
    cols = info["columns"]
    grid = []
    for r in range(MAP_TILE_GRID_ROWS):
        row = []
        for c in range(cols):
            screen = c // 16
            col_in_screen = c % 16
            row.append(rom[start + screen * 144 + r * 16 + col_in_screen])
        grid.append(row)
    return grid


def find_start(grid):
    """Find the START tile ($E5) position."""
    for r in range(len(grid)):
        for c in range(len(grid[0])):
            if grid[r][c] == TILE_START:
                return (r, c)
    return None


def entry_grid_position(rom, world_idx, entry_idx):
    """Get (grid_row, grid_col) for a pointer table entry."""
    world = WORLDS[world_idx]
    n = world["entry_count"]
    rt_off = world["rowtype_offset"]
    sc_off = rt_off + n
    row_nib = (rom[rt_off + entry_idx] >> 4) & 0x0F
    scrcol = rom[sc_off + entry_idx]
    screen = (scrcol >> 4) & 0x0F
    col = scrcol & 0x0F
    return (row_nib - 2, screen * 16 + col)


def read_pipe_pairs(rom):
    """Read pipe pairs from destination tables. Returns dict: world_idx -> list of (pos_a, pos_b)."""
    pipes_by_world = {i: [] for i in range(8)}
    for dest in range(0x18):
        if dest not in DEST_TO_WORLD:
            continue
        world_idx = DEST_TO_WORLD[dest]
        xhi = rom[PIPE_MAP_XHI + dest]
        x   = rom[PIPE_MAP_X + dest]
        y   = rom[PIPE_MAP_Y + dest]
        a_scr = (xhi >> 4) & 0x0F
        b_scr = xhi & 0x0F
        a_col = (x >> 4) & 0x0F
        b_col = x & 0x0F
        a_row = (y >> 4) & 0x0F
        b_row = y & 0x0F
        pipes_by_world[world_idx].append(
            ((a_row - 2, a_scr * 16 + a_col), (b_row - 2, b_scr * 16 + b_col))
        )
    # Add special transitions (e.g. W5 spiral castle -> screen 1 pipe)
    for world_idx, from_idx, to_idx in SPECIAL_TRANSITIONS:
        pos_a = entry_grid_position(rom, world_idx, from_idx)
        pos_b = entry_grid_position(rom, world_idx, to_idx)
        pipes_by_world[world_idx].append((pos_a, pos_b))

    return pipes_by_world


def read_fx_slots(rom):
    """Read all 17 FX slots — grid position and replacement tile."""
    slots = []
    for i in range(17):
        loc_row = rom[FX_MAP_LOC_ROW + i]
        loc = rom[FX_MAP_LOC + i]
        grid_row = (loc_row >> 4) - 2
        col_in_screen = (loc >> 4) & 0x0F
        screen = loc & 0x0F
        slots.append({
            "grid_row": grid_row,
            "grid_col": screen * 16 + col_in_screen,
            "replace_tile": rom[FX_MAP_TILE_REPLACE + i],
        })
    return slots


def read_world_fx_assignments(rom):
    """Read per-world FX slot assignments. Returns dict: world_idx -> list of slot indices."""
    assignments = {}
    for wi in range(8):
        fort_count = sum(1 for w, _ in FORTRESS_ENTRIES if w == wi)
        base = FX_WORLD_TABLE + wi * 4
        assignments[wi] = [rom[base + i] for i in range(min(fort_count, 4))]
    return assignments


def read_fortress_positions(rom):
    """Grid positions of fortress entries. Returns dict: world_idx -> list of (row, col)."""
    by_world = {}
    for wi, ei in sorted(FORTRESS_ENTRIES):
        by_world.setdefault(wi, []).append(ei)
    positions = {}
    for wi, entries in by_world.items():
        positions[wi] = [entry_grid_position(rom, wi, ei) for ei in entries]
    return positions


def walk_map(grid, pipe_pairs, start_pos=None):
    """BFS walk using SMB3's 2-tile movement model.

    Returns (nodes, edges, path_tiles, bfs_order) where bfs_order is a list
    of (row, col) in the order they were first discovered.
    """
    rows = len(grid)
    cols = len(grid[0])
    start = start_pos if start_pos is not None else find_start(grid)
    if start is None:
        return set(), {}, set(), []

    pipe_lookup = {}
    for a, b in pipe_pairs:
        pipe_lookup.setdefault(a, []).append(b)
        pipe_lookup.setdefault(b, []).append(a)

    nodes = set()
    edges = {}
    path_tiles = set()
    bfs_order = []
    queue = deque([start])
    nodes.add(start)
    bfs_order.append(start)

    while queue:
        r, c = queue.popleft()
        if (r, c) not in edges:
            edges[(r, c)] = []

        for dr, dc, valid_set in DIRECTIONS:
            pr, pc = r + dr, c + dc
            if pr < 0 or pr >= rows or pc < 0 or pc >= cols:
                continue
            if grid[pr][pc] not in valid_set:
                continue
            nr, nc = r + 2 * dr, c + 2 * dc
            if nr < 0 or nr >= rows or nc < 0 or nc >= cols:
                continue
            if grid[nr][nc] in BACKGROUND_TILES:
                continue
            path_tiles.add((pr, pc))
            edges[(r, c)].append(((nr, nc), (pr, pc), grid[pr][pc]))
            if (nr, nc) not in nodes:
                nodes.add((nr, nc))
                bfs_order.append((nr, nc))
                queue.append((nr, nc))

        if (r, c) in pipe_lookup:
            for dest in pipe_lookup[(r, c)]:
                if dest not in nodes:
                    nodes.add(dest)
                    bfs_order.append(dest)
                    queue.append(dest)
                edges[(r, c)].append((dest, None, "pipe"))

    return nodes, edges, path_tiles, bfs_order


def simulate_progression(rom, world_idx, pipe_pairs):
    """Simulate fortress progression: walk, beat forts, open locks, re-walk.

    Returns list of steps. Each step has:
        fort_idx, fort_pos, fx_pos, fx_old_tile, fx_new_tile,
        nodes, path_tiles, bfs_order, grid (snapshot)
    """
    grid = read_tile_grid(rom, world_idx)
    fx_slots = read_fx_slots(rom)
    fx_assignments = read_world_fx_assignments(rom)
    fort_positions = read_fortress_positions(rom)

    world_fx = fx_assignments.get(world_idx, [])
    world_forts = fort_positions.get(world_idx, [])
    beaten = set()
    steps = []

    nodes, edges, path_tiles, bfs_order = walk_map(grid, pipe_pairs)
    steps.append({
        "fort_idx": None, "fort_pos": None,
        "fx_pos": None, "fx_old_tile": None, "fx_new_tile": None,
        "nodes": set(nodes), "path_tiles": set(path_tiles),
        "bfs_order": list(bfs_order),
        "grid": [row[:] for row in grid],
    })

    while True:
        reachable = [i for i, pos in enumerate(world_forts)
                     if i not in beaten and pos in nodes]
        if not reachable:
            break
        fi = reachable[0]
        beaten.add(fi)
        fx_pos = fx_old = fx_new = None
        if fi < len(world_fx):
            si = world_fx[fi]
            slot = fx_slots[si]
            fr, fc = slot["grid_row"], slot["grid_col"]
            fx_old = grid[fr][fc]
            fx_new = slot["replace_tile"]
            grid[fr][fc] = fx_new
            fx_pos = (fr, fc)

        nodes, edges, path_tiles, bfs_order = walk_map(grid, pipe_pairs)
        steps.append({
            "fort_idx": fi, "fort_pos": world_forts[fi],
            "fx_pos": fx_pos, "fx_old_tile": fx_old, "fx_new_tile": fx_new,
            "nodes": set(nodes), "path_tiles": set(path_tiles),
            "bfs_order": list(bfs_order),
            "grid": [row[:] for row in grid],
        })

    return steps


def find_chokepoints(nodes, edges):
    """Find path tiles whose removal disconnects the node graph."""
    if not nodes:
        return set()
    adj = {n: [] for n in nodes}
    for node, nbrs in edges.items():
        for dest, path_pos, _ in nbrs:
            adj[node].append((dest, path_pos))
    path_positions = set()
    for node, nbrs in edges.items():
        for _, path_pos, _ in nbrs:
            if path_pos is not None:
                path_positions.add(path_pos)
    chokepoints = set()
    start = next(iter(nodes))
    for pp in path_positions:
        visited = {start}
        q = deque([start])
        while q:
            n = q.popleft()
            for dest, p in adj[n]:
                if p == pp:
                    continue
                if dest not in visited:
                    visited.add(dest)
                    q.append(dest)
        if len(visited) < len(nodes):
            chokepoints.add(pp)
    return chokepoints


# --------------------------------------------------------------------------
# Map visualization: entry lookup, BFS-numbered rendering
# --------------------------------------------------------------------------

def build_entry_lookup(rom, world_idx):
    """Build (grid_row, grid_col) -> entry info dict for a world's pointer table.

    Detects hammer bros via duplicate (obj, lay) pairs.
    """
    world = WORLDS[world_idx]
    grid = read_tile_grid(rom, world_idx)
    n = world["entry_count"]
    rt_off = world["rowtype_offset"]
    sc_off = rt_off + n
    obj_off = sc_off + n
    lay_off = obj_off + n * 2

    entries = []
    for i in range(n):
        rowtype = rom[rt_off + i]
        scrcol = rom[sc_off + i]
        obj_ptr = read_word(rom, obj_off + i * 2)
        lay_ptr = read_word(rom, lay_off + i * 2)
        tileset = rowtype & 0x0F
        row_nib = (rowtype >> 4) & 0x0F
        screen = (scrcol >> 4) & 0x0F
        col = scrcol & 0x0F
        grid_row = row_nib - 2
        grid_col = screen * 16 + col

        entry_type = classify_entry(world_idx, i, obj_ptr, lay_ptr, tileset, rom)
        # Check if this is the start tile
        if (0 <= grid_row < len(grid) and 0 <= grid_col < len(grid[0])
                and grid[grid_row][grid_col] == TILE_START):
            entry_type = "start"
        entries.append({
            "index": i, "type": entry_type, "tileset": tileset,
            "obj_ptr": obj_ptr, "lay_ptr": lay_ptr,
            "grid_row": grid_row, "grid_col": grid_col,
        })

    # Detect hammer bros: duplicate (obj, lay) pairs among levels
    pair_counts = defaultdict(int)
    for e in entries:
        if e["type"] == "level":
            pair_counts[(e["obj_ptr"], e["lay_ptr"])] += 1
    for e in entries:
        if e["type"] == "level" and pair_counts[(e["obj_ptr"], e["lay_ptr"])] > 1:
            e["type"] = "hammer_bro"

    # Second pass: levels sharing obj_ptr with known hammer bros are also hammer bros
    hammer_objs = {e["obj_ptr"] for e in entries if e["type"] == "hammer_bro"}
    for e in entries:
        if e["type"] == "level" and e["obj_ptr"] in hammer_objs:
            e["type"] = "hammer_bro"

    lookup = {}
    for e in entries:
        lookup[(e["grid_row"], e["grid_col"])] = e

    return entries, lookup


# Short type labels for the numbered map legend
TYPE_LABEL = {
    "level": "level", "fortress": "fort", "airship": "airship",
    "toad_house": "toad", "bonus_game": "spade", "bowser": "bowser",
    "hammer_bro": "hammer", "special": "special", "pipe": "pipe",
    "transition": "trans", "start": "start", "unknown": "???",
}

# Single-char symbols for grid rendering when no BFS number
TYPE_CHAR = {
    "level": "L", "fortress": "F", "airship": "A", "toad_house": "T",
    "bonus_game": "$", "bowser": "W", "hammer_bro": "H", "special": "!",
    "pipe": "P", "transition": "^", "start": "S", "unknown": "?",
}

# ANSI color per type
TYPE_COLOR = {
    "level": WHITE, "fortress": RED, "airship": YELLOW, "toad_house": CYAN,
    "bonus_game": CYAN, "bowser": RED, "hammer_bro": MAGENTA,
    "special": DIM, "pipe": MAGENTA, "transition": BLUE, "start": GREEN,
    "unknown": DIM,
}


def render_numbered_map(rom, world_idx, pipe_pairs):
    """Render a world map with BFS-ordered numbers at each node.

    Uses fortress progression to open locks, so nodes behind locks get
    numbered after the fortress is beaten. Returns a string.
    """
    steps = simulate_progression(rom, world_idx, pipe_pairs)
    _, entry_lookup = build_entry_lookup(rom, world_idx)

    # Merge all BFS orders across progression steps.
    # Nodes from later steps (after locks open) continue the numbering.
    seen = set()
    ordered_nodes = []
    for step in steps:
        for pos in step["bfs_order"]:
            if pos not in seen:
                seen.add(pos)
                ordered_nodes.append(pos)

    # Assign BFS numbers only to nodes that have a pointer table entry
    node_number = {}  # pos -> (bfs_num, entry)
    num = 1
    for pos in ordered_nodes:
        if pos in entry_lookup:
            node_number[pos] = (num, entry_lookup[pos])
            num += 1

    # Get final grid state (all locks opened)
    final_grid = steps[-1]["grid"]

    # Derive level names from tiles, with fortress numbering
    grid_for_tiles = read_tile_grid(rom, world_idx)
    # Count forts that will get default "NF" name (not overridden)
    total_forts = sum(1 for pos in ordered_nodes
                      if pos in node_number
                      and node_number[pos][1]["type"] == "fortress"
                      and (world_idx, node_number[pos][1]["index"])
                          not in LEVEL_NAME_OVERRIDES)
    entry_names = {}  # pos -> name string
    fort_count = 0
    for pos in ordered_nodes:
        if pos not in node_number:
            continue
        _, entry = node_number[pos]
        r, c = pos
        tile = grid_for_tiles[r][c] if 0 <= r < len(grid_for_tiles) and 0 <= c < len(grid_for_tiles[0]) else 0
        name = derive_level_name(world_idx, entry["index"], entry["type"], tile)
        # Number fortresses: NF if single, NF1/NF2/... if multiple
        if name and name.endswith("F") and len(name) <= 2:
            fort_count += 1
            if total_forts > 1:
                name = f"{name}{fort_count}"
        entry_names[pos] = name
    final_nodes = steps[-1]["nodes"]
    final_paths = steps[-1]["path_tiles"]
    cols = len(final_grid[0])

    # Collect pipe positions
    pipe_pos = set()
    for a, b in pipe_pairs:
        pipe_pos.add(a)
        pipe_pos.add(b)

    lines = []
    w_name = MAP_TILE_GRIDS[world_idx]["name"]
    lines.append(f"\n{WHITE}=== {w_name} ==={RESET}")

    # Column ruler
    ruler = "      "
    for c in range(cols):
        ruler += f"{c % 10:<3d}" if c % 10 == 0 else "   " if c % 5 == 0 else "  ."
    # Simpler: just print col numbers every cell, 3-wide
    ruler = "      "
    for c in range(cols):
        if c % 16 == 0:
            ruler += f"{GREEN}|{RESET}"
        else:
            ruler += " "
    lines.append(ruler)

    # Grid rows (3 chars per cell: number or symbol)
    for r in range(MAP_TILE_GRID_ROWS):
        row_str = f"  {r}: "
        for c in range(cols):
            pos = (r, c)
            tile = final_grid[r][c]

            if c % 16 == 0 and c > 0:
                row_str += f"{DIM}|{RESET}"

            if pos in node_number:
                bfs_n, entry = node_number[pos]
                color = TYPE_COLOR.get(entry["type"], WHITE)
                row_str += f"{color}{bfs_n:>2d}{RESET}"
            elif pos in final_paths:
                if tile in VALID_HORZ:
                    row_str += f"{DIM}--{RESET}"
                else:
                    row_str += f"{DIM} |{RESET}"
            elif pos in final_nodes:
                # Node without a pointer table entry (decoration, dead end)
                row_str += f"{DIM} *{RESET}"
            elif tile in BACKGROUND_TILES:
                row_str += f"{DIM} .{RESET}"
            else:
                row_str += f"{DIM} ~{RESET}"
        lines.append(row_str)

    # Legend
    lines.append("")
    lines.append(f"  {'#':>3s}  {'name':>5s}  {'pos':>7s}  {'idx':>3s}  {'type':>8s}  {'ts':>2s}  {'obj':>6s}  {'lay':>6s}")
    lines.append(f"  {'---':>3s}  {'-----':>5s}  {'-------':>7s}  {'---':>3s}  {'--------':>8s}  {'--':>2s}  {'------':>6s}  {'------':>6s}")

    for pos in ordered_nodes:
        if pos not in node_number:
            continue
        bfs_n, entry = node_number[pos]
        r, c = pos
        color = TYPE_COLOR.get(entry["type"], WHITE)
        label = TYPE_LABEL.get(entry["type"], entry["type"])
        name = entry_names.get(pos, "")
        lines.append(
            f"  {color}{bfs_n:>3d}{RESET}  {name:>5s}  ({r},{c:>2d})  {entry['index']:>3d}  "
            f"{color}{label:>8s}{RESET}  {entry['tileset']:>2d}  "
            f"0x{entry['obj_ptr']:04X}  0x{entry['lay_ptr']:04X}"
        )

    # Also list any entries NOT reached by BFS
    unreached = []
    for e in entry_lookup.values():
        pos = (e["grid_row"], e["grid_col"])
        if pos not in node_number:
            unreached.append(e)
    if unreached:
        lines.append(f"\n  {YELLOW}Entries not reached by BFS:{RESET}")
        for e in unreached:
            label = TYPE_LABEL.get(e["type"], e["type"])
            lines.append(
                f"  {'':>3s}  ({e['grid_row']},{e['grid_col']:>2d})  {e['index']:>3d}  "
                f"{label:>8s}  {e['tileset']:>2d}  "
                f"0x{e['obj_ptr']:04X}  0x{e['lay_ptr']:04X}"
            )

    return "\n".join(lines)


def render_walk_map(rom, world_idx, pipe_pairs, show_progression=False):
    """Render a colored BFS walk of a world map. Returns a string."""
    fort_positions = read_fortress_positions(rom)
    world_forts = fort_positions.get(world_idx, [])

    if show_progression:
        steps = simulate_progression(rom, world_idx, pipe_pairs)
        parts = []
        opened = set()
        for step_num, step in enumerate(steps):
            if step["fort_idx"] is None:
                header = f"  Step {step_num}: Initial state"
            else:
                fr, fc = step["fort_pos"]
                header = f"  Step {step_num}: Beat fortress #{step['fort_idx']+1} at ({fr},{fc})"
                if step["fx_pos"]:
                    fxr, fxc = step["fx_pos"]
                    header += f" -> opened ({fxr},{fxc}) [0x{step['fx_old_tile']:02X} -> 0x{step['fx_new_tile']:02X}]"
                    opened.add(step["fx_pos"])

            grid = step["grid"]
            nodes = step["nodes"]
            _, edges, path_tiles, _ = walk_map(grid, pipe_pairs)
            chokes = find_chokepoints(nodes, edges)

            part = _render_walk_grid(grid, nodes, path_tiles, chokes,
                                     pipe_pairs, world_forts, opened)
            parts.append(f"{header}\n  Reachable: {len(nodes)} nodes\n{part}")
        return "\n\n".join(parts)
    else:
        grid = read_tile_grid(rom, world_idx)
        nodes, edges, path_tiles, _ = walk_map(grid, pipe_pairs)
        chokes = find_chokepoints(nodes, edges)
        header = (f"  Start: {find_start(grid)}  "
                  f"Pipes: {len(pipe_pairs)}  "
                  f"Forts: {len(world_forts)}  "
                  f"Reachable: {len(nodes)}  "
                  f"Chokes: {len(chokes)}")
        return header + "\n" + _render_walk_grid(
            grid, nodes, path_tiles, chokes, pipe_pairs, world_forts, set())


def _render_walk_grid(grid, nodes, path_tiles, chokepoints,
                      pipe_pairs, fortress_positions, opened_positions):
    """Internal: render a colored walk grid."""
    cols = len(grid[0])
    fort_set = set(fortress_positions) if fortress_positions else set()
    pipe_pos = set()
    for a, b in pipe_pairs:
        pipe_pos.add(a)
        pipe_pos.add(b)

    lines = []
    for r in range(MAP_TILE_GRID_ROWS):
        row_str = f"  {r}: "
        for c in range(cols):
            tile = grid[r][c]
            pos = (r, c)
            if pos in opened_positions:
                row_str += f"{BLUE}O{RESET}"
            elif pos in fort_set and pos in nodes:
                row_str += f"{WHITE}F{RESET}"
            elif pos in fort_set:
                row_str += f"{YELLOW}F{RESET}"
            elif pos in chokepoints:
                row_str += f"{RED}X{RESET}"
            elif pos in pipe_pos and pos in nodes:
                row_str += f"{MAGENTA}P{RESET}"
            elif pos in nodes:
                row_str += f"{GREEN}*{RESET}"
            elif pos in path_tiles:
                ch = "-" if tile in VALID_HORZ else "|"
                row_str += f"{CYAN}{ch}{RESET}"
            elif tile in BACKGROUND_TILES:
                row_str += f"{DIM}.{RESET}"
            else:
                row_str += f"{YELLOW}~{RESET}"
        lines.append(row_str)

    lines.append(f"  {GREEN}*{RESET}=node  {RED}X{RESET}=choke  "
                 f"{CYAN}-|{RESET}=path  {MAGENTA}P{RESET}=pipe  "
                 f"{WHITE}F{RESET}=fort  {BLUE}O{RESET}=opened  "
                 f"{YELLOW}~{RESET}=blocked  {DIM}.{RESET}=void")
    return "\n".join(lines)


# --------------------------------------------------------------------------
# Main
# --------------------------------------------------------------------------

def main():
    rom_path = "Super Mario Bros. 3 (USA) (Rev 1).nes"
    output_path = "tools/rom_map.json"
    mode = "json"  # default: generate JSON
    world_filter = None

    args = sys.argv[1:]
    i = 0
    while i < len(args):
        if args[i] == "--json" and i + 1 < len(args):
            output_path = args[i + 1]
            i += 2
        elif args[i] == "--walk":
            mode = "walk"
            i += 1
        elif args[i] == "--progression":
            mode = "progression"
            i += 1
        elif args[i] == "--numbered":
            mode = "numbered"
            i += 1
        elif args[i] == "--viz":
            mode = "viz"
            i += 1
        elif args[i] == "--world" and i + 1 < len(args):
            world_filter = int(args[i + 1]) - 1  # 1-indexed input
            i += 2
        elif args[i] in ("--help", "-h"):
            print(__doc__)
            sys.exit(0)
        elif not args[i].startswith("-"):
            rom_path = args[i]
            i += 1
        else:
            print(f"Unknown option: {args[i]}")
            sys.exit(1)

    if not os.path.exists(rom_path):
        print(f"Error: ROM file not found: {rom_path}")
        print(__doc__)
        sys.exit(1)

    with open(rom_path, "rb") as f:
        rom = f.read()

    if len(rom) != ROM_SIZE:
        print(f"Warning: ROM size {len(rom)} != expected {ROM_SIZE}")

    worlds = [world_filter] if world_filter is not None else list(range(8))

    if mode == "json":
        print(f"Reading ROM: {rom_path} ({len(rom)} bytes)")
        rom_map = generate_rom_map(rom)
        print_summary(rom_map)
        with open(output_path, "w") as f:
            json.dump(rom_map, f, indent=2)
        print(f"ROM map written to: {output_path}")
        file_size = os.path.getsize(output_path)
        if file_size > 1024 * 1024:
            print(f"  Size: {file_size / 1024 / 1024:.1f} MB")
        else:
            print(f"  Size: {file_size / 1024:.1f} KB")

    elif mode in ("walk", "progression"):
        pipes_by_world = read_pipe_pairs(rom)
        for wi in worlds:
            info = MAP_TILE_GRIDS[wi]
            print(f"=== {info['name']} ===")
            output = render_walk_map(rom, wi, pipes_by_world[wi],
                                     show_progression=(mode == "progression"))
            print(output)
            print()

    elif mode == "numbered":
        pipes_by_world = read_pipe_pairs(rom)
        for wi in worlds:
            output = render_numbered_map(rom, wi, pipes_by_world[wi])
            print(output)
            print()


if __name__ == "__main__":
    main()
