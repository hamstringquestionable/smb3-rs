Look up a world-map tile byte: visual (CHR pattern), palette, behavior, and where vanilla uses it.

## Usage
`/tile <byte>`

Examples:
- `/tile 0xE6` — HANDTRAP tile
- `/tile 44` — corner path tile (hex without 0x prefix is fine)
- `/tile 0x50` — Toad House
- `/tile 4F` — a free byte (good candidate for re-purposing)

## Instructions

Run the rom_map.py tile lookup:

```
nix-shell -p python3 --run "python3 tools/rom_map.py --tile $ARGUMENTS"
```

Present the full output to the user. The output includes:

- **Special-entry name** (if any) — TOADHOUSE / SPADEBONUS / PIPE / ALTTOADHOUSE / CASTLEBOTTOM / SPIRAL / ALTSPIRAL / PATHANDNUB / DANCINGFLOWER / HANDTRAP / BOWSERCASTLELL — plus the dispatch op-code shared with other tiles routed to the same handler.
- **Palette page** (the high 2 bits of the byte): palette is encoded in the byte itself (no per-tile lookup table). Page 0 = `0x00–0x3F`, page 1 = `0x40–0x7F`, page 2 = `0x80–0xBF`, page 3 = `0xC0–0xFF`.
- **Visual (CHR pattern)** — 4 quadrant indices NW/NE/SW/SE from metatile bank 0x0C at file `0x18010 / 0x18110 / 0x18210 / 0x18310 + tile`. Plus any "visually identical sibling" tiles (same CHR; only palette page differs).
- **Behavior registries** — movement directions (LRDU), special-entry, removable, special-completion, background. To clone behavior to a free byte, replicate the same membership.
- **Vanilla usage** — every (row, col) in every world's tile grid where this byte appears.
- **Behavior table offsets** — for direct ROM editing.

## Notes

- The palette is **implicit in the byte value** (high 2 bits). To clone a tile so that it preserves the source's palette, pick the destination byte in the same page (same high 2 bits).
- HANDTRAP is currently bound only to `0xE6` (palette page 3). To create a HANDTRAP that uses palette 1 (path-colored), edit `Map_EnterSpecialTiles[9]` at file `0x14DC8` to a byte in `0x40–0x7F`.
- 115 byte values are unused in any world's grid AND in any behavior table — fully free for repurposing.

See also: `docs/smb3_rom_reference.md` § "World-Map Tile Behavior".
