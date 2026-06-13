#!/usr/bin/env python3
"""
Validate that collect_shuffleable logic matches rom_map.json ground truth.

Reimplements the Rust collect_shuffleable function in Python and compares
the results against the pre-computed "shuffleable" flags in rom_map.json.
"""

import json
import os
import sys
from collections import Counter

# ---------------------------------------------------------------------------
# Constants (must match src/randomize/map_walker.rs and src/randomize/levels.rs)
# ---------------------------------------------------------------------------

WORLDS = [
    (0x19438, 21),  # W1
    (0x194BA, 47),  # W2
    (0x195D8, 52),  # W3
    (0x19714, 34),  # W4
    (0x197E4, 42),  # W5
    (0x198E4, 57),  # W6
    (0x19A3E, 46),  # W7
    (0x19B56, 41),  # W8
]

FORTRESS_ENTRIES = {
    (0, 11), (1, 13), (2, 13), (2, 34), (3, 9), (3, 16),
    (4, 12), (4, 31), (5, 9), (5, 27), (5, 48),
    (6, 5), (6, 40), (7, 7), (7, 10), (7, 26), (7, 36),
}

AIRSHIP_ENTRIES = {(0, 17), (1, 36), (2, 49), (3, 6), (4, 35), (5, 53), (6, 43)}

MAP_TRANSITIONS = {(4, 5)}

PAGE_A000_BY_TILESET = [11, 15, 21, 16, 17, 19, 18, 18, 18, 20, 23, 19, 17, 19, 13, 26, 26, 26, 9]


# ---------------------------------------------------------------------------
# Helper functions
# ---------------------------------------------------------------------------

def read_word(rom: bytes, offset: int) -> int:
    """Read a 16-bit little-endian word from ROM."""
    return rom[offset] | (rom[offset + 1] << 8)


def table_offsets(rowtype_offset: int, entry_count: int):
    """Compute sub-table file offsets: (scrcol, objsets, layouts)."""
    n = entry_count
    scrcol = rowtype_offset + n
    objsets = scrcol + n
    layouts = objsets + n * 2
    return scrcol, objsets, layouts


def is_level_pointer(obj_ptr: int, lay_ptr: int) -> bool:
    """Returns True if this is a real level pointer (not toad house, etc.)."""
    return obj_ptr >= 0xC000 and lay_ptr != 0x0000


def layout_file_offset(cpu_addr: int, tileset: int):
    """Convert layout CPU address + tileset to ROM file offset, or None."""
    if tileset >= len(PAGE_A000_BY_TILESET) or cpu_addr < 0xA000:
        return None
    bank = PAGE_A000_BY_TILESET[tileset]
    return bank * 0x2000 + 0x10 + (cpu_addr - 0xA000)


def level_screen_count(rom: bytes, layout_offset: int) -> int:
    """Read screen count from level header byte 4, bits 3-0 = (screens - 1)."""
    return (rom[layout_offset + 4] & 0x0F) + 1


# ---------------------------------------------------------------------------
# collect_shuffleable reimplementation
# ---------------------------------------------------------------------------

def collect_shuffleable(rom: bytes, world_idx: int, rowtype_offset: int, entry_count: int):
    """
    Reimplement the Rust collect_shuffleable function in Python.

    Returns:
        (final_indices, filter_log) where filter_log is a list of dicts
        describing what happened to each entry at each filter step.
    """
    _scrcol, objsets, layouts = table_offsets(rowtype_offset, entry_count)

    # Read all entries
    entries = []
    for i in range(entry_count):
        obj_ptr = read_word(rom, objsets + i * 2)
        lay_ptr = read_word(rom, layouts + i * 2)
        tileset = rom[rowtype_offset + i] & 0x0F
        entries.append({
            "index": i,
            "obj_ptr": obj_ptr,
            "lay_ptr": lay_ptr,
            "tileset": tileset,
        })

    # First pass: count (obj, lay) pair occurrences
    pair_counts = Counter()
    for e in entries:
        if is_level_pointer(e["obj_ptr"], e["lay_ptr"]):
            pair_counts[(e["obj_ptr"], e["lay_ptr"])] += 1

    # Second pass: apply filters in order, tracking exclusion reasons
    filter_log = []
    remaining = set(range(entry_count))

    # Filter 1: is_level_pointer
    excluded_1 = set()
    for i in range(entry_count):
        e = entries[i]
        if not is_level_pointer(e["obj_ptr"], e["lay_ptr"]):
            excluded_1.add(i)
    remaining -= excluded_1

    # Filter 2: NOT in AIRSHIP_ENTRIES
    excluded_2 = set()
    for i in list(remaining):
        if (world_idx, i) in AIRSHIP_ENTRIES:
            excluded_2.add(i)
    remaining -= excluded_2

    # Filter 3: NOT in MAP_TRANSITIONS
    excluded_3 = set()
    for i in list(remaining):
        if (world_idx, i) in MAP_TRANSITIONS:
            excluded_3.add(i)
    remaining -= excluded_3

    # Filter 4: (obj, lay) pair must be unique (count == 1)
    excluded_4 = set()
    for i in list(remaining):
        e = entries[i]
        if pair_counts[(e["obj_ptr"], e["lay_ptr"])] > 1:
            excluded_4.add(i)
    remaining -= excluded_4

    # Filter 5: NOT in FORTRESS_ENTRIES
    excluded_5 = set()
    for i in list(remaining):
        if (world_idx, i) in FORTRESS_ENTRIES:
            excluded_5.add(i)
    remaining -= excluded_5

    # Filter 6: screen count >= 3
    excluded_6 = set()
    for i in list(remaining):
        e = entries[i]
        lay_off = layout_file_offset(e["lay_ptr"], e["tileset"])
        if lay_off is None:
            excluded_6.add(i)
        elif level_screen_count(rom, lay_off) < 3:
            excluded_6.add(i)
    remaining -= excluded_6

    # Build per-entry log
    for i in range(entry_count):
        e = entries[i]
        reason = None
        if i in excluded_1:
            reason = "not level pointer"
        elif i in excluded_2:
            reason = "airship entry"
        elif i in excluded_3:
            reason = "map transition"
        elif i in excluded_4:
            pair = (e["obj_ptr"], e["lay_ptr"])
            reason = f"duplicate (obj,lay) pair (count={pair_counts[pair]})"
        elif i in excluded_5:
            reason = "fortress entry"
        elif i in excluded_6:
            lay_off = layout_file_offset(e["lay_ptr"], e["tileset"])
            if lay_off is None:
                reason = "layout offset unresolvable"
            else:
                sc = level_screen_count(rom, lay_off)
                reason = f"too short (screens={sc})"

        # Compute screen count for surviving entries
        screens = None
        if i not in excluded_1:
            lay_off = layout_file_offset(e["lay_ptr"], e["tileset"])
            if lay_off is not None:
                screens = level_screen_count(rom, lay_off)

        filter_log.append({
            "index": i,
            "obj_ptr": e["obj_ptr"],
            "lay_ptr": e["lay_ptr"],
            "tileset": e["tileset"],
            "screens": screens,
            "excluded_reason": reason,
            "shuffleable": i in remaining,
        })

    return sorted(remaining), filter_log, {
        "total": entry_count,
        "after_is_level": entry_count - len(excluded_1),
        "excluded_airship": len(excluded_2),
        "excluded_transition": len(excluded_3),
        "excluded_duplicate": len(excluded_4),
        "excluded_fortress": len(excluded_5),
        "excluded_short": len(excluded_6),
        "final": len(remaining),
    }


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    project_dir = os.path.dirname(script_dir)
    rom_path = os.path.join(project_dir, "roms/Super Mario Bros. 3 (USA) (Rev 1).nes")
    map_path = os.path.join(script_dir, "rom_map.json")

    if not os.path.exists(rom_path):
        print(f"ERROR: ROM not found at {rom_path}")
        sys.exit(1)
    if not os.path.exists(map_path):
        print(f"ERROR: rom_map.json not found at {map_path}")
        sys.exit(1)

    with open(rom_path, "rb") as f:
        rom = f.read()

    with open(map_path, "r") as f:
        rom_map = json.load(f)

    print(f"ROM size: {len(rom)} bytes")
    print(f"rom_map.json worlds: {len(rom_map['worlds'])}")
    print()

    total_discrepancies = 0

    for world_idx in range(8):
        rowtype_offset, entry_count = WORLDS[world_idx]
        world_name = f"World {world_idx + 1}"

        indices, log, stats = collect_shuffleable(rom, world_idx, rowtype_offset, entry_count)

        # Load ground truth from rom_map.json
        map_world = rom_map["worlds"][world_idx]
        map_entries = map_world["entries"]
        map_shuffleable = {e["index"] for e in map_entries if e.get("shuffleable")}

        print("=" * 78)
        print(f"  {world_name}  (rowtype=0x{rowtype_offset:05X}, entries={entry_count})")
        print("=" * 78)

        # Filter summary
        print(f"  Total entries:            {stats['total']}")
        print(f"  Pass is_level_pointer:    {stats['after_is_level']}")
        print(f"  Excluded airship:        -{stats['excluded_airship']}")
        print(f"  Excluded map transition: -{stats['excluded_transition']}")
        print(f"  Excluded duplicate pair: -{stats['excluded_duplicate']}")
        print(f"  Excluded fortress:       -{stats['excluded_fortress']}")
        print(f"  Excluded too short:      -{stats['excluded_short']}")
        print(f"  FINAL shuffleable:        {stats['final']}")
        print()

        # Show excluded entries with reasons
        excluded = [e for e in log if not e["shuffleable"]]
        if excluded:
            print(f"  --- Excluded entries ({len(excluded)}) ---")
            for e in excluded:
                sc_str = f"screens={e['screens']}" if e["screens"] is not None else ""
                print(f"    [{e['index']:2d}] obj=0x{e['obj_ptr']:04X} lay=0x{e['lay_ptr']:04X} "
                      f"ts={e['tileset']:2d} {sc_str:12s} => {e['excluded_reason']}")
            print()

        # Show final shuffleable set
        shuffleable_entries = [e for e in log if e["shuffleable"]]
        print(f"  --- Shuffleable entries ({len(shuffleable_entries)}) ---")
        for e in shuffleable_entries:
            sc_str = f"screens={e['screens']}" if e["screens"] is not None else ""
            print(f"    [{e['index']:2d}] obj=0x{e['obj_ptr']:04X} lay=0x{e['lay_ptr']:04X} "
                  f"ts={e['tileset']:2d} {sc_str}")
        print()

        # Cross-reference with rom_map.json
        my_set = set(indices)
        discrepancies = []

        # Check entries we say are shuffleable but rom_map says no
        for idx in sorted(my_set - map_shuffleable):
            map_entry = map_entries[idx]
            discrepancies.append(
                f"    [{idx:2d}] Python=YES, rom_map=NO  "
                f"(type={map_entry.get('type','?')}, obj=0x{map_entry['obj_ptr']:04X})"
            )

        # Check entries rom_map says are shuffleable but we say no
        for idx in sorted(map_shuffleable - my_set):
            map_entry = map_entries[idx]
            our_entry = log[idx]
            discrepancies.append(
                f"    [{idx:2d}] Python=NO,  rom_map=YES "
                f"(type={map_entry.get('type','?')}, obj=0x{map_entry['obj_ptr']:04X}, "
                f"reason={our_entry['excluded_reason']})"
            )

        if discrepancies:
            print(f"  *** DISCREPANCIES vs rom_map.json ({len(discrepancies)}) ***")
            for d in discrepancies:
                print(d)
            total_discrepancies += len(discrepancies)
        else:
            print(f"  rom_map.json cross-reference: MATCH ({len(my_set)} entries)")
        print()

    # Summary
    print("=" * 78)
    all_shuffleable = []
    for world_idx in range(8):
        rowtype_offset, entry_count = WORLDS[world_idx]
        indices, _, _ = collect_shuffleable(rom, world_idx, rowtype_offset, entry_count)
        all_shuffleable.append(len(indices))

    print(f"  Total shuffleable per world: {all_shuffleable}")
    print(f"  Grand total: {sum(all_shuffleable)}")
    print(f"  Total discrepancies vs rom_map.json: {total_discrepancies}")

    if total_discrepancies == 0:
        print("\n  All worlds MATCH rom_map.json ground truth.")
    else:
        print(f"\n  WARNING: {total_discrepancies} discrepancies found!")
        sys.exit(1)


if __name__ == "__main__":
    main()
