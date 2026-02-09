#!/usr/bin/env python3
"""
SMB3 Level Tile Simulator

Simulates the SMB3 level generator system to produce a tile grid
showing exactly which tile IDs end up at each position in a level.

Usage: python3 tools/level_sim.py [rom_path] [level_offset_hex]
  Default: "Super Mario Bros. 3 (USA) (Rev 1).nes" at 0x1FB92 (World 1-1)
"""

import os
import sys

# --- Constants ---

# Tile memory: each screen is 27 rows x 16 columns = 432 ($1B0) bytes
SCREEN_ROWS = 27
SCREEN_COLS = 16
SCREEN_SIZE = 0x1B0  # 432
MAX_SCREENS = 15

# Tile_Mem_Addr base offsets (from prg030.asm)
TILE_MEM_BASES = [
    i * SCREEN_SIZE for i in range(MAX_SCREENS)
]

# LL_PowerBlocks table (ROM offset 0x1CAD4, 24 bytes)
LL_POWER_BLOCKS = [
    0x60, 0x61, 0x62, 0x64, 0x65, 0x66, 0x68, 0x69,
    0x6A, 0x6C, 0x6D, 0x6E, 0x6F, 0x70, 0x44, 0x45,
    0x03, 0x2F, 0x30, 0x31, 0x73, 0x74, 0x75, 0x46,
]

# LoadLevel_Blocks table (BRICK, QBLOCKCOIN, BRICKCOIN, WOODBLOCK,
#   GNOTE, NOTE, WOODBLOCKBOUNCE, COIN, ICEBRICK)
LOAD_LEVEL_BLOCKS = [0x67, 0x63, 0x6B, 0x79, 0xBC, 0x2E, 0x72, 0x40, 0x32]

# Ground tiles (TS1)
TILE1_GROUNDTL = 0x55
TILE1_GROUNDTM = 0x53
TILE1_GROUNDTR = 0x57
TILE1_GROUNDML = 0x56
TILE1_GROUNDMM = 0x54
TILE1_GROUNDMR = 0x58
TILE1_SKY = 0x80

# Variable-size dispatch indices that read an extra byte (TS1)
EXTRA_BYTE_DISPATCHES = {11, 12, 35, 36, 37, 38, 39, 40, 41, 42}

# Variable-size base offsets per group
VAR_BASES = [0, 15, 30, 45, 60, 75, 90, 105]

# Tile name lookup for powerup-related tiles
TILE_NAMES = {
    0x60: "QBLK-FLOWER", 0x61: "QBLK-LEAF", 0x62: "QBLK-STAR",
    0x63: "QBLK-COIN", 0x64: "QBLK-COIN/S", 0x65: "QBLK-COIN2",
    0x66: "MUNCHER", 0x67: "BRICK", 0x68: "BRICK-FLOWER",
    0x69: "BRICK-LEAF", 0x6A: "BRICK-STAR", 0x6B: "BRICK-COIN",
    0x6C: "BRICK-COINS", 0x6D: "BRICK-10C", 0x6E: "BRICK-1UP",
    0x6F: "BRICK-VINE", 0x70: "BRICK-PSWI",
    0x44: "INVIS-COIN", 0x45: "INVIS-1UP",
    0x53: "GROUND-TM", 0x54: "GROUND-MM", 0x55: "GROUND-TL",
    0x56: "GROUND-ML", 0x57: "GROUND-TR", 0x58: "GROUND-MR",
    0x79: "WOODBLOCK", 0xBC: "GNOTE", 0x2E: "NOTE",
    0x72: "WOOD-BOUNCE", 0x40: "COIN", 0x32: "ICEBRICK",
    0x80: "SKY",
}

POWER_NAMES = [
    "QBLOCKFLOWER", "QBLOCKLEAF", "QBLOCKSTAR", "QBLOCKCOINSTAR",
    "QBLOCKCOIN2", "MUNCHER", "BRICKFLOWER", "BRICKLEAF",
    "BRICKSTAR", "BRICKCOINSTAR", "BRICK10COIN", "BRICK1UP",
    "BRICKVINE", "BRICKPSWITCH", "INVISCOIN", "INVIS1UP",
    "NOTEINVIS", "NOTEFLOWER", "NOTELEAF", "NOTESTAR",
    "WOODBLOCKFLOWER", "WOODBLOCKLEAF", "WOODBLOCKSTAR", "NOTECOINHEAVEN",
    "PSWITCH"
]


class TileMemory:
    """Simulates the NES tile memory layout."""

    def __init__(self, num_screens=12):
        self.num_screens = num_screens
        # Flat array mimicking NES tile memory
        self.mem = bytearray(num_screens * SCREEN_SIZE + 0x200)  # extra space for hi flag
        # Initialize to sky
        for i in range(len(self.mem)):
            self.mem[i] = TILE1_SKY

    def addr_for_screen(self, screen):
        """Get base address for a screen number."""
        if screen < MAX_SCREENS:
            return TILE_MEM_BASES[screen]
        return TILE_MEM_BASES[-1]

    def write(self, base_addr, offset, tile_id):
        """Write a tile to memory at base_addr + offset."""
        addr = base_addr + offset
        if 0 <= addr < len(self.mem):
            self.mem[addr] = tile_id

    def read(self, base_addr, offset):
        """Read a tile from memory."""
        addr = base_addr + offset
        if 0 <= addr < len(self.mem):
            return self.mem[addr]
        return TILE1_SKY

    def get_grid(self, screen):
        """Get a 2D grid (rows x cols) for a screen."""
        base = self.addr_for_screen(screen)
        grid = []
        for row in range(SCREEN_ROWS):
            row_tiles = []
            for col in range(SCREEN_COLS):
                offset = (row << 4) | col
                addr = base + offset
                if addr < len(self.mem):
                    row_tiles.append(self.mem[addr])
                else:
                    row_tiles.append(TILE1_SKY)
            grid.append(row_tiles)
        return grid


class LevelSimulator:
    """Simulates the SMB3 level generator for one level."""

    def __init__(self, rom_data, level_offset):
        self.rom = rom_data
        self.level_offset = level_offset
        self.commands = []
        self.tile_mem = None
        self.num_screens = 12  # default

    def parse_header(self):
        """Parse the 9-byte level header."""
        off = self.level_offset
        header = self.rom[off:off+9]
        # Byte 4 lower nibble = level width in screens
        self.num_screens = (header[4] & 0x0F) + 1
        return header

    def parse_commands(self):
        """Parse all generator commands with correct variable-length handling."""
        off = self.level_offset + 9  # skip header
        self.commands = []

        while off < len(self.rom) and self.rom[off] != 0xFF:
            byte0 = self.rom[off]
            byte1 = self.rom[off + 1]
            byte2 = self.rom[off + 2]

            group = (byte0 & 0xE0) >> 5
            row = byte0 & 0x0F
            hi = (byte0 >> 4) & 1
            screen = (byte1 >> 4) & 0x0F
            col = byte1 & 0x0F

            cmd = {
                "offset": off,
                "byte0": byte0, "byte1": byte1, "byte2": byte2,
                "group": group, "row": row, "hi": hi,
                "screen": screen, "col": col,
                "extra_byte": None,
                "cmd_len": 3,
            }

            if group == 7:
                cmd["type"] = "junction"
            elif (byte2 & 0xF0) == 0:
                cmd["type"] = "fixed"
                cmd["fixed_idx"] = ((byte0 & 0xE0) >> 1) + byte2
            else:
                cmd["type"] = "variable"
                var_type = byte2 >> 4
                cmd["var_type"] = var_type
                cmd["width"] = byte2 & 0x0F
                cmd["dispatch"] = VAR_BASES[group] + var_type - 1

                if cmd["dispatch"] in EXTRA_BYTE_DISPATCHES:
                    cmd["extra_byte"] = self.rom[off + 3]
                    cmd["cmd_len"] = 4

            self.commands.append(cmd)
            off += cmd["cmd_len"]

        return self.commands

    def calc_tile_addr(self, cmd):
        """Calculate tile memory base address and offset from command fields."""
        screen = cmd["screen"]
        base_addr = TILE_MEM_BASES[min(screen, MAX_SCREENS - 1)]
        tile_off = (cmd["row"] << 4) | cmd["col"]

        if cmd["hi"]:
            base_addr += 0x100

        return base_addr, tile_off

    def next_column(self, base_addr, y):
        """Advance to next column, handling screen boundaries."""
        y += 1
        if (y & 0x0F) == 0:
            base_addr += SCREEN_SIZE
            y = y & 0xF0  # keep row, reset column to 0
        return base_addr, y

    def next_row(self, base_addr, y):
        """Advance to next row (add 16 to offset)."""
        y += 16
        if y >= 0x100:
            y -= 0x100
            base_addr += 1  # increment high byte
        return base_addr, y

    def execute(self):
        """Execute all parsed commands, building the tile grid."""
        self.tile_mem = TileMemory(self.num_screens)

        for i, cmd in enumerate(self.commands):
            if cmd["type"] == "junction":
                continue
            elif cmd["type"] == "fixed":
                self._exec_fixed(cmd, i)
            elif cmd["type"] == "variable":
                self._exec_variable(cmd, i)

    def _exec_fixed(self, cmd, cmd_idx):
        """Execute a fixed-size generator command."""
        group = cmd["group"]
        fixed_idx = cmd["fixed_idx"]
        base_addr, tile_off = self.calc_tile_addr(cmd)

        if group == 1 and 16 <= fixed_idx <= 40:
            # LoadLevel_PowerBlock
            power_idx = fixed_idx - 16
            if power_idx < len(LL_POWER_BLOCKS):
                tile_id = LL_POWER_BLOCKS[power_idx]
                self.tile_mem.write(base_addr, tile_off, tile_id)
        # Other fixed generators (group 0 bushes, group 2 end goal) — skip for now

    def _exec_variable(self, cmd, cmd_idx):
        """Execute a variable-size generator command."""
        dispatch = cmd["dispatch"]
        base_addr, tile_off = self.calc_tile_addr(cmd)
        width = cmd["width"]

        if 15 <= dispatch <= 22:
            self._exec_block_run(cmd, base_addr, tile_off)
        elif dispatch in (11, 12):
            self._exec_ground_run(cmd, base_addr, tile_off)
        elif 0 <= dispatch <= 3:
            self._exec_big_block(cmd, base_addr, tile_off)
        elif 4 <= dispatch <= 7:
            self._exec_floating_big_block(cmd, base_addr, tile_off)
        elif dispatch == 8:
            self._exec_little_bush_run(cmd, base_addr, tile_off)
        elif dispatch == 9:
            self._exec_pitfall(cmd, base_addr, tile_off)
        elif dispatch == 10:
            self._exec_little_cloud_run(cmd, base_addr, tile_off)
        elif dispatch == 13:
            self._exec_cloud_run(cmd, base_addr, tile_off)
        elif 23 <= dispatch <= 25:
            self._exec_pipe(cmd, base_addr, tile_off)
        # Others: skip (bytes already consumed correctly)

    def _exec_block_run(self, cmd, base_addr, tile_off):
        """LoadLevel_BlockRun: place a horizontal run of identical blocks."""
        byte2 = cmd["byte2"]
        block_x = (byte2 - 0x10) >> 4
        width = byte2 & 0x0F

        if 0 <= block_x < len(LOAD_LEVEL_BLOCKS):
            tile_id = LOAD_LEVEL_BLOCKS[block_x]
        else:
            return

        y = tile_off
        for _ in range(width + 1):
            self.tile_mem.write(base_addr, y, tile_id)
            base_addr, y = self.next_column(base_addr, y)

    def _exec_ground_run(self, cmd, base_addr, tile_off):
        """LoadLevel_GroundRun: place ground with edges, extending downward."""
        extra_width = cmd["extra_byte"]  # width from extra byte
        height = cmd["width"]  # lower nibble of byte2
        is_underwater = (cmd["dispatch"] == 12)

        if is_underwater:
            top = [0xF6, 0xF4, 0xF8]   # UW ground TL, TM, TR
            mid = [0xF7, 0xF5, 0xF9]   # UW ground ML, MM, MR
        else:
            top = [TILE1_GROUNDTL, TILE1_GROUNDTM, TILE1_GROUNDTR]
            mid = [TILE1_GROUNDML, TILE1_GROUNDMM, TILE1_GROUNDMR]

        if extra_width is None:
            return

        # Save starting position
        save_base = base_addr
        save_off = tile_off

        # Place top row: left edge, middle fill, right edge
        y = tile_off
        ba = base_addr

        # Left edge
        self.tile_mem.write(ba, y, top[0])
        ba, y = self.next_column(ba, y)

        # Middle tiles
        for _ in range(max(0, extra_width - 1)):
            self.tile_mem.write(ba, y, top[1])
            ba, y = self.next_column(ba, y)

        # Right edge
        if extra_width > 0:
            self.tile_mem.write(ba, y, top[2])

        # Now fill rows below (height times)
        for h in range(height):
            # Move to next row
            save_base, save_off = self.next_row(save_base, save_off)
            ba = save_base
            y = save_off

            # Left edge
            self.tile_mem.write(ba, y, mid[0])
            ba, y = self.next_column(ba, y)

            # Middle
            for _ in range(max(0, extra_width - 1)):
                self.tile_mem.write(ba, y, mid[1])
                ba, y = self.next_column(ba, y)

            # Right edge
            if extra_width > 0:
                self.tile_mem.write(ba, y, mid[2])

    def _exec_big_block(self, cmd, base_addr, tile_off):
        """LoadLevel_GenerateBigBlock: colored block columns extending to ground.
        Simplified: just place top tile and mark column."""
        # This is complex (searches downward for ground). Placeholder: mark top position.
        color = cmd["dispatch"]  # 0=white, 1=orange, 2=green, 3=blue
        width = cmd["width"]

        # Simplified: place a marker tile for each column
        ba = base_addr
        y = tile_off
        for _ in range(width + 1):
            self.tile_mem.write(ba, y, 0x03 + color)  # placeholder tile
            ba, y = self.next_column(ba, y)

    def _exec_floating_big_block(self, cmd, base_addr, tile_off):
        """LoadLevel_FloatingBigBlock: 2-row floating block."""
        width = cmd["width"]
        ba = base_addr
        y = tile_off

        # Top row
        for _ in range(width + 1):
            self.tile_mem.write(ba, y, 0x08)  # placeholder
            ba, y = self.next_column(ba, y)

        # Bottom row
        ba2, y2 = self.next_row(base_addr, tile_off)
        for _ in range(width + 1):
            self.tile_mem.write(ba2, y2, 0x09)  # placeholder
            ba2, y2 = self.next_column(ba2, y2)

    def _exec_little_bush_run(self, cmd, base_addr, tile_off):
        """LoadLevel_LittleBushRun: decorative bushes. Skip tile placement."""
        pass

    def _exec_pitfall(self, cmd, base_addr, tile_off):
        """LoadLevel_Pitfall: gap in ground. Replace ground tiles with sky."""
        width = cmd["width"]
        ba = base_addr
        y = tile_off

        for _ in range(width + 1):
            # Clear downward until we run out of rows
            ba_col = ba
            y_col = y
            for _ in range(SCREEN_ROWS):
                self.tile_mem.write(ba_col, y_col, TILE1_SKY)
                ba_col, y_col = self.next_row(ba_col, y_col)
            ba, y = self.next_column(ba, y)

    def _exec_little_cloud_run(self, cmd, base_addr, tile_off):
        """LoadLevel_LittleCloudRun: decorative. Skip."""
        pass

    def _exec_cloud_run(self, cmd, base_addr, tile_off):
        """LoadLevel_CloudRun: big clouds. Skip."""
        pass

    def _exec_pipe(self, cmd, base_addr, tile_off):
        """LoadLevel_VGroundPipeRun: vertical pipe. Simplified placeholder."""
        pass

    def print_command_log(self):
        """Print detailed log of all parsed commands."""
        print("=" * 100)
        print("COMMAND LOG")
        print("=" * 100)
        fmt = "{:>3} {:>6} {:12s} {:5s} {:>3} {:>3} {:>2} {:>3} {:>3}  {}"
        print(fmt.format("#", "Offset", "Bytes", "Type", "Grp", "Row", "Hi", "Scr", "Col", "Description"))
        print("-" * 100)

        for i, cmd in enumerate(self.commands):
            b = "%02X %02X %02X" % (cmd["byte0"], cmd["byte1"], cmd["byte2"])
            if cmd["extra_byte"] is not None:
                b += " %02X" % cmd["extra_byte"]

            if cmd["type"] == "junction":
                desc = "Junction"
            elif cmd["type"] == "fixed":
                idx = cmd["fixed_idx"]
                grp = cmd["group"]
                if grp == 1 and 16 <= idx <= 40:
                    desc = "POWER: " + POWER_NAMES[idx - 16]
                elif grp == 2 and idx == 41:
                    desc = "EndGoal"
                elif grp == 0:
                    g0 = {0: "MidBush", 1: "SmallBush", 2: "BigBush", 3: "RandClouds",
                          4: "Door2", 5: "Door1", 6: "Vine", 7: "BGCloud"}
                    desc = "Grp0: " + g0.get(idx, "idx=%d" % idx)
                else:
                    desc = "Fixed grp=%d idx=%d" % (grp, idx)
            else:
                d = cmd["dispatch"]
                w = cmd["width"]
                dispatch_names = {
                    0: "BigBlk-W", 1: "BigBlk-O", 2: "BigBlk-G", 3: "BigBlk-B",
                    4: "FltBlk-W", 5: "FltBlk-O", 6: "FltBlk-G", 7: "FltBlk-B",
                    8: "BushRun", 9: "Pitfall", 10: "CloudSmall", 11: "GroundRun",
                    12: "GroundRunUW", 13: "CloudRun", 14: "PitfallUW",
                    15: "Run:BRICK", 16: "Run:QCOIN", 17: "Run:BRICKCOIN",
                    18: "Run:WOOD", 19: "Run:GNOTE", 20: "Run:NOTE",
                    21: "Run:WOODBNC", 22: "Run:COIN", 23: "VPipe1", 24: "VPipe2",
                    25: "VPipe3",
                }
                name = dispatch_names.get(d, "disp=%d" % d)
                desc = "%s w=%d" % (name, w)
                if cmd["extra_byte"] is not None:
                    desc += " extra=$%02X" % cmd["extra_byte"]

            print(fmt.format(
                i, "0x%04X" % cmd["offset"], b, cmd["type"][:5],
                cmd["group"], cmd["row"], cmd["hi"],
                cmd["screen"], cmd["col"], desc
            ))

        print("\nTotal: %d commands" % len(self.commands))

    def print_powerup_summary(self):
        """Print summary of all powerup-related tiles in the grid."""
        print("\n" + "=" * 80)
        print("POWERUP TILE SUMMARY")
        print("=" * 80)

        # Scan tile memory for powerup tiles ($60-$70, $44-$45)
        powerup_tiles = set(range(0x60, 0x71)) | {0x44, 0x45}
        found = []

        for scr in range(self.num_screens):
            grid = self.tile_mem.get_grid(scr)
            for row in range(SCREEN_ROWS):
                for col in range(SCREEN_COLS):
                    tile = grid[row][col]
                    if tile in powerup_tiles:
                        name = TILE_NAMES.get(tile, "$%02X" % tile)
                        found.append((scr, row, col, tile, name))

        if found:
            print("\n  {:>3} {:>3} {:>3}  {:>4}  {}".format("Scr", "Row", "Col", "Tile", "Name"))
            print("  " + "-" * 40)
            for scr, row, col, tile, name in found:
                print("  {:>3} {:>3} {:>3}  ${:02X}  {}".format(scr, row, col, tile, name))
        else:
            print("\n  No powerup tiles found in tile memory.")

        # Also list from command log
        print("\n  From command parsing:")
        for i, cmd in enumerate(self.commands):
            if cmd["type"] == "fixed" and cmd["group"] == 1:
                idx = cmd["fixed_idx"]
                if 16 <= idx <= 40:
                    print("    cmd %2d: %s at scr=%d row=%d hi=%d col=%d" % (
                        i, POWER_NAMES[idx - 16], cmd["screen"], cmd["row"],
                        cmd["hi"], cmd["col"]))

    def print_tile_grid(self, screens=None):
        """Print ASCII tile grid for specified screens."""
        if screens is None:
            screens = range(self.num_screens)

        print("\n" + "=" * 80)
        print("TILE GRID")
        print("=" * 80)

        for scr in screens:
            grid = self.tile_mem.get_grid(scr)
            has_content = any(
                grid[r][c] != TILE1_SKY
                for r in range(SCREEN_ROWS) for c in range(SCREEN_COLS)
            )
            if not has_content:
                continue

            print("\n--- Screen %d ---" % scr)
            # Column headers
            print("     ", end="")
            for c in range(SCREEN_COLS):
                print(" %X " % c, end="")
            print()

            for row in range(SCREEN_ROWS):
                # Only print rows that have non-sky content
                if all(grid[row][c] == TILE1_SKY for c in range(SCREEN_COLS)):
                    continue
                print("  %2d " % row, end="")
                for col in range(SCREEN_COLS):
                    tile = grid[row][col]
                    if tile == TILE1_SKY:
                        print(" . ", end="")
                    elif 0x60 <= tile <= 0x65:
                        # Q-block: highlight
                        print("[%02X]" % tile, end="")
                    elif 0x66 <= tile <= 0x70:
                        # Brick powerup: highlight
                        print("<%02X>" % tile, end="")
                    elif tile in (0x44, 0x45):
                        # Invisible block
                        print("(%02X)" % tile, end="")
                    else:
                        print(" %02X" % tile, end="")
                print()


def main():
    rom_path = "Super Mario Bros. 3 (USA) (Rev 1).nes"
    level_offset = 0x1FB92  # World 1-1

    if len(sys.argv) >= 2:
        rom_path = sys.argv[1]
    if len(sys.argv) >= 3:
        level_offset = int(sys.argv[2], 16)

    if not os.path.exists(rom_path):
        print("Error: ROM file not found: %s" % rom_path)
        print("Usage: python3 tools/level_sim.py [rom_path] [level_offset_hex]")
        sys.exit(1)

    with open(rom_path, "rb") as f:
        rom_data = f.read()

    print("SMB3 Level Tile Simulator")
    print("ROM: %s (%d bytes)" % (rom_path, len(rom_data)))
    print("Level offset: 0x%05X" % level_offset)

    sim = LevelSimulator(rom_data, level_offset)
    header = sim.parse_header()
    print("Header: %s" % " ".join("%02X" % b for b in header))
    print("Level width: %d screens" % sim.num_screens)

    sim.parse_commands()
    sim.print_command_log()

    sim.execute()
    sim.print_powerup_summary()
    sim.print_tile_grid()


if __name__ == "__main__":
    main()
