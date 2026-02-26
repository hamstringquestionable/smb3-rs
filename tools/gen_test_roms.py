#!/usr/bin/env python3
"""Generate 8 test ROMs with every level replaced by 7-F1, one per starting world."""
import os

rom_path = "Super Mario Bros. 3 (USA) (Rev 1).nes"
rom = bytearray(open(rom_path, "rb").read())

F1_TILESET = 2
F1_OBJ = 0xD4E4
F1_LAY = 0xB28E

WORLDS = [
    (0x19438, 21),
    (0x194BA, 47),
    (0x195D8, 52),
    (0x19714, 34),
    (0x197E4, 42),
    (0x198E4, 57),
    (0x19A3E, 46),
    (0x19B56, 41),
]

os.makedirs("test_roms", exist_ok=True)

for start_world in range(8):
    patched = bytearray(rom)

    base, count = WORLDS[start_world]
    scrcol = base + count
    objsets = scrcol + count
    layouts = objsets + count * 2

    # Replace every real level entry with 7-F1
    replaced = 0
    for idx in range(count):
        obj_lo = patched[objsets + idx * 2]
        obj_hi = patched[objsets + idx * 2 + 1]
        lay_lo = patched[layouts + idx * 2]
        lay_hi = patched[layouts + idx * 2 + 1]
        obj = (obj_hi << 8) | obj_lo
        lay = (lay_hi << 8) | lay_lo

        if obj >= 0xC000 and lay != 0:
            old_brt = patched[base + idx]
            patched[base + idx] = (old_brt & 0xF0) | F1_TILESET
            patched[objsets + idx * 2] = F1_OBJ & 0xFF
            patched[objsets + idx * 2 + 1] = (F1_OBJ >> 8) & 0xFF
            patched[layouts + idx * 2] = F1_LAY & 0xFF
            patched[layouts + idx * 2 + 1] = (F1_LAY >> 8) & 0xFF
            replaced += 1

    # Set starting world (operand of LDA #$00 at 0x30CC2)
    patched[0x30CC3] = start_world

    # Set Mario big: World_Map_Power (operand of LDA #$00 at 0x30CCB)
    patched[0x30CCC] = 0x01

    fname = f"test_roms/7f1_w{start_world + 1}.nes"
    open(fname, "wb").write(patched)
    print(f"Wrote {fname} (start W{start_world + 1}, {replaced} levels -> 7-F1)")

print("\nDone!")
