#!/usr/bin/env python3
"""Search for level 1-3 data in the SMB3 ROM."""

import sys

rom_path = "Super Mario Bros. 3 (USA) (Rev 1).nes"
rom = open(rom_path, "rb").read()

def hexdump(data):
    return " ".join("{:02X}".format(b) for b in data)

# Verify 1-1 enemy data as baseline
print("=== 1-1 baseline (bank 14, CPU C527, file 0x1C537) ===")
print(hexdump(rom[0x1C537:0x1C537+32]))
print()

# The ObjSets pointer for 1-3 is 0xC2EE (CPU address)
# In MMC3, the C000-DFFF window is 8KB. Offset within window = 0xC2EE - 0xC000 = 0x02EE
# For 1-1, CPU C527 -> offset 0x0527 within bank, bank 14 -> file 14*0x2000+0x10+0x0527 = 0x1C537 OK

# Search all 32 banks at offset 0x02EE (C000-based)
print("=== Search: offset 0x02EE in each bank (CPU C2EE -> C000 window) ===")
for bank in range(32):
    foff = bank * 0x2000 + 0x10 + 0x02EE
    if foff + 16 <= len(rom):
        data = rom[foff:foff+16]
        if any(b != 0xFF for b in data):
            print("  Bank {:2d}: file 0x{:05X}: {}".format(bank, foff, hexdump(data)))

print()

# Search all 32 banks at offset 0x22EE (A000-based)
print("=== Search: offset 0x22EE in each bank (CPU C2EE -> A000 window) ===")
for bank in range(32):
    foff = bank * 0x2000 + 0x10 + 0x22EE
    if foff + 16 <= len(rom):
        data = rom[foff:foff+16]
        if any(b != 0xFF for b in data):
            print("  Bank {:2d}: file 0x{:05X}: {}".format(bank, foff, hexdump(data)))

print()

# Maybe the pointer 0xC2EE is not a CPU address but a direct pointer into the bank?
# Or maybe the pointer is a 2-byte value AT address 0xC2EE that points elsewhere?
# Let's check: In SMB3, the world map level layout table has pointers.
# The ObjSets value might be a pointer TO data, not the data itself.

# Let's check if 0xC2EE is an address containing a pointer
# First, what's the 1-1 pattern? obj=C527 and data IS at CPU C527 in bank 14
# So for 1-1: the pointer IS the data address. Same bank (14).

# For 1-3 with obj=C2EE: data should be at CPU C2EE in bank 14
# But bank 14 at that offset is all FF.
# Maybe the data was assembled into a DIFFERENT bank for tileset 1?

# Let me check what's around the level data area in various banks
# 1-1 is in bank 14. Let's see what other levels from world 1 look like.

# First let's understand the world 1 map data structure
# The world map stores level header pointers. Let's find where world 1's level list is.

# Actually - let me check if the ObjSets pointer might be 2 bytes at that location
# that redirect to actual data. For 1-1, C527 in bank 14:
print("=== Checking if pointers are indirect ===")
# Read 2 bytes at 1-1's pointer location
off_1_1 = 14 * 0x2000 + 0x10 + 0x0527
val = rom[off_1_1] | (rom[off_1_1+1] << 8)
print("At 1-1 pointer (0x{:05X}): first 2 bytes = 0x{:04X}".format(off_1_1, val))
# If C527 were indirect, it would read a pointer from C527.
# But we know the data IS at C527 (01 08 5B...), so it's direct.

print()

# Let me check: maybe the C000 bank for 1-3 is NOT bank 14?
# The PAGE_C000_ByTileset table says tileset 1 -> bank 14
# But what if 1-3 uses a different tileset than 1?

# Let me look at the disassembly data. The world map level table typically stores:
# - Object set (tileset) number
# - Pointer high/low for layout data
# - Pointer high/low for object/enemy data

# Let me search the ROM reference doc
print("=== Let me search for the world 1 level pointer table ===")

# In SMB3 disassembly, world map level data is in the Map_Objects area
# Each level entry on the world map has associated data including pointers

# Actually, let me approach this differently. Let me look for the level_sim.py
# tool that the project already has, which may know how to find level data.
print()

# Let me try ALL possible file offsets for the raw 16-bit value 0xC2EE
# If 0xC2EE appears as a pointer somewhere, we can trace it
print("=== Searching for bytes EE C2 (little-endian 0xC2EE) in ROM ===")
target = bytes([0xEE, 0xC2])
pos = 0
count = 0
while True:
    pos = rom.find(target, pos)
    if pos == -1:
        break
    bank = (pos - 0x10) // 0x2000
    bank_off = (pos - 0x10) % 0x2000
    context = rom[max(0,pos-4):pos+6]
    print("  File 0x{:05X} (bank {:2d}, +0x{:04X}): context = {}".format(
        pos, bank, bank_off, hexdump(context)))
    pos += 1
    count += 1
    if count > 30:
        print("  ... (truncated)")
        break

print()

# Now let me look at what's actually around the 1-1 data to understand structure
# In bank 14, where does 1-1 enemy data end? Find the FF terminator
print("=== 1-1 enemy data extent in bank 14 ===")
off = 0x1C537
i = 0
while off + i < len(rom) and rom[off + i] != 0xFF:
    i += 1
print("1-1 enemy data: 0x{:05X} to 0x{:05X} ({} bytes + FF terminator)".format(
    0x1C537, 0x1C537 + i, i))
print(hexdump(rom[0x1C537:0x1C537+i+1]))
print()

# What comes before 1-1 enemy data? Maybe 1-3 is before it?
print("=== Data before 1-1 enemy area (looking for other level data) ===")
# Look backwards from 1-1
for scan_start in range(0x1C537 - 1, 0x1C537 - 200, -1):
    if rom[scan_start] == 0xFF:
        # This could be a terminator for the previous block
        block_data = rom[scan_start+1:0x1C537]
        if len(block_data) > 2:
            print("  Block at 0x{:05X}-0x{:05X} ({} bytes): {}".format(
                scan_start+1, 0x1C537-1, len(block_data), hexdump(block_data[:32])))
            break

print()

# Let me also check what the level_sim.py knows
print("=== Checking level_sim.py for address mapping ===")
