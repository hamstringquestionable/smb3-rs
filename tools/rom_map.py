#!/usr/bin/env python3
# pyright: basic
"""
SMB3 ROM Map Generator

Walks the entire ROM using known pointer tables and level data structures
to produce a comprehensive JSON map of all levels, their powerup blocks,
enemy data, and key tables. The output avoids redundant ROM scanning in
future sessions.

Usage: python3 tools/rom_map.py [rom_path] [--json output.json]
  Default ROM: "Super Mario Bros. 3 (USA) (Rev 1).nes"
  Default output: tools/rom_map.json
"""

import json
import os
import sys
from collections import defaultdict

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
        "extra_byte_dispatches": {0, 35, 36, 37, 38, 39, 40, 41, 42},
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
    "Debug_Mode": {"offset": 0x309D5, "size": 1, "desc": "Debug toggle (0xCC=on, 0x35=off)"},
}

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

    while i + 9 < region["end"]:
        # Parse header
        header = parse_level_header(rom, i)
        i += 9

        # Parse commands
        commands, end = parse_level_commands(rom, i, region)

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
            "command_count": len(commands),
            "powerup_count": len(powerups),
            "powerups": powerups,
        }
        levels.append(level)

        # Skip terminator
        if end < len(rom) and rom[end] == 0xFF:
            i = end + 1
        else:
            break

    return levels


# --------------------------------------------------------------------------
# Level pointer table parsing
# --------------------------------------------------------------------------

def classify_entry(obj_ptr, lay_ptr):
    """Classify a level pointer table entry by type."""
    if obj_ptr == 0x0700:
        return "toad_house"
    if obj_ptr >= 0xD000:
        return "fortress"
    if obj_ptr == 0x0001 and lay_ptr == 0x0000:
        return "bonus_game"
    if obj_ptr >= 0xC000 and obj_ptr < 0xD000 and lay_ptr != 0x0000:
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
        entry_type = classify_entry(obj_ptr, lay_ptr)

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
        }

        # Resolve file offsets for levels
        if entry_type in ("level", "fortress"):
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

    # Mark duplicate (obj, lay) pairs as non-shuffleable (hammer bros)
    pair_counts = defaultdict(int)
    for e in entries:
        if e["type"] == "level":
            pair_counts[(e["obj_ptr"], e["lay_ptr"])] += 1

    for e in entries:
        if e["type"] == "level" and pair_counts.get((e["obj_ptr"], e["lay_ptr"]), 0) > 1:
            e["shuffleable"] = False
            e["exclude_reason"] = "duplicate_pair"

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

    # Protected offsets
    prot = rom_map["protected_offsets"]
    print(f"\nProtected Offsets:")
    for p in prot["powerup_byte2"]:
        print(f"  0x{p['offset']:05X}: {p['reason']}")
    for p in prot["enemy_obj_id"]:
        print(f"  0x{p['offset']:05X}: {p['reason']}")

    print()


def main():
    rom_path = "Super Mario Bros. 3 (USA) (Rev 1).nes"
    output_path = "tools/rom_map.json"

    args = sys.argv[1:]
    i = 0
    while i < len(args):
        if args[i] == "--json" and i + 1 < len(args):
            output_path = args[i + 1]
            i += 2
        else:
            rom_path = args[i]
            i += 1

    if not os.path.exists(rom_path):
        print(f"Error: ROM file not found: {rom_path}")
        print("Usage: python3 tools/rom_map.py [rom_path] [--json output.json]")
        sys.exit(1)

    with open(rom_path, "rb") as f:
        rom = f.read()

    if len(rom) != ROM_SIZE:
        print(f"Warning: ROM size {len(rom)} != expected {ROM_SIZE}")

    print(f"Reading ROM: {rom_path} ({len(rom)} bytes)")
    rom_map = generate_rom_map(rom)

    print_summary(rom_map)

    with open(output_path, "w") as f:
        json.dump(rom_map, f, indent=2)

    print(f"ROM map written to: {output_path}")

    # Print file size
    file_size = os.path.getsize(output_path)
    if file_size > 1024 * 1024:
        print(f"  Size: {file_size / 1024 / 1024:.1f} MB")
    else:
        print(f"  Size: {file_size / 1024:.1f} KB")


if __name__ == "__main__":
    main()
