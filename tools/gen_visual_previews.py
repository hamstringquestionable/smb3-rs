#!/usr/bin/env python3
"""Render a sprite preview PNG for every bundled visual IPS patch.

Each visual patch in web/visual-patches/ is applied to a clean ROM, and the
SAME player frame (big Mario, standing) is rendered from the patched CHR +
player palette. The web app shows these next to the patch pills so players can
see the re-skin before generating.

Exception: a patch listed in ENEMY_OVERRIDES re-skins enemies rather than (or
as well as) the player, so its player sprite would be a duplicate of another
pill's. Those render one of the enemy sprites the patch actually changes.

Output: web/assets/visual-previews/<ips-stem>.png (plus vanilla.png for the
"None" pill). Transparent background — the page scales them up with
image-rendering: pixelated.

Usage:
    nix-shell -p python3Packages.pillow --run "python3 tools/gen_visual_previews.py"

The ROM is never written to the repo; only the rendered sprites are.
"""

import os
import sys
from PIL import Image

ROM_PATH = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes"
PATCH_DIR = "web/visual-patches"
OUT_DIR = "web/assets/visual-previews"

CHR_BASE = 0x40010

# Player sprite palette: [bg mirror, body, highlight, outline] for
# Small/Big/Raccoon Mario (mirrors PALETTE_RANGES[0] in src/randomize/palettes.rs).
MARIO_PALETTE = 0x10539

# PRG029 ($C000-$DFFF) holds the player frame tables (Southbird disasm):
#   SPPF_Offsets ($CC00) — one byte per Player_Frame, offset/2 into SPPF_Table
#   SPPF_Table   ($CC51) — six patterns per frame; the routine reads from -4
#   Player_PUpRootPage   — root 1KB CHR page per power-up
PRG029 = 0x10 + 29 * 0x2000
SPPF_OFFSETS = PRG029 + (0xCC00 - 0xC000)
SPPF_TABLE = PRG029 + (0xCC51 - 0xC000)

# Player_PUpRootPage: Small, Big, Fire, Raccoon, Frog, Tanooki, Hammer.
ROOT_PAGE = [0x50, 0x54, 0x54, 0x00, 0x50, 0x40, 0x44]
SUIT_BIG = 1

# PF_WALKBIG_BASE — the first walk frame, which is also the standing pose for
# big/fire/hammer Mario. Its Player_FramePageOff entry is 0, so the CHR page is
# the power-up root page.
FRAME_STAND_BIG = 0x0C

# Patches that need an enemy sprite instead of the player one, keyed by IPS
# stem: (CHR page, upper tile, lower tile, 4-color palette).
#
# dr-mario-bros-3 is the "+ Viruses" variant of dr-mario-bros-3-player-only —
# the two ROMs differ only in enemy CHR, so their player sprites are identical.
# Diffing them shows the variant rewrites tiles 0x30-0x33 and 0x3A-0x3D of CHR
# page 0x12, which is the Thwomp page (docs/smb3_rom_reference.md) — the virus
# that replaces the Thwomp. The patch edits no palettes, so the virus renders
# in the Thwomp's own colors: black body, cyan limbs, white face.
ENEMY_OVERRIDES = {
    "dr-mario-bros-3": (0x12, 0x30, 0x3A, [0x0F, 0x0F, 0x2C, 0x30]),
}

# $F1 is the magic "don't display this sprite" pattern.
PATTERN_HIDDEN = 0xF1

# NES 2C02 master palette (same table as web/chr.js).
# fmt: off
NES_PALETTE = [
    (0x7C,0x7C,0x7C),(0x00,0x00,0xFC),(0x00,0x00,0xBC),(0x44,0x28,0xBC),
    (0x94,0x00,0x84),(0xA8,0x00,0x20),(0xA8,0x10,0x00),(0x88,0x14,0x00),
    (0x50,0x30,0x00),(0x00,0x78,0x00),(0x00,0x68,0x00),(0x00,0x58,0x00),
    (0x00,0x40,0x58),(0x00,0x00,0x00),(0x00,0x00,0x00),(0x00,0x00,0x00),
    (0xBC,0xBC,0xBC),(0x00,0x78,0xF8),(0x00,0x58,0xF8),(0x68,0x44,0xFC),
    (0xD8,0x00,0xCC),(0xE4,0x00,0x58),(0xF8,0x38,0x00),(0xE4,0x5C,0x10),
    (0xAC,0x7C,0x00),(0x00,0xB8,0x00),(0x00,0xA8,0x00),(0x00,0xA8,0x44),
    (0x00,0x88,0x88),(0x00,0x00,0x00),(0x00,0x00,0x00),(0x00,0x00,0x00),
    (0xF8,0xF8,0xF8),(0x3C,0xBC,0xFC),(0x68,0x88,0xFC),(0x98,0x78,0xF8),
    (0xF8,0x78,0xF8),(0xF8,0x58,0x98),(0xF8,0x78,0x58),(0xFC,0xA0,0x44),
    (0xF8,0xB8,0x00),(0xB8,0xF8,0x18),(0x58,0xD8,0x54),(0x58,0xF8,0x98),
    (0x00,0xE8,0xD8),(0x78,0x78,0x78),(0x00,0x00,0x00),(0x00,0x00,0x00),
    (0xFC,0xFC,0xFC),(0xA4,0xE4,0xFC),(0xB8,0xB8,0xF8),(0xD8,0xB8,0xF8),
    (0xF8,0xB8,0xF8),(0xF8,0xA4,0xC0),(0xF0,0xD0,0xB0),(0xFC,0xE0,0xA8),
    (0xF8,0xD8,0x78),(0xD8,0xF8,0x78),(0xB8,0xF8,0xB8),(0xB8,0xF8,0xD8),
    (0x00,0xFC,0xFC),(0xF8,0xD8,0xF8),(0x00,0x00,0x00),(0x00,0x00,0x00),
]
# fmt: on


def apply_ips(rom, patch):
    """Apply an IPS patch to a bytearray ROM in place."""
    if patch[:5] != b"PATCH":
        raise ValueError("not an IPS patch")
    pos = 5
    while pos < len(patch):
        if patch[pos:pos + 3] == b"EOF":
            break
        offset = (patch[pos] << 16) | (patch[pos + 1] << 8) | patch[pos + 2]
        pos += 3
        size = (patch[pos] << 8) | patch[pos + 1]
        pos += 2
        if size == 0:  # RLE record
            run = (patch[pos] << 8) | patch[pos + 1]
            pos += 2
            data = bytes([patch[pos]]) * run
            pos += 1
        else:
            data = patch[pos:pos + size]
            pos += size
        rom[offset:offset + len(data)] = data


def decode_tile(rom, tile_id, palette):
    """Decode one 8x8 CHR tile to a list of rows of RGBA tuples (color 0 = clear)."""
    base = CHR_BASE + tile_id * 16
    rows = []
    for y in range(8):
        p0, p1 = rom[base + y], rom[base + 8 + y]
        row = []
        for x in range(8):
            bit = 7 - x
            idx = (((p1 >> bit) & 1) << 1) | ((p0 >> bit) & 1)
            row.append(None if idx == 0 else NES_PALETTE[palette[idx] & 0x3F] + (255,))
        rows.append(row)
    return rows


def frame_patterns(rom, frame):
    """The six sprite patterns for a Player_Frame (upper row first, then lower)."""
    offset = rom[SPPF_OFFSETS + frame] * 2
    base = SPPF_TABLE - 4 + offset
    return [rom[base + i] for i in range(6)]


def render_player(rom, frame=FRAME_STAND_BIG, suit=SUIT_BIG):
    """Render the player composite: 6 8x16 sprites in 3 columns x 2 rows."""
    patterns = frame_patterns(rom, frame)
    page = ROOT_PAGE[suit]
    palette = [rom[MARIO_PALETTE + i] for i in range(4)]
    # Slots follow Player_Draw: upper row (y=0) is patterns 0-2, lower row
    # (y=16) is patterns 3-5; columns are 8px apart.
    slots = [(0, 0), (8, 0), (16, 0), (0, 16), (8, 16), (16, 16)]
    # Player_Draw mirrors the middle column onto the left when the two lower
    # patterns match (front-facing poses like the frog suit).
    mirrored = patterns[3] == patterns[4]
    img = Image.new("RGBA", (24, 32), (0, 0, 0, 0))
    for i, (pattern, (sx, sy)) in enumerate(zip(patterns, slots)):
        if pattern == PATTERN_HIDDEN:
            continue
        flip = mirrored and i in (1, 4)
        # 8x16 sprite mode: bit 0 selects the pattern table, so the tile pair is
        # (pattern & 0xFE, +1) within the 64-tile CHR page.
        top = page * 64 + (pattern & 0xFE)
        for half in (0, 1):
            for y, row in enumerate(decode_tile(rom, top + half, palette)):
                for x, color in enumerate(row):
                    if color:
                        px = sx + (7 - x if flip else x)
                        img.putpixel((px, sy + half * 8 + y), color)
    return img


def render_enemy(rom, page, upper, lower, palette):
    """Render a Thwomp-shaped enemy: 24x32, six 8x16 sprites.

    `Thwomp_Draw` (PRG004) draws a 16x32 sprite from four tile pairs, then adds
    an 8x32 right column at x+16 whose two patterns ($B1/$BB — tiles 0x30/0x3A
    of the page mapped at slot +4) are the LEFT edge tiles drawn horizontally
    flipped. So the creature is symmetric: unique left 16 px, mirrored right 8.
    """
    img = Image.new("RGBA", (24, 32), (0, 0, 0, 0))
    # (tile pair, x, y, h-flipped)
    parts = [(upper, 0, 0, False), (upper + 2, 8, 0, False), (upper, 16, 0, True),
             (lower, 0, 16, False), (lower + 2, 8, 16, False), (lower, 16, 16, True)]
    for tile, sx, sy, flip in parts:
        top = page * 64 + tile
        for half in (0, 1):
            for y, row in enumerate(decode_tile(rom, top + half, palette)):
                for x, color in enumerate(row):
                    if color:
                        img.putpixel((sx + (7 - x if flip else x), sy + half * 8 + y), color)
    return img


def main():
    if not os.path.exists(ROM_PATH):
        sys.exit(f"ROM not found: {ROM_PATH}")
    base_rom = bytearray(open(ROM_PATH, "rb").read())
    os.makedirs(OUT_DIR, exist_ok=True)

    jobs = [("vanilla", None)]
    for name in sorted(os.listdir(PATCH_DIR)):
        if name.endswith(".ips"):
            jobs.append((name[:-4], os.path.join(PATCH_DIR, name)))

    rendered = []
    for stem, patch_path in jobs:
        rom = bytearray(base_rom)
        if patch_path:
            apply_ips(rom, open(patch_path, "rb").read())
        override = ENEMY_OVERRIDES.get(stem)
        if override:
            rendered.append((stem, render_enemy(rom, *override), False))
        else:
            rendered.append((stem, render_player(rom), True))

    # Crop to keep the pill row aligned: every sprite shares one HEIGHT (the
    # page scales them by height, so equal heights mean equal pixel scale and a
    # common baseline), and the player sprites additionally share one x-range.
    # A wider enemy sprite keeps its own width — the pill centers it.
    player_x = [24, 0]
    y_range = [32, 0]
    for _, img, is_player in rendered:
        bbox = img.getbbox()
        if not bbox:
            continue
        y_range = [min(y_range[0], bbox[1]), max(y_range[1], bbox[3])]
        if is_player:
            player_x = [min(player_x[0], bbox[0]), max(player_x[1], bbox[2])]

    for stem, img, is_player in rendered:
        bbox = img.getbbox() or (0, 0, 24, 32)
        x0, x1 = player_x if is_player else (bbox[0], bbox[2])
        out = os.path.join(OUT_DIR, f"{stem}.png")
        cropped = img.crop((x0, y_range[0], x1, y_range[1]))
        cropped.save(out)
        print(f"{out}  {cropped.width}x{cropped.height}")


if __name__ == "__main__":
    main()
