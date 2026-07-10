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
  python3 tools/rom_map.py [rom] --check            # check for uncovered nodes
  python3 tools/rom_map.py [rom] --level 3-2        # look up a level by name
  python3 tools/rom_map.py [rom] --level 7F1        # fortress lookup
  python3 tools/rom_map.py [rom] --level 8B         # Bowser Castle
  python3 tools/rom_map.py [rom] --level β1         # beta (unreferenced) stage
  python3 tools/rom_map.py [rom] --level beta-1     # ASCII alias for β1
  python3 tools/rom_map.py [rom] --tile 0xE6        # look up a world-map tile byte
  python3 tools/rom_map.py [rom] --tile 45          # CHR pattern, behavior, palette
  python3 tools/rom_map.py [rom] --check-dispatches # validate 4-byte dispatch tables
  python3 tools/rom_map.py [rom] --antechamber      # antechamber-pattern shuffle pool

Default ROM: "roms/Super Mario Bros. 3 (USA) (Rev 1).nes"
"""

import json
import os
import re
import sys
from collections import defaultdict, deque

# --------------------------------------------------------------------------
# Single source of truth: parse classification sets from rom_data.rs so the
# Python tooling cannot drift from the Rust randomizer. The randomizer's lists
# are the authoritative source — when a fortress/airship moves or is added,
# only rom_data.rs needs to change.
# --------------------------------------------------------------------------

# rom_data was split from a single rom_data.rs into a rom_data/ submodule
# directory (access.rs, free_space.rs, grid.rs, tables.rs, mod.rs). Support
# both layouts: prefer the directory, fall back to the legacy single file.
_RANDOMIZE_DIR = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    "..", "src", "randomize",
)
_ROM_DATA_DIR = os.path.join(_RANDOMIZE_DIR, "rom_data")
_ROM_DATA_RS = os.path.join(_RANDOMIZE_DIR, "rom_data.rs")


def _parse_tuple_list(rs_src, name):
    """Parse `pub(super) const NAME: &[(usize, usize)] = &[ (a, b), ... ];`."""
    m = re.search(
        rf"const\s+{name}\s*:\s*&\[\(usize,\s*usize\)\]\s*=\s*&\[(.*?)\];",
        rs_src,
        re.DOTALL,
    )
    if not m:
        raise RuntimeError(f"Could not parse {name} from rom_data.rs")
    return {(int(a), int(b)) for a, b in re.findall(r"\((\d+)\s*,\s*(\d+)\)", m.group(1))}


def _parse_tuple(rs_src, name):
    """Parse `pub(super) const NAME: (usize, usize) = (a, b);`."""
    m = re.search(
        rf"const\s+{name}\s*:\s*\(usize,\s*usize\)\s*=\s*\((\d+)\s*,\s*(\d+)\);",
        rs_src,
    )
    if not m:
        raise RuntimeError(f"Could not parse {name} from rom_data.rs")
    return (int(m.group(1)), int(m.group(2)))


def _parse_beta_levels(rs_src):
    """Parse `BETA_LEVELS: &[BetaLevel] = &[ BetaLevel { ... }, ... ];`.

    Returns list of dicts with keys: tileset, obj_lo, obj_hi, lay_lo, lay_hi, name.
    Field order matches the struct definition in rom_data.rs. `name` decodes
    Rust's `\\u{HHHH}` escapes (e.g. β = U+03B2).
    """
    m = re.search(
        r"const\s+BETA_LEVELS\s*:\s*&\[BetaLevel\]\s*=\s*&\[(.*?)\];",
        rs_src,
        re.DOTALL,
    )
    if not m:
        raise RuntimeError("Could not parse BETA_LEVELS from rom_data.rs")
    pattern = re.compile(
        r"BetaLevel\s*\{\s*"
        r"tileset:\s*(\d+)\s*,\s*"
        r"obj_lo:\s*(0x[0-9A-Fa-f]+|\d+)\s*,\s*"
        r"obj_hi:\s*(0x[0-9A-Fa-f]+|\d+)\s*,\s*"
        r"lay_lo:\s*(0x[0-9A-Fa-f]+|\d+)\s*,\s*"
        r"lay_hi:\s*(0x[0-9A-Fa-f]+|\d+)\s*,\s*"
        r'name:\s*"([^"]+)"\s*,?\s*'
        r"\}",
        re.DOTALL,
    )
    entries = []
    for ts, ol, oh, ll, lh, name_raw in pattern.findall(m.group(1)):
        name = re.sub(
            r"\\u\{([0-9A-Fa-f]+)\}",
            lambda x: chr(int(x.group(1), 16)),
            name_raw,
        )
        entries.append({
            "tileset": int(ts, 0),
            "obj_lo": int(ol, 0),
            "obj_hi": int(oh, 0),
            "lay_lo": int(ll, 0),
            "lay_hi": int(lh, 0),
            "name": name,
        })
    return entries


def _parse_beta_patches(rs_src):
    """Parse `BETA_PATCHES: &[(usize, u8)] = &[ (off, byte), ... ];` (hex literals OK).

    Skips Rust line-comments so commented-out patch tuples are not picked up.
    """
    m = re.search(
        r"const\s+BETA_PATCHES\s*:.*?=\s*&\[(.*?)\];",
        rs_src,
        re.DOTALL,
    )
    if not m:
        raise RuntimeError("Could not parse BETA_PATCHES from rom_data.rs")
    body = re.sub(r"//[^\n]*", "", m.group(1))
    return [
        (int(a, 0), int(b, 0))
        for a, b in re.findall(
            r"\(\s*(0x[0-9A-Fa-f]+|\d+)\s*,\s*(0x[0-9A-Fa-f]+|\d+)\s*\)",
            body,
        )
    ]


def _read_rom_data_src():
    """Read rom_data source, concatenating all .rs files if it's a directory."""
    if os.path.isdir(_ROM_DATA_DIR):
        parts = []
        for fname in sorted(os.listdir(_ROM_DATA_DIR)):
            if fname.endswith(".rs"):
                with open(os.path.join(_ROM_DATA_DIR, fname)) as f:
                    parts.append(f.read())
        return "\n".join(parts)
    with open(_ROM_DATA_RS) as f:
        return f.read()


_RS_SRC = _read_rom_data_src()

# --------------------------------------------------------------------------
# Constants
# --------------------------------------------------------------------------

ROM_SIZE = 393232

# PRG bank layout
PRG_BANK_SIZE = 0x2000  # 8 KB
PRG_OFFSET = 0x10       # after 16-byte iNES header

# Level data regions by tileset (file offset ranges + extra-byte dispatch info)
# From powerups.rs / rom_data.rs::LEVEL_DATA_REGIONS.
# `randomize_note_wood` mirrors the Rust struct field — in TS2 / TS9 the same
# group-2 byte2 shapes map to bridge / desert decoration tiles instead of
# note/wood powerups, so they must not be flagged or shuffled.
LEVEL_DATA_REGIONS = [
    {
        "name": "Underground (TS14)",
        "tileset_ids": [14],
        "start": 0x1A587,
        "end": 0x1C005,
        "extra_byte_dispatches": {35, 36, 37, 38, 39, 40, 41, 42, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71},
        "randomize_note_wood": True,
    },
    {
        "name": "Plains (TS1)",
        "tileset_ids": [1],
        "start": 0x1E512,
        "end": 0x20005,
        "extra_byte_dispatches": {11, 12, 35, 36, 37, 38, 39, 40, 41, 42},
        "randomize_note_wood": True,
    },
    {
        "name": "Hilly (TS3)",
        "tileset_ids": [3],
        "start": 0x20587,
        "end": 0x22005,
        "extra_byte_dispatches": {35, 36, 37, 38, 39, 40, 41, 42, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71},
        "randomize_note_wood": True,
    },
    {
        "name": "Ice/Sky (TS4/12)",
        "tileset_ids": [4, 12],
        "start": 0x227E0,
        "end": 0x24005,
        "extra_byte_dispatches": {0, 35, 36, 37, 38, 39, 40, 41, 42, 54, 60, 112},  # +54 Muncher17
        "randomize_note_wood": True,
    },
    {
        # TS6/TS7/TS8 all map to PRG018 (PAGE_A000_ByTileset), sharing this
        # bank's generator set — the W7 pipe-maze interiors and 5-2's shaft
        # are referenced via ts6/ts8 alt pointers into this same region.
        "name": "Pipe/Water (TS6/7/8)",
        "tileset_ids": [7, 6, 8],
        "start": 0x24BA7,
        "end": 0x26005,
        "extra_byte_dispatches": {35, 36, 37, 38, 39, 40, 41, 42, 49, 57},  # +49 OrangeBlock
        "randomize_note_wood": True,
    },
    {
        "name": "Cloudy/Giant/Plant (TS5/11/13)",
        "tileset_ids": [5, 11, 13],
        "start": 0x26A6F,
        # 0x2800A, NOT 0x28C05: bank ends at 0x28010 and the next bank opens
        # with the desert metatile quadrant table (see rom_data.rs).
        "end": 0x2800A,
        "extra_byte_dispatches": {13, 35, 36, 37, 38, 39, 40, 41, 42, 45, 46, 48, 51},
        "randomize_note_wood": True,
    },
    {
        "name": "Desert (TS9)",
        "tileset_ids": [9],
        "start": 0x28F36,
        "end": 0x2A005,
        "extra_byte_dispatches": {10, 11, 12, 13, 35, 36, 37, 38, 39, 40, 41, 42},
        "randomize_note_wood": False,  # shapes 1-5 = palms/cacti in TS9
    },
    {
        "name": "Dungeon (TS2)",
        "tileset_ids": [2],
        "start": 0x2A7F7,
        "end": 0x2C005,
        "extra_byte_dispatches": {13, 14, 35, 36, 37, 38, 39, 40, 41, 42, 46, 47, 48, 57, 95, 96},  # +13 SolidBrick +14 BrightDiamondLong +57 BrightDiamond +95,96 Group6
        "randomize_note_wood": False,  # shapes 1-2 = CCBridge, 3-7 = TopDecoBlocks in TS2
    },
    {
        "name": "Ship (TS10)",
        "tileset_ids": [10],
        "start": 0x2EC07,
        "end": 0x30005,
        "extra_byte_dispatches": {1, 2, 35, 36, 37, 38, 39, 40, 41, 42, 48, 51},  # 49 (Crate) is 3-byte, NOT 4-byte
        "randomize_note_wood": True,
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
    "ground": [0x2B, 0x29, 0x2A, 0x33, 0x39, 0x3F, 0x40, 0x55, 0x6B, 0x71, 0x72],
    "shell": [0x6C, 0x6D, 0x70],
    "big": [0x7A, 0x7B, 0x7C, 0x7D, 0x7E, 0x7F],
    "flying": [0x6E, 0x6F, 0x73, 0x74, 0x80],
    "water": [0x61, 0x62, 0x63, 0x64, 0x6A],
    "bro": [0x81, 0x82, 0x86, 0x87],
    "piranha": [0xA0, 0xA2, 0xA4, 0xA6],
    "piranha_ceil": [0xA1, 0xA3, 0xA5, 0xA7],
    "cheep": [0x77, 0x88],
    "thwomp": [0x8A, 0x8B, 0x8C, 0x8D, 0x8E, 0x8F],
    "ghost": [0x2F, 0x30, 0x45],
    "cannon": [0xBC, 0xBD, 0xBE, 0xBF, 0xC0, 0xC1, 0xC2, 0xC3,
               0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xCB,
               0xCC, 0xCD, 0xCE, 0xCF, 0xD0],
    "big_q_block": [0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A],
}

ENEMY_NAMES = {
    0x2B: "GoombaShoе", 0x29: "Spike", 0x2A: "Patooie", 0x33: "Nipper", 0x39: "NipperHopping",
    0x3F: "DryBones", 0x40: "BusterBeatle", 0x55: "BobOmb", 0x6B: "PileDriver",
    0x71: "Spiny", 0x72: "Goomba",
    0x6C: "GreenTroopa", 0x6D: "RedTroopa", 0x70: "BuzzyBeatle",
    0x7A: "BigGreenTroopa", 0x7B: "BigRedTroopa", 0x7C: "BigGoomba",
    0x7D: "BigGreenPiranha", 0x7E: "BigGreenHopper", 0x7F: "BigRedPiranha",
    0x6E: "ParatroopaGreenHop", 0x6F: "FlyingRedParatroopa", 0x73: "ParaGoomba",
    0x74: "ParaGoombaMicros", 0x80: "FlyingGreenParatroopa",
    0x61: "BlooperWithKids", 0x62: "Blooper", 0x63: "BigBertha", 0x64: "CheepHopper",
    0x6A: "BlooperChildShoot",
    0x81: "HammerBro", 0x82: "BoomerangBro", 0x86: "HeavyBro", 0x87: "FireBro",
    0xA0: "GreenPiranha", 0xA1: "GreenPiranhaFlipped", 0xA2: "RedPiranha",
    0xA3: "RedPiranhaFlipped", 0xA4: "GreenPiranhaFire", 0xA5: "GreenPiranhaFireC",
    0xA6: "VenusFireTrap", 0xA7: "VenusFireTrapCeil",
    0x77: "GreenCheep", 0x88: "OrangeCheep",
    0x8A: "Thwomp", 0x8B: "ThwompLeftSlide", 0x8C: "ThwompRightSlide",
    0x8D: "ThwompUpDown", 0x8E: "ThwompDiagonalUL", 0x8F: "ThwompDiagonalDL",
    0x2F: "Boo", 0x30: "HotFootShy", 0x45: "HotFoot",
    0xBC: "CannonFire_BC", 0xBD: "CannonFire_BD", 0xBE: "CannonFire_BE",
    0xBF: "CannonFire_BF", 0xC0: "CannonFire_C0", 0xC1: "CannonFire_C1",
    0xC2: "CannonFire_C2", 0xC3: "CannonFire_C3", 0xC4: "CannonFire_C4",
    0xC5: "CannonFire_C5", 0xC6: "CannonFire_C6", 0xC7: "CannonFire_C7",
    0xC8: "CannonFire_C8", 0xC9: "CannonFire_C9", 0xCA: "CannonFire_CA",
    0xCB: "CannonFire_CB", 0xCC: "CannonFire_CC", 0xCD: "CannonFire_CD",
    0xCE: "CannonFire_CE", 0xCF: "CannonFire_CF", 0xD0: "CannonFire_D0",
    0x94: "BigQ_3Up", 0x95: "BigQ_Mushroom", 0x96: "BigQ_FireFlower",
    0x97: "BigQ_SuperLeaf", 0x98: "BigQ_Tanooki", 0x99: "BigQ_Frog", 0x9A: "BigQ_Hammer",
    # Roto-Disc family (fortress fire wheels) — see docs/smb3_rom_reference.md
    0x51: "RotodiscDualCW", 0x5A: "RotodiscCW", 0x5B: "RotodiscCCW",
    0x5E: "RotodiscDualOppH", 0x5F: "RotodiscDualOppV", 0x60: "RotodiscDualCCW",
    # Fortress / hazard sprites
    0x4F: "ChainChompFree", 0x53: "PodobooCeiling", 0x58: "FireChomp",
    0x59: "FireSnake", 0x5D: "Tornado", 0x67: "LavaLotus", 0x89: "ChainChomp",
    0x9E: "Podoboo", 0xAF: "AngrySun", 0xAD: "RockyWrench",
    0x31: "StretchBoo", 0x32: "StretchBooFlip",
    0x46: "PiranhaSpikeBall", 0x56: "PiranhaSidewaysL", 0x57: "PiranhaSidewaysR",
    # Cheeps / water hazards
    0x17: "SpinyCheep", 0x2D: "BigBerthaEater", 0x3B: "ChargingCheep",
    0x3D: "NipperFireBreather", 0x42: "CheepPoolHop3", 0x43: "CheepPoolHop2",
    0x48: "TinyCheep", 0x76: "JumpingCheep",
    # Misc enemies / projectiles
    0x68: "TwirlingBuzzy", 0x69: "TwirlingSpiny", 0x78: "BulletBill",
    0x79: "BulletBillHoming", 0x83: "Lakitu", 0x9F: "Parabeetle",
    0x50: "BobOmbExplode", 0x75: "BossStatueFire", 0x84: "SpinyEgg", 0x85: "SpinyEggDud",
}

# Protected offsets
PROTECTED_POWERUP_OFFSETS = [0x23DB0, 0x23E1F, 0x23EA0]  # 7-7 Q-stars
PROTECTED_ENEMY_OFFSET = 0x0C9B7  # W7 Big Q room Tanooki (required for 7-F1)

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

# Classification entries — parsed from rom_data.rs. Update Rust, not here.
FORTRESS_ENTRIES = _parse_tuple_list(_RS_SRC, "FORTRESS_ENTRIES")
AIRSHIP_ENTRIES_SET = _parse_tuple_list(_RS_SRC, "AIRSHIP_ENTRIES")
BOWSER_ENTRY_PAIR = _parse_tuple(_RS_SRC, "BOWSER_ENTRY")

# Beta (unreferenced) levels — parsed from rom_data.rs::BETA_LEVELS.
# These have no pointer-table entry; the randomizer injects them when
# `include_beta_stages` is enabled. obj_ptr is borrowed from a vanilla level.
BETA_LEVELS = _parse_beta_levels(_RS_SRC)
BETA_PATCHES = _parse_beta_patches(_RS_SRC)

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

    # Numbered level tiles: 0x03 = level 1, ..., 0x0B = level 9
    if 0x03 <= tile <= 0x0B:
        return f"{w}-{tile - 2}"

    # Double-digit level tiles: 0x0C = level 10, ..., 0x14 = level 18
    if 0x0C <= tile <= 0x14:
        return f"{w}-{tile - 2}"

    # Repurposed tiles for levels 19-20: 0x68 = 19, 0x69 = 20
    if tile == 0x68:
        return f"{w}-19"
    if tile == 0x69:
        return f"{w}-20"

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

            # Group 1 (0x20): byte2 0..15 encode Q-block / brick variants (and
            # munchers / invis blocks) — see POWER_NAMES.
            # Group 2 (0x40): byte2 1..3 = note blocks (flower/leaf/star),
            # byte2 4..6 = wood blocks (flower/leaf/star). These are powerups
            # *only* in regions where `randomize_note_wood` is true (in TS2/TS9
            # the same shapes are bridges / desert decorations).
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
                elif byte2 in (0x06, 0x07, 0x08, 0x0B):
                    cmd["randomize_class"] = "brick"
            elif (group == 2 and 1 <= byte2 <= 6
                  and region.get("randomize_note_wood", False)
                  and 16 <= fixed_idx < 16 + len(LL_POWER_BLOCKS)):
                power_idx = fixed_idx - 16
                cmd["powerup"] = True
                cmd["tile_id"] = LL_POWER_BLOCKS[power_idx]
                cmd["byte2_offset"] = i + 2
                cmd["protected"] = (i + 2) in PROTECTED_POWERUP_OFFSETS
                if 1 <= byte2 <= 3:
                    cmd["randomize_class"] = "note"
                    cmd["power_name"] = ["NOTEFLOWER", "NOTELEAF", "NOTESTAR"][byte2 - 1]
                else:
                    cmd["randomize_class"] = "wood"
                    cmd["power_name"] = ["WOODFLOWER", "WOODLEAF", "WOODSTAR"][byte2 - 4]
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
        "alt_layout": header[0] | (header[1] << 8),
        "alt_objects": header[2] | (header[3] << 8),
        "alt_tileset": header[6] & 0x0F,
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
            "alt_layout": header["alt_layout"],
            "alt_objects": header["alt_objects"],
            "alt_tileset": header["alt_tileset"],
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

def build_layout_index(all_region_levels):
    """Build a lookup (tileset, layout_cpu) → level_dict across all regions.

    Multiple tilesets can share a bank (e.g., TS4/TS12 both in Ice/Sky),
    so we key by (tileset, cpu_addr) for each tileset the region covers."""
    index = {}
    for region_data in all_region_levels:
        for lv in region_data["levels"]:
            cpu = lv["layout_cpu"]
            if cpu is None:
                continue
            for ts_id in region_data["tileset_ids"]:
                index[(ts_id, cpu)] = lv
    return index


def trace_sub_areas(entry_level, layout_index):
    """Follow the header chain from an entry point to find all sub-areas.

    Each 9-byte layout header contains alt_layout/alt_tileset pointing to
    the level's alternate (pipe/door destination) area. Follow the chain as
    long as the alt pointer resolves to a real, distinct level in the layout
    index — the resolution check itself rejects dead/garbage pointers, so no
    junction_count gate is needed (8F has a live Podoboo sub-area despite zero
    junction commands). A visited set by header_offset prevents infinite loops
    (e.g., Pyramid has a two-way loop)."""
    result = [entry_level]
    visited = {entry_level["header_offset"]}
    current = entry_level

    while True:
        alt_layout = current["alt_layout"]
        alt_tileset = current["alt_tileset"]

        if alt_layout == 0 or alt_layout < 0xA000:
            break

        found = layout_index.get((alt_tileset, alt_layout))
        if found is None or found["header_offset"] in visited:
            break

        visited.add(found["header_offset"])
        result.append(found)
        current = found

    return result


def build_level_groups(rom, all_region_levels, worlds_data):
    """For each pointer table entry, trace sub-areas via the header chain
    (alt_layout/alt_tileset pointers) rather than contiguous segments.

    Returns a list of level groups, each containing:
      - entry_layout_cpu: CPU address of the entry-point level
      - entry_obj_ptr: enemy data pointer from the entry header
      - world_refs: list of (world, index) pointer table entries
      - sub_areas: list of sub-area info dicts (entry + chain)
      - has_boomboom/has_koopaling/has_bowser: aggregate boss flags
    """
    layout_index = build_layout_index(all_region_levels)

    # Collect all pointer table entries and group by entry-point header_offset
    # entry_header_offset -> [(world, index, lay_ptr, obj_ptr)]
    entry_groups = defaultdict(list)

    for wd in worlds_data:
        for entry in wd["entries"]:
            lay = entry["lay_ptr"]
            if lay == 0 or entry["type"] not in ("level", "fortress", "airship", "bowser", "pipe", "hammer_bro"):
                continue
            tileset = entry["tileset"]
            lv = layout_index.get((tileset, lay))
            if lv is None:
                continue
            entry_groups[lv["header_offset"]].append(
                (wd["world"], entry["index"], lay, entry["obj_ptr"]))

    groups = []

    for header_off, refs in entry_groups.items():
        # Find the entry-point level dict
        _, _, lay_ptr, _ = refs[0]
        tileset = None
        for wd in worlds_data:
            for entry in wd["entries"]:
                if entry["lay_ptr"] == lay_ptr:
                    tileset = entry["tileset"]
                    break
            if tileset is not None:
                break

        entry_level = layout_index.get((tileset, lay_ptr)) if tileset is not None else None
        if entry_level is None:
            continue

        # Trace sub-areas via header chain
        chain = trace_sub_areas(entry_level, layout_index)

        sub_areas = []
        for ci, lv in enumerate(chain):
            # enemy_ptr for display: sub-areas get their enemies from the
            # previous level's alt_objects; the entry level (ci==0) keeps
            # its own alt_objects (entry-point enemies come from the pointer
            # table obj_ptr and are displayed separately)
            if ci > 0:
                ep = chain[ci - 1]["alt_objects"]
            else:
                ep = lv["enemy_ptr"]
            # Boss flags for sub-areas should reflect the sub-area's OWN enemies
            if ci > 0:
                boss_info = scan_enemy_segment_bosses(rom, ep)
            else:
                boss_info = {"has_boomboom": lv["has_boomboom"],
                             "has_koopaling": lv["has_koopaling"],
                             "has_bowser": lv["has_bowser"]}
            sub_areas.append({
                "header_offset": lv["header_offset"],
                "layout_cpu": lv.get("layout_cpu"),
                "enemy_ptr": ep,
                "screens": lv["header"]["screens"],
                "command_count": lv["command_count"],
                "junction_count": lv["junction_count"],
                "has_boomboom": boss_info["has_boomboom"],
                "has_koopaling": boss_info["has_koopaling"],
                "has_bowser": boss_info["has_bowser"],
            })

        world_refs = [(w, idx) for w, idx, _, _ in refs]
        obj_ptrs = list(set(obj for _, _, _, obj in refs))
        entry_enemy = sub_areas[0]["enemy_ptr"] if sub_areas else 0

        # Boss flags: check BOTH layout header enemy ptrs AND pointer table obj_ptrs
        has_boomboom = any(sa["has_boomboom"] for sa in sub_areas)
        has_koopaling = any(sa["has_koopaling"] for sa in sub_areas)
        has_bowser = any(sa["has_bowser"] for sa in sub_areas)

        for obj_ptr in obj_ptrs:
            obj_boss = scan_enemy_segment_bosses(rom, obj_ptr)
            has_boomboom = has_boomboom or obj_boss["has_boomboom"]
            has_koopaling = has_koopaling or obj_boss["has_koopaling"]
            has_bowser = has_bowser or obj_boss["has_bowser"]

        groups.append({
            "region": chain[0]["region"],
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


def classify_entry(world_idx, entry_idx, obj_ptr, lay_ptr, tileset, rom=None, map_tile=None):
    """Classify a level pointer table entry by type.

    When map_tile is provided (from the actual tile grid), it takes priority
    for fortress/pipe/airship/bowser detection.  This is essential for shuffled
    ROMs where entries have been redistributed across worlds.
    """
    # Tile-based classification (works on shuffled ROMs where entries have
    # been redistributed across worlds — the tile moves with the entry).
    if map_tile is not None:
        if map_tile == 0xE5:
            return "start"
        if map_tile == 0x67:
            return "fortress"
        if map_tile == 0xC9:
            return "airship"
        if map_tile == 0xCC:
            return "bowser"
        if map_tile in (0xBC, 0x5F):
            return "pipe"

    # Entry-set fallback (always runs): catches vanilla cases where the map
    # tile is unique/unrecognized — e.g. W5 5F-2 (tile 0xEB) and W8 fortresses
    # on tiles 0xAF/0x47. These sets are parsed from rom_data.rs so they stay
    # in sync with the randomizer.
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

    grid = read_tile_grid(rom, world_idx)

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

        # Read the actual map tile at this entry's position
        map_tile = None
        if 0 <= grid_row < len(grid) and 0 <= grid_col < len(grid[0]):
            map_tile = grid[grid_row][grid_col]

        entry_type = classify_entry(world_idx, i, obj_ptr, lay_ptr, tileset, rom, map_tile)

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


def read_world_fx_assignments(rom, fort_positions=None):
    """Read per-world FX slot assignments. Returns dict: world_idx -> list of slot indices.

    Uses fort_positions (from read_fortress_positions) to determine how many
    FX slots each world uses — essential for shuffled ROMs where fortress
    counts per world change.
    """
    if fort_positions is None:
        fort_positions = read_fortress_positions(rom)
    assignments = {}
    for wi in range(8):
        fort_count = len(fort_positions.get(wi, []))
        base = FX_WORLD_TABLE + wi * 4
        assignments[wi] = [rom[base + i] for i in range(min(fort_count, 4))]
    return assignments


def read_fortress_positions(rom):
    """Grid positions of fortress tiles ($67). Scans the actual tile grid
    so this works on both vanilla and shuffled ROMs.

    Returns dict: world_idx -> list of (row, col), sorted in row-major order.
    """
    positions = {}
    for wi in range(8):
        grid = read_tile_grid(rom, wi)
        forts = []
        for r in range(len(grid)):
            for c in range(len(grid[0])):
                if grid[r][c] == 0x67:
                    forts.append((r, c))
        forts.sort()
        positions[wi] = forts
    return positions


def walk_map(grid, pipe_pairs, start_pos=None, traverse_rocks=False):
    """BFS walk using SMB3's 2-tile movement model.

    Returns (nodes, edges, path_tiles, bfs_order) where bfs_order is a list
    of (row, col) in the order they were first discovered.

    If traverse_rocks is True, rock tiles ($50, $51) are treated as valid
    horizontal path tiles (simulates using a hammer to clear them).
    """
    rows = len(grid)
    cols = len(grid[0])
    start = start_pos if start_pos is not None else find_start(grid)
    if start is None:
        return set(), {}, set(), []

    ROCK_TILES = {0x50, 0x51}
    if traverse_rocks:
        extra_horz = VALID_HORZ | ROCK_TILES
        extra_vert = VALID_VERT | ROCK_TILES
        directions = [
            (0, +1, extra_horz), (0, -1, extra_horz),
            (+1, 0, extra_vert), (-1, 0, extra_vert),
        ]
    else:
        directions = DIRECTIONS

    # W3 canoe dock teleport edges (always on, like pipes)
    CANOE_EDGES = [
        ((6, 20), (5, 24)),   # mainland dock -> island 1
        ((6, 20), (0, 32)),   # mainland dock -> island 2
    ]

    canoe_lookup = {}
    for a, b in CANOE_EDGES:
        canoe_lookup.setdefault(a, []).append(b)
        canoe_lookup.setdefault(b, []).append(a)

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

        for dr, dc, valid_set in directions:
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

        if (r, c) in canoe_lookup:
            for dest in canoe_lookup[(r, c)]:
                if dest not in nodes:
                    nodes.add(dest)
                    bfs_order.append(dest)
                    queue.append(dest)
                edges[(r, c)].append((dest, None, "canoe"))

    return nodes, edges, path_tiles, bfs_order


def simulate_progression(rom, world_idx, pipe_pairs, traverse_rocks=False):
    """Simulate fortress progression: walk, beat forts, open locks, re-walk.

    Applies FX slots sequentially (slot 0, 1, 2, ...) since the builder writes
    them in section order.  Each step fires the next FX slot when any reachable
    fortress hasn't been beaten yet.

    Returns list of steps. Each step has:
        fort_idx, fort_pos, fx_pos, fx_old_tile, fx_new_tile,
        nodes, path_tiles, bfs_order, grid (snapshot)
    """
    grid = read_tile_grid(rom, world_idx)
    fx_slots = read_fx_slots(rom)
    fort_positions = read_fortress_positions(rom)
    fx_assignments = read_world_fx_assignments(rom, fort_positions)

    world_fx = fx_assignments.get(world_idx, [])
    world_forts = fort_positions.get(world_idx, [])
    beaten = set()
    next_fx = 0  # sequential FX slot counter
    steps = []

    nodes, edges, path_tiles, bfs_order = walk_map(grid, pipe_pairs, traverse_rocks=traverse_rocks)
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

        # Apply the next FX slot in sequence (not indexed by fortress position).
        fx_pos = fx_old = fx_new = None
        if next_fx < len(world_fx):
            si = world_fx[next_fx]
            slot = fx_slots[si]
            fr, fc = slot["grid_row"], slot["grid_col"]
            fx_old = grid[fr][fc]
            fx_new = slot["replace_tile"]
            grid[fr][fc] = fx_new
            fx_pos = (fr, fc)
            next_fx += 1

        nodes, edges, path_tiles, bfs_order = walk_map(grid, pipe_pairs, traverse_rocks=traverse_rocks)
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

        # Read the actual map tile for tile-based classification
        map_tile = None
        if 0 <= grid_row < len(grid) and 0 <= grid_col < len(grid[0]):
            map_tile = grid[grid_row][grid_col]

        entry_type = classify_entry(world_idx, i, obj_ptr, lay_ptr, tileset, rom, map_tile)
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


def render_numbered_map(rom, world_idx, pipe_pairs, traverse_rocks=False):
    """Render a world map with BFS-ordered numbers at each node.

    Uses fortress progression to open locks, so nodes behind locks get
    numbered after the fortress is beaten. Returns a string.
    """
    steps = simulate_progression(rom, world_idx, pipe_pairs, traverse_rocks=traverse_rocks)
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


def render_walk_map(rom, world_idx, pipe_pairs, show_progression=False, traverse_rocks=False):
    """Render a colored BFS walk of a world map. Returns a string."""
    fort_positions = read_fortress_positions(rom)
    world_forts = fort_positions.get(world_idx, [])

    if show_progression:
        steps = simulate_progression(rom, world_idx, pipe_pairs, traverse_rocks=traverse_rocks)
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
            _, edges, path_tiles, _ = walk_map(grid, pipe_pairs, traverse_rocks=traverse_rocks)
            chokes = find_chokepoints(nodes, edges)

            part = _render_walk_grid(grid, nodes, path_tiles, chokes,
                                     pipe_pairs, world_forts, opened)
            parts.append(f"{header}\n  Reachable: {len(nodes)} nodes\n{part}")
        return "\n\n".join(parts)
    else:
        grid = read_tile_grid(rom, world_idx)
        nodes, edges, path_tiles, _ = walk_map(grid, pipe_pairs, traverse_rocks=traverse_rocks)
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
    rom_path = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes"
    output_path = "tools/rom_map.json"
    mode = "json"  # default: generate JSON
    world_filter = None
    level_query = None
    tile_query = None
    traverse_rocks = False

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
        elif args[i] == "--check":
            mode = "check"
            i += 1
        elif args[i] == "--level" and i + 1 < len(args):
            mode = "level"
            level_query = args[i + 1]
            i += 2
        elif args[i] == "--tile" and i + 1 < len(args):
            mode = "tile"
            tile_query = args[i + 1]
            i += 2
        elif args[i] == "--check-dispatches":
            mode = "check_dispatches"
            i += 1
        elif args[i] == "--antechamber":
            mode = "antechamber"
            i += 1
        elif args[i] == "--rocks":
            traverse_rocks = True
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
                                     show_progression=(mode == "progression"),
                                     traverse_rocks=traverse_rocks)
            print(output)
            print()

    elif mode == "numbered":
        pipes_by_world = read_pipe_pairs(rom)
        for wi in worlds:
            output = render_numbered_map(rom, wi, pipes_by_world[wi], traverse_rocks=traverse_rocks)
            print(output)
            print()

    elif mode == "check":
        pipes_by_world = read_pipe_pairs(rom)
        total_issues = 0
        for wi in worlds:
            uncovered = check_node_coverage(rom, wi, pipes_by_world[wi])
            info = MAP_TILE_GRIDS[wi]
            uncovered_set = {(r, c) for r, c, _ in uncovered}
            output = render_check_map(rom, wi, pipes_by_world[wi], uncovered_set)
            print(output)
            if uncovered:
                total_issues += len(uncovered)
            print()
        if total_issues:
            print(f"\033[1;31m{total_issues} total uncovered node(s)\033[0m")
        else:
            print(f"\033[1;32mAll nodes covered.\033[0m")

    elif mode == "level":
        print(render_level_lookup(rom, level_query))

    elif mode == "tile":
        print(render_tile_lookup(rom, tile_query))

    elif mode == "check_dispatches":
        check_dispatch_tables(rom)

    elif mode == "antechamber":
        print(render_antechamber_report(rom))


def check_dispatch_tables(rom):
    """Validate extra_byte_dispatches by parsing all levels and checking alignment.

    For each level region, parses every level and checks:
    1. Each level ends exactly at an 0xFF terminator
    2. Screen numbers are monotonically non-decreasing (within hi-bit pages)
    3. No variable-size commands produce dispatch values that seem suspicious

    Then probes: for each 3-byte variable command, what if it were 4-byte?
    Would downstream alignment improve? This detects missing dispatches.
    """
    total_levels = 0
    total_issues = 0
    total_suspects = 0

    for region in LEVEL_DATA_REGIONS:
        region_name = region["name"]
        dispatches = region["extra_byte_dispatches"]
        levels = scan_level_data_region(rom, region)
        issue_count = 0
        suspect_dispatches = {}  # dispatch -> count of times it would fix alignment

        for lv_idx, lv in enumerate(levels):
            total_levels += 1
            end_off = lv["end_offset"]
            header_off = lv["header_offset"]

            # Check 1: Does the level end at 0xFF?
            if end_off >= len(rom):
                print(f"  {RED}ISSUE{RESET}: Level {lv_idx} at 0x{header_off:05X} "
                      f"runs past ROM end")
                issue_count += 1
                continue

            if rom[end_off] != 0xFF:
                print(f"  {RED}ISSUE{RESET}: Level {lv_idx} at 0x{header_off:05X} "
                      f"ends at 0x{end_off:05X} = 0x{rom[end_off]:02X} (expected 0xFF)")
                issue_count += 1

            # Check 2: Screen monotonicity
            # Commands should have non-decreasing screen values (with hi bit as page)
            cmds_offset = header_off + 9
            commands, _ = parse_level_commands(rom, cmds_offset, region)

            prev_abs_screen = -1
            screen_violations = []
            for cmd in commands:
                if cmd.get("type") == "junction":
                    prev_abs_screen = -1  # junctions reset screen tracking
                    continue
                abs_screen = cmd["screen"] + (cmd["hi"] * 16)
                if abs_screen < prev_abs_screen:
                    screen_violations.append(cmd)
                prev_abs_screen = abs_screen

            if screen_violations:
                print(f"  {YELLOW}SCREEN{RESET}: Level {lv_idx} at 0x{header_off:05X} "
                      f"has {len(screen_violations)} screen-order violation(s):")
                for cmd in screen_violations[:3]:
                    d = cmd.get("dispatch", "?")
                    print(f"    offset 0x{cmd['offset']:05X}: "
                          f"scr={cmd['screen']} hi={cmd['hi']} "
                          f"group={cmd['group']} dispatch={d} "
                          f"bytes={[f'0x{b:02X}' for b in cmd['bytes']]}")
                issue_count += len(screen_violations)

        # Probe for missing dispatches:
        # Re-parse each level, and for each 3-byte variable command whose dispatch
        # is NOT in the set, try treating it as 4-byte and re-parse the rest.
        # If the 4-byte version produces fewer issues, flag it as a suspect.
        for lv_idx, lv in enumerate(levels):
            cmds_offset = lv["header_offset"] + 9
            commands, original_end = parse_level_commands(rom, cmds_offset, region)

            for ci, cmd in enumerate(commands):
                if cmd.get("type") != "variable":
                    continue
                dispatch = cmd.get("dispatch")
                if dispatch is None or dispatch in dispatches:
                    continue

                # This is a 3-byte variable command not in the dispatch set.
                # What if it should be 4-byte?
                # Re-parse from this command's offset, treating it as 4-byte.
                probe_offset = cmd["offset"] + 4  # skip 4 bytes instead of 3
                probe_region = dict(region)
                probe_dispatches = dispatches | {dispatch}
                probe_region["extra_byte_dispatches"] = probe_dispatches
                probe_cmds, probe_end = parse_level_commands(rom, probe_offset, probe_region)

                # Check if probe alignment is better: does it land on 0xFF?
                if (probe_end < len(rom) and rom[probe_end] == 0xFF
                        and (original_end >= len(rom) or rom[original_end] != 0xFF)):
                    # The 4-byte version fixed a broken level!
                    suspect_dispatches[dispatch] = suspect_dispatches.get(dispatch, 0) + 1

                # Also check: does original end on 0xFF but probe also does,
                # AND probe has fewer screen violations?
                elif (probe_end < len(rom) and rom[probe_end] == 0xFF
                      and original_end < len(rom) and rom[original_end] == 0xFF):
                    # Both end OK — check screen monotonicity improvement
                    orig_violations = 0
                    prev_s = -1
                    for c in commands[ci:]:
                        if c.get("type") == "junction":
                            prev_s = -1
                            continue
                        s = c["screen"] + c["hi"] * 16
                        if s < prev_s:
                            orig_violations += 1
                        prev_s = s

                    probe_violations = 0
                    prev_s = cmd["screen"] + cmd["hi"] * 16  # keep current cmd's screen
                    for c in probe_cmds:
                        if c.get("type") == "junction":
                            prev_s = -1
                            continue
                        s = c["screen"] + c["hi"] * 16
                        if s < prev_s:
                            probe_violations += 1
                        prev_s = s

                    if orig_violations > 0 and probe_violations < orig_violations:
                        suspect_dispatches[dispatch] = suspect_dispatches.get(dispatch, 0) + 1

        # Report
        print(f"\n{WHITE}=== {region_name} ==={RESET}")
        print(f"  Levels parsed: {len(levels)}")
        print(f"  Dispatches: {sorted(dispatches)}")

        if issue_count:
            print(f"  {RED}Issues: {issue_count}{RESET}")
            total_issues += issue_count
        else:
            print(f"  {GREEN}No alignment issues{RESET}")

        if suspect_dispatches:
            print(f"  {YELLOW}Suspect missing dispatches:{RESET}")
            for d, count in sorted(suspect_dispatches.items()):
                group = 0
                for g in range(8):
                    if VAR_BASES[g] <= d < VAR_BASES[g] + 15:
                        group = g
                        break
                var_type = d - VAR_BASES[group] + 1
                print(f"    dispatch {d} (group {group}, var_type {var_type}): "
                      f"would fix {count} level(s) if 4-byte")
                total_suspects += 1
        else:
            print(f"  {GREEN}No suspect missing dispatches{RESET}")

        total_levels += 0  # already counted above

    print(f"\n{WHITE}=== Summary ==={RESET}")
    print(f"  Total levels: {total_levels}")
    if total_issues:
        print(f"  {RED}Total issues: {total_issues}{RESET}")
    else:
        print(f"  {GREEN}No alignment issues found{RESET}")
    if total_suspects:
        print(f"  {YELLOW}Total suspect dispatches: {total_suspects}{RESET}")
    else:
        print(f"  {GREEN}No suspect missing dispatches{RESET}")


def enumerate_beta_entries(rom):
    """Synthesize entry dicts for the 9 beta (unreferenced) levels.

    Beta levels have no pointer-table slot — they are injected by the
    randomizer when `include_beta_stages` is enabled. The returned dicts
    mirror the shape of vanilla entries enough for `render_level_lookup`,
    but `type` is set to ``"beta"`` and grid/screen fields are placeholders.
    """
    out = []
    for i, bl in enumerate(BETA_LEVELS):
        obj_ptr = bl["obj_lo"] | (bl["obj_hi"] << 8)
        lay_ptr = bl["lay_lo"] | (bl["lay_hi"] << 8)
        entry = {
            "index": i,
            "obj_ptr": obj_ptr,
            "lay_ptr": lay_ptr,
            "tileset": bl["tileset"],
            "type": "beta",
            "grid_row": -1,
            "grid_col": -1,
            "screen": -1,
            "col_in_screen": -1,
            "row_nib": -1,
        }
        lay_file = layout_file_offset(lay_ptr, bl["tileset"])
        if lay_file is not None:
            entry["layout_file_offset"] = lay_file
            if lay_file + 4 < len(rom):
                entry["screens"] = (rom[lay_file + 4] & 0x0F) + 1
        out.append((bl["name"], entry))
    return out


def beta_patches_for_entry(entry):
    """Return BETA_PATCHES that fall within this beta level's layout range.

    Range is [lay_file_offset, next_beta_lay_file_offset) where beta levels
    are sorted by file offset (not by index — banks differ across tilesets).
    """
    lay_off = entry.get("layout_file_offset")
    if lay_off is None:
        return []
    starts = []
    for bl in BETA_LEVELS:
        lp = bl["lay_lo"] | (bl["lay_hi"] << 8)
        s = layout_file_offset(lp, bl["tileset"])
        if s is not None:
            starts.append(s)
    starts = sorted(set(starts))
    try:
        idx = starts.index(lay_off)
    except ValueError:
        return []
    end = starts[idx + 1] if idx + 1 < len(starts) else lay_off + 0x2000
    return [(o, b) for o, b in BETA_PATCHES if lay_off <= o < end]


def resolve_level_name(rom, query):
    """Resolve a human level name to a list of (world_idx, entry) matches.

    Accepted formats:
      3-2, 3F, 3F1, 3F2, 3A, 8B, 8-Tank, 8-Navy, 2-QS, 2-Pyr, 5-SC, 7-P1, 7-P2,
      β1..β9 (also accepted: beta1, beta-1)
    Returns list of (world_idx, entry_dict, canonical_name) tuples.
    For beta levels world_idx is ``None``.
    """
    q = query.strip().upper().replace(" ", "")

    # Build full name table across all worlds
    all_names = []  # (canonical_name, world_idx, entry_dict)
    for wi in range(8):
        grid = read_tile_grid(rom, wi)
        _, entry_lookup = build_entry_lookup(rom, wi)

        # Collect entries with names, tracking fortress counts
        world_entries = []
        for pos in sorted(entry_lookup.keys()):
            entry = entry_lookup[pos]
            r, c = pos
            tile = grid[r][c] if 0 <= r < len(grid) and 0 <= c < len(grid[0]) else 0
            name = derive_level_name(wi, entry["index"], entry["type"], tile)
            if name:
                world_entries.append((name, entry, pos, tile))

        # Number fortresses if multiple
        fort_entries = [(n, e, p, t) for n, e, p, t in world_entries
                        if n.endswith("F") and len(n) <= 2]
        if len(fort_entries) > 1:
            numbered = []
            for idx, (n, e, p, t) in enumerate(world_entries):
                if n.endswith("F") and len(n) <= 2:
                    count = sum(1 for nn, _, _, _ in world_entries[:idx + 1]
                                if nn.endswith("F") and len(nn) <= 2)
                    numbered.append((f"{n}{count}", e, p, t))
                else:
                    numbered.append((n, e, p, t))
            world_entries = numbered

        for name, entry, pos, tile in world_entries:
            all_names.append((name.upper(), wi, entry, name))

    # Beta levels (no world). Register each under canonical name plus
    # ASCII aliases so `beta1`, `beta-1`, `B1`-via-Greek-Β1 all work.
    for cname, entry in enumerate_beta_entries(rom):
        n_idx = entry["index"] + 1
        aliases = [cname, f"beta{n_idx}", f"beta-{n_idx}"]
        for a in aliases:
            all_names.append((a.upper(), None, entry, cname))

    # Try exact match first
    matches = [(wi, e, cn) for (n, wi, e, cn) in all_names if n == q]
    if matches:
        return _dedupe_matches(matches)

    # Try with dash removed (e.g., "3F1" matches "3-F1" or "3F1")
    q_nodash = q.replace("-", "")
    matches = [(wi, e, cn) for (n, wi, e, cn) in all_names if n.replace("-", "") == q_nodash]
    if matches:
        return _dedupe_matches(matches)

    # Try suffix match for override names (e.g., "TANK" matches "8-TANK")
    matches = [(wi, e, cn) for (n, wi, e, cn) in all_names if q in n]
    if matches:
        return _dedupe_matches(matches)

    return []


def _dedupe_matches(matches):
    """Drop duplicate (wi, entry, cname) triples introduced by alias registration."""
    seen = set()
    out = []
    for wi, e, cn in matches:
        key = (wi, id(e), cn)
        if key in seen:
            continue
        seen.add(key)
        out.append((wi, e, cn))
    return out


def parse_enemy_entries(rom, obj_cpu_ptr):
    """Parse all enemy/object entries from an enemy data segment.
    Returns list of dicts with offset, obj_id, name, class, x, y, screen."""
    if obj_cpu_ptr < 0xC000 or obj_cpu_ptr > 0xDFFF:
        return []
    file_off = obj_file_offset(obj_cpu_ptr)
    if file_off is None or file_off >= len(rom):
        return []

    entries = []
    page = rom[file_off]
    pos = file_off + 1  # skip page flag byte
    while pos + 2 < len(rom):
        oid = rom[pos]
        if oid == 0xFF:
            break
        x_byte = rom[pos + 1]
        y_byte = rom[pos + 2]
        screen = (x_byte >> 4) & 0x0F
        x_col = x_byte & 0x0F
        y_row = y_byte & 0x0F

        entry = {
            "offset": pos,
            "obj_id": oid,
            "x_col": x_col,
            "y_row": y_row,
            "screen": screen,
            "page": page,
        }
        name = ENEMY_NAMES.get(oid)
        if name:
            entry["name"] = name
        cls = find_enemy_class(oid)
        if cls:
            entry["class"] = cls
        boss = BOSS_ENEMY_IDS.get(oid)
        if boss:
            entry["boss"] = boss
        entries.append(entry)
        pos += 3
    return entries


def _render_enemy_lines(enemies):
    """Format parsed enemies into indented display lines (offset, name, tags)."""
    out = []
    for e in enemies:
        name = e.get("name", f"0x{e['obj_id']:02X}")
        cls = e.get("class", "")
        boss = e.get("boss", "")
        tags = []
        if cls:
            tags.append(f"class:{cls}")
        if boss:
            tags.append(f"{RED}BOSS:{boss}{RESET}")
        tag_str = f"  ({', '.join(tags)})" if tags else ""
        out.append(f"    0x{e['offset']:05X}: {name} "
                   f"scr={e['screen']} col={e['x_col']} row={e['y_row']}"
                   f"{tag_str}")
    return out


def trace_beta_sub_areas(rom, main_lay_off):
    """Follow a beta level's alt-area chain from its main header.

    Each area's 9-byte header points (alt_layout/alt_objects/alt_tileset) at the
    NEXT area, exactly as the engine loads a sub-area when Mario takes a pipe or
    door. Area N's enemies come from area N-1's header `alt_objects`. We walk the
    chain, seeding the visited set with the main layout offset so an alt pointer
    that loops back to the main area (the common "no sub-area" encoding) stops
    the trace. Returns a list of dicts for each sub-area (index >= 1):
        {idx, lay_off, enemy_ptr, screens, tileset, enemies}
    """
    subs = []
    if main_lay_off is None or main_lay_off + 9 > len(rom):
        return subs
    visited = {main_lay_off}
    hdr = parse_level_header(rom, main_lay_off)
    next_lay, next_obj, next_ts = hdr["alt_layout"], hdr["alt_objects"], hdr["alt_tileset"]
    idx = 1
    while idx <= 8:  # depth guard; real chains are 1-2 deep
        if next_lay < 0xA000:
            break
        lay_off = layout_file_offset(next_lay, next_ts)
        if lay_off is None or lay_off + 9 > len(rom) or lay_off in visited:
            break
        visited.add(lay_off)
        sub_hdr = parse_level_header(rom, lay_off)
        enemies = parse_enemy_entries(rom, next_obj) if next_obj >= 0xC000 else []
        subs.append({
            "idx": idx,
            "lay_off": lay_off,
            "enemy_ptr": next_obj,
            "screens": sub_hdr["screens"],
            "tileset": next_ts,
            "enemies": enemies,
        })
        next_lay, next_obj, next_ts = (
            sub_hdr["alt_layout"], sub_hdr["alt_objects"], sub_hdr["alt_tileset"])
        idx += 1
    return subs


def _render_beta_entry(rom, cname, entry):
    """Render the lookup output for a beta (unreferenced) level.

    Beta levels lack a pointer-table slot, world position, and powerup index.
    We show identity, header, main-area enemies (from the borrowed obj_ptr),
    sub-area enemies (traced through the header's alt_layout/alt_objects chain),
    and any BETA_PATCHES that target this layout.
    """
    obj = entry["obj_ptr"]
    lay = entry["lay_ptr"]
    ts = entry["tileset"]
    lay_off = entry.get("layout_file_offset")
    screens = entry.get("screens")

    lines = []
    lines.append(f"{YELLOW}{cname}{RESET}  (beta stage, no pointer-table entry)")
    extras = f"  Lay: 0x{lay:04X}"
    if lay_off is not None:
        extras += f" (file 0x{lay_off:05X})"
    if screens is not None:
        extras += f"  Screens: {screens}"
    lines.append(f"  Tileset: {ts}  Obj: 0x{obj:04X}{extras}")
    lines.append(f"  {DIM}obj_ptr is borrowed from a vanilla level — enemy "
                 f"data isn't unique to this beta{RESET}")

    # Header
    if lay_off is not None:
        try:
            h = parse_level_header(rom, lay_off)
            lines.append("")
            lines.append(f"  {WHITE}Header (10 bytes @ 0x{lay_off:05X}):{RESET}")
            lines.append(f"    bytes: {' '.join(f'{b:02X}' for b in h['bytes'])}")
            lines.append(
                f"    screens={h['screens']}  bg_pal={h['bg_palette']}  "
                f"obj_pal={h['obj_palette']}  music={h['music']}  timer={h['timer']}"
            )
            lines.append(
                f"    alt_layout=0x{h['alt_layout']:04X}  "
                f"alt_objects=0x{h['alt_objects']:04X}  "
                f"alt_tileset={h['alt_tileset']}"
            )
        except Exception:
            pass

    # Main-area enemies (from the borrowed pointer-table obj_ptr)
    enemies = parse_enemy_entries(rom, obj)
    lines.append("")
    lines.append(f"  {WHITE}Enemies (obj 0x{obj:04X}, {len(enemies)} entries):{RESET}")
    if enemies:
        lines.extend(_render_enemy_lines(enemies))
    else:
        lines.append("    (none)")

    # Sub-areas (trace the header's alt_layout/alt_objects chain, like the engine)
    if lay_off is not None:
        for sa in trace_beta_sub_areas(rom, lay_off):
            lines.append("")
            lines.append(f"  {WHITE}Sub-area {sa['idx']} "
                         f"(enemy_ptr 0x{sa['enemy_ptr']:04X}, ts{sa['tileset']}, "
                         f"{sa['screens']} screens, {len(sa['enemies'])} enemies):{RESET}")
            if sa["enemies"]:
                lines.extend(_render_enemy_lines(sa["enemies"]))
            else:
                lines.append("    (none)")

    # BETA_PATCHES for this entry's layout range
    patches = beta_patches_for_entry(entry)
    if patches:
        lines.append("")
        lines.append(f"  {WHITE}Layout fixes applied (BETA_PATCHES, "
                     f"{len(patches)} bytes):{RESET}")
        for off, b in patches:
            old = rom[off] if 0 <= off < len(rom) else 0
            rel = off - lay_off if lay_off is not None else 0
            lines.append(f"    0x{off:05X} (lay+0x{rel:03X}): "
                         f"0x{old:02X} -> 0x{b:02X}")

    lines.append("")
    return lines


def render_level_lookup(rom, query):
    """Look up a level by name and render its details."""
    matches = resolve_level_name(rom, query)
    if not matches:
        return f"No level found matching '{query}'.\n" \
               f"Examples: 1-1, 3-2, 3F1, 5A, 8B, 8-Tank, 2-QS, 7-P1, β1"

    # Build level groups for sub-area + powerup info
    all_region_levels = []
    for region in LEVEL_DATA_REGIONS:
        levels = scan_level_data_region(rom, region)
        all_region_levels.append({
            "region": region["name"],
            "tileset_ids": region["tileset_ids"],
            "start": region["start"],
            "end": region["end"],
            "extra_byte_dispatches": sorted(region["extra_byte_dispatches"]),
            "level_count": len(levels),
            "levels": levels,
        })

    # Build worlds data for level group matching
    worlds_data = []
    for w_idx, w_info in enumerate(WORLDS):
        worlds_data.append(parse_world_tables(rom, w_idx, w_info))

    level_groups = build_level_groups(rom, all_region_levels, worlds_data)

    lines = []
    for wi, entry, cname in matches:
        eidx = entry["index"]
        obj = entry["obj_ptr"]
        lay = entry["lay_ptr"]
        ts = entry["tileset"]
        etype = entry["type"]

        if etype == "beta":
            lines.extend(_render_beta_entry(rom, cname, entry))
            continue

        w = wi + 1
        lines.append(f"{YELLOW}{cname}{RESET}  (World {w}, entry {eidx}, type: {etype})")
        lines.append(f"  Tileset: {ts}  Obj: 0x{obj:04X}  Lay: 0x{lay:04X}")
        lines.append(f"  Grid: row={entry['grid_row']}, col={entry['grid_col']}")

        # Find matching level group
        group = None
        for g in level_groups:
            if (wi + 1, eidx) in [(wr[0], wr[1]) for wr in g["world_refs"]]:
                group = g
                break

        # Entry-point enemies
        enemies = parse_enemy_entries(rom, obj)
        lines.append(f"")
        lines.append(f"  {WHITE}Enemies (obj 0x{obj:04X}, {len(enemies)} entries):{RESET}")
        if enemies:
            for e in enemies:
                name = e.get("name", f"0x{e['obj_id']:02X}")
                cls = e.get("class", "")
                boss = e.get("boss", "")
                tags = []
                if cls:
                    tags.append(f"class:{cls}")
                if boss:
                    tags.append(f"{RED}BOSS:{boss}{RESET}")
                tag_str = f"  ({', '.join(tags)})" if tags else ""
                lines.append(f"    0x{e['offset']:05X}: {name} "
                             f"scr={e['screen']} col={e['x_col']} row={e['y_row']}"
                             f"{tag_str}")
        else:
            lines.append(f"    (none)")

        # Sub-areas from level group
        if group and group["sub_area_count"] > 0:
            for sa_idx, sa in enumerate(group["sub_areas"]):
                if sa_idx == 0:
                    # Entry-point powerups
                    continue
                ep = sa["enemy_ptr"]
                sa_enemies = parse_enemy_entries(rom, ep) if ep and ep >= 0xC000 else []
                lines.append(f"")
                lines.append(f"  {WHITE}Sub-area {sa_idx} "
                             f"(enemy_ptr 0x{ep:04X}, {sa['screens']} screens, "
                             f"{sa['command_count']} cmds, {len(sa_enemies)} enemies):{RESET}")
                for e in sa_enemies:
                    name = e.get("name", f"0x{e['obj_id']:02X}")
                    cls = e.get("class", "")
                    boss = e.get("boss", "")
                    tags = []
                    if cls:
                        tags.append(f"class:{cls}")
                    if boss:
                        tags.append(f"{RED}BOSS:{boss}{RESET}")
                    tag_str = f"  ({', '.join(tags)})" if tags else ""
                    lines.append(f"    0x{e['offset']:05X}: {name} "
                                 f"scr={e['screen']} col={e['x_col']} row={e['y_row']}"
                                 f"{tag_str}")

        # Powerups from level group
        if group:
            all_powerups = []
            for sa_idx, sa in enumerate(group["sub_areas"]):
                # Find matching parsed level by header_offset
                for region_data in all_region_levels:
                    for lv in region_data["levels"]:
                        if lv["header_offset"] == sa["header_offset"]:
                            for p in lv["powerups"]:
                                p_copy = dict(p)
                                p_copy["sub_area"] = sa_idx
                                all_powerups.append(p_copy)

            lines.append(f"")
            lines.append(f"  {WHITE}Items ({len(all_powerups)} powerup blocks):{RESET}")
            if all_powerups:
                for p in all_powerups:
                    prot = f"  {RED}PROTECTED{RESET}" if p.get("protected") else ""
                    rcls = f"  rand:{p['randomize_class']}" if p.get("randomize_class") else ""
                    area = f"  [sub-area {p['sub_area']}]" if p["sub_area"] > 0 else ""
                    lines.append(f"    0x{p['byte2_offset']:05X}: {p['name']} "
                                 f"scr={p['screen']} row={p['row']} col={p['col']}"
                                 f"{rcls}{prot}{area}")
            else:
                lines.append(f"    (none)")

        # Boss summary
        if group:
            bosses = []
            if group["has_boomboom"]:
                bosses.append("Boom-Boom")
            if group["has_koopaling"]:
                bosses.append("Koopaling")
            if group["has_bowser"]:
                bosses.append("Bowser")
            if bosses:
                lines.append(f"")
                lines.append(f"  {RED}Bosses: {', '.join(bosses)}{RESET}")

        lines.append("")

    return "\n".join(lines)


# --------------------------------------------------------------------------
# World-map tile byte lookup
# --------------------------------------------------------------------------
#
# A world-map tile byte (0x00..0xFF) is fully described by:
#   1. Its CHR pattern — 4 quadrant CHR indices in metatile bank 0x0C
#      (file 0x18010..0x18410, four parallel 256-byte tables)
#   2. Its palette — encoded in the high 2 bits of the tile byte itself
#      (per southbird disasm: "palette is determined by the upper 2 bits
#      of a TILE"). See PALETTE_PAGES below.
#   3. Behavioral classification — which of the small registries below
#      it appears in. Each registry adds a behavior to a tile.

# Metatile pattern bank (PRG012 / bank 0x0C).
TILE_BANK_NW = 0x18010   # 256 bytes: NW (top-left) CHR index per tile
TILE_BANK_NE = 0x18110   # 256 bytes: NE
TILE_BANK_SW = 0x18210   # 256 bytes: SW
TILE_BANK_SE = 0x18310   # 256 bytes: SE

# Direction-walk tables (PRG010). Each is 9 bytes — listing tile bytes
# walkable in that direction. Padded with duplicates if fewer than 9.
WALK_LEFT  = 0x15258
WALK_RIGHT = 0x15261
WALK_DOWN  = 0x1526A
WALK_UP    = 0x15273

# Special-entry tile list + parallel dispatch op-code (PRG010). 11 entries.
# Stepping on a tile in the list fires the handler keyed by its op-code.
# The CPU loop reads up to index $1A (bug) — entries 11..26 still match
# but dispatch to garbage handlers.
ENTER_TILES_OFF    = 0x14DBF
ENTER_DISPATCH_OFF = 0x14DCA
ENTER_NAMES = [
    "TOADHOUSE", "SPADEBONUS", "PIPE", "ALTTOADHOUSE", "CASTLEBOTTOM",
    "SPIRAL", "ALTSPIRAL", "PATHANDNUB", "DANCINGFLOWER", "HANDTRAP",
    "BOWSERCASTLELL",
]

# Removable obstacles (8 bytes) — locks/rocks/water cleared after fortress.
REMOVABLE_OFF = 0x18447
# Special-completion (5 bytes) — one-shot tiles tracked in Map_Completions.
SPECIAL_COMPL_OFF = 0x18457
# Per-page completion thresholds (4 bytes) — tiles >= threshold for their
# palette page are completion-tracked. Page = high 2 bits of tile byte.
THRESHOLDS_OFF = 0x18410

# Background / void tiles — non-walkable visual fill.
BACKGROUND_TILES = {0x02, 0xB4, 0xFF}


def _parse_tile_query(query):
    """Parse a tile byte from various string forms: 0xE6, E6, 230."""
    q = query.strip().lower()
    if q.startswith("0x"):
        return int(q, 16)
    # Try hex first if it has hex chars, else decimal
    try:
        if any(c in "abcdef" for c in q) or len(q) <= 2:
            return int(q, 16)
        return int(q)
    except ValueError:
        return int(q, 16)


def render_tile_lookup(rom, query):
    """Look up a world-map tile byte: visual pattern, palette, behavior."""
    try:
        tile = _parse_tile_query(query)
    except ValueError:
        return f"Could not parse tile byte: '{query}'. Try '0xE6' or 'E6'."
    if not (0 <= tile <= 0xFF):
        return f"Tile byte must be 0x00..0xFF (got 0x{tile:X})"

    nw = rom[TILE_BANK_NW + tile]
    ne = rom[TILE_BANK_NE + tile]
    sw = rom[TILE_BANK_SW + tile]
    se = rom[TILE_BANK_SE + tile]
    palette_page = tile >> 6  # high 2 bits

    enter_tiles = list(rom[ENTER_TILES_OFF:ENTER_TILES_OFF + 11])
    enter_disp  = list(rom[ENTER_DISPATCH_OFF:ENTER_DISPATCH_OFF + 11])
    walk_left   = set(rom[WALK_LEFT:WALK_LEFT + 9])
    walk_right  = set(rom[WALK_RIGHT:WALK_RIGHT + 9])
    walk_down   = set(rom[WALK_DOWN:WALK_DOWN + 9])
    walk_up     = set(rom[WALK_UP:WALK_UP + 9])
    removable   = set(rom[REMOVABLE_OFF:REMOVABLE_OFF + 8])
    spec_compl  = set(rom[SPECIAL_COMPL_OFF:SPECIAL_COMPL_OFF + 5])
    thresholds  = list(rom[THRESHOLDS_OFF:THRESHOLDS_OFF + 4])
    page_thresh = thresholds[palette_page]

    # Find usage in vanilla world grids
    usage = []  # list of (world_idx, [(row, col), ...])
    for wi, info in enumerate(MAP_TILE_GRIDS[:8]):
        positions = []
        cols = info["columns"]
        screens = info["screens"]
        base = info["file_offset"]
        for s in range(screens):
            for r in range(9):
                for c in range(16):
                    if rom[base + s*144 + r*16 + c] == tile:
                        positions.append((r, s*16 + c))
        if positions:
            usage.append((wi, positions))

    # Find visually identical siblings (same NW/NE/SW/SE pattern)
    siblings = []
    for other in range(256):
        if other == tile:
            continue
        if (rom[TILE_BANK_NW + other] == nw and
            rom[TILE_BANK_NE + other] == ne and
            rom[TILE_BANK_SW + other] == sw and
            rom[TILE_BANK_SE + other] == se):
            siblings.append(other)

    L = []
    L.append(f"{YELLOW}Tile 0x{tile:02X}{RESET}")

    # Special-entry classification — find name if any
    enter_name = None
    enter_idx = None
    enter_op = None
    if tile in enter_tiles[:11]:
        enter_idx = enter_tiles.index(tile)
        enter_name = ENTER_NAMES[enter_idx]
        enter_op = enter_disp[enter_idx]
        L.append(f"  Special-entry: {WHITE}{enter_name}{RESET}  "
                 f"(idx {enter_idx} in Map_EnterSpecialTiles, dispatch op 0x{enter_op:02X})")

    # Palette
    L.append(f"")
    L.append(f"  {WHITE}Palette page:{RESET} {palette_page} "
             f"(high 2 bits = 0b{(tile >> 6):02b})")
    L.append(f"    palette index {palette_page} of the current world's "
             f"Map_Tile_ColorSet")
    L.append(f"    page range:   0x{palette_page << 6:02X}..0x{(palette_page << 6) | 0x3F:02X}")
    L.append(f"    completion threshold for this page: 0x{page_thresh:02X} "
             f"(tile {'>=' if tile >= page_thresh else '<'} threshold "
             f"-> {'completion-tracked' if tile >= page_thresh else 'not tracked'})")

    # Visual pattern
    L.append(f"")
    L.append(f"  {WHITE}Visual (metatile bank 0x0C):{RESET}")
    L.append(f"    NW=0x{nw:02X}  NE=0x{ne:02X}")
    L.append(f"    SW=0x{sw:02X}  SE=0x{se:02X}")
    L.append(f"    file offsets: 0x{TILE_BANK_NW + tile:05X} 0x{TILE_BANK_NE + tile:05X} "
             f"0x{TILE_BANK_SW + tile:05X} 0x{TILE_BANK_SE + tile:05X}")
    if siblings:
        L.append(f"    visually identical to: " +
                 ", ".join(f"0x{b:02X}" for b in siblings) +
                 "  (same CHR; differs only by palette page)")

    # Behavior registries
    L.append(f"")
    L.append(f"  {WHITE}Behavior:{RESET}")
    dirs = []
    if tile in walk_left:  dirs.append("LEFT")
    if tile in walk_right: dirs.append("RIGHT")
    if tile in walk_down:  dirs.append("DOWN")
    if tile in walk_up:    dirs.append("UP")
    L.append(f"    Movement       : {', '.join(dirs) if dirs else '(blocks all directions)'}")
    if enter_name:
        L.append(f"    Special-entry  : {enter_name} (op 0x{enter_op:02X})")
    else:
        L.append(f"    Special-entry  : -")
    L.append(f"    Removable      : {'yes (cleared after fortress)' if tile in removable else '-'}")
    L.append(f"    Special-compl. : {'yes (one-shot, tracked)' if tile in spec_compl else '-'}")
    L.append(f"    Background     : {'yes (non-walkable)' if tile in BACKGROUND_TILES else '-'}")

    # Vanilla usage
    L.append(f"")
    L.append(f"  {WHITE}Vanilla usage:{RESET}")
    if not usage:
        L.append(f"    (not used in any world's tile grid)")
    else:
        for wi, positions in usage:
            wname = MAP_TILE_GRIDS[wi]["name"]
            preview = positions[:8]
            extra = "" if len(positions) <= 8 else f" ... +{len(positions)-8} more"
            L.append(f"    {wname}: " +
                     ", ".join(f"({r},{c})" for r, c in preview) + extra)

    # Footer: registries reference
    L.append(f"")
    L.append(f"  {DIM}Behavior tables (file offsets):{RESET}")
    L.append(f"  {DIM}  Map_EnterSpecialTiles  0x{ENTER_TILES_OFF:05X} (11 bytes){RESET}")
    L.append(f"  {DIM}  Special-entry dispatch 0x{ENTER_DISPATCH_OFF:05X} (11 bytes){RESET}")
    L.append(f"  {DIM}  Walk LEFT  0x{WALK_LEFT:05X} (9 bytes){RESET}")
    L.append(f"  {DIM}  Walk RIGHT 0x{WALK_RIGHT:05X} (9 bytes){RESET}")
    L.append(f"  {DIM}  Walk DOWN  0x{WALK_DOWN:05X} (9 bytes){RESET}")
    L.append(f"  {DIM}  Walk UP    0x{WALK_UP:05X} (9 bytes){RESET}")
    L.append(f"  {DIM}  Removable  0x{REMOVABLE_OFF:05X} (8 bytes){RESET}")
    L.append(f"  {DIM}  Special-completion 0x{SPECIAL_COMPL_OFF:05X} (5 bytes){RESET}")
    L.append(f"  {DIM}  Page thresholds    0x{THRESHOLDS_OFF:05X} (4 bytes){RESET}")
    return "\n".join(L)


def render_check_map(rom, world_idx, pipe_pairs, uncovered_set):
    """Render a world map highlighting uncovered blank nodes in red."""
    steps = simulate_progression(rom, world_idx, pipe_pairs, traverse_rocks=True)
    _, entry_lookup = build_entry_lookup(rom, world_idx)

    seen = set()
    ordered_nodes = []
    for step in steps:
        for pos in step["bfs_order"]:
            if pos not in seen:
                seen.add(pos)
                ordered_nodes.append(pos)

    node_number = {}
    num = 1
    for pos in ordered_nodes:
        if pos in entry_lookup:
            node_number[pos] = (num, entry_lookup[pos])
            num += 1

    final_grid = steps[-1]["grid"]
    final_nodes = steps[-1]["nodes"]
    final_paths = steps[-1]["path_tiles"]
    cols = len(final_grid[0])

    RED_BG = "\033[1;37;41m"

    info = MAP_TILE_GRIDS[world_idx]
    status = f"\033[1;31m{len(uncovered_set)} uncovered\033[0m" if uncovered_set else f"\033[1;32mOK\033[0m"
    lines = [f"\n{WHITE}=== {info['name']} === {status}{RESET}"]

    ruler = "      "
    for c in range(cols):
        if c % 16 == 0:
            ruler += f"{GREEN}|{RESET}"
        else:
            ruler += " "
    lines.append(ruler)

    for r in range(MAP_TILE_GRID_ROWS):
        row_str = f"  {r}: "
        for c in range(cols):
            pos = (r, c)
            tile = final_grid[r][c]

            if c % 16 == 0 and c > 0:
                row_str += f"{DIM}|{RESET}"

            if pos in uncovered_set:
                row_str += f"{RED_BG}!!{RESET}"
            elif pos in node_number:
                bfs_n, entry = node_number[pos]
                color = TYPE_COLOR.get(entry["type"], WHITE)
                row_str += f"{color}{bfs_n:>2d}{RESET}"
            elif pos in final_paths:
                if tile in VALID_HORZ:
                    row_str += f"{DIM}--{RESET}"
                else:
                    row_str += f"{DIM} |{RESET}"
            elif pos in final_nodes:
                row_str += f"{DIM} *{RESET}"
            elif tile in BACKGROUND_TILES:
                row_str += f"{DIM} .{RESET}"
            else:
                row_str += f"{DIM} ~{RESET}"
        lines.append(row_str)

    if uncovered_set:
        lines.append(f"\n  {RED_BG}!!{RESET} = uncovered node (no pointer table entry)")
        for r, c, tile in sorted((r, c, final_grid[r][c]) for r, c in uncovered_set):
            lines.append(f"  ({r},{c:>2d}) tile=${tile:02X}")

    return "\n".join(lines)


def check_node_coverage(rom, world_idx, pipe_pairs):
    """Check that every reachable node has a pointer table entry.

    Uses fortress progression to open locks step by step, so nodes behind
    fortresses are included. Returns a list of (row, col, tile) for
    uncovered nodes.
    """
    VALID_BLANK_TILES = {0x44, 0x47, 0x48, 0x4A, 0xAE, 0xAF, 0xB5, 0xB6,
                         0xD9, 0xDC, 0xDD, 0xDE}

    steps = simulate_progression(rom, world_idx, pipe_pairs)

    # Collect all nodes reachable across all progression steps.
    all_nodes = set()
    for step in steps:
        all_nodes |= step["nodes"]

    # Use the final grid (all locks opened) for tile checks.
    final_grid = steps[-1]["grid"]

    # Build set of positions that have pointer table entries
    _, entry_lookup = build_entry_lookup(rom, world_idx)
    covered_positions = set(entry_lookup.keys())

    # Find uncovered nodes — reachable blank tiles with no entry
    uncovered = []
    for (r, c) in sorted(all_nodes):
        if r < 0 or r >= len(final_grid) or c < 0 or c >= len(final_grid[0]):
            continue
        tile = final_grid[r][c]
        if tile not in VALID_BLANK_TILES:
            continue  # non-blank nodes (levels, forts, etc.) already have entries
        if (r, c) not in covered_positions:
            uncovered.append((r, c, tile))

    return uncovered


# --------------------------------------------------------------------------
# Antechamber pattern detection (--antechamber)
# --------------------------------------------------------------------------

def _junction_commands(rom, level, region_by_name):
    """Re-parse a level's layout commands and return its junction commands,
    decoded per the 'Junction Spawn Positions' section of the ROM reference."""
    region = region_by_name[level["region"]]
    commands, _ = parse_level_commands(rom, level["data_offset"], region)
    juncts = []
    for cmd in commands:
        if cmd.get("type") != "junction":
            continue
        b0, b1, b2 = cmd["bytes"][:3]
        juncts.append({
            "offset": cmd["offset"],
            "bytes": [b0, b1, b2],
            "slot": b0 & 0x0F,          # Level_JctY/XLHStart index
            "exit_dir": b1 & 0x0F,      # Level_PipeExitDir
            "ystart_idx": (b1 >> 4) & 0x07,
            "vertical": bool(b1 & 0x80),
            "spawn_screen": b2 & 0x0F,  # X Hi
            "spawn_col": (b2 >> 4) & 0x0F,
        })
    return juncts


def find_antechamber_candidates(rom):
    """Find levels matching the antechamber pattern: an entry area whose
    front-door pipe (a junction near the start) leads into the level's
    interior. Candidates must be safe for interior shuffling:

    - the interior resolves and is self-contained — it defines its own exit
      junction command(s), or never junctions out at all (dead alt pointer,
      goal inside the interior). Spawn slots are read from the SOURCE area's
      parse, so an interior relying on stale slots left by the entry area's
      parse would break when hosted behind a foreign entry.
    - no bosses in the pair.
    - shape: the front-door junction sits on screens 0-2 of the entry area
      and the interior is >= 6 screens. This separates the pattern from
      bonus dips and end rooms (a level piping into a small room), which
      are mechanically shuffle-safe but a different feature.

    Returns a list of candidate dicts keyed by entry header offset."""
    all_region_levels = []
    for region in LEVEL_DATA_REGIONS:
        levels = scan_level_data_region(rom, region)
        all_region_levels.append({
            "region": region["name"],
            "tileset_ids": region["tileset_ids"],
            "levels": levels,
        })
    layout_index = build_layout_index(all_region_levels)
    region_by_name = {r["name"]: r for r in LEVEL_DATA_REGIONS}

    candidates = {}  # entry header_offset -> candidate

    for w_idx, w_info in enumerate(WORLDS):
        world_data = parse_world_tables(rom, w_idx, w_info)
        grid = read_tile_grid(rom, w_idx)
        for entry in world_data["entries"]:
            if entry["type"] != "level":
                continue
            tileset = entry["tileset"]
            entry_lv = layout_index.get((tileset, entry["lay_ptr"]))
            if entry_lv is None:
                continue

            # Human-readable name from the map tile under this entry
            gr, gc = entry["grid_row"], entry["grid_col"]
            tile = grid[gr][gc] if 0 <= gr < len(grid) and 0 <= gc < len(grid[0]) else 0
            name = derive_level_name(w_idx, entry["index"], entry["type"], tile) \
                or f"W{w_idx + 1}[{entry['index']}]"

            ec = candidates.get(entry_lv["header_offset"])
            if ec is not None:
                ec["refs"].append(name)
                continue

            main_lv = layout_index.get((entry_lv["alt_tileset"], entry_lv["alt_layout"]))
            if main_lv is None or main_lv is entry_lv:
                continue

            entry_juncts = _junction_commands(rom, entry_lv, region_by_name)
            if not entry_juncts:
                continue

            # Shape: front-door pipe near the level start, substantial
            # interior. Excludes bonus dips / end rooms (inverse pattern).
            if min(j["slot"] for j in entry_juncts) > 2:
                continue
            if main_lv["header"]["screens"] < 6:
                continue

            # The interior must be self-contained: it defines its own exit
            # junction command(s), or never junctions out (dead alt). An
            # interior with zero junction commands but a live alt pointer
            # would rely on stale slots from the entry area's parse, which
            # the shuffle corrupts.
            back = layout_index.get((main_lv["alt_tileset"], main_lv["alt_layout"]))
            if main_lv["junction_count"] < 1 and back is not None:
                continue

            # No bosses anywhere in the pair (koopalings/boom-booms/bowser
            # rooms are managed by other systems and must not move).
            main_boss = scan_enemy_segment_bosses(rom, entry_lv["alt_objects"])
            if (entry_lv["has_boomboom"] or entry_lv["has_koopaling"]
                    or entry_lv["has_bowser"] or any(main_boss.values())):
                continue

            loop_back = back is entry_lv

            candidates[entry_lv["header_offset"]] = {
                "refs": [name],
                "entry": entry_lv,
                "entry_tileset": tileset,
                "main": main_lv,
                "loop_back": loop_back,
                "entry_junctions": entry_juncts,
                "main_junctions": _junction_commands(rom, main_lv, region_by_name),
            }

    return sorted(candidates.values(), key=lambda c: c["refs"][0])


def render_antechamber_report(rom):
    """Human-readable report of antechamber-pattern candidates, with the
    offsets needed for shuffle constants (entry header, main area pointers,
    entry-junction slot in the main area's layout)."""
    lines = []
    cands = find_antechamber_candidates(rom)
    for c in cands:
        e, m = c["entry"], c["main"]
        ret = "loop-back" if c["loop_back"] else "NO loop-back (generic exit?)"
        lines.append(f"{CYAN}{'/'.join(c['refs'])}{RESET} [{e['region']}]")
        lines.append(f"  entry: header 0x{e['header_offset']:05X} "
                     f"lay=${e['layout_cpu']:04X} ts={c['entry_tileset']} "
                     f"{e['header']['screens']}scr timer={e['header']['timer']}")
        lines.append(f"  main:  header 0x{m['header_offset']:05X} "
                     f"alt_layout=${e['alt_layout']:04X} alt_objects=${e['alt_objects']:04X} "
                     f"alt_ts={e['alt_tileset']} {m['header']['screens']}scr  return: {ret}")
        for tag, juncts in (("main", c["main_junctions"]),
                            ("entry", c["entry_junctions"])):
            for j in juncts:
                vert = " vert" if j["vertical"] else ""
                lines.append(
                    f"    {tag} junction @0x{j['offset']:05X} slot={j['slot']} "
                    f"exit_dir={j['exit_dir']} ystart={j['ystart_idx']} "
                    f"spawn scr {j['spawn_screen']} col {j['spawn_col']}{vert}")
        # Multi-pipe entries are supported (all commands get the donor's
        # spawn bytes; the lowest-slot command is the front door / donor
        # source) but deserve a closer look when curating the pool.
        if len(c["entry_junctions"]) != 1:
            lines.append(f"    {YELLOW}note: {len(c['entry_junctions'])} "
                         f"junction commands in entry area{RESET}")
        lines.append("")
    lines.append(f"{len(cands)} candidate(s)")
    return "\n".join(lines)


if __name__ == "__main__":
    main()
