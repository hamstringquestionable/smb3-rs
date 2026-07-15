#!/usr/bin/env python3
"""Render the 8 SMB3 ending-screen mini-map vignettes (the framed boxes) to PNGs.

Each world's `EndPic_WorldN` buffer is a 16x12 tile nametable image drawn with
BG CHR page $7c/$7d (tiles $00-$7F) and $76/$77 (tiles $80-$FF), colored by the
per-world ending palette. This tool decodes and upscales them so we can see and
verify the mini-maps (vanilla or from a randomized ROM).

Usage: credits_render.py <rom.nes> <out_dir> [scale]
"""
import sys
from pathlib import Path

# EndPicByWorld pointer tables (file offsets) and palette pointer table.
ENDPIC_H = 0x32126
ENDPIC_L = 0x3212E
PAL_PTR = 0x32684
CHR_BASE = 0x40010
BG_PAGES = (0x7c, 0x7d, 0x76, 0x77)  # tiles $00-3F,$40-7F,$80-BF,$C0-FF

# Standard NES palette (64 colors) -> RGB.
NES = [
 (84,84,84),(0,30,116),(8,16,144),(48,0,136),(68,0,100),(92,0,48),(84,4,0),(60,24,0),
 (32,42,0),(8,58,0),(0,64,0),(0,60,0),(0,50,60),(0,0,0),(0,0,0),(0,0,0),
 (152,150,152),(8,76,196),(48,50,236),(92,30,228),(136,20,176),(160,20,100),(152,34,32),(120,60,0),
 (84,90,0),(40,114,0),(8,124,0),(0,118,40),(0,102,120),(0,0,0),(0,0,0),(0,0,0),
 (236,238,236),(76,154,236),(120,124,236),(176,98,236),(228,84,236),(236,88,180),(236,106,100),(212,136,32),
 (160,170,0),(116,196,0),(76,208,32),(56,204,108),(56,180,204),(60,60,60),(0,0,0),(0,0,0),
 (236,238,236),(168,204,236),(188,188,236),(212,178,236),(236,174,236),(236,174,212),(236,180,176),(228,196,144),
 (204,210,120),(180,222,120),(168,226,144),(152,226,180),(160,214,228),(160,162,160),(0,0,0),(0,0,0),
]


def decompress(rom, off):
    out = []
    i = off
    while len(out) < 0xC1:
        b = rom[i]; i += 1
        if b & 0x80:
            out += [b & 0x7f, b & 0x7f]
        else:
            out.append(b)
    return out[:0xC1]


def chr_tile(rom, tid):
    """Return 8x8 list of 2-bit color indices for BG tile `tid`."""
    if tid < 0x40: page = BG_PAGES[0]; local = tid
    elif tid < 0x80: page = BG_PAGES[1]; local = tid - 0x40
    elif tid < 0xC0: page = BG_PAGES[2]; local = tid - 0x80
    else: page = BG_PAGES[3]; local = tid - 0xC0
    base = CHR_BASE + page * 0x400 + local * 16
    px = [[0] * 8 for _ in range(8)]
    for y in range(8):
        lo = rom[base + y]; hi = rom[base + y + 8]
        for x in range(8):
            bit = 7 - x
            px[y][x] = ((lo >> bit) & 1) | (((hi >> bit) & 1) << 1)
    return px


def render_world(rom, w, scale):
    from PIL import Image
    # palette (first 16 bytes = 4 BG sub-palettes)
    ptr = rom[PAL_PTR + w * 2] | (rom[PAL_PTR + w * 2 + 1] << 8)
    poff = 0x32010 + (ptr - 0xC000)
    pal = rom[poff + 3: poff + 3 + 16]
    cpu = rom[ENDPIC_L + w] | (rom[ENDPIC_H + w] << 8)
    off = 0x32010 + (cpu - 0xC000)
    tiles = decompress(rom, off)
    img = Image.new('RGB', (16 * 8, 12 * 8))
    px = img.load()
    for r in range(12):
        for c in range(16):
            tid = tiles[r * 16 + c]
            t = chr_tile(rom, tid)
            # Attribute unknown here: try BG sub-palette 0. (Shapes are what we
            # need to confirm the node mapping; color can be refined later.)
            for y in range(8):
                for x in range(8):
                    ci = t[y][x]
                    color = NES[pal[ci] & 0x3f]
                    px[c * 8 + x, r * 8 + y] = color
    img = img.resize((16 * 8 * scale, 12 * 8 * scale), Image.NEAREST)
    return img


def main():
    rom = bytearray(Path(sys.argv[1]).read_bytes())
    out = Path(sys.argv[2]); out.mkdir(parents=True, exist_ok=True)
    scale = int(sys.argv[3]) if len(sys.argv) > 3 else 4
    for w in range(8):
        img = render_world(rom, w, scale)
        img.save(out / f'world{w + 1}.png')
    print(f'wrote 8 mini-maps to {out}')


if __name__ == '__main__':
    main()
