#!/usr/bin/env python3
"""Build a canary ROM for finding safe CHR tiles in pages 0x16/0x17.

Pages 0x16/0x17 are loaded ONLY as the world-map BG bank (R1 = $16, set at
file 0x3C52A and 0x3D178; never used as a sprite or level CHR source). So any
CHR data we put there only affects what's drawn on the 8 world maps.

This tool paints every CHR tile in 0x80-0xFF that no overworld metatile uses
with a bright "ID stamp" (the tile's hex ID drawn in dark pixels on a bright
fill). Walk every world map; any visible bright stamp = THAT tile is being
written directly to the nametable somewhere (status bar, panel border, etc.)
and is NOT safe to overwrite. Tiles whose stamps never appear are confirmed
safe.

For convenience the tool also:
- Applies the practice ROM's open-movement patches (PRG010-011 records of
  patches/smb3practice_SE.ips) so Mario can walk over level/lock/fortress tiles.
- Injects 3 warp whistles + 5 lives into the starting inventory via the same
  PRG031 trampoline used by src/randomize/qol.rs::write_starting_items.

Usage:
    python3 tools/canary_chr.py [source.nes [output.nes]]

Defaults: source = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes", output = canary.nes
"""

import sys
from pathlib import Path

# 3-wide x 5-tall hex digit glyphs ('#' = digit pixel).
FONT = {
    "0": ["###", "#.#", "#.#", "#.#", "###"],
    "1": [".#.", "##.", ".#.", ".#.", "###"],
    "2": ["##.", "..#", ".#.", "#..", "###"],
    "3": ["##.", "..#", ".##", "..#", "##."],
    "4": ["#.#", "#.#", "###", "..#", "..#"],
    "5": ["###", "#..", "##.", "..#", "##."],
    "6": [".##", "#..", "###", "#.#", "###"],
    "7": ["###", "..#", ".#.", "#..", "#.."],
    "8": ["###", "#.#", "###", "#.#", "###"],
    "9": ["###", "#.#", "###", "..#", "##."],
    "A": [".#.", "#.#", "###", "#.#", "#.#"],
    "B": ["##.", "#.#", "##.", "#.#", "##."],
    "C": [".##", "#..", "#..", "#..", ".##"],
    "D": ["##.", "#.#", "#.#", "#.#", "##."],
    "E": ["###", "#..", "##.", "#..", "###"],
    "F": ["###", "#..", "##.", "#..", "#.."],
}

CHR_BASE = 0x40010
METATILE_BANK_BASE = 0x18010  # overworld metatile bank 0x0C, 1024-byte quadrant table

# qol.rs offsets (mirror src/randomize/qol.rs::write_starting_items)
FS_STARTING_ITEMS = 0x3E260  # 28 bytes free in PRG031, mapped at CPU $E250
LIVES_INIT_BASE = 0x308E0    # 8 bytes in PRG024 to replace with JSR $E250 + NOPx5

PROJECT_ROOT = Path(__file__).resolve().parent.parent


def make_canary_tile(tid: int) -> bytes:
    """Return 16-byte CHR pattern: bright color-3 fill with the tile's hex ID
    stamped in dark color-0 pixels (two 3x5 glyphs, side by side)."""
    hi = f"{tid >> 4:X}"
    lo = f"{tid & 0xF:X}"
    rows = [0xFF] * 8  # all bright
    # High nibble glyph at (rows 1..5, cols 1..3)
    for r, line in enumerate(FONT[hi]):
        for c, ch in enumerate(line):
            if ch == "#":
                rows[r + 1] &= ~(1 << (7 - (c + 1))) & 0xFF
    # Low nibble glyph at (rows 1..5, cols 5..7)
    for r, line in enumerate(FONT[lo]):
        for c, ch in enumerate(line):
            if ch == "#":
                rows[r + 1] &= ~(1 << (7 - (c + 5))) & 0xFF
    # Both bit planes identical -> dark pixels = color 0, bright = color 3.
    return bytes(rows) + bytes(rows)


def find_metatile_unused_chr_tiles(rom: bytes) -> list[int]:
    """CHR tile IDs in 0x80-0xFF that no overworld metatile (bank 0x0C) uses."""
    base = METATILE_BANK_BASE
    quads = [
        rom[base : base + 256],
        rom[base + 256 : base + 512],
        rom[base + 512 : base + 768],
        rom[base + 768 : base + 1024],
    ]
    used = set()
    for mt in range(256):
        for q in quads:
            used.add(q[mt])
    return [t for t in range(0x80, 0x100) if t not in used]


def chr_offset(tid: int) -> int:
    """File offset of CHR tile `tid` in the BG-bank-2 region (page 0x16 or 0x17)."""
    if tid < 0xC0:
        return CHR_BASE + 0x16 * 0x400 + (tid - 0x80) * 16
    return CHR_BASE + 0x17 * 0x400 + (tid - 0xC0) * 16


def apply_ips_subset(rom: bytearray, ips_path: Path, start: int, end: int) -> int:
    """Apply IPS records whose target offset falls in [start, end) to `rom`."""
    data = ips_path.read_bytes()
    if data[:5] != b"PATCH":
        sys.exit(f"ERROR: bad IPS header in {ips_path}")
    pos = 5
    applied = 0
    while data[pos : pos + 3] != b"EOF":
        offset = (data[pos] << 16) | (data[pos + 1] << 8) | data[pos + 2]
        pos += 3
        size = (data[pos] << 8) | data[pos + 1]
        pos += 2
        if size == 0:
            rle_len = (data[pos] << 8) | data[pos + 1]
            value = data[pos + 2]
            pos += 3
            payload = bytes([value] * rle_len)
        else:
            payload = data[pos : pos + size]
            pos += size
        if start <= offset < end:
            if offset + len(payload) > len(rom):
                sys.exit(f"ERROR: record at 0x{offset:06X} (+{len(payload)}) overruns ROM")
            rom[offset : offset + len(payload)] = payload
            applied += 1
    return applied


def inject_starting_items(rom: bytearray, lives: int = 5, items=(0x0C, 0x0C, 0x0C)) -> None:
    """Lives + intro skip + up to 3 inventory items via PRG031 trampoline.
    Mirrors src/randomize/qol.rs::write_starting_items exactly."""
    lives = max(1, min(99, lives))
    buf = bytearray()
    buf += bytes([0xA9, lives])               # LDA #lives
    buf += bytes([0x8D, 0x36, 0x07])          # STA $0736
    buf += bytes([0x8D, 0x37, 0x07])          # STA $0737
    buf += bytes([0xA9, 0x06])                # LDA #$06
    buf += bytes([0x85, 0xDE])                # STA $DE  (Title_State = IntroSkip)
    for i, item in enumerate(items[:3]):
        buf += bytes([0xA9, item, 0x8D, 0x80 + i, 0x7D])  # STA $7D80+i
    buf += bytes([0x60])                      # RTS
    if len(buf) > 33:
        sys.exit(f"ERROR: trampoline {len(buf)} bytes exceeds FS_STARTING_ITEMS budget (33)")
    rom[FS_STARTING_ITEMS : FS_STARTING_ITEMS + len(buf)] = buf
    rom[LIVES_INIT_BASE : LIVES_INIT_BASE + 8] = bytes(
        [0x20, 0x50, 0xE2, 0xEA, 0xEA, 0xEA, 0xEA, 0xEA]  # JSR $E250 + NOPx5
    )


def main():
    args = sys.argv[1:]
    src = Path(args[0]) if len(args) >= 1 else PROJECT_ROOT / "roms/Super Mario Bros. 3 (USA) (Rev 1).nes"
    dst = Path(args[1]) if len(args) >= 2 else PROJECT_ROOT / "canary.nes"
    if len(args) > 2:
        sys.exit("Usage: canary_chr.py [source.nes [output.nes]]")

    print(f"Source: {src}")
    rom = bytearray(src.read_bytes())
    if len(rom) != 393232:
        print(f"WARNING: source size {len(rom)} != expected 393232 bytes")

    print("Applying open-movement patches (PRG010-011 of patches/smb3practice_SE.ips)...")
    n = apply_ips_subset(rom, PROJECT_ROOT / "patches/smb3practice_SE.ips", 0x14010, 0x18010)
    print(f"  {n} records applied")

    print("Injecting 5 lives + 3 warp whistles into starting inventory...")
    inject_starting_items(rom, lives=5, items=(0x0C, 0x0C, 0x0C))

    candidates = find_metatile_unused_chr_tiles(bytes(rom))
    print(f"\nPainting {len(candidates)} candidate CHR tiles with hex-ID stamps:")
    for tid in candidates:
        page = "0x16" if tid < 0xC0 else "0x17"
        off = chr_offset(tid)
        rom[off : off + 16] = make_canary_tile(tid)
        print(f"  0x{tid:02X} (page {page}, file 0x{off:05X})")

    dst.write_bytes(rom)
    print(f"\nWrote {dst}.")
    print()
    print("Walk every world map (use the 3 whistles to skip across worlds).")
    print("Any tile that appears as a bright block with a 2-character HEX stamp")
    print("is being written DIRECTLY to the nametable somewhere = NOT safe.")
    print("Tiles whose IDs never appear anywhere = confirmed safe to overwrite.")


if __name__ == "__main__":
    main()
