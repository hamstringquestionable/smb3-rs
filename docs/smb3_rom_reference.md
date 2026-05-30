# Super Mario Bros. 3 (USA Rev 1) ROM Hacking Reference

All offsets are for the iNES file (includes 16-byte header at 0x00000–0x0000F).
To convert CPU address to file offset: `file_offset = bank_start + (cpu_address - bank_base)`.

## ROM Layout

| Field | Value |
|---|---|
| File size | 393,232 bytes (0x5FFD0) |
| iNES header | 16 bytes: `4E 45 53 1A 10 10 40 00 ...` |
| PRG ROM | 16 pages x 16 KB = 256 KB (0x00010–0x4000F) |
| CHR ROM | 16 pages x 8 KB = 128 KB (0x40010–0x6000F) |
| Mapper | 4 (MMC3) |
| Mirroring | Horizontal |
| SRAM | Disabled |

## PRG Bank Layout (32 banks x 8 KB)

MMC3 maps two switchable 8 KB banks + two fixed banks:
- Banks 30–31 always at $8000–$9FFF and $E000–$FFFF

| Bank | File Offset | Contents |
|------|------------|----------|
| PRG000 | 0x00010–0x0200F | Object code bank 0 |
| PRG001 | 0x02010–0x0400F | Object IDs $00–$23 |
| PRG002 | 0x04010–0x0600F | Object IDs $24–$47 |
| PRG003 | 0x06010–0x0800F | Object IDs $48–$6B |
| PRG004 | 0x08010–0x0A00F | Object IDs $6C–$8F |
| PRG005 | 0x0A010–0x0C00F | Object IDs $90–$B3+ |
| PRG006 | 0x0C010–0x0E00F | All level object layouts |
| PRG007 | 0x0E010–0x1000F | Special objects, cannon fire, misc |
| PRG008 | 0x10010–0x1200F | Player code and animation |
| PRG009 | 0x12010–0x1400F | 2P Vs mode, auto-scroll |
| PRG010 | 0x14010–0x1600F | World map functionality (code) |
| PRG011 | 0x16010–0x1800F | World map data (objects, layouts) |
| PRG012 | 0x18010–0x1A00F | World map tileset (Tileset 0) |
| PRG013 | 0x1A010–0x1C00F | Underground tileset (Tileset 14) |
| PRG014 | 0x1C010–0x1E00F | Level tileset |
| PRG015 | 0x1E010–0x2000F | Plains tileset (Tileset 1) |
| PRG016 | 0x20010–0x2200F | Hilly tileset (Tileset 3) |
| PRG017 | 0x22010–0x2400F | Ice/Sky tileset (Tileset 4/12) |
| PRG018 | 0x24010–0x2600F | Pipe/Water tileset (Tileset 7) |
| PRG019 | 0x26010–0x2800F | Giant/Plant tileset (Tileset 5/11/13) |
| PRG020 | 0x28010–0x2A00F | Desert tileset (Tileset 2) |
| PRG021 | 0x2A010–0x2C00F | Dungeon tileset (Tileset 9) |
| PRG022 | 0x2C010–0x2E00F | Bonus tileset (Tileset 15/16/17) |
| PRG023 | 0x2E010–0x3000F | Ship tileset (Tileset 10) |
| PRG024 | 0x30010–0x3200F | Title screen, endings |
| PRG025 | 0x32010–0x3400F | Title screen (cont.), cinematics |
| PRG026 | 0x34010–0x3600F | Status bar, inventory |
| PRG027 | 0x36010–0x3800F | Shared gameplay routines |
| PRG028 | 0x38010–0x3A00F | Music engine (part 1) |
| PRG029 | 0x3A010–0x3C00F | Music engine (part 2) |
| PRG030 | 0x3C010–0x3E00F | Core game loop (always at $8000) |
| PRG031 | 0x3E010–0x4000F | Interrupt handlers (always at $E000) |

## CHR ROM (Graphics Tiles)

| Range | Size | Contents |
|-------|------|----------|
| 0x40010–0x6000F | 128 KB | All graphics tile data (sprites + backgrounds) |

---

## Level Data

### Level Definitions by Tileset

Each range contains the level layout generators for that tileset/theme.

| File Offset | Size | Theme |
|------------|------|-------|
| 0x1A587–0x1C005 | ~6.6 KB | Underground |
| 0x1E512–0x20005 | ~6.7 KB | Plains |
| 0x20587–0x22005 | ~6.6 KB | Hilly |
| 0x227E0–0x24005 | ~6.2 KB | Ice / Sky |
| 0x24BA7–0x26005 | ~5.2 KB | Pipe / Water |
| 0x26A6F–0x28C05 | ~8.6 KB | Cloudy / Giant / Piranha Plant |
| 0x28F36–0x2A005 | ~4.3 KB | Desert |
| 0x2A7F7–0x2C005 | ~6.1 KB | Dungeon |
| 0x2EC07–0x30005 | ~5.1 KB | Ship |

### Level Header Format (9 bytes per level)

| Byte | Bitmask | Contents |
|------|---------|----------|
| 0–1 | `aaaaaaaaaaaaaaaa` | Transition scenery address (16-bit pointer) |
| 2–3 | `aaaaaaaaaaaaaaaa` | Transition actor/enemy address (16-bit pointer) |
| 4 | `aaa0bbbb` | a = Y-start properties (indexed table); b = course end page (0-15 screens) |
| 5 | `_abbccddd` | a = unused; b = X-start properties; c = object palette (2 bits); d = BG palette (3 bits) |
| 6 | `abbcdddd` | a = pipe transition type; b = vertical scroll mode; c = scroll direction; d = transition course type |
| 7 | `aaabbbbb` | a = friction factor (3 bits); b = BG banks / CHR selection (5 bits) |
| 8 | `aa00bbbb` | a = timer seed (2 bits, indexed); b = music track selection (4 bits) |

### Level Tile Generator Format

Levels use "tile generators" (variable/fixed-size construction routines), not raw tile grids.
World maps are stored as raw tile grids instead.

Each level's layout data consists of a **9-byte header** followed by **3-byte generator commands**
terminated by `0xFF`.

#### 3-Byte Generator Commands

Each command is `[byte0] [byte1] [byte2]` where:

```
byte0 (Temp_Var15):
  bits 7-5 = Generator group (0-7). Group 7 (0xE0) = level junction, not a tile generator.
  bit  4   = Address high flag (increments Map_Tile_AddrH, selects second half of screen memory)
  bits 3-0 = Row position (0-15) within the screen

byte1 (Temp_Var16):
  bits 7-4 = Screen number (0-15)
  bits 3-0 = Column position (0-15) within the screen

byte2 (LL_ShapeDef):
  If upper nibble = 0x0: Fixed-size generator path
  If upper nibble != 0:  Variable-size generator path
```

**Tile memory address calculation** (from `LoadLevel_Set_TileMemAddr` in PRG030):
- `TileAddr_Off = (byte0_lower4 << 4) | (byte1 & 0x0F)` — encodes row + column
- Screen base address: `Tile_Mem_Addr[(byte1 & 0xF0) >> 3]` (word-indexed table)
- If bit 4 of byte0 is set, `Map_Tile_AddrH` is incremented (second half of screen)
- Tile memory is column-major: next column = Y+1, next row = TileAddr_Off + 16
- Screen boundary: when `Y & 0x0F == 0`, add `$1B0` to Map_Tile_Addr

#### Fixed-Size Generators

Dispatch index = `((byte0 & 0xE0) >> 1) + byte2`

This gives a logical index into the tileset's `LeveLoad_FixedSizeGen_TSx` dispatch table
(which uses `DynJump` — internally does `ASL A` to convert to word offset).

**Tileset 1 (Plains) fixed-size dispatch table** (42 entries, indices 0-41):

| Index Range | Group | Handler | Description |
|-------------|-------|---------|-------------|
| 0-7 | 0 | Various | Bushes, clouds, doors, vines, etc. |
| 8-15 | 0 | $0000 (null) | Reserved/unused |
| 16-40 | 1 | `LoadLevel_PowerBlock` | Power blocks (see below) |
| 41 | 2 | `LoadLevel_EndGoal` | End-of-level goal |

**`LoadLevel_PowerBlock`** (PRG014): Takes the fixed-size index, subtracts 16, uses result
to index the `LL_PowerBlocks` table (24 entries) which maps to tile IDs:

| byte2 | Index-16 | Tile ID | Tile Name | Visual | Item |
|-------|----------|---------|-----------|--------|------|
| 0x00 | 0 | $60 | QBLOCKFLOWER | Q-block | Mushroom/Flower |
| 0x01 | 1 | $61 | QBLOCKLEAF | Q-block | Mushroom/Leaf |
| 0x02 | 2 | $62 | QBLOCKSTAR | Q-block | Star |
| 0x03 | 3 | $64 | QBLOCKCOINSTAR | Q-block | Coin/Star |
| 0x04 | 4 | $65 | QBLOCKCOIN2 | Q-block | Coin |
| 0x05 | 5 | $66 | MUNCHER | Muncher | — |
| 0x06 | 6 | $68 | BRICKFLOWER | Brick | Mushroom/Flower |
| 0x07 | 7 | $69 | BRICKLEAF | Brick | Mushroom/Leaf |
| 0x08 | 8 | $6A | BRICKSTAR | Brick | Star |
| 0x09 | 9 | $6C | BRICKCOINSTAR | Brick | Coin/Star |
| 0x0A | 10 | $6D | BRICK10COIN | Brick | 10-coin |
| 0x0B | 11 | $6E | BRICK1UP | Brick | 1-Up |
| 0x0C | 12 | $6F | BRICKVINE | Brick | Vine |
| 0x0D | 13 | $70 | BRICKPSWITCH | Brick | P-Switch |
| 0x0E | 14 | $44 | INVISCOIN | Invisible | Coin |
| 0x0F | 15 | $45 | INVIS1UP | Invisible | 1-Up |

ROM offset: `LL_PowerBlocks` table at **0x1CAD4** (24 bytes, PRG014).

**Important:** The `LL_PowerBlocks` table and `LoadLevel_PowerBlock` routine are **shared
across all tilesets** — group 1 fixed-size generators always dispatch to the same handler
regardless of tileset. This means byte2 values 0x00-0x0F have identical meaning in every level.

##### Group 2 Fixed-Size: Note and Wood Powerup Blocks

Group 2 fixed-size commands (`byte0 & 0xE0 == 0x40`) place specific tiles whose IDs
appear in `LL_PowerBlocks` at indices 16-23, so the runtime "hit a powerup tile" check
spawns an item just as it does for group 1. The same dispatch math applies
(`fixed_idx = ((byte0 & 0xE0) >> 1) + byte2`), so group-2 byte2 in 0x00-0x07 maps to
`fixed_idx` 32-39.

In tilesets where shapes 1-6 resolve to note/wood block tiles (most tilesets — see
`randomize_note_wood` per region in `rom_data.rs`), the layout is:

| byte2 | fixed_idx | Tile ID | Visual | Item |
|-------|-----------|---------|--------|------|
| 0x01 | 33 | $2F | Note block | Mushroom/Flower |
| 0x02 | 34 | $30 | Note block | Mushroom/Leaf |
| 0x03 | 35 | $31 | Note block | Star |
| 0x04 | 36 | $73 | Wood block (`?`) | Mushroom/Flower |
| 0x05 | 37 | $74 | Wood block (`?`) | Mushroom/Leaf |
| 0x06 | 38 | $75 | Wood block (`?`) | Star |

The canonical example is 1-3's "wood-block-with-leaf" at file offset **0x1EE95**
(`57 3C 05` → scr=3, col=12, row=7), which empirically produces the wood-textured
`?` block that drops a leaf. The randomizer's `powerups.rs` shuffles these within
`NOTE_SHAPES = {1, 2, 3}` and `WOOD_SHAPES = {4, 5, 6}`.

**Exceptions** (`randomize_note_wood: false` regions):
- **TS2 (Dungeon):** shapes 1-2 = `CCBridge`, shapes 3-7 = `TopDecoBlocks` decorations.
- **TS9 (Desert):** shapes 1-5 = palms / cacti decorations.

In these tilesets the dispatch produces non-powerup tiles, so swapping byte2 would
corrupt level geometry.

#### Variable-Size Generators

Dispatch index = `base_table[group] + (byte2 >> 4) - 1`

Where `base_table = {0, 15, 30, 45, 60, 75, 90, 105}` (15 slots per group).

The lower nibble of byte2 (`byte2 & 0x0F`) typically encodes width/size parameter.

**`LoadLevel_BlockRun`** (PRG014): Used for runs of identical block tiles.
Block type = `(byte2 - 0x10) >> 4` indexes into `LoadLevel_Blocks` table:

| Block Index | Dispatch Index | Tile Name | Description |
|-------------|---------------|-----------|-------------|
| 0 | 15 | BRICK | Plain brick |
| 1 | 16 | QBLOCKCOIN | Q-block with coin |
| 2 | 17 | BRICKCOIN | Brick with coin |
| 3 | 18 | WOODBLOCK | Wood block |
| 4 | 19 | GNOTE | Green note block |
| 5 | 20 | NOTE | Note block |
| 6 | 21 | WOODBLOCKBOUNCE | Bouncing wood block |
| 7 | 22 | COIN | Floating coin |
| 8 | 43 (special) | ICEBRICK | Ice brick |

Width = `byte2 & 0x0F`, tiles placed = width + 1 (loop uses BPL = inclusive).

ROM offset: `LoadLevel_Blocks` table at PRG014 (9 bytes, immediately before `LoadLevel_BlockRun`).

#### Variable-Length Commands (Extra Byte)

Most generator commands are 3 bytes, but some variable-size routines read a **4th byte**
from the layout data stream. If a parser assumes all commands are 3 bytes, every command
after the first extra-byte routine will be misaligned.

**Identifying 4-byte commands:**

The correct method is a **dispatch-based lookup** using tileset-specific extra-byte
dispatch lists. Each tileset has its own set of variable-size dispatches that consume
an extra byte.

**Tileset-specific extra-byte dispatches:**

| Tileset | Extra-Byte Dispatches | Source |
|---------|----------------------|--------|
| TS1 (Plains) | 11, 12, 35-42 | GroundRun, TopDecoBlocks |
| TS2 (Dungeon) | 13, 14, 35-42, 46, 47, 48, 57, 95, 96 | SolidBrick, BrightDiamondLong, TopDecoBlocks, Background, Lava, BrightDiamond, Group6 handlers |
| TS3 (Hilly) | 35-42, 60-71 | TopDecoBlocks, BGOrWater, DecoGround, DecoCeiling |
| TS4/12 (Ice/Sky) | 0, 35-42, 54, 60, 112 | LongWoodBlock, TopDecoBlocks, Muncher17, Group4 var, Group7 var |
| TS5/11/13 (Cloudy) | 13, 35-42, 45, 46, 48, 51 | DoubleCloud, TopDecoBlocks, CloudGoal, RoundCloudTop, CloudSpace, Lava |
| TS7 (Pipe/Water) | 35-42, 49, 57 | TopDecoBlocks, OrangeBlock, WaterFill |
| TS9 (Desert) | 10-13, 35-42 | DiagRect variants, TopDecoBlocks |
| TS10 (Ship) | 1, 2, 35-42, 48, 49, 51 | WoodBodyLong, TopDecoBlocks, MetalPlate, Crate, DoubleTipBodyWood |

**High-bit fallback rule (NOT universally reliable):**

Some external documentation suggests using `byte0 & 0x80` (high bit set = 4-byte command)
as a universal rule. This works for **TS3 and TS4/12** but produces incorrect alignment
for **TS1, TS2, and other tilesets**. The high-bit rule should only be used as a last
resort or for tilesets where the dispatch list is unknown. Always prefer dispatch-based
detection when the tileset is known.

**Tileset 1 (Plains) extra-byte examples:**

| Dispatch | Handler | Extra Byte Meaning |
|----------|---------|-------------------|
| 11, 12 | `LoadLevel_GroundRun` | Ground fill width |
| 35-42 | `LoadLevel_TopDecoBlocks` | Rectangle width |

**Other tilesets** have additional extra-byte routines (e.g., `LoadLevel_LavaRun`,
`LoadLevel_DecoGround`, `LoadLevel_DecoCeiling`). Each tileset's variable-size dispatch
table must be checked individually to identify which dispatches consume extra bytes.

The level simulator at `tools/level_sim.py` tracks extra-byte dispatches per tileset,
but it currently only implements TS1 dispatches (hardcoded).

#### 1-1 Level Data Reference

File offset: **0x1FB92** (CPU $BB82 in PRG015, bank mapped at $A000).
Header: 9 bytes at 0x1FB92. Generator data: 0x1FB9B–0x1FCA0 (86 commands + 0xFF terminator).

Bonus room: at **0x1FCA3** (CPU $BC93), entered via junction.

**Important:** Some generator routines consume a 4th byte from the data stream (see
"Variable-Length Commands" above). In TS1, `GroundRun` (dispatches 11-12) and
`TopDecoBlocks` (dispatches 35-42) read an extra byte. Parsing all commands as 3 bytes
will misalign every command after the first extra-byte routine, producing wrong results.
The level simulator at `tools/level_sim.py` handles this correctly.

**Group 1 power blocks found in 1-1 (verified by simulator):**

| ROM Offset | Bytes | Tile | Screen | Row | Col |
|-----------|-------|------|--------|-----|-----|
| 0x1FBB4 | 33 0F 01 | QBLOCKLEAF ($61) | 0 | 3 | 15 |
| 0x1FBE2 | 38 29 01 | QBLOCKLEAF ($61) | 2 | 8 | 9 |
| 0x1FC25 | 28 5A 0B | BRICK1UP ($6E) | 5 | 8 | 10 |
| 0x1FC28 | 37 5C 01 | QBLOCKLEAF ($61) | 5 | 7 | 12 |
| 0x1FC6C | 37 7F 0D | BRICKPSWITCH ($70) | 7 | 7 | 15 |

This matches the MarioWiki count of 3 mushroom/leaf powerups (all QBLOCKLEAF).

**Tile visual verification:** The `Tile_Layout_TS1` table confirms that Q-block tiles
($60-$65) use CHR patterns $98/$99 (animated "?" appearance), while brick tiles
($67-$6F) all use patterns $B4/$B5 (standard brick appearance).

---

### Junctions and Sub-Areas

#### How Junctions Work

Group 7 commands (`byte0 & 0xE0 == 0xE0`) are **level junctions** — they do not
generate tiles. Junction byte2 encodes player spawn positions, NOT target addresses.
The actual sub-area target is determined by the **header chain**: each 9-byte level
header contains pointers to the next sub-area's layout and enemy data.

#### Header Chain (How Sub-Areas Connect)

Each 9-byte layout header contains three fields that point to the next sub-area:

| Header Bytes | Field | Purpose |
|-------------|-------|---------|
| 0-1 | `alt_layout` | CPU address ($A000–$BFFF) of the sub-area's layout data |
| 2-3 | `alt_objects` | CPU address ($C000–$DFFF) of the sub-area's enemy data |
| 6, bits 0-3 | `alt_tileset` | Tileset for the sub-area |

When a junction is encountered during gameplay, the game loads the sub-area layout
from `alt_layout` (in the bank for `alt_tileset`) and enemies from `alt_objects`.
This means sub-areas can be in **different tilesets and different PRG banks** than
their entry point.

**Example: W2 Pyramid** — entry is tileset 9 (Desert) at `0x28F36`, with
`alt_layout=0xA577, alt_tileset=3, alt_objects=0xC5BC`. The interior sub-area
lives in tileset 3 (Hilly) at `0x20587` — a completely different PRG bank.

**Two-way loops**: Sub-area headers often point back to the entry level (the Pyramid
interior's `alt_layout` points back to the exterior). A visited set by `header_offset`
prevents infinite loops when tracing.

**Dead pointers**: Headers WITHOUT junctions (`junction_count == 0`) still contain
`alt_layout`/`alt_objects` values, but these are dead — they are never followed
during gameplay. Only follow the header chain when `junction_count > 0`.

#### Level Data Stream Structure

Each level data region contains a contiguous stream of level segments:

```
[9-byte header A][commands...][0xFF]
[9-byte header B][commands...][0xFF]   ← may or may not be a sub-area of A
[9-byte header C][commands...][0xFF]   ← another segment
...
```

**Important**: Contiguous position in the data stream does NOT imply a sub-area
relationship. Sub-areas are determined by the header chain (`alt_layout`/`alt_tileset`),
not by physical adjacency. A sub-area can be in an entirely different tileset region.

#### Entry Points vs Sub-Areas

- **Entry point**: A level segment whose layout CPU address is referenced by a world
  pointer table entry. When Mario steps on a map tile, the game looks up the
  `LevelLayouts` pointer to find the entry point.
- **Sub-area**: A level segment reached by following the header chain from an entry
  point. Sub-areas are identified by `alt_layout`/`alt_tileset` pointers, not by
  position in the data stream.

**Critical detail**: Multiple pointer table entries can point into the **same** data
segment at different byte offsets. For example, W1[11] (`lay=0xA95D`) and W3[34]
(`lay=0xAA79`) both point into the same 136-command Dungeon segment. They share the
same sub-areas (via header chain).

#### Chain Traversal: Junctions Hop Header-to-Header

**Each sub-area has its own 9-byte header.** When a junction fires from inside a
sub-area, the game uses **that sub-area's own** `alt_layout` / `alt_objects` /
`alt_tileset` — not the entry point's. This is how a player experiences "two pipes
in sequence":

1. Entry level header points at sub-area A.
2. Player triggers a junction inside sub-area A.
3. Game loads sub-area A's header and follows *its* `alt_layout` to reach sub-area B.

To redirect the player directly to sub-area B, rewrite the entry header's pointers
to sub-area B's values (skipping the visit to A entirely). To trace the full chain,
walk from the entry header through successive sub-area headers, using a visited-set
keyed on `header_offset` to stop on loops (see "Two-way loops" note above).

#### Redirecting a Junction Destination

To change where a header's junction leads, overwrite these 4 bytes plus one nibble:

| Target Bytes | Source Value |
|-------------|--------------|
| header + 0..=1 | new `alt_layout` (little-endian, `$A000–$BFFF`) |
| header + 2..=3 | new `alt_objects` (little-endian, `$C000–$DFFF`) |
| header + 6, bits 0-3 | new `alt_tileset` |

**Byte 6 caveat**: bits 4-7 carry unrelated state (scroll lock flag, etc.).
Read-modify-write: `byte6 = (byte6 & 0xF0) | new_tileset`.

**File offset of a header**: the entry-point header sits at the file offset of the
level's `lay_ptr`, i.e. `layout_file_offset(lay_ptr, tileset)` from
`tools/rom_map.py` (uses `PAGE_A000_BY_TILESET[tileset]`). For nested sub-areas,
resolve each `alt_layout` through its own `alt_tileset` before indexing.

**Working example** (W8 Hand → 3-7 coin heaven, from `hand_rooms.rs`):
- 8-Hnd1 main header at file `0x27D50`, vanilla bytes `17 BE CF D0 63 0B CB 0B 81`.
- Overwrite bytes 0..=3 with `4F AB 89 CE` (`alt_layout=$AB4F`, `alt_objects=$CE89`).
- Overwrite byte 6 low nibble: `0x0B` (preserves upper nibble = 0).
- Result: entering 8-Hnd1 and triggering its junction lands directly in the
  cloud-tileset coin-heaven room at `$AB4F` / `$CE89` / ts=11 in PRG019.

#### OBJ_TREASURESET (0xD6) — Treasure Box Items

A `0xD6` entry in an enemy data stream sets the contents of the next
`OBJ_TREASUREBOX` (`0x52`) that spawns. The 3-byte layout is the same as any enemy
entry, but the Y-byte encodes the **item ID**, not a row position:

| Byte | Meaning |
|------|---------|
| 0 | `0xD6` (object ID) |
| 1 | `(screen << 4) \| col` (position X-byte) |
| 2 | Item ID (vanilla uses mushroom=0x01, flower=0x02, leaf=0x03, etc.) |

A complete treasure room is a 3-entry enemy stream:
```
D6 <xy> <item>   ; item setter
52 <xy> <xy>     ; OBJ_TREASUREBOX sprite
BA <xy> <xy>     ; OBJ_TREASUREBOXAPPEAR event
FF               ; terminator
```

**All 5 `OBJ_TREASURESET` chests in the vanilla ROM** (item byte offsets,
randomized by `items::randomize` via a hardcoded `TREASURE_CHEST_OFFSETS` list in
`src/randomize/items.rs` — no auto-discovery):

| Y-byte offset | Sub-area enemy_ptr | Vanilla item | Where |
|--------------|-------------------|--------------|-------|
| `0x0C427` | `$C414` | MusicBox (0x0D) | Princess cutscene chest |
| `0x0CE9F` | `$CE89` | Cloud (0x07) | 3-7 coin-heaven sub-area |
| `0x0D0E2` | `$D0CF` | Leaf (0x03) | Shared 8-Hnd1/2/3 treasure room |
| `0x0D36A` | `$D351` | Whistle (0x0C) | Hidden warp-whistle chest |
| `0x0DA3F` | `$DA29` | Star (0x09) | Star chest |

Any new chest (e.g. via cloning an enemy stream or redirecting a junction) must
have its new Y-byte offset appended to `TREASURE_CHEST_OFFSETS` or it will stay
stuck on its vanilla value across every seed.

#### Reusable Sub-Area Destinations

Known sub-areas that work cleanly as redirect targets (verified playable):

| Destination | Target | Notes |
|-------------|--------|-------|
| 3-7 Coin Heaven | `alt_layout=$AB4F`, `alt_objects=$CE89`, `ts=11` | Vanilla D3 autoscroll at file `0x0CE9A` — NOP to disable (done by `autoscroll::disable_autoscroll`). Contains the Cloud chest at `0x0CE9F`. |

#### Enemy Data Pointers: Entry vs Sub-Area

Each level has TWO sources of enemy data:

1. **Pointer table `obj_ptr`** (ObjSets word): The enemy data used when entering the
   level from the world map. This is what the game loads first.
2. **Previous header's `alt_objects`** (bytes 2-3): When transitioning to a sub-area,
   the game loads enemies from the parent header's `alt_objects` pointer.

These are usually **different pointers**. A fortress entry might have `obj_ptr=0xD32B`
(containing Boom-Boom) while its layout header has `alt_objects=0xD351` (pointing to
the sub-area's enemies). The Boom-Boom enemy is often in a sub-area reached via
header chain, not in the entry point's `obj_ptr`.

#### Fortress Detection via Sub-Area Tracing

To identify all fortress/boss levels, you must check for Boom-Boom enemies
(IDs 0x4A, 0x4B, 0x4C) in BOTH:

1. The pointer table entry's `obj_ptr` enemy data
2. All sub-area headers' `enemy_ptr` enemy data reachable from the entry point

The `tools/rom_map.py` `build_level_groups()` function implements this by:
- Building a `(tileset, layout_cpu)` index across all parsed level regions
- For each pointer table entry, finding its entry-point level via the index
- Following the header chain (`alt_layout`/`alt_tileset`) to trace all sub-areas
- Scanning all enemy pointers (both obj_ptr and sub-area enemy_ptrs) for boss IDs

#### Boom-Boom Groups (13 total in unmodified ROM)

These are all pointer table entries whose level group contains a Boom-Boom enemy,
identified by `level_groups` in `tools/rom_map.json`:

| World Refs | Region | Detection Method |
|-----------|--------|-----------------|
| W1[11], W3[34] | Dungeon (TS2) | W1[11] obj_ptr has Boom-Boom; W3[34] shares segment |
| W2[13] | Desert (TS9) | obj_ptr has Boom-Boom |
| W3[13], W5[12] | Dungeon (TS2) | W3[13] obj_ptr has Boom-Boom; W5[12] shares segment |
| W4[9] | Dungeon (TS2) | obj_ptr has Boom-Boom |
| W4[16], W8[26], W8[40] | Dungeon (TS2) | W4[16], W8[26] obj_ptrs; W8[40] is Bowser |
| W5[31] | Dungeon (TS2) | obj_ptr has Boom-Boom |
| W6[9], W6[48], W7[40] | Dungeon (TS2) | W6[9], W6[48] obj_ptrs; W7[40] shares segment |
| W6[27] | Ice/Sky (TS4/12) | Sub-area enemy_ptr 0xCACE has Boom-Boom |
| W7[5] | Dungeon (TS2) | obj_ptr has Boom-Boom |
| W8[8,17,18,28,35] | Underground (TS14) | Sub-area enemy_ptr 0xD528 has Boom-Boom (tanks) |
| W8[7] | Ship (TS10) | Layout header enemy_ptr 0xDA1F has Boom-Boom |
| W8[10] | Ship (TS10) | Layout header enemy_ptr 0xDA24 has Boom-Boom |
| W8[36] | Ship (TS10) | Layout header enemy_ptr 0xDA1A has Boom-Boom |

**Entries that leak if only checking entry-point obj_ptr:**
W3[34] (ts=2), W5[12] (ts=2), W6[27] (ts=12), W7[40] (ts=2), W8[7] (ts=10),
W8[10] (ts=10), W8[36] (ts=10) — these 7 entries have no boss in their obj_ptr
enemy data and would incorrectly appear in `collect_shuffleable()` as regular levels,
causing tileset leakage when shuffled.

#### Boom-Boom Detection: Approaches Tried and Lessons Learned

Identifying the 17 fortress/boss entries was harder than expected. This section
documents the approaches tried, why they failed, and what was ultimately chosen.
This is essential context if revisiting dynamic detection or implementing sub-area
shuffling in the future.

**Approach 1: obj_ptr range heuristic (`obj >= 0xD000`)**

The initial approach assumed fortress entries always have `obj_ptr >= 0xD000`. This
is wrong — many regular action levels also have enemy data in the $D000+ range
(e.g., World 2 desert levels, World 4 giant levels, World 8 tanks/ships). This
produced both false positives (regular levels flagged as fortresses) and false
negatives (fortresses with $C000-range obj_ptrs missed). The root cause of the
"fortress tileset leaking" bug: 7 entries with Boom-Boom only in sub-area enemy
data were not detected as fortresses and ended up in the regular level shuffle pool.

**Approach 2: Dynamic enemy scanning of entry-point obj_ptr only**

Scanning each entry's `obj_ptr` enemy data for Boom-Boom IDs (0x4A, 0x4B, 0x4C)
correctly identifies 10 of the 17 fortress entries but misses the 7 listed above
where Boom-Boom lives in a sub-area reached via junction. This approach is necessary
but not sufficient.

**Approach 3: Forward scanning past 0xFF terminators (sub-area tracing in Rust)**

After scanning the entry-point's enemy data, continue scanning forward past 0xFF
terminators to find sub-area headers and check their `enemy_ptr` fields for
Boom-Boom. This was implemented in `levels.rs` with `has_boomboom_in_sub_areas()`.

**Why it failed:** There is no reliable way to know where one level's sub-areas end
and another level's data begins. Level data regions pack multiple levels contiguously,
and sub-areas can cross tilesets (e.g., W2 Pyramid exterior is tileset 9 but interior
is tileset 3). Forward scanning within a single region inevitably crosses into other
levels' data, producing massive false positives. In testing, this identified **58
entries** as fortresses instead of the correct 17.

**Approach 3b: Sub-area boundary detection heuristics**

Attempted to detect sub-area boundaries using:
- Q-Ball (0x4A) as an end marker: **Zero** Q-Ball objects exist in ROM enemy data.
  Q-Ball is spawned by code when Boom-Boom is defeated, never placed as an enemy.
- Command count limits (break at >700 commands): Helps filter garbage data past
  real level regions but doesn't solve the inter-level boundary problem.
- Empty segment detection (0-command levels with invalid enemy_ptr): Filters some
  garbage but insufficient for general boundary detection.

None of these heuristics reliably distinguish "sub-area of current level" from
"start of next level."

**Approach 4: Header-chain tracing in rom_map.py (chosen for tooling)**

The `build_level_groups()` function in `tools/rom_map.py` follows the header chain:
each 9-byte header has `alt_layout` (bytes 0-1), `alt_objects` (bytes 2-3), and
`alt_tileset` (byte 6, bits 0-3) pointing to the next sub-area. The function:

1. Builds a `(tileset, layout_cpu)` → level_dict index across all parsed regions
2. For each pointer table entry, looks up its entry-point level in the index
3. Follows the header chain (`alt_layout`/`alt_tileset`) until a dead end or loop
4. Only follows when `junction_count > 0` (dead alt_layout pointers otherwise)
5. Uses a visited set by `header_offset` to prevent infinite loops

This correctly handles cross-tileset sub-areas (e.g., W2 Pyramid: tileset 9 →
tileset 3) that the old contiguous-segment heuristic missed entirely.

**Known false positive**: The W8 layout at `lay=0xB0F7` groups 5 entries
(W8[8,17,18,28,35]) whose sub-area chain eventually reaches an enemy segment
containing Boom-Boom. However, none of these map tiles actually lead to a
Boom-Boom fight in gameplay. This layout's sub-area data overlaps with other
levels' sub-areas in the data stream — the grouping algorithm cannot distinguish
"reachable via junction" from "happens to follow in the byte stream." This group
is excluded from FORTRESS_ENTRIES.

**Approach 5: Hardcoded constant (chosen for Rust implementation)**

Given that the ROM is fixed (USA Rev 1) and the fortress set never changes, all
17 entries are hardcoded as `FORTRESS_ENTRIES` in `src/randomize/levels.rs`. This
is consistent with the existing `AIRSHIP_ENTRIES` and `BOWSER_CASTLE` patterns.
The values were derived from `rom_map.py`'s `build_level_groups()` analysis and
manually verified against known gameplay.

**Bowser's castle exclusion:** W8[40] (`BOWSER_CASTLE` constant, `(7, 40)`) is
explicitly excluded from level shuffle in `collect_shuffleable()`. The game ending
sequence is hardcoded to trigger from this specific level — shuffling it to another
map slot would make the game unwinnable. This exclusion is separate from the
`FORTRESS_ENTRIES` filter (which also excludes W8[40] as a Boom-Boom group member).

If dynamic detection is ever needed (e.g., for ROM hacks with modified fortress
placement), Approach 4 is the correct foundation — but it requires the full
level-group analysis that `rom_map.py` provides, not the simplified forward
scanning attempted in Approach 3.

#### Sub-Area Structure: Notes for Future Shuffling

If sub-area shuffling is implemented in the future, the following structural
details are important.

**Sub-area composition of fortresses:**

Fortresses typically contain 2-4 areas connected by pipe/door junctions:
- Area 0: Entry room (referenced by pointer table, has its own enemy data via obj_ptr)
- Area 1-N: Sub-rooms reached via junction commands in the layout data
- Final area: Boss room containing Boom-Boom (enemy data via that area's header enemy_ptr)

Example: World 3's second fortress (W3[34]) has 4 areas — Mario starts in area 0,
travels through 2 intermediate rooms via pipe transitions, and reaches the Boom-Boom
boss in area 3.

**What makes a sub-area shuffleable:**

Not all sub-areas are interchangeable. Constraints include:
1. **Tileset compatibility**: Sub-areas specify their own tileset via `alt_tileset`,
   but the graphics (CHR banks) must be loaded for that tileset. Cross-tileset
   sub-areas work because the game reloads the CHR bank during transition.
2. **Enemy data coupling**: Each sub-area gets its enemies from the parent header's
   `alt_objects`. The enemies must make sense for the tileset and room geometry.
3. **Header chain consistency**: Sub-areas are linked by `alt_layout`/`alt_tileset`
   in each header. Reordering sub-areas requires updating these pointers in the
   parent headers. The parent's junction count must match the number of transitions.
4. **Boss room preservation**: The final sub-area (boss room) must always contain
   a Boom-Boom enemy for the fortress to be completable.
5. **Shared segments**: Multiple pointer table entries can share the same entry-point
   header (e.g., W1[11] and W3[34]). Modifying shared data affects all entries
   that reference it.

**Data available for sub-area analysis:**

`tools/rom_map.json` `level_groups` contains per-group:
- `sub_areas`: list of all areas in the group, each with `header_offset`,
  `layout_cpu`, `enemy_ptr`, `screens`, `command_count`, `junction_count`,
  and boss flags (`has_boomboom`, `has_koopaling`, `has_bowser`)
- `world_refs`: which pointer table entries share this level group
- `entry_obj_ptrs`: enemy data pointers for the entry points

**Simplest sub-area shuffle approach:**

Rather than shuffling individual sub-areas (which requires solving all the
constraints above), shuffle entire "fortress interiors" as atomic units. This
is what `randomize_fortresses()` already does — it swaps the complete
(obj_ptr, lay_ptr, tileset) tuple between fortress map slots. The entire level
including all its sub-areas moves as one piece. This is safe because:
- The layout data stream is read-only (not modified, just re-pointed)
- The pointer table entry carries the tileset with it
- All sub-areas follow the entry point in the data stream and move with it

For more granular sub-area shuffling (e.g., mixing rooms between fortresses),
the junction target problem must be solved — likely by rewriting layout data
to reorder sub-area headers in the stream, which requires careful management
of the limited space in each tileset's data region.

---

## Enemy / Object Data

| File Offset | Size | Description |
|------------|------|-------------|
| 0x0BFD8–0x0E00D | ~8.2 KB | Enemy/object data for all levels (PRG006) |

### Data Format

The enemy/object data is a sequence of **segments** separated by `0xFF` terminators.
Each segment represents one level's (or sub-area's) object set:

```
[0xFF]                          ; Terminator / separator
[page_flag]                     ; 1 byte: page/screen flag (usually 0x00 or 0x01)
[obj_id] [x_pos] [y_pos]       ; 3 bytes per object entry
[obj_id] [x_pos] [y_pos]       ; ...repeated for each object
...
[0xFF]                          ; Terminator
```

Each entry is exactly **3 bytes**: object ID, X position, Y position.
The leading `0xFF` bytes at the start of the block are empty/unused segments.
Per-level object files in the disassembly (e.g. `PRG/objects/1-1.asm`) confirm this format.

### Position Byte Resolution

The X and Y bytes are **tile-resolution** (1 unit = 16 pixels = 1 tile column/row).
There is no sub-tile granularity in the data byte itself. To shift a sprite by
less than one tile, the spawn code in PRG must be patched to bias the in-RAM
pixel position after the data is read.

### Per-ID Sprite Anchoring Quirks

Some object sprites render with a hardcoded pixel offset from their data-byte
anchor — meaning the sprite does not appear centered on the (X, Y) tile in the
data. When randomization swaps an enemy whose anchor convention differs from
the slot's original tenant, the replacement looks visually offset.

Known cases:

- **`OBJ_BIGREDPIRANHA` (0x7F)** — sprite renders approximately **8 pixels
  (½ tile) right** of its data X. When BRP is randomized into a slot that
  previously held a different piranha (e.g. `OBJ_VENUSFIRETRAP` 0xA6 in 1-1),
  the BRP head visibly leans right of the pipe. Decrementing the data X by 1
  overshoots in the opposite direction by the same ~½ tile (verified by
  comparing X+0, X−1, X−2, X−3 test ROMs). A correct fix would require a
  6502 patch in PRG004 (where IDs 0x6C–0x8F live) to add +8 to the sprite's
  pixel X at spawn — not implemented; the cosmetic offset is left as-is.

- **`OBJ_BOSSATTACK` (0x75)** — these are the **Bowser-statue fireballs**
  in 8-Bowser, NOT a generic boss attack despite the name. Per the
  southbird disasm at `PRG/prg004.asm:455`:
  > "NOTE: This initialization state is used ONLY for the in-level Bowser
  > Fireballs (that you see just prior to Bowser himself), even though
  > this object is actually intended for use by Bowser or the Koopalings
  > as a respective attack."
  >
  At spawn, `ObjInit_BossAttack` (prg004.asm:453) sets XVel toward player
  (±0x10), Var5 = Var4 = 2, and queues the flame sound. They arc toward
  the player. There is no separate "statue" object — the statue head is
  drawn by background art at the spawn position.

### Enemy Data Segment Layout (8-Bowser, 5-F2)

These two segments have known "shooter/projectile" layouts where the
projectile origins line up with visible-art statue or podoboo positions.
Documented here because randomization must respect the position pool,
not just the obj_id, to keep the visuals coherent.

#### 8-Bowser Sub-Area 1 (file offset `0xD61B`, 14 entries)

The pre-Bowser corridor. Vanilla:

| # | Offset | ID | X | Y | Role |
|---|--------|----|----|----|------|
| 0 | 0xD61C | 0x3F | 0x04 | 0x18 | DryBones |
| 1 | 0xD61F | 0x3F | 0x0A | 0x18 | DryBones |
| 2 | 0xD622 | 0x8C | 0x16 | 0x10 | ThwompRightSlide |
| 3 | 0xD625 | 0xD0 | 0x40 | 0x15 | CFIRE_LASER (statue 1) |
| 4–7 | … | 0x75 | 0x62–0x7E | 0x15–0x17 | Fireballs |
| 8 | 0xD634 | 0xD0 | 0xA3 | 0x16 | CFIRE_LASER (statue 2) |
| 9–13 | … | 0x75 | 0xD1–0xE5 | 0x14–0x17 | Fireballs |

**9 known laser-capable statue positions** (empirically derived by
diffing 21 fred-randomizer outputs — fred ships a curated table; we
lifted these from observed laser placements):

```
(0x40, 0x15)   (0x45, 0x16)   (0x4C, 0x14)   (0x52, 0x15)
(0x7C, 0x11)   (0xA3, 0x16)   (0xA9, 0x13)   (0xB0, 0x13)   (0xBC, 0x13)
```

Vanilla uses `(0x40, 0x15)` and `(0xA3, 0x16)`. The other 7 are
"shootable but vanilla-decorative" — the level art has a statue head
there but no `CFIRE_LASER` entry points at it. SMB3R's `bowser_castle`
composer picks any 2 of the 9 and writes them at the two laser entry
slots, coordinating fireball placement so the segment stays X-sorted.

Fireball X spread in fred's data spans the whole segment
(`0x18..0xEF`). Y values cluster in `0x11..0x17` (7-row band).

#### 5-F2 Sub-Area 1 (file offset `0xD2C9`, 26 entries)

Podoboo gauntlet — 16 Podoboo (0x9E) + 6 Ceiling Podoboo (0x53) + 2
DryBones (0x3F) + 2 Boos (0x65). Y high nibble = vertical page (page
0 for ceiling podoboos, page 1 for regular). The composer preserves
the high nibble so a ceiling podoboo can't fall to a regular page or
vice versa.

Tight vanilla X gaps where naive ±2 jitter could break sort order:
`0x0B↔0x0D` (gap 2), `0x2C↔0x2E` (gap 2), `0x78↔0x79` (gap 1).

### Segment Writer Architecture

Enemy data segments are the level loader's input — entries within a
segment must stay in ascending X order or activation timing breaks.
SMB3R routes all segment edits through `src/randomize/segment_writer.rs`
which sorts by X, validates count and X-collision invariants, and
writes back. Per-level "composer" modules (`bowser_castle.rs`,
`podoboo_gauntlet.rs`, etc.) build a full proposed entry list and pass
it through the writer rather than editing bytes directly. This avoids
the class of bug where two independent randomizers touching the same
segment produce sort-order violations or collisions.

### Complete Object ID List

Source: `smb3.asm` from the [Southbird disassembly](https://github.com/captainsouthbird/smb3).

**Special objects (must NEVER be randomized):**

| ID | Name | Description |
|----|------|-------------|
| 0x06 | OBJ_BOUNCEDOWNUP | Down/up block bounce effect |
| 0x07 | OBJ_WARPHIDE | Hidden warp whistle trigger (1-3) |
| 0x08 | OBJ_PSWITCHDOOR | P-Switch door |
| 0x09 | OBJ_AIRSHIPANCHOR | Airship anchor |
| 0x0B | OBJ_POWERUP_1UP | 1-Up Mushroom |
| 0x0C | OBJ_POWERUP_STARMAN | Starman / super suits |
| 0x0D | OBJ_POWERUP_MUSHROOM | Super Mushroom |
| 0x0E | OBJ_BOSS_KOOPALING | Koopaling boss |
| 0x18 | OBJ_BOSS_BOWSER | King Bowser |
| 0x19 | OBJ_POWERUP_FIREFLOWER | Fire Flower |
| 0x1B | OBJ_BOUNCELEFTRIGHT | Left/right block bounce effect |
| 0x1E | OBJ_POWERUP_SUPERLEAF | Super Leaf |
| 0x1F | OBJ_GROWINGVINE | Growing vine |
| 0x21 | OBJ_POWERUP_MUSHCARD | Free mushroom card |
| 0x22 | OBJ_POWERUP_FIRECARD | Free flower card |
| 0x23 | OBJ_POWERUP_STARCARD | Free star card |
| 0x25 | OBJ_PIPEWAYCONTROLLER | Pipe-to-pipe location setter |
| 0x34 | OBJ_TOAD | Toad and house message |
| 0x35 | OBJ_TOADHOUSEITEM | Toad House treasure box item |
| 0x41 | OBJ_ENDLEVELCARD | End-of-level card |
| 0x47 | OBJ_GIANTBLOCKCTL | Giant World block enabler |
| 0x4A | OBJ_BOOMBOOMQBALL | Boom Boom end-level ball |
| 0x4B | OBJ_BOOMBOOMJUMP | Jumping Boom-Boom (boss) |
| 0x4C | OBJ_BOOMBOOMFLY | Flying Boom-Boom (boss) |
| 0x50 | OBJ_BOBOMBEXPLODE | Ready-to-explode Bob-Omb |
| 0x52 | OBJ_TREASUREBOX | Treasure box |
| 0x5C | OBJ_ICEBLOCK | Ice block (held item) |
| 0x75 | OBJ_BOSSATTACK | Bowser-statue fireball (in-level use only) — see Per-ID Sprite Anchoring Quirks |
| 0x84 | OBJ_SPINYEGG | Spiny egg (from Lakitu) |
| 0x85 | OBJ_SPINYEGGDUD | Dud spiny egg |
| 0x94 | OBJ_BIGQBLOCK_3UP | Big ? block (3 1-ups) |
| 0x95 | OBJ_BIGQBLOCK_MUSHROOM | Big ? block (mushroom) |
| 0x96 | OBJ_BIGQBLOCK_FIREFLOWER | Big ? block (fire flower) |
| 0x97 | OBJ_BIGQBLOCK_SUPERLEAF | Big ? block (leaf) |
| 0x98 | OBJ_BIGQBLOCK_TANOOKI | Big ? block (tanooki) |
| 0x99 | OBJ_BIGQBLOCK_FROG | Big ? block (frog suit) |
| 0x9A | OBJ_BIGQBLOCK_HAMMER | Big ? block (hammer suit) |
| 0xB4 | OBJ_CHEEPCHEEPBEGIN | Event: cheep cheep swarm |
| 0xB5 | OBJ_GREENCHEEPBEGIN | Event: spike cheeps |
| 0xB6 | OBJ_LAKITUFLEE | Event: Lakitu flee |
| 0xB7 | OBJ_PARABEETLESBEGIN | Event: parabeetles flyby |
| 0xB8 | OBJ_CLOUDSINBGBEGIN | Event: floating clouds |
| 0xB9 | OBJ_WOODPLATFORMBEGIN | Event: random wood platforms |
| 0xBA | OBJ_TREASUREBOXAPPEAR | Event: treasure box appear |
| 0xBB | OBJ_CANCELEVENT | Event: cancel level event |
| 0xBC–0xD0 | OBJ_CFIRE_* | Cannons, pipes, launchers (21 types) |
| 0xD1 | OBJ_SPAWN3GREENTROOPAS | Spawner: 3 green paratroopas |
| 0xD2 | OBJ_SPAWN3ORANGECHEEPS | Spawner: 3 orange cheep cheeps |
| 0xD3 | OBJ_AUTOSCROLL | Autoscroll controller |
| 0xD4 | OBJ_BONUSCONTROLLER | White Toad House / Coin Ship judge |
| 0xD5 | OBJ_TOADANDKING | Toad and king (end of world) |
| 0xD6 | OBJ_TREASURESET | Treasure box item setter |

**Platforms & environmental objects (must NEVER be randomized):**

| ID | Name | Description |
|----|------|-------------|
| 0x24 | OBJ_CLOUDPLATFORM_FAST | Fast cloud platform |
| 0x26 | OBJ_WOODENPLAT_RIDER | Riding log |
| 0x27 | OBJ_OSCILLATING_H | Horizontal oscillating platform |
| 0x28 | OBJ_OSCILLATING_V | Vertical oscillating platform |
| 0x2C | OBJ_CLOUDPLATFORM | Cloud platform |
| 0x2E | OBJ_INVISIBLELIFT | Invisible lift |
| 0x36 | OBJ_WOODENPLATFORM | Floating wooden platform |
| 0x37 | OBJ_OSCILLATING_HS | Short horizontal oscillation |
| 0x38 | OBJ_OSCILLATING_VS | Short vertical oscillation |
| 0x3A | OBJ_FALLINGPLATFORM | Donut lift platform |
| 0x3C | OBJ_WOODENPLATFORMFALL | Falling wooden platform |
| 0x3E | OBJ_WOODENPLATFORMFLOAT | Floating log (on water) |
| 0x44 | OBJ_WOODENPLATUNSTABLE | Fall-after-touch log |
| 0x49 | OBJ_FLOATINGBGCLOUD | Background cloud |
| 0x54 | OBJ_DONUTLIFTSHAKEFALL | Donut lift shake/fall |
| 0x65 | OBJ_WATERCURRENTUPWARD | Upward water current |
| 0x66 | OBJ_WATERCURRENTDOWNARD | Downward water current |
| 0x90 | OBJ_TILTINGPLATFORM | Tilting platform |
| 0x91 | OBJ_TWIRLINGPLATCWNS | Twirling platform (CW non-stop) |
| 0x92 | OBJ_TWIRLINGPLATCW | Twirling platform (CW) |
| 0x93 | OBJ_TWIRLINGPERIODIC | Twirling platform (periodic) |
| 0x9D | OBJ_FIREJET_UPWARD | Upward fire jet |
| 0xA8 | OBJ_ARROWONE | One-direction arrow platform |
| 0xA9 | OBJ_ARROWANY | Changeable arrow platform |
| 0xAA | OBJ_AIRSHIPPROP | Airship propeller |
| 0xAC | OBJ_FIREJET_LEFT | Left fire jet |
| 0xAE | OBJ_BOLTLIFT | Bolt lift |
| 0xB0 | OBJ_BIGCANNONBALL | Big cannonball |
| 0xB1 | OBJ_FIREJET_RIGHT | Right fire jet |
| 0xB2 | OBJ_FIREJET_UPSIDEDOWN | Upside-down fire jet |

**Enemies (safe to randomize within class):**

| ID | Name | Class |
|----|------|-------|
| 0x29 | OBJ_SPIKE | Ground |
| 0x2A | OBJ_PATOOIE | Ground |
| 0x2B | OBJ_GOOMBAINSHOE | Ground (Kuribo's Shoe) |
| 0x33 | OBJ_NIPPER | Ground |
| 0x39 | OBJ_NIPPERHOPPING | Ground |
| 0x3F | OBJ_DRYBONES | Ground |
| 0x40 | OBJ_BUSTERBEATLE | Ground |
| 0x55 | OBJ_BOBOMB | Ground |
| 0x6B | OBJ_PILEDRIVER | Ground |
| 0x70 | OBJ_BUZZYBEATLE | Ground |
| 0x71 | OBJ_SPINY | Ground |
| 0x72 | OBJ_GOOMBA | Ground |
| 0x6C | OBJ_GREENTROOPA | Koopa (shell-bearing) |
| 0x6D | OBJ_REDTROOPA | Koopa (shell-bearing) |
| 0x7A | OBJ_BIGGREENTROOPA | Big enemy |
| 0x7B | OBJ_BIGREDTROOPA | Big enemy |
| 0x7C | OBJ_BIGGOOMBA | Big enemy |
| 0x7E | OBJ_BIGGREENHOPPER | Big enemy |
| 0x6E | OBJ_PARATROOPAGREENHOP | Flying |
| 0x6F | OBJ_FLYINGREDPARATROOPA | Flying |
| 0x73 | OBJ_PARAGOOMBA | Flying |
| 0x74 | OBJ_PARAGOOMBAWITHMICROS | Flying |
| 0x80 | OBJ_FLYINGGREENPARATROOPA | Flying |
| 0x61 | OBJ_BLOOPERWITHKIDS | Water |
| 0x62 | OBJ_BLOOPER | Water |
| 0x63 | OBJ_BIGBERTHABIRTHER | Water |
| 0x64 | OBJ_CHEEPCHEEPHOPPER | Water |
| 0x6A | OBJ_BLOOPERCHILDSHOOT | Water |
| 0x81 | OBJ_HAMMERBRO | Bro |
| 0x82 | OBJ_BOOMERANGBRO | Bro |
| 0x86 | OBJ_HEAVYBRO | Bro |
| 0x87 | OBJ_FIREBRO | Bro |
| 0xA0 | OBJ_GREENPIRANHA | Piranha |
| 0xA1 | OBJ_GREENPIRANHA_FLIPPED | Piranha |
| 0xA2 | OBJ_REDPIRANHA | Piranha |
| 0xA3 | OBJ_REDPIRANHA_FLIPPED | Piranha |
| 0xA4 | OBJ_GREENPIRANHA_FIRE | Piranha |
| 0xA5 | OBJ_GREENPIRANHA_FIREC | Piranha |
| 0xA6 | OBJ_VENUSFIRETRAP | Piranha |
| 0xA7 | OBJ_VENUSFIRETRAP_CEIL | Piranha |
| 0x77 | OBJ_GREENCHEEP | Cheep |
| 0x88 | OBJ_ORANGECHEEP | Cheep |

**Other enemies (not randomized — unique behavior):**

| ID | Name | Description |
|----|------|-------------|
| 0x17 | OBJ_SPINYCHEEP | Spiny cheep (unique water enemy) |
| 0x2D | OBJ_BIGBERTHA | Big Bertha (eats player) |
| 0x2F | OBJ_BOO | Boo Diddly |
| 0x30 | OBJ_HOTFOOT_SHY | Hot Foot (shy variant) |
| 0x31 | OBJ_BOOSTRETCH | Stretch Boo (upright) |
| 0x32 | OBJ_BOOSTRETCH_FLIP | Stretch Boo (flipped) |
| 0x3B | OBJ_CHARGINGCHEEPCHEEP | Charging cheep cheep |
| 0x3D | OBJ_NIPPERFIREBREATHER | Fire-breathing nipper |
| 0x42 | OBJ_CHEEPCHEEPPOOL2POOL | Pool-hopping cheep (3 pool) |
| 0x43 | OBJ_CHEEPCHEEPPOOL2POOL2 | Pool-hopping cheep (2 pool) |
| 0x45 | OBJ_HOTFOOT | Hot Foot (random walk) |
| 0x46 | OBJ_PIRANHASPIKEBALL | Tall plant with spike ball |
| 0x48 | OBJ_TINYCHEEPCHEEP | Tiny cheep cheep |
| 0x4F | OBJ_CHAINCHOMPFREE | Chain chomp (freed) |
| 0x51 | OBJ_ROTODISCDUAL | Dual rotodisc (CW sync) |
| 0x53 | OBJ_PODOBOOCEILING | Podoboo from ceiling |
| 0x56 | OBJ_PIRANHASIDEWAYSLEFT | Sideways piranha (left) |
| 0x57 | OBJ_PIRANHASIDEWAYSRIGHT | Sideways piranha (right) |
| 0x58 | OBJ_FIRECHOMP | Fire Chomp |
| 0x59 | OBJ_FIRESNAKE | Fire Snake |
| 0x5A | OBJ_ROTODISCCLOCKWISE | Rotodisc (CW) |
| 0x5B | OBJ_ROTODISCCCLOCKWISE | Rotodisc (CCW) |
| 0x5D | OBJ_TORNADO | Tornado |
| 0x5E | OBJ_ROTODISCDUALOPPOSE | Dual rotodisc (opposed H) |
| 0x5F | OBJ_ROTODISCDUALOPPOSE2 | Dual rotodisc (opposed V) |
| 0x60 | OBJ_ROTODISCDUALCCLOCK | Dual rotodisc (CCW sync) |
| 0x67 | OBJ_LAVALOTUS | Lava lotus |
| 0x68 | OBJ_TWIRLINGBUZZY | Twirling buzzy beetle |
| 0x69 | OBJ_TWIRLINGSPINY | Twirling spiny |
| 0x76 | OBJ_JUMPINGCHEEPCHEEP | Jumping cheep cheep |
| 0x78 | OBJ_BULLETBILL | Bullet Bill |
| 0x79 | OBJ_BULLETBILLHOMING | Homing Bullet Bill |
| 0x7D | OBJ_BIGGREENPIRANHA | Big green piranha |
| 0x7F | OBJ_BIGREDPIRANHA | Big red piranha |
| 0x83 | OBJ_LAKITU | Lakitu |
| 0x89 | OBJ_CHAINCHOMP | Chain Chomp |
| 0x8A | OBJ_THWOMP | Thwomp (standard) |
| 0x8B | OBJ_THWOMPLEFTSLIDE | Thwomp (left slide) |
| 0x8C | OBJ_THWOMPRIGHTSLIDE | Thwomp (right slide) |
| 0x8D | OBJ_THWOMPUPDOWN | Thwomp (up/down) |
| 0x8E | OBJ_THWOMPDIAGONALUL | Thwomp (diagonal UL) |
| 0x8F | OBJ_THWOMPDIAGONALDL | Thwomp (diagonal DL) |
| 0x9E | OBJ_PODOBOO | Podoboo |
| 0x9F | OBJ_PARABEETLE | Parabeetle |
| 0xAD | OBJ_ROCKYWRENCH | Rocky Wrench |
| 0xAF | OBJ_ENEMYSUN | Angry Sun |

---

## Power-Up / Item Data

### Global Item ID Table

| ID | Item |
|----|------|
| 0x00 | Nothing / Mushroom (context-dependent) |
| 0x01 | Mushroom |
| 0x02 | Fire Flower |
| 0x03 | Super Leaf (Raccoon) |
| 0x04 | Frog Suit |
| 0x05 | Tanooki Suit |
| 0x06 | Hammer Suit |
| 0x07 | Jugem's Cloud |
| 0x08 | P-Wing |
| 0x09 | Starman |
| 0x0A | Anchor |
| 0x0B | Hammer |
| 0x0C | Warp Whistle |
| 0x0D | Music Box |

### Inventory Item-Use Dispatch (PRG026)

When the player uses an inventory item on the map, `Inv_UseItem` at CPU $A53A (file 0x3454A)
loads the item ID from `$7D80,Y` and dispatches via a `DynJump` table at CPU $A540 (file 0x34550).

**DynJump table (14 word entries, little-endian):**

| Index | Item | Handler | CPU Addr | File Offset |
|-------|------|---------|----------|-------------|
| 1 | Mushroom | Inv_UseItem_Powerup | $A5B6 | 0x345C6 |
| 2 | Fire Flower | Inv_UseItem_Powerup | $A5B6 | 0x345C6 |
| 3 | Super Leaf | Inv_UseItem_Powerup | $A5B6 | 0x345C6 |
| 4 | Frog Suit | Inv_UseItem_Powerup | $A5B6 | 0x345C6 |
| 5 | Tanooki Suit | Inv_UseItem_Powerup | $A5B6 | 0x345C6 |
| 6 | Hammer Suit | Inv_UseItem_Powerup | $A5B6 | 0x345C6 |
| 7 | Cloud | Inv_UseItem_Powerup | $A5B6 | 0x345C6 |
| 8 | P-Wing | Inv_UseItem_Powerup | $A5B6 | 0x345C6 |
| 9 | Starman | Inv_UseItem_Starman | $A672 | 0x34682 |
| 10 | Anchor | Inv_UseItem_Anchor | $A682 | 0x34692 |
| 11 | Hammer | Inv_UseItem_Hammer | $A6BC | 0x346CC |
| 12 | Whistle | Inv_UseItem_Whistle | $A705 | 0x34715 |
| 13 | Music Box | Inv_UseItem_MusicBox | $A733 | 0x34743 |

Items 1–8 all route to the shared `Inv_UseItem_Powerup` handler. Items 9+ have dedicated
handlers with incompatible animation/state machine layouts.

Inside `Inv_UseItem_Powerup`, the instruction `LDX $7D80,Y` at CPU $A5C8 (file 0x345D8)
re-reads the item ID into X. This value drives the powerup animation state machine in
PRG031 via `$07F5`. The handler also stores the item to `$07F5` at $A5D8.

`Inv_UseItem_Anchor` ($A682) sets `Map_Anchored`, plays the anchor sound, removes the
item from inventory, and returns — it never enters the powerup animation path.

### Inventory Item Draw (PRG026)

`Inventory_DrawItemsOrCards` at CPU **$A366** (file **0x34376**) draws every non-empty
reserve slot. Per slot it does:

```
LDY $0D                ; slot index 0-27
LDA $7D80,Y            ; item ID (file 0x34378 = B9 80 7D)
BEQ +0x1E              ; empty? skip past draw
ASL A / ASL A / TAY    ; Y = item * 4 (file 0x3437D = 0A 0A A8)
LDA $03E5 / AND #$07 / CMP #$04 / BEQ +4 / TYA / ORA #$02 / TAY  ; bottom-row select
LDX $0C
LDA ($0E),Y            ; fetch from InvItem_Tile_Layout
STA $0301,X            ; -> sprite RAM
...
```

`InvItem_Tile_Layout` is 14 rows × 4 bytes (one row per Global Item ID 0x00-0x0D); each
row is a 2×2 grid of 8×8 CHR pattern IDs. The Anchor row (item 0x0A) is at table
offset **0x28** and contains `02 03 12 13`.

Palette is hardcoded `#$03` (no per-item attribute table; the routine STAs `#$03` to the
two attribute slots of each sprite pair).

**Patch site to force every drawn slot to a single item's tiles:** replace `0A 0A A8` at
file `0x3437D` with `LDY #<offset>; NOP`. The `LDA $7D80,Y / BEQ` prologue is preserved
so empty slots still skip; non-empty slots fetch from the chosen row.

**Hilite (cursor-selected slot)** uses a separate routine `Inv_Display_Hilite` at CPU
**$A86B** (file **0x3487B**) with its own table `InvItem_Hilite_Layout` at CPU **$A84C**
(file **0x3485C**) — 14 rows × 2 bytes (left/right CHR pattern). The routine loads the
hovered slot's item ID via `LDX $7D80,Y` at CPU $A88E (file 0x3489E) then computes the
2-byte-stride index with `TXA; ASL A; TAX` at CPU $A899 (file **0x348A9** = `8A 0A AA`).
**Patch:** `0x348A9` `8A 0A AA` → `A2 <id*2> EA` (`LDX #<idx>; NOP`). For the Anchor
(`InvItem_Hilite_Layout` row offset `0x14` = `95 97`) the patch is `A2 14 EA`.

**Hilite palette** is uploaded separately by `InvItem_SetColor` (called from
`Inventory_DoHilites`). The vanilla per-item palette table `InvItem_Pal` at CPU **$A514**
(file **0x34524**, 14 bytes) is read via `LDA InvItem_Pal,X` at CPU $A52A
(file **0x3453A** = `BD 14 A5`); the value is then written to `Palette_Buffer+$1E`
($07DF), which colors the highlighted slot's tiles. The routine early-exits when
`Level_Tileset == 7` (Toad House interior) so it has no effect there.
**Patch site to lock the hilite color to a single palette entry:** `0x3453A` `BD 14 A5` →
`A9 <pal> EA`. For the Anchor (`InvItem_Pal[0x0A] = $07`) the patch is `A9 07 EA`.

### Toad House Item Reveal (PRG002)

Toad House chests are object **OBJ $35 / `OBJ_TOADHOUSEITEM`**, distinct from in-level
treasure boxes (OBJ $52). The handler `ObjNorm_ToadHouseItem` lives in PRG002
(southbird `PRG/prg002.asm` lines 4082-4240). It reads `Objects_Frame,X` (the actual
item ID, RAM `$0669`) three times:

| CPU | File | Role | Safe to redirect for visual-only? |
|-----|------|------|-----------------------------------|
| $B4F7 | 0x05507 | `LDY Objects_Frame,X` then `LDA ToadItem_PalPerItem,Y` — sets BG palette | yes |
| $B55A | 0x0556A | `LDA Objects_Frame,X` then `STA Inventory_Items,Y` — gives item to player | **no** — patching changes the reward |
| $B57A | 0x0558A | `LDA Objects_Frame,X` then `TAX; LDA ToadItem_PatternLeft-1,X; ...` — selects floating-item sprite tiles + attr | yes |

**Patches to force the visual reveal to a fixed item without changing the reward:**
- `0x05507` `BC 69 06` → `A0 <id> EA` (`LDY #<id>; NOP`) — for Anchor: `A0 0A EA`.
- `0x0558A` `BD 69 06` → `A9 <id> EA` (`LDA #<id>; NOP`) — for Anchor: `A9 0A EA`.
- Leave `0x0556A` alone so the player still receives the real item.

### Treasure Box (In-Level Chest) Draw

The Toad-House / fortress chest object is handled in PRG003:
- `ObjInit_TreasureBox` at CPU **$A297** (file **0x62A7**)
- `ObjNorm_TreasureBox` at CPU **$A2C6** (file **0x62D6**) with a per-item branch at
  CPU **$A33A** (file **0x634A**)

Both handlers read the chest's payload from **`Level_TreasureItem`** at RAM **`$7963`**
via `LDA $7963` (3 bytes `AD 63 79`). Three reads total in PRG003:

| CPU | File | Role | Safe to redirect for visual-only? |
|-----|------|------|-----------------------------------|
| $A297 | 0x62A7 | Init: seeds palette via `ToadItem_PalPerItem,Y` + stores `Objects_Var5,X` | yes |
| $A321 | 0x6331 | Calls `Player_GetItem` — actually awards the item to the player | **no** — patching changes the reward |
| $A33A | 0x634A | Sets `Objects_Frame,X` (sprite frame) + indexes `TBoxItem_MirrorFlags,Y` | yes |

`TBoxItem_MirrorFlags` (PRG003, just before `ObjNorm_IceBlock`) is 14 bytes:
`00 81 82 03 80 81 82 03 00 81 02 03 00 01` — item 0x0A (Anchor) = `02`.

`ToadItem_PalPerItem` lives in PRG000 (CPU base $C000) around CPU $C400; full 14-byte
table: `30 16 2A 2A 2A 17 27 36 27 30 07 36 27 27` (item 0x0A = `$07`).

**Patch sites to force the visual reveal to a specific item without changing the
reward:** replace `AD 63 79` at `0x62A7` and `0x634A` with `LDA #<id>; NOP`
(`A9 <id> EA`). Leave `0x6331` alone so `Player_GetItem` still receives the real item.

### LATP_QBlocks — ? Block Item Table

File offset: **0x1168D** (17 bytes, in PRG008)

This table maps ? block tile types to the item they produce. Tile IDs start at
`TILEA_QBLOCKFLOWER` ($60), so tile $60 = index 0, tile $61 = index 1, etc.

| Index | Tile | Name | Default | Item |
|-------|------|------|---------|------|
| 0 | $60 | QBLOCKFLOWER | $01 | Mushroom / Fire Flower |
| 1 | $61 | QBLOCKLEAF | $02 | Super Leaf |
| 2 | $62 | QBLOCKSTAR | $03 | Starman |
| 3 | $63 | QBLOCKCOIN | $04 | Coin |
| 4 | $64 | QBLOCKCOINSTAR | $05 | Coin or Star |
| 5 | $65 | QBLOCKCOIN2 | $04 | Coin |
| 6 | $66 | MUNCHER | $00 | Mushroom (context) |
| 7+ | $67+ | BRICK etc. | varies | Bricks, special blocks |

LATP item IDs (different from Global Item IDs and Player_Suit values):
- $00 = Mushroom (context-dependent)
- $01 = Mushroom / Fire Flower
- $02 = Super Leaf
- $03 = Starman
- $04 = Coin
- $05 = Coin or Star

**Important:** Index 1 (leaf) must not be randomized — World 6-5 requires a
leaf ? block to beat the level (flying needed).

### Other Block / Power-Up Offsets

| File Offset | Size | Description |
|------------|------|-------------|
| 0x02611–0x02618 | 8 bytes | Bumped block attribute data (unknown secondary table) |
| 0x0261B–0x0262A | 16 bytes | Bumped block tile mappings |
| 0x003F0–0x003F6 | 7 bytes | Power-up properties (bit format: xxxxxx SF, S=no slide, F=flight) |
| 0x024EE–0x024F4 | 7 bytes | ? Block sprite output table |
| 0x01A3E | 1 byte | Post-hit transformation (what form after taking damage) |

### Player Power-Up States (RAM)

| Value | Form |
|-------|------|
| 0x00 | Small Mario |
| 0x01 | Super Mario |
| 0x02 | Fire Mario |
| 0x03 | Raccoon Mario |
| 0x04 | Frog Mario |
| 0x05 | Tanooki Mario |
| 0x06 | Hammer Mario |

---

## Palette Data

### NES Palette Notes

Valid NES colors: 0x00–0x3F (64 colors). Avoid 0x0D (causes issues on some hardware).
Each palette entry is typically 3 color bytes + 1 shared background color.

### Character Palettes

| File Offset | Size | Description |
|------------|------|-------------|
| 0x10539–0x1053C | 4 bytes | Small/Big/Raccoon Mario |
| 0x1053D–0x10540 | 4 bytes | Small/Big/Raccoon Luigi |
| 0x10541–0x10544 | 4 bytes | Fire Mario/Luigi |
| 0x10549–0x1054C | 4 bytes | Frog Mario/Luigi |
| 0x1054D–0x10550 | 4 bytes | Tanooki Mario/Luigi |
| 0x10551–0x10554 | 4 bytes | Hammer Mario/Luigi |

### Other Palettes

| File Offset | Size | Description |
|------------|------|-------------|
| 0x36DAA–0x36DAD | 4 bytes | Lava / Rotodisc palette |
| 0x36DFE–0x36E01 | 4 bytes | Bowser / Donut Lift palette |

### Per-Level Palette Selection

Each level header byte 5 (`_abbccddd`) embeds a `c` field (object palette, 2 bits)
and a `d` field (BG palette, 3 bits) — see *Level Header Format*. The values index
into per-tileset palette tables loaded by the level loader. The same value can mean
different colors in different tilesets (e.g., BG palette index 2 in plains is greens,
but in fortress it is grays).

### Per-Tileset / Per-Area Palette Tables (PRG012–PRG013)

These are PPU-upload "scripts" — sequences of `00 3F xx LL <LL bytes>` blocks that
the SMB3 PPU upload routine streams directly to PPU `$2007`. The leading `00 3F xx`
is the destination VRAM address (palette area starts at `$3F00`); `LL` is the byte
count; the body is raw NES color bytes. Identified by reverse-engineering the
"Super Mario Bros. 3 Recolored v1.0" IPS — every cluster below is wholly rewritten
by Recolored, proving these are the master per-tileset/area palette tables.

| File Offset | Size | Pattern | Likely Purpose |
|-------------|------|---------|----------------|
| 0x33046–0x331A2 | 349 B | 8 × `00 3F 00 20 0F 0F …32 colors…` (32-byte full BG+sprite palette set) | **Per-tileset full-palette upload table** — 8 entries; one per BG palette index used by level loader |
| 0x331BB–0x331DE | 35 B  | dense ≤0x3F bytes | Adjunct palette set (FG vs BG?) |
| 0x331EE–0x331F8 | 11 B  | dense ≤0x3F bytes | Small palette block (3 entries × ≈4 B) |
| 0x33201–0x3320B | 11 B  | dense ≤0x3F bytes | Small palette block |
| 0x33214–0x33277 | 100 B | mixed (palette bytes + 6502 code patterns `bd 03 ff`/`2a`) | Palette upload **routine** (FG draw helper) |
| 0x332A5–0x3338E | 234 B | 8 × `00 3F 1x` mini-uploads + interspersed code | Sprite-palette dispatch routine |
| 0x333A7–0x333B1 | 11 B  | dense ≤0x3F bytes | Small palette block |
| 0x333CA–0x333D4 | 11 B  | dense ≤0x3F bytes | Small palette block |
| 0x333ED–0x333F7 | 11 B  | dense ≤0x3F bytes | Small palette block |
| 0x33410–0x33496 | 135 B | 4 × `00 3F 00 20 0F 0F …16 bytes…` | **Per-area BG palette set** — 4 entries (likely sky/forest/water/dark) |
| 0x3349D–0x334AB | 15 B  | dense ≤0x3F bytes | Small palette block |
| 0x334C4–0x33530 | 109 B | 5 × `00 3F 10 10 0F 0F …16 bytes…` (sprite palettes only) | **Per-area sprite palette set** — 5 entries; loads only `$3F10–$3F1F` |
| 0x36BE4–0x36DA5 | ~450 B | per-palette-slot sub-tables of ~56 B each | **Themed palette slot table** — bands likely correspond to BG palette indices (`d` field of level header byte 5); levels share slots across tilesets. Rainbow probe confirmed: |
| 0x36BE4–0x36C1C | 56 B  |  | (band 0, red) **Used by W6 sky overworld map + map HUD** |
| 0x36C1C–0x36C54 | 56 B  |  | (band 1, orange) **Used by W7 (pipe) overworld map** |
| 0x36C54–0x36C8C | 56 B  |  | (band 2, yellow) **Used by hammer bro overworld sprites + "HELP" message text + world-label sprites** |
| 0x36C8C–0x36CC4 | 56 B  |  | (band 3, green) **Used by plains 1-1 BG + HUD** (confirmed via targeted single-band probe). Writes $3F00 universal BG + likely sprite palette 0. |
| 0x36CC4–0x36CFC | 56 B  |  | (band 4, cyan) **Used by giant tileset (W4)** |
| 0x36CFC–0x36D34 | 56 B  |  | (band 5, blue) **Used by plains enemies AND W7-5 sub-area BG** (shared slot) |
| 0x36D34–0x36D6C | 56 B  |  | (band 6, purple) **Used by W4-F1 and W8 fortress HUD** + some W8 brick/door tiles |
| 0x36D6C–0x36DA6 | 56 B  |  | (band 7, magenta) **Used by fortress BG (windows, bricks) AND W7-5 sub-area enemies AND most of W4-F1** (shared slot) |
|                 |        |  | **Coverage caveat**: rainbow probe affected overworld + specific fortress/sub-area levels but NOT the majority of regular levels — those load palettes from a *different* table (most likely 0x33046 et al., still untested). |
|                 |        |  | **Note**: opening inventory triggers a palette re-upload that reverts level-screen palettes to vanilla mid-frame, then restores them on close |
| 0x36DAA–0x36DAD | 4 B   | (pre-existing) Lava/Rotodisc | (already known) |
| 0x36DFE–0x36E01 | 4 B   | (pre-existing) Bowser/Donut | (already known) |
| 0x36E20–0x36EBD | 158 B | 4-byte palette quartets ending in `0f` | Per-tileset palette quartet table (~40 palettes); not yet confirmed empirically |
| 0x36EE2–0x37000 | 286 B | mixed alignment, ~36-byte sub-tables | Confirmed sub-regions (W6 sky and water tested across 5 tilesets): |
| 0x36F00–0x36F05 | 5 B   |  | Drives a water-context sprite palette (circular underwater sprites) |
| 0x36F4B–0x36F6E | 35 B  |  | Drives sky-tileset enemies + animated note-block frames (W6 sky only) |
| 0x36EE2–0x36F05 (rest) | 30 B | | Subtle effects only at fine granularity; previous "HUD red" reading was a $3F00 universal-background mirror artifact when entire range was one color |
| 0x36F05–0x36F4B,0x36F6E–0x37000 | rest | | Untested across all tilesets; band-per-tileset hypothesis unverified |
| 0x37000–0x37200 | 512 B  | 8 × ~64 B sub-tables | **Water-tileset palette pool** (CONFIRMED): |
|                 |        |                      | • band 1 (0x37040–0x37080) = underwater BG (W2-1) |
|                 |        |                      | • band 3 (0x370C0–0x37100) = water-level enemies (W2-1) |
|                 |        |                      | • other bands had no visible effect on plains/sky/desert/underground/fortress, so this slice appears to be water-specific |
| 0x37200–0x37400 | 512 B  | 8 × ~64 B sub-tables | **Desert + fortress + airship palette pool** (CONFIRMED): |
|                 |        |                      | • band 2 (0x37280–0x372C0) = desert BG (2-1) |
|                 |        |                      | • band 3 (0x372C0–0x37300) = fortress HUD + highlights (2-F) |
|                 |        |                      | • band 4 (0x37300–0x37340) = desert enemies (2-1) |
|                 |        |                      | • band 5 (0x37340–0x37380) = airship BG/HUD + fortress enemies (2-F, 1-airship, 2-airship) |
|                 |        |                      | • band 6 (0x37380–0x373C0) = airship foreground variants (2-airship, 3-airship) |
|                 |        |                      | • band 7 (0x373C0–0x37400) = airship enemies (1/2/3 airships) |
|                 |        |                      | • bands 0/1 untested (likely additional fortress variants) |
| 0x37400–0x37600 | 512 B  | 8 × ~64 B sub-tables | **Giant tileset + water pipe/decoration palettes** (CONFIRMED): |
|                 |        |                      | • band 0 (0x37400–0x37440) = giant BG (W4-1) |
|                 |        |                      | • band 2 (0x37480–0x374C0) = giant enemies (W4-1) |
|                 |        |                      | • band 5 (0x37540–0x37580) = water-tileset pipe accents (W3-1) |
|                 |        |                      | • band 7 (0x375C0–0x37600) = water decoration (W3-1) |
|                 |        |                      | • other bands did not light up in plains/sky/underground/fortress |
| 0x37600–0x377DF | 480 B  | palette data (8 × ~60 B bands) | Slice 4 — partial: |
|                 |        |                      | • band 0 (0x37600–0x3763C) = Sky-Land (W5) enemy palette (observed in 5-7, 5-8) |
|                 |        |                      | • band 3 (0x376B4–0x376F0 safe / 0x376D8–0x37720 full) = Plains 1-1 BG palette variant (observed under full slice 4 probe) |
|                 |        |                      | • band 5 (0x37540–0x37580 safe / 0x37768–0x377B0 full) = Plains 1-1 enemy palette |
|                 |        |                      | • other bands untested in sky-bg/underground/ice/hilly |
| 0x377E0–0x37807 | ~40 B  | **level layout CPU pointer table** (pointers in `$ABD2-$B412` range) | **DO NOT PAINT — painting crashes level loading.** Pointer table used by the level loader to resolve layout/enemy references. |
| 0x37808–0x37846 | ~60 B  | palette data | Slice 4-B — separate paint probe if needed |
|                 |        |  | **Lesson**: the "master pool" 0x36EE2-0x37846 is NOT pure palette data. Interleaved pointer tables / lookup tables must be preserved. Any randomizer needs per-sub-region byte maps to know what's safe to touch. |

> **Empirical confirmations** are from `tools/gen_palette_probes.py` runs in an emulator
> (paint each table to NES `0x24` hot magenta, observe which graphics turn pink).
> Probes apply `smb3practice_SE.ips` for warp whistles + level select + open movement
> so all worlds are reachable. Filenames: `test_roms/palette_probe_<name>_wN.nes`.

> **Quartet alignment varies** across these tables — outline `0F` is at byte 2 in
> 0x36BE4 but at byte 1 in 0x36EE2. Hardcoding "outline at byte 3" is unsafe; either
> probe each table for its alignment, or paint every byte that isn't `0x00` or `0x0F`
> (the `raw` painter strategy in `gen_palette_probes.py`).

> **Note**: Specific table semantics (tileset assignment, index mapping) are inferred from
> structural patterns and the Recolored IPS, not yet verified against the SMB3 disassembly.
> Confirm with disassembly cross-reference before basing critical writes on these offsets.
>
> Diagnostic tool: `nix-shell -p python3 --run 'python3 tools/palette_inspect.py'` dumps
> every Recolored cluster, classifies it, and shows vanilla vs. recolored hex side-by-side.

### Jump Engine — `$FE99` (Fixed Bank, NOT Palette-Specific)

`$FE99` (file 0x3FEA9) is the SMB3 **inline-table jump engine** (often labeled
`Jump_Engine` / `JE` in disassemblies):

```
$FE99: 0A          ASL A          ; index *= 2
       A8          TAY
       68 85 00    PLA; STA $00   ; pull return-addr lo
       68 85 01    PLA; STA $01   ; pull return-addr hi → $0000/$0001 = base of inline table
       C8 B1 00    INY; LDA ($00),Y
       85 02       STA $02
       C8 B1 00    INY; LDA ($00),Y
       85 03       STA $03
       6C 02 00    JMP ($0002)    ; indirect-jump to selected handler
```

Calling convention: `JSR $FE99 / .DW handler0, handler1, …` with the index in `A`.
Used by ~105 callsites across all 16 PRG banks for general state-machine dispatch
(not exclusively palette code). Anything that wants to wholesale replace SMB3
behavior often hooks here — the Recolored IPS, for example, relocates this routine
to `$FE92` and rewrites every `JSR $FE99` to `JSR $FE92` so it can wedge custom
logic into the engine without rebuilding callers.

### Title Screen

| File Offset | Description |
|------------|-------------|
| 0x30ABA–0x30AC1 | Title screen "3" flashing color sequence |
| 0x32AC2+ | Title screen background fade sequences |
| 0x32AFE | Title screen background final color |
| 0x317B1 | Sprite loop hook: vanilla `JSR $B7D6`, patched to `JMP $E914` for seed hash sprites |
| 0x31976 | Sprite palette data (4 palettes × 4 bytes); palette 3 is modifiable here |
| 0x3E924 | Free space in PRG031 (CPU $E914): seed hash sprite copy routine (25 bytes) |
| 0x3E93D | Free space in PRG031 (CPU $E92D): seed hash sprite data table (40 bytes) |

**Title screen seed hash sprites:** 5 icons displayed vertically in the top-left corner.
Each icon is 16×16 (two 8×16 sprites side by side). Uses 8x16 sprite mode — odd tile IDs
select PT1 ($1000–$1FFF). Tiles can be drawn from any CHR slot (R2–R5) since the slot is
determined by tile ID, not a global setting. The ASM routine copies sprite data to OAM with
stride (every 8th sprite slot) to avoid the 8-sprites-per-scanline hardware limit.

---

## Metatile Banking System

11 metatile banks (0x0C–0x17, 0x1A), each containing 256 slots at CPU $A000 with 1024-byte maps.

| Bank | Tileset Style | BG Bank 0 CHR Page | BG Bank 2 CHR Page |
|------|--------------|-------------------|-------------------|
| 0x0C | World Map | 0x14 (Ani: 70, 72, 74) | 0x16 |
| 0x0D | Underground | 0x1C | 0x60 |
| 0x0E | Battle | 0x58 | 0x60 |
| 0x0F | Plains | 0x08 | 0x60 |
| 0x10 | Hills | 0x1C | 0x60 |
| 0x11 | Mountains/Ice | 0x0C | 0x60 |
| 0x12 | Water/Toad/Pipes | 0x58/0x5C/0x58 | 0x60/0x5E/0x60 |
| 0x13 | Pipe/Giant/Clouds | 0x58/0x6E/0x38 | 0x3E/0x60/0x60 |
| 0x14 | Desert | 0x30 | 0x60 |
| 0x15 | Fortress | 0x10 | 0x60 |
| 0x16 | Bonus/Slots/Cards | 0x24/0x2C/0x5C | 0x5E/0x2E/0x5E |
| 0x17 | Airship | 0x34 | 0x6A |
| 0x1A | HUD | 0x5C | 0x5E |

### Tileset-to-PRG Page Mapping

Maps 19 tilesets (0–18) to their ROM page banks:

```
PAGE_C000_ByTileset: 10, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 22, 22, 22, 14
PAGE_A000_ByTileset: 11, 15, 21, 16, 17, 19, 18, 18, 18, 20, 23, 19, 17, 19, 13, 26, 26, 26,  9
```

Tileset index → PRG bank at $A000 (level data) and $C000 (tileset code).

---

## World Map Data

### Overworld Map Tiles

**Tile grid pointer table:** 9 × 2-byte little-endian CPU pointers at file offset **0x185A8** (CPU $A598, PRG012). Each points to a world's tile grid data. Entry 9 is the Warp Zone.

**Storage format:** Row-major per screen (confirmed from `Map_Reload_with_Completions` in prg012.asm). Each screen is a 144-byte block of 9 rows × 16 columns, stored row-major (16 consecutive bytes per row). Multi-screen worlds have consecutive 144-byte blocks. A `0xFF` terminator byte follows each world's grid data. Total tile data spans **0x185BA–0x19102** (~2.9 KB).

The loading code copies each 144-byte screen block with a sequential `LDA [src],Y / STA [dst],Y` loop (Y = 0..143), then advances the destination pointer by $1B0 for the next screen (the gap accommodates unused vertical space in tile memory).

World maps are stored as raw tile grids (unlike levels which use generators).

**Per-world tile grid details:**

| World | CPU Addr | File Offset | Columns | Screens | Data Bytes | End Offset |
|-------|----------|-------------|---------|---------|------------|------------|
| W1 | $A5AA | 0x185BA | 16 | 1 | 144 + 1 | 0x1864A |
| W2 | $A63B | 0x1864B | 32 | 2 | 288 + 1 | 0x1876B |
| W3 | $A75C | 0x1876C | 48 | 3 | 432 + 1 | 0x1891C |
| W4 | $A90D | 0x1891D | 32 | 2 | 288 + 1 | 0x18A3D |
| W5 | $AA2E | 0x18A3E | 32 | 2 | 288 + 1 | 0x18B5E |
| W6 | $AB4F | 0x18B5F | 48 | 3 | 432 + 1 | 0x18D0F |
| W7 | $AD00 | 0x18D10 | 32 | 2 | 288 + 1 | 0x18E30 |
| W8 | $AE21 | 0x18E31 | 64 | 4 | 576 + 1 | 0x19071 |
| Warp | $B062 | 0x19072 | — | — | — | — |

**Row-major per-screen addressing:** Tile at grid (row R, column C) is at file offset:
```
world_start + (C // 16) * 144 + R * 16 + (C % 16)
```

**36 unique tile IDs** appear under pointer table entries (confirmed via 100% hit rate mapping across all 340 entries in 8 worlds). Key categories:

| Tile ID(s) | Category | Notes |
|------------|----------|-------|
| 0x03–0x0C | Level panel tiles | Border-range tiles reused as level entry dots |
| 0x44 | Path tile | Horizontal path segment |
| 0x47, 0x48, 0x4A, 0x4B | Path tiles | Various directional path segments (most common under entries) |
| 0x50 | Toad house / special | Toad houses and special map nodes |
| 0x5F | Path tile | Rare path variant |
| 0x67 | Fortress tile | Mini-fortress entrance |
| 0x68, 0x69 | Pipe tiles | Map pipe connectors |
| 0xAE, 0xAF | Fortress parts | Alternate fortress tiles |
| 0xB4 | Background (void) | Empty space / water fill (no entries land here) |
| 0xB5, 0xBB, 0xBC | Path/level tiles | Various level-associated tiles |
| 0xC9 | Airship dock | Airship landing tile |
| 0xCC | Bowser's castle | Final castle tile |
| 0xD9, 0xDC–0xDE | Dark Land tiles | W8-specific level tiles |
| 0xE0 | Special node | Alternate toad house / special |
| 0xE5, 0xE6 | Level tiles | Various level entries |
| 0xE8 | Bonus game tile | Spade panel / N-Spade |
| 0xEB | Fortress tile | Alternate fortress |
| 0xFF | Border / unused | Map edge |

### World Map Functionality (PRG010: 0x14010–0x1600F)

Key tables in PRG010 (indexed by World_Num 0–7):

| Label | File Offset | Description |
|-------|-------------|-------------|
| `World_BGM_Arrival` | — | 9-byte table: music track per world (8 worlds + warp zone) |
| `FortressFXBase_ByWorld` | 0x148A8 | 8-byte table: fortress effect indices per world |
| `World_Map_Max_PanR` | 0x14F44 | 8-byte table: max rightward scroll per world (see below) |
| `Map_EnterSpecialTiles` | — | Tile types that trigger level entry (see bug note below) |

**`World_Map_Max_PanR` values** (8 bytes at 0x14F44):

```
W1=0x10, W2=0x20, W3=0x30, W4=0x30, W5=0x00, W6=0x30, W7=0x20, W8=0x00
```

Units: 0x10 = 1 screen of rightward scroll. Screens visible = (value >> 4) + 1.

**Max_PanR vs tile grid size discrepancies:** W4 has Max_PanR=0x30 (4 screens) but only 32 columns (2 screens) of tile data. W5 has Max_PanR=0x00 (1 screen) but 32 columns (2 screens) — the ground/sky halves are stored as 16 columns each. W8 has Max_PanR=0x00 (1 screen) but 64 columns (4 screens) — the linear stage sequence uses different screen segments, not scrolling.

**`Map_EnterSpecialTiles` list:** TOADHOUSE, SPADEBONUS, PIPE, ALTTOADHOUSE, CASTLEBOTTOM,
SPIRAL, ALTSPIRAL, PATHANDNUB, DANCINGFLOWER, HANDTRAP, BOWSERCASTLELL

**Known bug:** The tile entry check loop iterates up to index $1A instead of $0A,
causing subsequent palette data bytes to be incorrectly treated as enterable tile types.

### World-Map Tile Behavior

The behavior of any world-map tile byte (0x00–0xFF) is determined by which of these
small registries it appears in. Each registry adds one behavior; the tile's full
identity is the union of its registry memberships, plus its CHR pattern + palette page.

| Registry | File offset | Size | Effect |
|---|---|---|---|
| Map_EnterSpecialTiles | `0x14DBF` | 11 bytes | Pressing A on tile triggers the uniform "enter level" path (see below) |
| Parallel byte block (unused for dispatch) | `0x14DCA` | 11 bytes | Looks like an op-code table but is **not consumed** by special-entry dispatch — see "Special-entry dispatch is uniform" below |
| Walk LEFT  | `0x15258` | 9 bytes | Tile is walkable leftward |
| Walk RIGHT | `0x15261` | 9 bytes | Walkable rightward |
| Walk DOWN  | `0x1526A` | 9 bytes | Walkable downward |
| Walk UP    | `0x15273` | 9 bytes | Walkable upward |
| Removable obstacles | `0x18447` | 8 bytes | Cleared after fortress beat (locks/rocks/water) |
| Special-completion | `0x18457` | 5 bytes | One-shot tracked in Map_Completions |
| Page thresholds | `0x18410` | 8 bytes | Universal level-gate thresholds per palette page (`03 67 BF E9` duplicated). All worlds use the same values via tileset `0x0E`. See "World-Map Level Gating" below. |
| Background (hardcoded) | — | — | `0x02 0xB4 0xFF` non-walkable |

**Map_EnterSpecialTiles entries** (tile byte → name; the third column is the byte at the
parallel offset `0x14DCA`, kept here for reference but not used for dispatch):

| idx | tile | name | parallel byte |
|---|---|---|---|
| 0 | `0x50` | TOADHOUSE      | `0x16` |
| 1 | `0xE8` | SPADEBONUS     | `0x16` |
| 2 | `0xBC` | PIPE           | `0x27` |
| 3 | `0xE0` | ALTTOADHOUSE   | `0x16` |
| 4 | `0xC9` | CASTLEBOTTOM   | `0x2A` |
| 5 | `0x5F` | SPIRAL         | `0x17` |
| 6 | `0xDF` | ALTSPIRAL      | `0x30` |
| 7 | `0x66` | PATHANDNUB     | `0x16` |
| 8 | `0xBD` | DANCINGFLOWER  | `0x1A` |
| 9 | `0xE6` | HANDTRAP       | `0x0F` |
| 10 | `0xCC` | BOWSERCASTLELL | `0x0F` |

Per-tile visual differences come from the metatile pattern bank (PRG012 bank 0x0C) and
the palette page encoded in the high 2 bits of the tile byte — not from any per-tile
handler dispatch.

#### Special-entry dispatch is uniform

A common misreading of the table at `0x14DCA` is that it acts as a parallel jump-target
table — that pressing A on `0xBC` (PIPE) dispatches to handler `0x27`, on `0xE6`
(HANDTRAP) to handler `0x0F`, etc. This is **wrong**. Tracing the PRG010 search loop:

```
$CEC9: CMP $CDAF,Y              ; search Map_EnterSpecialTiles (Y from $1A down)
       BEQ $CEA7                ; on match — Y is the matched index
       ...
$CEA7: LDA #$10                 ; ALL matches set the same Map_Operation
       STA $0729                ; → "begin enter level" effect
       ...
       JMP $CF29                ; continue into pre-level-load setup
```

Y (the matched index) is **never used** to look up the parallel byte. Every special-entry
match — TOADHOUSE, PIPE, HANDTRAP, BOWSERCASTLELL, etc. — runs the same handler at
`$CEA7` and enters the slot's pointer-table-entry as a regular level. This is the same
path taken when Mario presses A on a level number tile (which falls into `$CEA7` via the
gate-threshold check at `$CDF8` rather than via the special-entry search).

What about HANDTRAP's grab and PIPE's transit-pipe behavior?

- **HANDTRAP** is a **separate** post-walk-arrival check at `$CF15` (`CMP #$E6 / BNE skip
  / ... / INC $0729` to bump `Map_Operation` from `$10` to `$11` = grab). It fires while
  Mario is walking onto the tile, not on A-press, and is keyed on the tile byte directly.
- **PIPE transit** is not driven by op `0x27` either. The "where does this pipe go"
  lookup is a property of the slot's pointer-table-entry (a PipeTransit-type entry that
  loads a transit level whose `OBJ_PIPEWAYCONTROLLER` reads the pipe-destination tables
  in PRG002). On a regular-level slot, stamping `0xBC` produces a pipe-look tile that
  enters the underlying regular level on A — no transit, no destination lookup. This is
  exactly what `troll_pipes` exploits (`src/randomize/troll_pipes.rs`).

The 11 parallel bytes at `0x14DCA` may be vestigial dev-time data, may be consumed by
some other code path entirely, or may have been a planned-but-cut dispatch mechanism.
Whatever they are, the special-entry path doesn't read them.

### World-Map Metatile Pattern Bank (PRG012 bank 0x0C)

Each tile byte is rendered as 4 CHR pattern indices forming a 2×2 metatile (16×16 px).
The bank is **shared across all 8 worlds** — re-skinning a tile changes its appearance in
every world.

| Quadrant | File offset | Size |
|---|---|---|
| NW | `0x18010 + tile` | 256 bytes |
| NE | `0x18110 + tile` | 256 bytes |
| SW | `0x18210 + tile` | 256 bytes |
| SE | `0x18310 + tile` | 256 bytes |

Total: 4 × 256 = 1024 bytes (matches the doc's "1024-byte maps" per metatile bank).

### World-Map Palette Rule

**Palette is encoded in the upper 2 bits of the tile byte itself** — there is no per-tile
palette lookup table (per southbird disasm comment in PRG013: *"Remember that palette is
determined by the upper 2 bits of a TILE (not the PATTERN)"*).

| Range | High 2 bits | Palette code |
|---|---|---|
| `0x00–0x3F` | `00` | 0 |
| `0x40–0x7F` | `01` | 1 |
| `0x80–0xBF` | `10` | 2 |
| `0xC0–0xFF` | `11` | 3 |

Each world picks one of 9 ColorSets via `Map_Tile_ColorSets`
(`.byte $00, $01, $00, $03, $04, $05, $06, $07, $02` — W1 and W3 share ColorSet 0).
The 4 palettes within a ColorSet define what the 4 pages look like in that world.

This is also why `Tile_Attributes_TS0` / THRESHOLDS at `0x18410` is 4 bytes (`03 67 BF E9`)
— one per palette page.

**Practical consequence**: two tile bytes with the same CHR pattern but different palette
pages render as visually-different tiles (e.g. `0x44` and `0xD9` share CHR `FE FE FE CD`
but `0x44` uses palette 1 and `0xD9` uses palette 3). To clone a tile's full visual,
choose the destination byte in the same palette page as the source.

### Tile-Byte Lookup Tool

Use `tools/rom_map.py --tile <byte>` (or the `/tile` slash command) to inspect any tile
byte: CHR pattern, palette page, behavior registry membership, vanilla usage, and visually
identical siblings (same CHR; differs only by palette).

### World-Map Level Gating

The "you can only back out of a level slot" rule that lets the player walk freely along
paths but blocks them from crossing through level/fortress slots is implemented purely as
a **per-palette-page byte-value threshold**, not a per-tile flag table. This is the
level-gate mechanism.

**Routine** — `MO_NormalMoveEnter` at PRG010 `$CDDC`. When Mario stands on a tile with a
pointer-table entry and presses a direction:

```
LDA <World_Map_Tile          ; A = current tile byte ($E5)
AND #$C0 / ROL ROL ROL / TAY ; Y = palette page of current tile (0..3)
LDA <World_Map_Tile
CMP $7E98,Y                  ; threshold for that palette page
BCS $CEA7                    ; gate fires → reverse-direction-only handler
                             ; (else fall through to normal move)
```

The gate applies to the **current** tile (the one Mario is standing on). When it fires,
operation `0x10` is dispatched, which restricts Mario to backing out the way he came.

**Rule:** a tile byte is gated if `byte >= threshold[byte's palette page]`. Bytes
**below** the threshold pass freely; bytes **at or above** trigger the reverse-only gate.

**Thresholds are universal across all 8 worlds.** A common misreading is to expect a
per-world threshold table, but the load chain is:

1. World init runs `LDY $0727 / LDA $A92B,Y / STA $070A` (at file `0x349FF`). The table
   at `$A92B` (file `0x3493B`) maps `World_Num → tileset` and contains
   `0E 0E 0E 0E 0E 0E 0E 0E` — i.e. **all 8 worlds resolve to tileset `0x0E` (14)** for
   the world-map renderer.
2. The `$F52E` STA loop reads `$070A`, doubles it as Y, fetches a 16-bit pointer from
   `$94F1,Y / $94F2,Y` (a 16-entry tileset → data-pointer table at file `0x3D501`), then
   copies 8 bytes from that pointer into `$7E94..$7E9B`.
3. The pointer table has tilesets 0–14 all pointing to `$A400` (with PRG006 mapped at
   `$A000–$BFFF`, that's file `0x18410`). Tileset 15 points elsewhere and is unused for
   world maps.
4. The 8 bytes at file `0x18410` are `03 67 BF E9 03 67 BF E9` — duplicated (only the
   second half at `$7E98–$7E9B` is used by the level-gate `CMP $7E98,Y`; what `$7E94–
   $7E97` is consumed by, if anything, isn't documented here).

So **all worlds use the same thresholds**:

| Page | Range       | Threshold | Gated bytes (≥ threshold)       |
|------|-------------|-----------|---------------------------------|
| 0    | `0x00–0x3F` | `0x03`    | `0x03–0x3F` (level numbers)     |
| 1    | `0x40–0x7F` | `0x67`    | `0x67–0x7F`                     |
| 2    | `0x80–0xBF` | `0xBF`    | only `0xBF`                     |
| 3    | `0xC0–0xFF` | `0xE9`    | `0xE9–0xFF`                     |

To change thresholds for a world, two routes:

- **Edit the shared table at `0x18410`** — affects every world identically.
- **Hijack `$070A`** — write a non-`0x0E` value before world load and add a new entry to
  the pointer table at `$94F1` pointing to a custom 8-byte block. The single unused entry
  is tileset 15 (currently `$9517`).

**Design rationale.** Vanilla didn't need a separate "is level slot" flag table because
the tileset's byte-to-visual mapping was chosen so that level tiles land in the high
range of each palette page and walkable nodes (paths, HB, toad-houses, pipes, spades,
hand-traps, dancing flowers) land in the low range. The 4-byte threshold draws the
dividing line per page. Examples:

| Tile  | Page | Byte | vs threshold       | Gated? |
|-------|------|------|--------------------|--------|
| `0x03` (level "1")     | 0 | `0x03` | `>= 0x03`            | ✓ (gates in every world) |
| `0x44` (grass path)    | 1 | `0x44` | `< 0x67`             | — |
| `0x50` (toad house)    | 1 | `0x50` | `< 0x67`             | — |
| `0x66` (path-and-nub)  | 1 | `0x66` | `< 0x67`             | — |
| `0x6D` (high page-1)   | 1 | `0x6D` | `>= 0x67`            | ✓ |
| `0xBC` (pipe)          | 2 | `0xBC` | `< 0xBF`             | — |
| `0xE6` (HANDTRAP)      | 3 | `0xE6` | `< 0xE9`             | — |
| `0xE8` (spade)         | 3 | `0xE8` | `< 0xE9`             | — |

`0xE6` and `0xE8` are **deliberately** placed just below the threshold so they walk
freely. There is **no per-tile CMP or exemption hook** — exemption is purely a function
of the byte being below the line.

**Relationship to walk tables.** The level gate decides "can Mario *leave* this tile?"
(checked when standing-and-pressing-direction). Walk tables (`0x15258 / 0x15261 / 0x1526A
/ 0x15273`, one per direction) decide "can Mario *step onto* the destination tile?"
(checked during pre-move validation). Both must pass for movement to proceed. They are
independent — a tile can be in walk tables but still gated (e.g., level tiles), or
below-threshold but not in walk tables (rare; usually background).

**Hooking the gate.** The actual `CMP $7E98,Y / BCS gate` instruction sequence sits at
file offset `0x14E08–0x14E0C` (CPU `$CDF8`, 5 bytes). Replace with `JMP custom + 2 NOP`
to install a tile-aware bypass for selected new tile bytes (used by the hidden-hand POC
`/tmp/poc_hidden_hand_1_1.py`).

**Connection to row 8 completion bits.** The same threshold table also acts as a
completion-unsafe heuristic in `Map_Reload_with_Completions` (see "MAP_COMPLETE_BITS
coverage" below). A row-7 byte that is at-or-above its page threshold counts as
completion-unsafe and blocks the row-8 fallthrough. This is the same data doing double
duty — original purpose is movement gating; the completion path reuses it as a "this
position is a level/special tile, don't touch" filter.

### Pipe Destination Tables (PRG002: 0x046AA–0x0470D)

Four 24-byte tables control where Mario appears on the overworld map after exiting a pipe transit level. Each table is indexed by the **dest byte** from the `OBJ_PIPEWAYCONTROLLER` (object 0x25) in the pipe transit level's enemy data. Each byte packs **two nibble values**: upper nibble = "left" pipe endpoint, lower nibble = "right" pipe endpoint. The game selects which nibble based on Mario's position within the pipe transit level (left/upper vs right/lower half).

| Table | File Offset | Description |
|-------|-------------|-------------|
| `PipewayCtlr_MapXHi` | 0x046AA | Screen number for each endpoint (packed nibbles) |
| `PipewayCtlr_MapX` | 0x046C2 | Column position for each endpoint (packed nibbles) |
| `PipewayCtlr_MapY` | 0x046DA | Row nibble for each endpoint (packed nibbles) |
| `PipewayCtlr_MapScrlXHi` | 0x046F2 | Scroll screen; bit 3 = center flag (adds 128px camera offset). Vanilla: A=0, B=1 always. Pipe shuffle sets equal to MapXHi (no center) to avoid camera misalignment at screen boundaries |

**Dest byte assignments** (from pipe transit level enemy data `01 25 02 XX FF`):

| Dest | World | Pair |
|------|-------|------|
| 0x00 | — | Unused/unknown |
| 0x01 | W2 | Single pipe pair |
| 0x02–0x03 | W6 | Two pipe pairs |
| 0x04–0x0B | W7 | Eight pipe pairs |
| 0x0C–0x11 | W8 | Six pipe pairs |
| 0x12–0x14 | W3 | Three pipe pairs |
| 0x15–0x16 | W4 | Two pipe pairs |
| 0x17 | W5 | Single pipe pair |

**Example** — W2 pipe pair (dest 0x01):
- `MapY[1] = 0x86` → upper=8 (row_nibble 8, entry 19), lower=6 (row_nibble 6, entry 16)
- `MapX[1] = 0x8E` → upper=8 (col 8, entry 19), lower=E (col 14, entry 16)
- `MapXHi[1] = 0x00` → both endpoints on screen 0

**Pipe transit level structure:**
- Each pipe pair shares a single `obj_ptr` (enemy data) containing `01 25 02 XX FF`
- Both endpoints have tileset 14, 1-screen layout, and are classified as `too_short` in level shuffle
- The two endpoints have different `lay_ptr` values but their layout data is chained: entry A's area 2 = entry B's area 1 (via junction at 0xFF terminator)
- Layout header byte 5 differs: `0x04` vs `0x44` (bit 6 controls pipe direction / vertical scroll mode)
- **A-side entry** has byte5 bit 6 = 0 (`0x04`): player enters from the left, exits right. The game reads the **lower nibble** (B position) as the exit destination.
- **B-side entry** has byte5 bit 6 = 1 (`0x44`): player enters from the right, exits left. The game reads the **upper nibble** (A position) as the exit destination.

**Critical**: When assigning pipe pool entries to positions, the A-side entry (byte5 bit 6 = 0) **must** be placed at `pos_a` (upper nibble) and the B-side entry (byte5 bit 6 = 1) at `pos_b` (lower nibble). If swapped, the exit nibble points back to the entry position, creating a self-referencing pipe.

**When moving a pipe endpoint**, update the corresponding nibble (upper or lower) in all four tables to match the new map position. The nibble assignment (upper vs lower) corresponds to which side of the pipe transit level that endpoint enters from.

### World Map Object Data (PRG011: 0x16010–0x1800F)

Pointer tables indexed by World_Num (8 entries each):

| Label | Description |
|-------|-------------|
| `Map_List_Object_Ys` | Pointers to per-world Y coordinate tables |
| `Map_List_Object_XHis` | Pointers to per-world X high-byte tables |
| `Map_List_Object_XLos` | Pointers to per-world X low-byte tables |
| `Map_List_Object_IDs` | Pointers to per-world object type tables |
| `Map_List_Object_Items` | Pointers to per-world item reward tables |

9 objects max per world (Hammer Bros, bonus objects, HELP bubble, Airship, etc.)

### Map_Unused7EEA — Dead Code LUT (PRG011: 0x16018)

An 8-byte LUT at PRG011 CPU `$A008` (file `0x16018`), labeled `Map_Unused7EEA_Vals` in
southbird, indexed by `World_Num` (0–7). Loaded into `Map_Unused7EEA` at RAM `$7EEA`
during `Map_Init` (PRG011 CPU `$A1E1`, file `0x161F1`) and **never read anywhere else in
vanilla**. Southbird comment: *"Unused; Value retrieved from LUT at initialization of
world, but never used otherwise."*

Vanilla values: `02 04 03 FF 03 04 03 05`.

**Hijack potential:** Safe to repurpose as a per-world indirected byte under full
randomizer control — the vanilla engine writes it automatically at the right moment with
zero side effects. Used by `qol::random_koopalings` (source: fcoughlin/Fred) as a
"Koopaling identity remap": the LUT is overwritten with a fresh permutation of
Koopaling original world indices (`0..=6`, W8 kept at `0x05` for Bowser), and 11 sites
in PRG001's Koopaling handler are rewritten from `LDA $0727` → `LDA $7EEA`.

Vanilla CMP constants at those sites match Koopaling original world indices
(Wendy=W3=2, Roy=W5=4, Lemmy=W6=5, Ludwig=W7=6), so existing branches fire for the
correct Koopaling identity without further rewrites.

#### PRG001 Patch Sites (11 total)

Each site is a 3-byte instruction; only the 2-byte operand is rewritten.

| File Offset | CPU Addr | Opcode   | Vanilla Op | Enclosing Routine          | Controls |
|-------------|----------|----------|------------|----------------------------|----------|
| `0x02E30`   | `$AE20`  | `LDA`    | `$0727`    | `ObjInit_Koopaling`        | Palette base index (`* 4 → Koopaling_Palettes`) |
| `0x02ED4`   | `$AEC4`  | `LDY`    | `$0727`    | `ObjNorm_Koopaling`        | CHR bank selection (`KoopalingPatSet4/5` → `PatTable_BankSel+4/5`) |
| `0x02F3B`   | `$AF2B`  | `LDA`    | `$0727`    | `Koopaling_Normal`         | `CMP #$05` Lemmy AI replacement (→ `PRG001_B671` ball routine) |
| `0x02FAE`   | `$AF9E`  | `LDA`    | `$0727`    | wand-fire branch           | `CMP #$02` Wendy ring projectile (vs vanilla wand blast) |
| `0x02FE5`   | `$AFD5`  | `ADC`    | `$0727`    | jump-selection branch      | Jump table index (`hit_count*7 + world` into `Koopaling_JumpChanceMask` / `Koopaling_JumpYVels`) |
| `0x02FF6`   | `$AFE6`  | `LDA`    | `$0727`    | idle/ready branch          | `CMP #$02` Wendy firing cadence (hit-count gate vs bitmask) |
| `0x03020`   | `$B010`  | `LDY`    | `$0727`    | pre-fire setup             | `CPY #$02` Wendy straight aim (skip `Object_CalcHomingVels`) |
| `0x03181`   | `$B171`  | `LDA`    | `$0727`    | stomp-response (`B15D`)    | `CMP #$05` Lemmy ball respawn on stomp |
| `0x03372`   | `$B362`  | `LDY`    | `$0727`    | `Koopaling_DrawAndAnimate` | Sprite tile layout (`Koopaling_PatLookup`) |
| `0x033E8`   | `$B3D8`  | `LDY`    | `$0727`    | `Draw_KoopalingWand`       | Wand sprite offset relative to hand |
| `0x03612`   | `$B602`  | `LDA`    | `$0727`    | `Koopaling_DetectWorld`    | `CMP #$04`/`CMP #$06` Roy+Ludwig heavy physics (enhanced gravity, floor-shake, player paralysis) |

Per-Koopaling uniqueness:
- **Larry, Morton, Iggy** — fully generic (only differ via visual/jump tables)
- **Wendy** — unique ring projectile + firing cadence + straight aim (3 sites)
- **Roy, Ludwig** — share heavy-gravity branch (1 site)
- **Lemmy** — most unique, entire AI replaced + ball respawn on stomp (2 sites)

Not controlled by `$0727` (and therefore not remapped): hit count to defeat (hardcoded
`#$0A` in `ObjInit_Koopaling`, then `#$03` in `PRG001_B185`), timer constants, and
wand-state dispatch (`Level_GetWandState`).

### Airship Travel Data

| Label | Description |
|-------|-------------|
| `Map_Airship_Travel_BaseIdx` | Per-world base index (W1=0, W2=3, W3=6, ...) |
| `MAT_Y_W[1-8][A-C]` | Y destinations: 3 sets x 6 values per world |
| `MAT_X_W[1-8][A-C]` | X destinations: packed (lo=screen, hi=X pos) |

### Free Space (PRG012)

**0x19103–0x193D9**: Region between overworld tile grid data and the InitIndex master pointer table (starts at 0x193DA). **WARNING:** 0x19103–0x1910F contains a tile lookup table, and 0x19110+ contains active map screen code (level-entry logic: `ROL $07`, `LDA $073C,X`, etc.). This is NOT free space — writing here corrupts the map screen and crashes on level entry.

**0x19DD0–0x19FFF** (560 bytes): Free space after overworld tile/code region. The randomizer stamps a 17-byte identification block at **0x19DF0**:

| Offset | Size | Content |
|--------|------|---------|
| +0 | 3 | `S3R` magic bytes |
| +3 | 1 | Version (0x02) |
| +4 | 5 | Flag key bytes (encoded Options) |
| +9 | 8 | Seed (little-endian u64) |

**Note:** The Big ? Block trampoline and flag stamp both live in this region.

**0x35530–0x35592** (99 bytes): Used by Big ? Block lookup routine (PRG026).

### Level Pointer Tables (PRG012: 0x18010–0x1A00F)

`Map_PrepareLevel` uses the player's world map position to look up level data via
per-world tables. Five master pointer tables (9 words each, one per world + warp zone)
index into per-world sub-tables:

| Master Table | File Offset | Description |
|-------------|-------------|-------------|
| `Map_ByXHi_InitIndex` | 0x193DA | Per-screen search start indices |
| `Map_ByRowType` | 0x193EC | Row/type + tileset (lower nibble = tileset ID) |
| `Map_ByScrCol` | 0x193FE | Screen/column positions for matching |
| `Map_ObjSets` | 0x19410 | Enemy/object data CPU address pointers |
| `Map_LevelLayouts` | 0x19422 | Level layout data CPU address pointers |

Each master table entry is a 16-bit CPU address pointing to the per-world sub-table
in PRG012. Per-world sub-tables are contiguous: ByRowType (N bytes), ByScrCol (N bytes),
ObjSets (N words), LevelLayouts (N words).

**InitIndex sub-table structure:** Each world's `Map_ByXHi_InitIndex` sub-table is **always 4 bytes** (the gap between InitIndex and ByRowType CPU pointers is always 4), located immediately before the `ByRowType` sub-table. Each byte is the entry index to start searching from for that screen (optimization so the game doesn't scan the entire table). Screens beyond the world's actual screen count use the entry count N as a sentinel (= "no entries"). Must be recomputed if entries are reordered within a world.

Example — W1 InitIndex (1 screen, 4 bytes): `00 15 15 15` (screen 0 at entry 0, screens 1-3 sentinel=21).
Example — W3 InitIndex (3 screens, 4 bytes): `00 1B 2F 34` (screens 0–2 at entries 0, 27, 47; screen 3 sentinel=52).

**Per-world sub-table locations:**

| World | RowType Offset | Entries | Description |
|-------|---------------|---------|-------------|
| 1 | 0x19438 | 21 | Grass Land |
| 2 | 0x194BA | 47 | Desert Land |
| 3 | 0x195D8 | 52 | Water Land |
| 4 | 0x19714 | 34 | Giant Land |
| 5 | 0x197E4 | 42 | Sky Land |
| 6 | 0x198E4 | 57 | Ice Land |
| 7 | 0x19A3E | 46 | Pipe Land |
| 8 | 0x19B56 | 41 | Dark Land |

**ByRowType byte encoding:** upper nibble = row position ("row_nibble"), lower nibble = tileset ID.

**Coordinate mapping (confirmed from disassembly — 100% hit rate across all 340 entries):**

Row mapping (derived from `Map_GetTile` in prg012.asm):
```
grid_row = row_nibble - 2
```
Map tiles are loaded at `Tile_Mem_Addr + $110`, but `Map_GetTile` uses base `Tile_Mem_Addr + $100`. The tile offset is `((World_Map_Y - 16) & 0xF0) | column`. With `World_Map_Y = row_nibble * 16`, this yields `grid_row = row_nibble - 2`.

| row_nibble | World_Map_Y | grid_row |
|-----------|-------------|----------|
| 0x2 | 0x20 | 0 |
| 0x3 | 0x30 | 1 |
| 0x4 | 0x40 | 2 |
| 0x5 | 0x50 | 3 |
| 0x6 | 0x60 | 4 |
| 0x7 | 0x70 | 5 |
| 0x8 | 0x80 | 6 |
| 0x9 | 0x90 | 7 |
| 0xA | 0xA0 | 8 |

Vanilla game only uses even row_nibbles (2,4,6,8,A) → even grid rows (0,2,4,6,8). Odd grid rows contain path/decoration but no enterable nodes.

Column mapping (from `Map_PrepareLevel`):
```
screen = ByScrCol >> 4
column = ByScrCol & 0x0F
grid_col = screen * 16 + column
```
The game computes ByScrCol as `(World_Map_XHi << 4) | (World_Map_X >> 4)`.

**Entry type identification by ObjSets pointer value:**
- `obj >= 0xC000 && lay != 0x0000`: action level (regular or fortress)
- `obj == 0x0700`: Toad House
- `obj == 0x0001` with `lay == 0x0000`: bonus game / N-Spade
- `obj < 0x1000` (other small values): hand traps, pipe junctions, special

**Note on obj_ptr ranges:** The `obj >= 0xD000` range does NOT reliably indicate
fortresses. Many regular action levels have enemy data in $D000+ (World 2 desert,
World 4 giant, World 8 tanks/ships). Fortress identification requires checking for
Boom-Boom enemies — see "Boom-Boom Detection" section above.

**Level loading flow:** Player map position → match against ByRowType + ByScrCol →
extract tileset from lower nibble → load ObjSets pointer into `Level_ObjPtr_AddrL/H` →
load LevelLayouts pointer into `Level_LayPtr_AddrL/H` → bank-switch via
`PAGE_A000_ByTileset[Level_Tileset]` → execute level generators.

### World Map Starting Positions

| Label | Description |
|-------|-------------|
| `Map_Y_Starts` | Per-world initial Y coordinate |
| Fixed X = 0x20 | Same X start for all worlds |

### Fortress Lock & Bridge FX (PRG010: 0x147CD–0x148B7)

When a fortress is cleared (Boom-Boom defeated, magic ball collected), the game triggers
a map effect that busts a lock or builds a bridge, opening progression on the overworld.
The entire system lives in PRG010 and uses **17 FX slots** (0x00–0x10), one per
fortress/ship in the game.

**Mechanism (`MO_DoFortressFX` at Map_Operation 8):**

1. Clearing a fortress sets `Map_DoFortressFX` to a 1-based index (which fortress
   *within this world* was just cleared: 1st, 2nd, 3rd, or 4th).
2. `MO_DoFortressFX` decrements to 0-based, then computes:
   `absolute_index = FortressFXBase_ByWorld[World_Num] + Map_DoFortressFX`
3. Reads the FX slot value from `FortressFX_W1[absolute_index]` (0x00–0x10).
4. Uses the FX slot to index into all visual/map replacement tables below.

**Data tables (all 17 entries, indexed by FX slot 0x00–0x10):**

| File Offset | Size | Label | Description |
|------------|------|-------|-------------|
| 0x147CD | 17 | `FortressFX_VAddrH` | VRAM high byte for lock/bridge tile position |
| 0x147DE | 17 | `FortressFX_VAddrL` | VRAM low byte for lock/bridge tile position |
| 0x147EF | 34 | `FortressFX_MapCompIdx` | `Map_Completions` column + bit per FX slot (17×2 bytes) |
| 0x14811 | 68 | `FortressFX_Patterns` | Replacement 8×8 patterns per FX slot (17×4 bytes) |
| 0x14855 | 17 | `FortressFX_MapLocationRow` | Map row (Y position) for tile replacement |
| 0x14866 | 17 | `FortressFX_MapLocation` | Map screen (lo nibble) + column (hi nibble) |
| 0x14877 | 17 | `FortressFX_MapTileReplace` | Replacement map tile ID |
| 0x14888 | 32 | `FortressFX_W1–W8` | Per-world FX slot assignments (4 slots per world, 0-padded) |
| 0x148A8 | 8+8 | `FortressFXBase_ByWorld` | Per-world base index into `FortressFX_Wx` (8 used + 8 extra) |

**Per-world FX slot assignments (`FortressFX_W1–W8` at 0x14888):**

```
W1: 00 00 00 00   →  slot 0x00 (1 fortress, 3 unused)
W2: 01 00 00 00   →  slot 0x01 (1 fortress, 3 unused)
W3: 02 03 00 00   →  slots 0x02, 0x03 (2 fortresses)
W4: 04 05 00 00   →  slots 0x04, 0x05 (2 fortresses)
W5: 06 07 00 00   →  slots 0x06, 0x07 (2 fortresses)
W6: 08 09 0A 00   →  slots 0x08, 0x09, 0x0A (3 fortresses)
W7: 0B 0C 00 00   →  slots 0x0B, 0x0C (2 fortresses)
W8: 0D 0E 0F 10   →  slots 0x0D, 0x0E, 0x0F, 0x10 (4 fortresses/ships)
```

**`FortressFXBase_ByWorld` (0x148A8):** `00 04 08 0C 10 14 18 1C` — each world's
entries are 4 bytes apart (matching the 4-slot-per-world layout above).

**FX slot details (17 slots, 0x00–0x10):**

| Slot | World | VRAM Addr | Scr | Col | Row | Tile | Type |
|------|-------|-----------|-----|-----|-----|------|------|
| 0x00 | W1 | $2948 | 0 | 4 | $50 | $46 | Lock |
| 0x01 | W2 | $2A50 | 0 | 8 | $90 | $46 | Lock |
| 0x02 | W3 | $2A12 | 0 | 9 | $80 | $45 | Bridge |
| 0x03 | W3 | $294C | 1 | 6 | $50 | $46 | Lock |
| 0x04 | W4 | $2906 | 1 | 3 | $40 | $45 | Bridge |
| 0x05 | W4 | $2996 | 0 | 11 | $60 | $B3 | Bridge (water) |
| 0x06 | W5 | $2986 | 0 | 3 | $60 | $B3 | Bridge (water) |
| 0x07 | W5 | $298E | 1 | 7 | $60 | $DA | Bridge (sky) |
| 0x08 | W6 | $299A | 0 | 13 | $60 | $DA | Bridge (sky) |
| 0x09 | W6 | $2892 | 1 | 9 | $20 | $B3 | Bridge (water) |
| 0x0A | W6 | $298A | 2 | 5 | $60 | $45 | Bridge |
| 0x0B | W7 | $291A | 0 | 13 | $40 | $46 | Lock |
| 0x0C | W7 | $29CE | 1 | 7 | $70 | $45 | Bridge |
| 0x0D | W8 | $2910 | 0 | 8 | $40 | $46 | Lock |
| 0x0E | W8 | $2952 | 1 | 9 | $50 | $45 | Bridge |
| 0x0F | W8 | $2998 | 2 | 12 | $60 | $46 | Lock |
| 0x10 | W8 | $29CA | 3 | 5 | $70 | $45 | Bridge |

**Lock/bridge tile IDs (before → after clearing):**

| Type | Original Tile | Replacement Tile | Patterns |
|------|--------------|-----------------|----------|
| Lock | $54 | $46 (open path) | FE C0 FE C0 |
| Bridge | $56 | $45 (bridge) | FE FE E1 E1 |
| Water bridge | $9D | $B3 (water bridge) | D4 D6 D5 D7 |
| Sky bridge | $E4 | $DA (sky bridge) | FE FE E1 E1 |
| Lock (W8-3) | $54 | $46 (open path) | FF FF FF FF |

The "replacement tile" (`FortressFX_MapTileReplace`) is whatever path tile was at the
lock/bridge position before the lock was placed. When placing a lock at a new position,
read the current tile first and store it as the replacement.

**CRITICAL — Pattern bytes must match the replacement tile, not the gap type:**

The 4-byte `FortressFX_Patterns` entry for each slot determines the VRAM CHR tiles
written when the lock/gap opens. These must match the visual appearance of the
replacement tile. Using the wrong patterns causes the tile to render incorrectly
(e.g., a horizontal path looking like a vertical path) even if the collision map tile
(`FortressFX_MapTileReplace`) is correct.

| Replace Tile | Patterns | Visual |
|-------------|----------|--------|
| $46 (vertical path) | FE C0 FE C0 | Vertical path segment |
| $45 (horizontal path) | FE FE E1 E1 | Horizontal path/bridge |
| $DA (sky bridge path) | FE FE E1 E1 | Horizontal path/bridge |
| $B3 (water bridge path) | D4 D6 D5 D7 | Water bridge tiles |
| $B7 (horizontal path) | FE FE E1 E1 | Horizontal path/bridge |
| (W8 slot 0x0F only) | FF FF FF FF | Special W8 tiles |

When placing a lock on an arbitrary path tile, derive the patterns from the original
tile at that position — NOT from whether the gap tile is a lock ($54) or bridge ($56).

**VRAM address formula (verified against all 17 slots):**

```
VRAM = 0x2880 + grid_row * 64 + col_in_screen * 2
```

Where `grid_row = (FortressFX_MapLocationRow >> 4) - 2` and `col_in_screen = col % 16`.
The screen number does not factor into the VRAM address because the game only renders
one screen at a time — the FX triggers on whichever screen is currently displayed.

**Cross-screen FX animation bug (patched by randomizer):**

In vanilla, each fortress and its lock/bridge are always on the same screen. When the
player beats a fortress and returns to the map, the camera shows the fortress screen,
which is also the lock screen, so the VRAM pattern write and poof sprites land on the
correct tiles.

When fortress/lock positions are shuffled, the lock can end up on a different screen.
The `MO_DoFortressFX` routine (CPU $C8A9 in PRG010) does NOT scroll to the lock's
screen before animating — it writes VRAM patterns and places sprites relative to the
currently displayed screen. This causes two visual artifacts:

1. VRAM patterns written to nametable tiles that belong to the fortress's screen, not
   the lock's screen (wrong tile modified on screen).
2. Poof sprites placed at the lock's `col_in_screen` position on the wrong screen.

The map DATA update (replacement tile via screen pointer table + `Map_Completions`)
is NOT screen-relative and always works correctly. So the correct tile IS placed at
the lock position; the visual animation is what goes wrong.

**Fix:** Hook 3 bytes at file 0x148F6 (CPU $C8E6) to `JMP $D544` (PRG010 free
space at file 0x15554, 39 bytes). Custom code checks whether the lock is on a
visible screen by comparing `FortressFX_MapLocation[slot] & 0x0F` (lock screen)
against the current viewport state. The map scrolls in 128-pixel half-screen
steps: `$12` (Map_Scroll_XHi) is the scroll page and `$FD` (Map_Scroll_X) is
either 0 or 128. When `$FD=128`, the viewport straddles two grid screens. The
lock is considered visible if `lock_screen == $12`, OR if `lock_screen == $12+1`
AND `$FD >= $80`. If visible, the full animation plays normally. If not visible,
`$20` is set to 6 (last animation frame) and execution jumps to $C952
(Map_Completions update), skipping the VRAM write and abbreviating the poof to
a single frame.

**`MO_DoFortressFX` flow (CPU $C8A9, PRG010 bank at $C000):**

1. If `$20` ≠ 0: jump to animation loop at $C9A4 (continue existing animation).
2. If `$0745` (`Map_DoFortressFX`) = 0: nothing to do, exit.
3. Init fortress crumble timer `$0711` = $20 (32 frames). Each frame calls $C9D6
   which toggles CHR bank ($16 between $18/$19) and decrements $0711. No scrolling.
4. When `$0711` reaches 0: decrement `$0745`, look up FX slot via
   `FortressFXBase_ByWorld[World_Num] + $0745` → `FortressFX_W1[$slot]`.
5. Set `$20` = 1 (animation start), then:
   - $C8EA–$C94F: Write VRAM patterns to PPU buffer ($0300+) at FX_VAddr address.
   - $C952–$C9A2: Update `Map_Completions`, write replacement tile to map data via
     screen pointer table at $8000.
   - $C9A4–$C9C6: Animation frame loop — every 4 game frames, INC `$20`. When
     `$20` = 7, animation done. Poof sprites via `DoFortressFXPoof` ($ABCF).

**Scroll state variables during map screen:**

| Address | Name | Description |
|---------|------|-------------|
| $FD | Map_Scroll_X | PPU horizontal scroll (0–255), written to $2005 |
| $12 | Map_Scroll_XHi | Scroll screen / page number (0–3), updated with $FD |
| $FC | Map_Scroll_Y | PPU vertical scroll, written to $2005 (second write) |

**PRG030 free space usage (file 0x3DF20+ / CPU $9F10+):**

PRG030 is the fixed bank, always mapped at $8000–$9FFF. Free space starts at 0x3DF20.

| Offset | Size | CPU | Purpose |
|--------|------|-----|---------|
| 0x3DF20 | 28 | $9F10 | World order: lookup routine (12) + next-world table (8) + display table (8) |
| 0x3DF3C | 20 | $9F2C | Big ? Block: save obj_ptr trampoline (level init hook) |

**PRG010 free space usage (file 0x15554 / CPU $D544):**

| Offset | Size | Purpose |
|--------|------|---------|
| 0x15554 | 46 | FX screen-check patch (JMP target from $C8E6) |
| 0x15DF0 | 35 | Canoe softlock fix: save death respawn position (JSR target from $C6EA) |

**PRG011 free space usage (file 0x17D00 / CPU $BCF0):**

| Offset | Size | Purpose |
|--------|------|---------|
| 0x17D00 | 66 | Canoe softlock fix: backup/restore map tile data (JSR targets from $A22F and $C6EA) |

**`FortressFX_MapLocationRow` encoding:** `(grid_row + 2) << 4`

**CRITICAL — low nibble must remain 0.** The engine's map-data tile write at
`$C99B` (file `0x149AB`) is `ORA $C845,X`: it ORs the whole row byte into the
in-screen column index when computing the destination offset for the
replacement tile. Any bit set in `FortressFX_MapLocationRow & 0x0F` corrupts
the column — for even `col_in_screen` values with bit 0 set, the tile lands one
column right of the lock, leaving the original lock tile in place. The
animation plays, `Map_Completions` is updated correctly (separate table), so
the symptom is "lock breaks visually but stays solid until the world is
exited and re-entered." Do not stash side-channel flags in this byte.

**`FortressFX_MapLocation` encoding:** `(col_in_screen << 4) | screen`

**`FortressFX_MapCompIdx` (0x147EF):** Each FX slot has a 2-byte entry: the first byte is
the column index into `Map_Completions` RAM ($7E40+), the second byte is the bit mask to
OR into that column. Both Mario's and Luigi's `Map_Completions` arrays are updated
(offset $00 and $40 respectively). This prevents the lock/bridge from reverting.

**CRITICAL — `Map_Completions` encoding (verified via PRG012 disassembly):**

The `Map_Completions` array is shared between level completion tracking and fortress FX
persistence. `Map_Reload_with_Completions` (PRG012) iterates every column and bit,
checking for completions and applying tile replacements.

- **Column** = the map grid column: `screen * 16 + col_in_screen`
- **Bit** = row position via `Map_Complete_Bits` LUT (PRG012):
  `$80, $40, $20, $10, $08, $04, $02, $01` → rows 0, 1, 2, 3, 4, 5, 6, 7

So for a lock at grid position (row, col) on screen S:
```
comp_col = S * 16 + col_in_screen
comp_bit = Map_Complete_Bits[row] = $80 >> row
```

Example: lock at grid (1, 8) on screen 0 → col=0x08, bit=0x40.
Example: lock at grid (3, 4) on screen 0 → col=0x04, bit=0x10 (vanilla W1 lock).

**Map_Removable_Tiles (PRG012):** The game also has a separate `Map_Removable_Tiles`
table that lists tile IDs eligible for removal during map completion processing:
`TILE_ROCKBREAKH, TILE_ROCKBREAKV, TILE_LOCKVERT ($54), TILE_FORT ($67),
TILE_ALTFORT, TILE_ALTLOCK, TILE_LOCKHORZ ($56), TILE_RIVERVERT`. These tiles are checked
during `Map_Reload_with_Completions` and replaced with their `Map_RemoveTo_Tiles`
counterparts when the corresponding completion bit is set.

**CRITICAL — Gap tile selection must match path orientation:**
The `Map_RemoveTo_Tiles` replacements are hardcoded: `$54` → `$46` (vertical path),
`$56` → `$45` (horizontal path). When placing an obstacle on the map, the gap tile
must match the underlying path direction: use `$54` (lock) on vertical paths and
`$56` (bridge gap) on horizontal paths. Using the wrong gap tile causes the path
to change orientation on map reload (e.g., horizontal path turns vertical).

**Complete procedure for repointing a lock to a new position:**

1. Read the current tile at the new position (this becomes `FortressFX_MapTileReplace`)
2. Write the appropriate gap tile at the new position: $54 for vertical paths, $56 for horizontal paths
3. Restore the old lock position to its original path tile (e.g., $46)
4. Update FX slot tables:
   - `FortressFX_VAddrH/L` = `0x2880 + grid_row * 64 + col_in_screen * 2`
   - `FortressFX_MapLocationRow` = `(grid_row + 2) << 4`
   - `FortressFX_MapLocation` = `(col_in_screen << 4) | screen`
   - `FortressFX_MapTileReplace` = saved original tile
   - `FortressFX_MapCompIdx` = `(screen * 16 + col_in_screen, 0x80 >> grid_row)` — **encodes the LOCK/OBSTACLE position, not the fortress position** (verified across all 17 vanilla slots)
   - `FortressFX_Patterns` = 4 bytes per type (see table above)
5. If the fortress moved to a different world, update:
   - `FortressFX_W1–W8` slot assignments for source and destination worlds
   - Boom-Boom Y-byte upper nibble to the new ordinal within the destination world
   - Pre-open the old lock/bridge position if no fortress remains to clear it

**Boom-Boom Y-byte and Map_DoFortressFX:**

The fortress ordinal (which fortress within the world was cleared) originates from the
Boom-Boom enemy's Y-byte in the level's enemy data. The upper nibble encodes the 1-based
ordinal; the lower nibble is Boom-Boom's spawn Y position on screen.

- Boom-Boom init at `$A9EA` (PRG003): copies Y-byte from `$88,X` to `$7F,X`, then
  overwrites `$88,X` with 1 (resetting the Y-page for gameplay).
- Crystal ball handler at `$A8F6` (PRG003): reads `$7F,X` and stores it to
  `Map_DoFortressFX` (`$0745`).
- `MO_DoFortressFX` at `$A8B0` (PRG010): decrements `$0745`, adds
  `FortressFXBase_ByWorld[World_Num]`, and indexes into `FortressFX_W1–W8` to get
  the FX slot.

All 17 Boom-Boom Y-byte ROM offsets are in PRG006 enemy data (`$C000` bank, file
base `0x0C010`). See `BOOMBOOM_Y_OFFSETS` in `src/randomize/levels.rs` for the
complete list.

**Interaction with fortress shuffling:**

When `randomize_fortresses` swaps level data between fortress map slots, the Boom-Boom
enemy data travels with the level — including the Y-byte whose upper nibble determines
which lock/bridge to break. After shuffling, `randomize_fortresses` patches each
Boom-Boom's Y-byte upper nibble to match its new position's ordinal within the
destination world (preserving the lower nibble spawn position). The `FortressFX_W1–W8`
table is **not** modified — it remains correct because each fortress now reports the
right ordinal for its new world.

### Lock Shuffle Design Constraints

Key constraints discovered while implementing lock shuffle (see `lock-shuffle-wip` branch
for the failed attempt):

**Execution order:** Lock shuffle MUST run after pipe shuffle, because pipe shuffle calls
`resort_pointer_table()` which reorders pointer table entries. Use `grid_pos` (map
coordinates) instead of `entry_idx` (pointer table index) to identify fortress positions,
since grid positions are stable across resort.

**Ordinal semantics:** Beating fort with ordinal N opens FX slot N-1. The pair
(fort, lock) at ordinal N means: beating that fort opens that lock. The lock should
unblock the NEXT fort in the progression — not the fort it's paired with. Getting this
backwards creates deadlocks where a fort can't be reached because its own lock blocks it.

**Combined blocking:** N locks picked individually for their blocking quality may
collectively block ALL forts. Each lock scored in isolation may block one fort, but
2+ locks together may create an impassable barrier. Must validate that the full set of
chosen locks still allows at least one fort to be reached, and that a valid beat→open
progression exists.

**MAP_COMPLETE_BITS coverage:** The LUT has 8 entries mapping rows 0–7 to bits 7–0.
The `Map_Reload_with_Completions` loop (`$A508–$A512`) searches indices 7 down to 1
via `DEX / BNE`; index 0 (`$80` = row 0) is handled by fallthrough when no match is
found.  Bit 0 (`$01`, index 7) maps to row offset `$80` = **row 7**.

**Row 8 fallthrough** (`$A55C–$A56D`): When the current bit is `$01` and the tile at
row 7 was NOT caught by any completion/replacement check, the code adds `$10` to the
tile offset (moving to row 8, offset `$90`) and re-checks.  This means row 8
completion works **only if the row 7 tile in the same column is "safe"** — i.e. not
matched by the special-tiles table (`$A447`), the page thresholds (`$A400`), or
`Map_Removable_Tiles`.  If the row 7 tile IS caught, it gets replaced and the row 8
tile is never reached.

Tiles that block the row 8 fallthrough (completion-unsafe at row 7):
- Special: `$50, $E8, $E6, $BD, $E0`
- Fortress: `$67, $EB` (→ `Map_Removable_Tiles` path)
- Page thresholds: page0 ≥ `$03`, page1 ≥ `$67`, page2 ≥ `$BF`, page3 ≥ `$E9`
- Removable: `$51, $52, $54, $67, $EB, $E4, $56, $9D`

**Randomizer constraints:**
- `find_blank_slots` skips row 8 positions where the existing row 7 tile is
  completion-unsafe (prevents the builder from placing a level there).
- `populate_sections` enforces that no two completable tiles (Level, Fortress, Pipe)
  are orthogonally adjacent — this prevents both the row 7/8 bit collision and
  visually cluttered numbered tiles.
- `place_locks` skips row 7 candidates to avoid the `$01` bit collision with row 8.

**Vanilla FX positions:** Bridges ($56), water gaps ($9D), and sky gaps ($E4) should only
appear at the 13 vanilla FX positions. Locks ($54) can be placed on any path tile.

**W1 fortress secret exit:** The W1 fortress can be completed via a secret exit that does
NOT trigger the Boom-Boom FX (no crystal ball). Its lock must not block progression —
the airship should be reachable even if the W1 fortress lock stays closed.

### W3 Drawbridges

World 3 has 4 drawbridge tiles on its overworld map that toggle between passable and
blocked every time the player completes a regular level. Two are horizontal (`$B2`) and
two are vertical (`$B1`). The toggle means only one set is passable at a time, which
creates unpredictable routing for randomizer play.

**Toggle mechanism (PRG010):**

After completing a level, the post-level handler checks:
```
IF Map_NoLoseTurn == 0 AND World_Num == 2 THEN
    World3_Bridge = World3_Bridge XOR 0x01
```

| File Offset | Bytes | Instruction | Description |
|------------|-------|-------------|-------------|
| 0x14A5E | AD 6E 79 | LDA Map_NoLoseTurn | Check if turn consumed |
| 0x14A61 | D0 26 | BNE skip | Skip if no-lose turn (Toad House, pipe) |
| 0x14A64 | AD 27 07 | LDA World_Num | Check world |
| 0x14A67 | C9 02 | CMP #$02 | World 3? |
| 0x14A69 | D0 08 | BNE skip | Skip if not W3 |
| 0x14A6B | AD BB 07 | LDA World3_Bridge | Load bridge state |
| 0x14A6E | 49 01 | EOR #$01 | Flip bit 0 |
| 0x14A70 | 8D BB 07 | STA World3_Bridge | Store back |

RAM `$07BB` (`World3_Bridge`): 0 = horizontal bridges passable, 1 = vertical bridges passable.

**Drawbridge map tile positions:**

| ROM Offset | Tile | Type | Screen | Row | Col |
|-----------|------|------|--------|-----|-----|
| 0x18777 | $B2 | Horizontal | 0 | 0 | 11 |
| 0x18779 | $B2 | Horizontal | 0 | 0 | 13 |
| 0x1880C | $B1 | Vertical | 0 | 1 | 16 |
| 0x188F3 | $B1 | Vertical | 1 | 6 | 39 |

**Passability check (PRG010, 0x15346):** Uses two lookup tables per direction:
- `Map_DrawBridgeCheck` = [B2, B2, B1, B1] (tile to check, per direction R/L/D/U)
- `Map_DrawBridgeCheckV` = [00, 00, 01, 01] (required World3_Bridge value)

**QoL fix:** Replace all 4 drawbridge tiles with regular bridge tile $B3 (always passable,
has bridge graphic) and NOP the toggle code at 0x14A6B (8 bytes → EA×8). Using $B3 instead
of plain path tiles ($45/$46) preserves the visual bridge appearance on the map.

### Breakable Rocks (Hammer Item)

Breakable rocks ($51 horizontal, $52 vertical) can be destroyed by the Hammer inventory
item. This uses a separate system from the FX locks — handled by `Inv_UseItem_Hammer`
in PRG026 (0x34010 bank).

**Mechanism:**
1. Hammer use checks 4 adjacent tiles for $51 or $52
2. Replaces rock with path tile ($51→$45, $52→$46) via `RockBreak_Replace` table
3. Sets `Map_Completions` bit via `Map_SetCompletion_By_Poof` for persistence
4. On map reload, `Map_Removable_Tiles[0..1]` handles rock→path restoration

**Key data tables:**

| Data | ROM Offset | Contents |
|------|-----------|----------|
| Map_Removable_Tiles | 0x18447 | 51 52 54 67 EB E4 56 9D (8 entries) |
| Map_RemoveTo_Tiles | 0x1844F | 45 46 46 60 E3 DA 45 B3 (8 entries) |
| RockBreak_Replace | 0x346C1 | 45 46 (replacement path tiles) |
| RockBreak_TileFix | 0x346C3 | FE FE E1 FE FE C0 E1 C0 (VRAM CHR patterns) |

**All breakable rocks in the ROM (9 total):**

| World | ROM Offset | Tile | Screen | Row | Col | Grid Col |
|-------|-----------|------|--------|-----|-----|----------|
| W2 | 0x186B8 | $51 | 0 | 6 | 13 | 13 |
| W2 | 0x186E0 | $51 | 1 | 0 | 5 | 21 |
| W3 | 0x187DB | $51 | 0 | 6 | 15 | 15 |
| W3 | 0x187F1 | $51 | 0 | 8 | 5 | 5 |
| W3 | 0x187F3 | $51 | 0 | 8 | 7 | 7 |
| W4 | 0x189E3 | $52 | 1 | 3 | 6 | 22 |
| W4 | 0x18A16 | $51 | 1 | 6 | 9 | 25 |
| W6 | 0x18B8C | $51 | 0 | 2 | 13 | 13 |
| W6 | 0x18C58 | $51 | 1 | 6 | 9 | 25 |

Unbreakable rocks ($53) are decorative barriers — the hammer cannot break them.

**QoL fix (W2 secret rock only):** Replace the rock at 0x186E0 ($51→$45) to open the
secret path on W2 screen 1 without requiring a hammer item.

### World Progression

World advancement is sequential via `INC World_Num` at file offset **0x3D0A1** (PRG030, CPU $9091).

Original bytes: `EE 27 07 4C A0 84` (INC $0727; JMP $84A0)

The code runs after the king's room cinematic (wand return) when a world boss is defeated. There is no "next world" lookup table in the original ROM — progression is always +1.

**Free space for patches:** PRG030 has unused space at **0x3DF20–0x3DF4F** (CPU $9F10–$9F3F), 48 bytes of $FF. The Big ? Block obj_ptr save trampoline uses 0x3DF20–0x3DF33 (20 bytes).

**Free space (PRG031):** **0x3FFF0–0x40009** (CPU $FFE0–$FFF9), 26 bytes. Originally 3 unused `$FF` bytes + "SUPER MARIO 3" ASCII string + dead padding before the interrupt vectors at $FFFA. Not referenced by any code. The card speed clear trampoline uses all 26 bytes (0x3FFF0–0x40009).

World BGM table (PRG030): file offset **0x3C424**, 9 bytes (worlds 1-8 + warp whistle): `01 02 03 04 05 06 07 08 0B`

#### Autoscroll / Pointer Table Resort Ordering

The autoscroll patch writes airship pointer table redirects (ByRowType, ObjSets,
LevelLayouts) to **hardcoded vanilla offsets** for each world's airship entry. The
overworld builder's `resort_pointer_table()` rearranges pointer table entries by sort
key `(screen, row_nib, col)`, which can displace airship entries from their vanilla
indices.

**Critical ordering requirement:** The autoscroll patch must run **before** the
overworld builder. If it runs after, the resort may have moved airship entries to
different indices, and the autoscroll patch overwrites the wrong entries — corrupting
non-airship levels and leaving actual airship entries unpatched. This causes crashes
(reset to title screen) after beating airships.

Running autoscroll first writes to the correct vanilla offsets, then the resort
correctly re-sorts everything (including the patched airship entries) into sort order.

#### World Order Debug Flag Fix

The world-init routine at ROM **0x30CC0** (PRG024) initializes both `World_Num` ($0727)
and `Debug_Flag` ($0160) from the same `LDA #$00` operand at **0x30CC3**:

```
0x30CC0:  A9 00        LDA #$00
0x30CC2:  --           (operand byte at 0x30CC3)
0x30CC4:  8D 27 07     STA $0727    ; World_Num
0x30CC7:  8D 60 01     STA $0160    ; Debug_Flag
```

When world order randomization patches the `LDA #$00` operand to the starting world
number (e.g., `LDA #$05`), the same value leaks into `Debug_Flag`. A non-zero debug
flag enables debug mode, which breaks normal gameplay.

**Fix:** NOP out the `STA $0160` instruction (3 bytes at **0x30CC7** replaced with
`EA EA EA`). The reset handler already clears $0160 to zero on power-on, so skipping
this redundant write is safe.

**Note:** An earlier approach used a JMP-to-free-space trampoline to split the two
STA instructions with separate LDA operands. This caused a 2-player switching bug
and was abandoned in favor of the simpler NOP approach.

### Per-World Specific Offsets

| File Offset | Size | Description |
|------------|------|-------------|
| 0x16190 | ~32 bytes | Hammer Bros item table (uses Global Item IDs) |
| 0x1625B | varies | Map horizontal spawn positions |

---

## Rewards & Mini-Games

| File Offset | Size | Description |
|------------|------|-------------|
| 0x360DE | 7 bytes | Princess reward items (one per world, uses Global Item IDs) |
| 0x2D721–0x2D732 | 18 bytes | N-Spade card deck layout |
| 0x3B14B | ~48 bytes | Mushroom house chest contents (3-byte groups) |
| 0x2D1AD–0x2D1B0 | 4 bytes | Roulette 1-up match counts |

### N-Spade Card Values

| Value | Card |
|-------|------|
| 0x00 | Mushroom |
| 0x01 | Flower |
| 0x02 | Star |
| 0x03 | 1-Up |
| 0x04 | 10 Coins |
| 0x05 | 20 Coins |

---

## Sprite Data

| File Offset | Size | Description |
|------------|------|-------------|
| 0x1E010–0x1E3FF | ~1 KB | Background level sprites |
| 0x3AC10–0x3AC60 | ~80 bytes | Mario/Luigi sprite pointer table |
| 0x3AC61–0x3AE46 | ~485 bytes | Mario/Luigi sprite raw data |
| 0x3AE47–0x3AE97 | ~80 bytes | Mario/Luigi sprite tile set |

### Enemy Sprite CHR Bank Switching (PatTable_BankSel)

SMB3 uses a 6-byte RAM array at **$0719–$071E** (`PatTable_BankSel`) to control
which 1KB CHR ROM pages are mapped into the NES PPU's pattern tables via MMC3:

| Index | PPU Address | Size | MMC3 Reg | Purpose |
|-------|------------|------|----------|---------|
| +0 | $0000–$07FF | 2KB | R0 | BG tiles first half |
| +1 | $0800–$0FFF | 2KB | R1 | BG tiles second half |
| +2 | $1000–$13FF | 1KB | R2 | Player sprites (base) |
| +3 | $1400–$17FF | 1KB | R3 | Player sprites (anim) |
| +4 | $1800–$1BFF | 1KB | R4 | Enemy sprite bank A |
| +5 | $1C00–$1FFF | 1KB | R5 | Enemy sprite bank B |

Each enemy has a `PatTableSel` entry in its object group's dispatch table
(PRG000–PRG005). The encoding:
- `$00` (OPTS_NOCHANGE): no bank switch, uses whatever is loaded
- `$01–$7F` (bit 7 clear): load value into `PatTable_BankSel+4` (slot +4)
- `$80–$FF` (bit 7 set): load `value & $7F` into `PatTable_BankSel+5` (slot +5)

**Conflict rule:** Only one CHR page can be active per slot at a time. If two
on-screen enemies both write to the same slot with different pages, the last
one rendered wins and the other draws garbled sprites.

#### CHR Pages for Randomizable Enemies

| Enemy ID | Name | CHR Page | Slot |
|----------|------|----------|------|
| 0x2B | Goomba in Shoe (Kuribo) | $0B | +4 |
| 0x29 | Spike | $0A | +4 |
| 0x2A | Patooie | $0A | +4 |
| 0x2F | Boo (Boo Diddly) | $12 | +4 |
| 0x30 | Hot Foot (shy) | $12 | +4 |
| 0x33 | Nipper | $0A | +4 |
| 0x39 | NipperHopping | $0A | +4 |
| 0x3F | Dry Bones | $13 | +5 |
| 0x40 | Buster Beetle | $0A | +4 |
| 0x45 | Hot Foot | $12 | +4 |
| 0x55 | Bob-omb | $0B | +4 |
| 0x61 | Blooper w/ Kids | $1A | +4 |
| 0x62 | Blooper | $1A | +4 |
| 0x63 | Big Bertha | $1A | +4 |
| 0x64 | CheepCheep Hopper | $4F | +5 |
| 0x6A | Blooper Child Shoot | $1A | +4 |
| 0x6B | Piledriver | $4F | +5 |
| 0x6C | Green Troopa | $4F | +5 |
| 0x6D | Red Troopa | $4F | +5 |
| 0x6E | Paratroopa Green Hop | $4F | +5 |
| 0x6F | Flying Red Paratroopa | $4F | +5 |
| 0x70 | Buzzy Beetle | $0B | +4 |
| 0x71 | Spiny | $0B | +4 |
| 0x72 | Goomba | $4F | +5 |
| 0x73 | Para-Goomba | $4F | +5 |
| 0x74 | Para-Goomba w/ Micros | $4F | +5 |
| 0x77 | Green Cheep | — | NOCHANGE |
| 0x7A | Big Green Troopa | $3D | +4 |
| 0x7B | Big Red Troopa | $3D | +4 |
| 0x7C | Big Goomba | $3D | +4 |
| 0x7D | Big Green Piranha | $3D | +4 |
| 0x7E | Big Green Hopper | $3D | +4 |
| 0x7F | Big Red Piranha | $3D | +4 |
| 0x80 | Flying Green Paratroopa | $4F | +5 |
| 0x81 | Hammer Bro | $4E | +4 |
| 0x82 | Boomerang Bro | $4E | +4 |
| 0x83 | Lakitu | $0B | +4 |
| 0x86 | Heavy Bro | $4E | +4 |
| 0x87 | Fire Bro | $4E | +4 |
| 0x88 | Orange Cheep | $4F | +5 |
| 0x8A | Thwomp | $12 | +4 |
| 0x8B | Thwomp Left Slide | $12 | +4 |
| 0x8C | Thwomp Right Slide | $12 | +4 |
| 0x8D | Thwomp Up/Down | $12 | +4 |
| 0x8E | Thwomp Diagonal UL | $12 | +4 |
| 0x8F | Thwomp Diagonal DL | $12 | +4 |
| 0xA0–0xA7 | Piranha variants | $4F | +5 |

#### CHR Pages for Non-Swappable Objects (used by two-pass pre-scan)

| Object ID | Name | CHR Page | Slot |
|-----------|------|----------|------|
| 0x18 | Bowser | $3A | +4 |
| 0x24 | Platform Drop | $0E | +4 |
| 0x26–0x28 | Tilt/Seesaw/Circle Platform | $0E | +4 |
| 0x2C | Cloud Platform | $0E | +4 |
| 0x31–0x32 | Stretch Boo variants | $12 | +4 |
| 0x36–0x38 | Scale/Waterfall Platforms | $0E | +4 |
| 0x3C | Circle Platform | $0E | +4 |
| 0x44 | Platform URLL | $0E | +4 |
| 0x4A | Boom-Boom | $13 | +4 |
| 0x4B–0x4C | Boom-Boom Fly/Split | $33 | +5 |
| 0x51 | Rotodisc CW | $12 | +4 |
| 0x53 | Podoboo | $12 | +4 |
| 0x54 | Missile Bill | $0E | +4 |
| 0x58 | Fire Chomp | $0E | +4 |
| 0x5A–0x5B | Rotodisc CCW/CW2 | $12 | +4 |
| 0x5E–0x60 | Rotodisc Fast/1.5 | $12 | +4 |
| 0x90–0x93 | Moving Platforms | $4F | +5 |
| 0x94–0x9A | Big ? Blocks | $4C | +4 |
| 0x9D | Podoboo Fire Jet | $37 | +5 |
| 0x9E | Podoboo Fire Jet 2 | $12 | +4 |
| 0xA8 | Muncher | $5A | +4 |
| 0xAC | Fire Jet Upward | $37 | +5 |
| 0xB1–0xB2 | Fire Jet Down/Right | $37 | +5 |

#### PatTableSel ROM Table Locations

Each object group's PatTableSel table is at offset +0x144 within its PRG bank:

| Group | PRG Bank | File Offset | Object ID Range |
|-------|----------|-------------|-----------------|
| 1 | PRG001 | 0x02154 | 0x00–0x23 |
| 2 | PRG002 | 0x04154 | 0x24–0x47 |
| 3 | PRG003 | 0x06154 | 0x48–0x6B |
| 4 | PRG004 | 0x08154 | 0x6C–0x8F |
| 5 | PRG005 | 0x0A154 | 0x90–0xB3 |

Raw byte encoding: `$00` = NOCHANGE, `$01–$7F` = page for slot +4, `$80–$FF` = `(val & $7F)` for slot +5.

Raw byte encoding: each byte is read by the object init routine. `$00` means
no bank switch (the object uses whatever CHR page is already loaded). For
non-zero values, bit 7 selects which CHR slot to load: clear = slot +4,
set = slot +5 (with the page number in bits 6–0).

Source: ROM PatTableSel tables, verified against Southbird disassembly.

#### Complete PatTableSel Dump (All 180 Object IDs)

Verified byte-for-byte against ROM at the five table offsets above. 36 entries
per group, one byte per object ID. This is the authoritative reference — the
partial tables in the sections above are subsets of this data.

**Group 1 — PRG001 (0x02154): IDs 0x00–0x23**

| ID | Raw | Page | Slot | Name |
|----|-----|------|------|------|
| 0x00 | $00 | — | NOCHANGE | ObjectEntry |
| 0x01 | $48 | $48 | +4 | MicroGoomba |
| 0x02 | $4C | $4C | +4 | QuestionBlock |
| 0x03 | $48 | $48 | +4 | Poof |
| 0x04 | $48 | $48 | +4 | DVPlatform |
| 0x05 | $48 | $48 | +4 | DVPlatform2 |
| 0x06 | $00 | — | NOCHANGE | ObjectEntry06 |
| 0x07 | $00 | — | NOCHANGE | ObjectEntry07 |
| 0x08 | $93 | $13 | +5 | FireChomp Flame |
| 0x09 | $B7 | $37 | +5 | FireChomp Flame2 |
| 0x0A | $48 | $48 | +4 | MicroGoomba (alt) |
| 0x0B | $00 | — | NOCHANGE | IceBlock |
| 0x0C | $00 | — | NOCHANGE | FireChomp Fire |
| 0x0D | $00 | — | NOCHANGE | ChainChompFree |
| 0x0E | $00 | — | NOCHANGE | Koopaling |
| 0x0F | $00 | — | NOCHANGE | EndLevelCard (unused) |
| 0x10 | $00 | — | NOCHANGE | Podoboo Ceiling |
| 0x11 | $00 | — | NOCHANGE | WoodenPlat FLR |
| 0x12 | $00 | — | NOCHANGE | WoodenPlat FR |
| 0x13 | $00 | — | NOCHANGE | PSwitchDoor |
| 0x14 | $00 | — | NOCHANGE | GoldenCoin |
| 0x15 | $00 | — | NOCHANGE | MovingCoin |
| 0x16 | $48 | $48 | +4 | Poof (alt) |
| 0x17 | $1A | $1A | +4 | Airship Propeller |
| 0x18 | $3A | $3A | +4 | Bowser |
| 0x19 | $00 | — | NOCHANGE | Bowser Fire |
| 0x1A | $00 | — | NOCHANGE | RecBrickFloor |
| 0x1B | $00 | — | NOCHANGE | TreasureBox |
| 0x1C | $00 | — | NOCHANGE | DonutLift |
| 0x1D | $48 | $48 | +4 | DVPlatform (alt) |
| 0x1E | $00 | — | NOCHANGE | DVPlatform Drop |
| 0x1F | $00 | — | NOCHANGE | DVPlatform3 |
| 0x20 | $0A | $0A | +4 | DVPlatform Drop3 |
| 0x21 | $00 | — | NOCHANGE | DVBigPlatform |
| 0x22 | $00 | — | NOCHANGE | BoltPlatform |
| 0x23 | $00 | — | NOCHANGE | AirLift |

**Group 2 — PRG002 (0x04154): IDs 0x24–0x47**

| ID | Raw | Page | Slot | Name |
|----|-----|------|------|------|
| 0x24 | $0E | $0E | +4 | PlatformDrop |
| 0x25 | $00 | — | NOCHANGE | PipeWayController |
| 0x26 | $0E | $0E | +4 | TiltPlatform |
| 0x27 | $0E | $0E | +4 | Seesaw |
| 0x28 | $0E | $0E | +4 | PlatformClockwise |
| 0x29 | $0A | $0A | +4 | Spike |
| 0x2A | $0A | $0A | +4 | Patooie |
| 0x2B | $0B | $0B | +4 | Goomba in Shoe |
| 0x2C | $0E | $0E | +4 | ChainChomp |
| 0x2D | $1A | $1A | +4 | ChainChomp Strained |
| 0x2E | $93 | $13 | +5 | WoodBlock |
| 0x2F | $12 | $12 | +4 | Boo |
| 0x30 | $12 | $12 | +4 | HotFoot (shy) |
| 0x31 | $12 | $12 | +4 | Stretch Boo |
| 0x32 | $12 | $12 | +4 | Stretch Boo Diagonal |
| 0x33 | $0A | $0A | +4 | Nipper |
| 0x34 | $05 | $05 | +4 | Boss Fireball |
| 0x35 | $05 | $05 | +4 | Boss Fireball2 |
| 0x36 | $0E | $0E | +4 | ScalePlatform |
| 0x37 | $0E | $0E | +4 | PlatformWFalls |
| 0x38 | $0E | $0E | +4 | PlatformWFalls2 |
| 0x39 | $0A | $0A | +4 | NipperHopping |
| 0x3A | $93 | $13 | +5 | RocketSled |
| 0x3B | $CF | $4F | +5 | FireJet Left |
| 0x3C | $0E | $0E | +4 | PlatformCircle |
| 0x3D | $0A | $0A | +4 | Airship Anchor |
| 0x3E | $1A | $1A | +4 | PlatformULDR |
| 0x3F | $93 | $13 | +5 | Dry Bones |
| 0x40 | $0A | $0A | +4 | Buster Beetle |
| 0x41 | $00 | — | NOCHANGE | EndLevelCard |
| 0x42 | $CF | $4F | +5 | ObjectEntry42 |
| 0x43 | $CF | $4F | +5 | ObjectEntry43 |
| 0x44 | $0E | $0E | +4 | PlatformURLL |
| 0x45 | $12 | $12 | +4 | HotFoot |
| 0x46 | $0A | $0A | +4 | WonderWing |
| 0x47 | $00 | — | NOCHANGE | WaterCurrent Down |

**Group 3 — PRG003 (0x06154): IDs 0x48–0x6B**

| ID | Raw | Page | Slot | Name |
|----|-----|------|------|------|
| 0x48 | $1A | $1A | +4 | WaterCurrent Right |
| 0x49 | $36 | $36 | +4 | WaterCurrent Left |
| 0x4A | $13 | $13 | +4 | Boom-Boom |
| 0x4B | $B3 | $33 | +5 | Boom-Boom Fly |
| 0x4C | $B3 | $33 | +5 | Boom-Boom Split |
| 0x4D | $00 | — | NOCHANGE | FakeFloor Drop |
| 0x4E | $00 | — | NOCHANGE | ObjectEntry4E |
| 0x4F | $0A | $0A | +4 | ChainChomp (freed) |
| 0x50 | $36 | $36 | +4 | NipperSpawner |
| 0x51 | $12 | $12 | +4 | Rotodisc CW |
| 0x52 | $05 | $05 | +4 | NipperSunflower |
| 0x53 | $12 | $12 | +4 | Podoboo |
| 0x54 | $0E | $0E | +4 | Missile Bill |
| 0x55 | $0B | $0B | +4 | Bob-omb |
| 0x56 | $5A | $5A | +4 | ToadHouse Host |
| 0x57 | $5A | $5A | +4 | ToadHouse Chest |
| 0x58 | $0E | $0E | +4 | Fire Chomp |
| 0x59 | $0E | $0E | +4 | Wandering Hammer |
| 0x5A | $12 | $12 | +4 | Rotodisc CCW |
| 0x5B | $12 | $12 | +4 | Rotodisc CW2 |
| 0x5C | $CF | $4F | +5 | ObjectEntry5C |
| 0x5D | $00 | — | NOCHANGE | ObjectEntry5D |
| 0x5E | $12 | $12 | +4 | Rotodisc CW Fast |
| 0x5F | $12 | $12 | +4 | Rotodisc CCW Fast |
| 0x60 | $12 | $12 | +4 | Rotodisc CW 1.5 |
| 0x61 | $1A | $1A | +4 | Blooper w/ Kids |
| 0x62 | $1A | $1A | +4 | Blooper |
| 0x63 | $1A | $1A | +4 | Big Bertha |
| 0x64 | $CF | $4F | +5 | CheepCheep Hopper |
| 0x65 | $00 | — | NOCHANGE | WaterCheep |
| 0x66 | $00 | — | NOCHANGE | Jellyfish |
| 0x67 | $9B | $1B | +5 | ObjectEntry67 |
| 0x68 | $0B | $0B | +4 | Lava Flotsam Right |
| 0x69 | $0B | $0B | +4 | Lava Flotsam Left |
| 0x6A | $1A | $1A | +4 | Blooper Child Shoot |
| 0x6B | $CF | $4F | +5 | Piledriver |

**Group 4 — PRG004 (0x08154): IDs 0x6C–0x8F**

| ID | Raw | Page | Slot | Name |
|----|-----|------|------|------|
| 0x6C | $CF | $4F | +5 | Green Troopa |
| 0x6D | $CF | $4F | +5 | Red Troopa |
| 0x6E | $CF | $4F | +5 | Paratroopa Green Hop |
| 0x6F | $CF | $4F | +5 | Flying Red Paratroopa |
| 0x70 | $0B | $0B | +4 | Buzzy Beetle |
| 0x71 | $0B | $0B | +4 | Spiny |
| 0x72 | $CF | $4F | +5 | Goomba |
| 0x73 | $CF | $4F | +5 | Para-Goomba |
| 0x74 | $CF | $4F | +5 | Para-Goomba w/ Micros |
| 0x75 | $00 | — | NOCHANGE | Goomba Redirect |
| 0x76 | $CF | $4F | +5 | ObjectEntry76 |
| 0x77 | $00 | — | NOCHANGE | Green Cheep |
| 0x78 | $CF | $4F | +5 | Bullet Bill |
| 0x79 | $CF | $4F | +5 | Bullet Bill Homing |
| 0x7A | $3D | $3D | +4 | Big Green Troopa |
| 0x7B | $3D | $3D | +4 | Big Red Troopa |
| 0x7C | $3D | $3D | +4 | Big Goomba |
| 0x7D | $3D | $3D | +4 | Big Green Piranha |
| 0x7E | $3D | $3D | +4 | Big Green Hopper |
| 0x7F | $3D | $3D | +4 | Big Red Piranha |
| 0x80 | $CF | $4F | +5 | Flying Green Paratroopa |
| 0x81 | $4E | $4E | +4 | Hammer Bro |
| 0x82 | $4E | $4E | +4 | Boomerang Bro |
| 0x83 | $0B | $0B | +4 | Lakitu |
| 0x84 | $0B | $0B | +4 | Spiny Egg |
| 0x85 | $0B | $0B | +4 | ObjectEntry85 |
| 0x86 | $4E | $4E | +4 | Heavy Bro |
| 0x87 | $4E | $4E | +4 | Fire Bro |
| 0x88 | $CF | $4F | +5 | Orange Cheep |
| 0x89 | $0A | $0A | +4 | ObjectEntry89 |
| 0x8A | $12 | $12 | +4 | Thwomp |
| 0x8B | $12 | $12 | +4 | Thwomp Left Slide |
| 0x8C | $12 | $12 | +4 | Thwomp Right Slide |
| 0x8D | $12 | $12 | +4 | Thwomp Up/Down |
| 0x8E | $12 | $12 | +4 | Thwomp Diagonal UL |
| 0x8F | $12 | $12 | +4 | Thwomp Diagonal DL |

**Group 5 — PRG005 (0x0A154): IDs 0x90–0xB3**

| ID | Raw | Page | Slot | Name |
|----|-----|------|------|------|
| 0x90 | $CF | $4F | +5 | Moving Platform |
| 0x91 | $CF | $4F | +5 | Moving Platform 2 |
| 0x92 | $CF | $4F | +5 | Moving Platform 3 |
| 0x93 | $CF | $4F | +5 | Moving Platform Fall |
| 0x94 | $4C | $4C | +4 | Big ? Block 3-UP |
| 0x95 | $4C | $4C | +4 | Big ? Block Mushroom |
| 0x96 | $4C | $4C | +4 | Big ? Block Fire Flower |
| 0x97 | $4C | $4C | +4 | Big ? Block Super Leaf |
| 0x98 | $4C | $4C | +4 | Big ? Block Tanooki |
| 0x99 | $4C | $4C | +4 | Big ? Block Frog |
| 0x9A | $4C | $4C | +4 | Big ? Block Hammer |
| 0x9B | $00 | — | NOCHANGE | ObjectEntry9B |
| 0x9C | $00 | — | NOCHANGE | ObjectEntry9C |
| 0x9D | $B7 | $37 | +5 | Podoboo Fire Jet |
| 0x9E | $12 | $12 | +4 | Podoboo Fire Jet 2 |
| 0x9F | $0E | $0E | +4 | ObjectEntry9F |
| 0xA0 | $CF | $4F | +5 | Green Piranha |
| 0xA1 | $CF | $4F | +5 | Green Piranha (flipped) |
| 0xA2 | $CF | $4F | +5 | Red Piranha |
| 0xA3 | $CF | $4F | +5 | Red Piranha (flipped) |
| 0xA4 | $CF | $4F | +5 | Green Piranha Fire |
| 0xA5 | $CF | $4F | +5 | Green Piranha Fire (ceil) |
| 0xA6 | $CF | $4F | +5 | Venus Fire Trap |
| 0xA7 | $CF | $4F | +5 | Venus Fire Trap (ceil) |
| 0xA8 | $5A | $5A | +4 | Muncher |
| 0xA9 | $5A | $5A | +4 | Muncher (alt) |
| 0xAA | $36 | $36 | +4 | ObjectEntryAA |
| 0xAB | $36 | $36 | +4 | ObjectEntryAB |
| 0xAC | $B7 | $37 | +5 | Fire Jet Upward |
| 0xAD | $36 | $36 | +4 | ObjectEntryAD |
| 0xAE | $36 | $36 | +4 | ObjectEntryAE |
| 0xAF | $32 | $32 | +4 | ObjectEntryAF |
| 0xB0 | $36 | $36 | +4 | ObjectEntryB0 |
| 0xB1 | $B7 | $37 | +5 | Fire Jet Down |
| 0xB2 | $B7 | $37 | +5 | Fire Jet Right |
| 0xB3 | $0B | $0B | +4 | ObjectEntryB3 |

All 180 entries verified byte-for-byte against ROM (2025-04-13). Matches
`sprite_bank()` in `enemies.rs` with zero discrepancies.

---

## Gameplay Mechanics (ROM Offsets)

### Enemy Behavior

| File Offset | Description |
|------------|-------------|
| 0x00F22 | Shell behavior code (replace byte with A9 to modify) |
| 0x0133C | Shell stay duration |
| 0x09368 | Enemy left speed |
| 0x09369 | Enemy right speed |
| 0x0A837 | Venus Flytrap cycle time |
| 0x0BDC3 | ParaBeetle right flight speed |
| 0x0BDCD | ParaBeetle left flight speed |
| 0x0FD75 | Goomba generator output sprite |
| 0x080A4 | Koopa Paratroopa de-wing sprite |
| 0x080AE | Para-Goomba de-wing sprite |
| 0x06EA9 | Boom-Boom drop sprite |

### Piranha Plant Visibility (PRG004 / PRG005)

Piranha plants use `Objects_Var4` (zero-page `$7F`) as their state machine:
`0=HideInPipe`, `1=Emerge`, `2=Attack`, `3=Retract`. Init routines do **not**
write `Var4`, so it starts at 0 and the first frame of `ObjNorm_Piranha` skips
the draw call entirely — the plant is invisible until a per-object timer +
"Mario not too close" gate transitions it to state 1.

That hide-on-spawn is correct for pipe-mounted piranhas (vanilla placement),
but when wild-shuffle drops a piranha into a non-pipe slot the player sees
nothing and the plant pops into view unfairly. The randomizer fixes this by
priming `Var4 = 1` (Emerge) at the end of each piranha init via two small
thunks. The fix is gated on `piranhas == Wild` **and** at least one other
enemy class also Wild — outside that case the wild pool can't put piranhas
into foreign slots, so vanilla hide-then-emerge is preserved.

**Small piranhas (0xA0–0xA7) — PRG005:**

All eight small-piranha init routines share a tail at CPU `$A63A`…`$A654` ending
in `LDA <$91,X / CLC / ADC #$08 / STA <$91,X / RTS`. The patch replaces the
last 3 bytes with a JMP to a 7-byte thunk in PRG005 free space.

| Item | Value |
|------|-------|
| Patch site (file) | **0x0A662** (CPU `$A652`) — bytes `95 91 60` → `4C C6 BF` |
| Thunk (file) | **0x0BFD6** (CPU `$BFC6`), 7 bytes: `95 91 A9 01 95 7F 60` |
| Effect | Re-do displaced `STA <$91,X`, then `LDA #$01 / STA <$7F,X / RTS` |

**Big piranhas (0x7D / 0x7F) — PRG004:**

Both `ObjInit_GiantGreenPiranha` and `ObjInit_GiantRedPiranha` converge on a
shared tail at CPU `$B75A`…`$B776` ending in `STA $0679,X / INC $7FF7,X / RTS`
(the `Objects_IsGiant` flag bump). The patch replaces the last 4 bytes with a
JMP + dead-byte filler so the timer reload table at `$B777` stays put.

| Item | Value |
|------|-------|
| Patch site (file) | **0x09783** (CPU `$B773`) — bytes `FE F7 7F 60` → `4C 56 BE 60` |
| Thunk (file) | **0x09E66** (CPU `$BE56`), 8 bytes: `FE F7 7F A9 01 95 7F 60` |
| Effect | Re-do displaced `INC $7FF7,X`, then `LDA #$01 / STA <$7F,X / RTS` |

In both cases `$7F` is `Objects_Var4` — confirmed by the dispatch instruction
`LDA <$7F,X / AND #$03` at the top of `ObjNorm_Piranha` in both banks.

**Per-frame hitbox skip (distance-based gate around the hidden state):**

The visibility prime above fixes spawn, but the state machine cycles back to
HideInPipe every period. During that hide phase the sprite is skipped but the
per-frame `JSR Player_HitEnemy` keeps firing — for vanilla pipe placements
the hitbox is inside the pipe geometry so it never matters, but a wild
piranha in mid-level becomes an invisible damaging spot.

`ObjNorm_BigPiranha` (PRG004, `$B77B`) already short-circuits state 0:
`AND #$03 / BNE main / LDA #$FF / STA SprHVis,X / JMP $B79D`. The
`JSR Player_HitEnemy` at `$B79A` is unreachable from state 0, so no patch is
needed for `0x7D / 0x7F`.

`ObjNorm_Piranha` (PRG005, `$A661`) runs `JSR Player_HitEnemy` every frame
regardless of orientation or state. The thunk gates the call on the
piranha's distance from its hidden-position Var5: skip when
`|Objects_Y - Objects_Var5| < 10 px`. That window covers the fully-hidden
state plus ~10 frames of safety at each transition (Retract end /
Emerge start). `Piranha_Retract` (`$A7C4`) advances Y by `+1` per frame, so
10 px ≈ 10 frames. The piranha's emerge height is ~24 px so the Attack
state sits well outside the window — no risk of unintended skipping when
fully extended.

Distance is orientation-agnostic — upright piranhas have `Y < Var5` (large
positive wrap, ≥ $F6), ceiling piranhas have `Y > Var5` (small positive,
< $0A). The thunk uses a two-tail compare and needs no FlipBits dispatch.

| Item | Value |
|------|-------|
| Patch site (file) | **0x0A7A4** (CPU `$A794`) — bytes `20 BA D1` → `4C CD BF` |
| Thunk (file) | **0x0BFDD** (CPU `$BFCD`), 18 bytes: `B5 A3 38 F5 9A 18 69 0A C9 15 90 03 20 BA D1 4C 97 A7` |
| Effect | `LDA Y,X / SEC / SBC Var5,X / CLC / ADC #$0A / CMP #$15 / BCC skip / JSR $D1BA / skip: JMP $A797` |
| Math | Bias `Y - Var5` by `+10` so the window `[-10, +10]` maps onto `[0, 20]`; a single `BCC #$15` catches both upright (`Y < Var5`) and ceiling (`Y > Var5`) orientations. |

`Objects_Y = $A3,X`, `Objects_Var5 = $9A,X` — verified from `Piranha_Retract`
at `$A7C4` (`LDA $A3,X / ADD #$01 / ... / CMP $9A,X`).

Same wild+other-wild gate as the visibility patch above.

### Koopaling Stomp Threshold (PRG001)

The Koopalings (object ID `$0E`) use `Objects_Var4` (zero-page `$7F–$83`, indexed by
object slot X) as a stomp counter. The handler `ObjHit_Koopaling` at CPU `$B185`
increments the counter and checks against a hardcoded threshold of 3:

```
$B185: F6 7F       INC $7F,X        ; stomp counter++
$B187: B5 7F       LDA $7F,X        ; A = new count
$B189: C9 03       CMP #$03         ; >= 3?
$B18B: B0 06       BCS $B193        ; yes → defeated
$B18D: A9 80       LDA #$80         ; no → invulnerability timer
$B18F: 9D 20 05    STA $0520,X
$B192: 60          RTS              ; survive
$B193: ...                          ; defeat sequence
```

| Item | Value |
|------|-------|
| PRG bank | PRG001 (file 0x02010–0x0400F, CPU $A000–$BFFF) |
| Patch site | File **0x03197** (3 bytes: `B5 7F C9` = `LDA $7F,X; CMP #$03`) |
| Threshold operand | File **0x03199** (single byte `$03`) |
| Survive path | CPU **$B18D** (sets `Objects_Timer2`, RTS) |
| Defeat path | CPU **$B193** (Koopaling death sequence) |
| Free space | 0x0382A (102 bytes), 0x03FC0 (80 bytes) |

Fireballs use a separate counter `Objects_HitCount` (`$7CF6–$7CFA`), initialized to
10 (`$0A`) during `ObjInit_Koopaling`. When it reaches 0, the code sets `Objects_Var4`
to 2 and jumps into the stomp-kill path, forcing defeat as if the third stomp landed.
Bowser uses only `Objects_HitCount` (initialized to 34), no stomp counter.

### Koopaling Softlock Fix (PRG001)

When airship levels are shuffled across worlds, Koopalings can softlock due to an
object init table value at file **0x02186** (CPU `$A176`). The vanilla byte `$05`
specifies a behavior state that breaks when the Koopaling loads outside its native
world. Changing it to `$09` prevents the softlock.

| Item | Value |
|------|-------|
| PRG bank | PRG001 (file 0x02010–0x0400F, CPU $A000–$BFFF) |
| Patch offset | File **0x02186** (CPU `$A176`) |
| Vanilla value | `$05` |
| Patched value | `$09` |
| Source | "SMB3 - Koopaling Softlock Fix.ips" |

### Hammer Vulnerable Koopalings (PRG000)

Koopalings are normally invulnerable to thrown hammers. The object attribute byte at
file **0x00312** (CPU `$8302`, PRG000) has bit 7 set (`$89`), which flags them as
hammer-immune. Clearing bit 7 (`$09`) makes hammers damage them like any other enemy.

| Item | Value |
|------|-------|
| PRG bank | PRG000 (file 0x00010–0x0200F, CPU $8000–$9FFF) |
| Patch offset | File **0x00312** (CPU `$8302`) |
| Vanilla value | `$89` (bit 7 = hammer invulnerable) |
| Patched value | `$09` (bit 7 cleared) |
| Source | "SMB3 - Koopaling Softlock Fix + Hammers Can Hit Koopalings.ips" |

### Coin Ship End-Pipe Bro Fight

When a wandering Hammer Bro overworld sprite transforms into a Coin Ship sprite
(triggered by specific coin/score conditions in `OBJ_BONUSCONTROLLER`, ID `0xD4`),
walking into the Coin Ship loads an autoscroll ship level in the Ship tileset
(PRG023). The end of that level contains a pipe junction whose destination is a
small sub-area with **two BoomerangBros** as the fight reward.

| Item | Value |
|------|-------|
| Sub-area enemy pointer | CPU `$DA0F` (file `0x0DA1F`) |
| Junction reference | File `0x2FC27` in PRG023 (Ship tileset) — bytes `BC 0F DA` |
| Enemy contents | 2× `0x82` (BoomerangBro) + `0xBA` terminator (3 entries) |

The sub-area has no world pointer table entry — it's reached only via the
in-layout junction — so the randomizer protects it via
`HAMMER_BRO_SEGMENT_OFFSETS` (file offset `0x0DA1F`) rather than via
`HAMMER_BRO_OBJ_PTRS`. This routes its enemy randomization through the HB-wild
path (stompable-only pool, optionally one shell-killable + one shell partner).

### Enemy Stompability Classification

Used by the randomizer for Hammer Bro encounter constraints. Enemies are classified
by whether the player can defeat them by jumping on them (no powerups required).

**Stompable** (safe for single-enemy HB encounters):
Goomba (0x72), BigGoomba (0x7C), BobOmb (0x55), PileDriver (0x6B), GoombShoe (0x2B),
BusterBeetle (0x40), DryBones (0x3F), FireChomp (0x58), Spike (0x29),
GreenTroopa (0x6C), RedTroopa (0x6D), BuzzyBeetle (0x70), BigGreenTroopa (0x7A),
BigRedTroopa (0x7B), ParatroopaGreenHop (0x6E), FlyingRedParatroopa (0x6F),
Paragoomba (0x73), ParagoombaMicros (0x74), BigGreenHopper (0x7E),
FlyingGreenParatroopa (0x80), HammerBro (0x81), BoomerangBro (0x82),
HeavyBro (0x86), FireBro (0x87).

Note: Bullet Bill projectiles (0x78/0x79) are NOT used as HB enemies — they
are cannon-spawned objects whose movement state is only initialized by the
cannon firing routine. The Bullet Bill **cannons** (0xBC/0xBD) are handled
by the `bullet_bills` enemy class, which swaps regular ↔ homing in place.

**Non-stompable, killable with shell** (allowed in 2-enemy HB encounters with a shell partner):
Spiny (0x71), Patooie (0x2A), Nipper (0x33), NipperHopping (0x39), BigBertha (0x63).

**Not used in HB encounters** (water/ghost/piranha/thwomp/rotodisc/cannon classes, Boo, etc.):
These enemies are either unkillable by stomping, require specific level geometry (pipes),
or have behavior unsuitable for the flat HB encounter arenas.

### Shell-Protected Levels

Some levels require shell enemies for progression (breaking bricks to access paths).
Shell-class enemies at protected offsets always shuffle within the shell class regardless
of wild mode settings. Current protected levels:
- **2-Pyr** sub-area (0xC5BC): 11 Buzzy Beetles — shells needed to break brick barriers
- **2-3** (0xD1F0): 2 GreenTroopas at end — shells needed to break bricks
- **6-5** sub-area (0xC5EB): 1 GreenTroopa — shell needed for progression

### Player Physics

| File Offset | Description |
|------------|-------------|
| 0x104F8 | Maximum running speed (must be >= 0x7F for flight) |
| 0x10CAA | Flight duration cap |
| 0x103F1 | Tanooki statue duration |

### Blocks & Tiles

| File Offset | Description |
|------------|-------------|
| 0x11618 | Coin tile identifier |
| 0x11634 | P-switch tile identifier |
| 0x11653 | P-switch duration (default: 0x80) |
| 0x11657 | P-switch music value (default: 0xA0) |
| 0x1167E | Ice block melt time |
| 0x118A5 | Multi-coin block max value |
| 0x118BA | Multi-coin time window |
| 0x11E6A | Magic block hold time |
| 0x11E6F | Magic block effect duration |

**Bumped block mechanism (RAM):**
- $036C: nametable address of bumped block (0 = no pending write)
- $036E: metatile data in NW-NE-SW-SE order
- On next VBlank, writes metatile to nametable; adds 32 to address for next row
- No scroll boundary adjustment is performed

### Misc

| File Offset | Description |
|------------|-------------|
| 0x309D5 | Debug mode: low byte of jump table entry (0x35=disable). GG code KKKZSPIU. Enable value uncertain — rom_map.py says 0xCC, earlier notes said 0xC5, neither worked as a ROM patch (broke title screen). GG code itself works but corrupts PRG030 since $89C5 is shared across banks. |
| 0x3509B | 1-Up coin threshold (coins needed for extra life) |
| 0x1451F | World spawn delay (frames before Mario appears) |

---

## RAM Map (Key Addresses)

### Game State

| Address | Description |
|---------|-------------|
| $0014 | Flag to return to map |
| $0015 | Frame counter (incremented each cycle) |
| $0376 | Pause flag |
| $0727 | World number (0-indexed) |
| $0726 | Current player (0=Mario, 1=Luigi) |
| $070A | Current Object Set / tileset |
| $0781 | RNG (72-bit LFSR) |

### Player State

| Address | Description |
|---------|-------------|
| $00ED | Current power-up form (0x00–0x06) |
| $00EF | Facing direction (0x40=right, 0x00=left) |
| $00BD | Horizontal velocity (signed) |
| $00CF | Vertical velocity (signed) |
| $00D8 | In-air flag |
| $0577 | Kuribo's Boot equipped (0/1) |
| $0736 | Mario lives (max 99 decimal / 63 hex) |
| $0737 | Luigi lives |

### Player Position (In-Level)

| Address | Description |
|---------|-------------|
| $0090 | Horizontal position |
| $00A2 | Vertical position |
| $074D | Horizontal subpixel (1/16 pixel) |
| $075F | Vertical subpixel (1/16 pixel) |

### World Map Position

| Address | Description |
|---------|-------------|
| $7976 | Mario's map Y position |
| $7977 | Luigi's map Y position |
| $7978–$7979 | Map X position high byte (Mario/Luigi) |
| $797A–$797B | Map X position low byte (Mario/Luigi) |
| $797E–$797F | Death respawn map Y (Mario/Luigi) |
| $7980–$7981 | Death respawn map X high (Mario/Luigi) |
| $7982–$7983 | Death respawn map X low (Mario/Luigi) |

### Enemy / Object State

| Address | Description |
|---------|-------------|
| $007F–$0083 | `Objects_Var4` — per-slot variable (Koopaling stomp counter) |
| $0428 | Current enemy slot index |
| $0520–$0524 | `Objects_Timer2` — per-slot timer (Koopaling invulnerability) |
| $7CF6–$7CFA | `Objects_HitCount` — per-slot fireball HP (Koopaling=10, Bowser=34) |

### Physics Constants (RAM-accessible)

| Address | Value | Description |
|---------|-------|-------------|
| $A648 | varies | Initial jump velocity |
| $ACA2 | 0x05 | Falling gravity |
| $ACA6 | 0xE0 | Default upward jump velocity |
| $ACB3 | 0x01 | Jump gravity |

**Jump velocity calculation:** At $AC5A, horizontal velocity is loaded, divided by 16,
and used to index a subtraction table (`00, 02, 04, 08`) that reduces the default jump
velocity — faster horizontal movement = higher jump. Fall velocity is clamped at 0x40
(effective max 0x45 after gravity).

### P-Meter & Flight

| Address | Description |
|---------|-------------|
| $03DD | P-meter display (bits 0–5 arrows, bit 6 = P) |
| $0515 | P-timer |
| $056E | Flight duration timer (0xFF = unlimited) |
| $057B | Flight mode flag (1 when P-meter full + jumping) |

### Inventory

| Address | Size | Description |
|---------|------|-------------|
| $7D80–$7D9B | 28 bytes | Mario's items (13 slots, Global Item IDs) |
| $7DA3–$7DBE | 28 bytes | Luigi's items (13 slots) |
| $7D9C–$7D9E | 3 bytes | Mario's goal cards (0=none, 1=mushroom, 2=flower, 3=star) |
| $7DBF–$7DC1 | 3 bytes | Luigi's goal cards |
| $7DA2 | 1 byte | Mario's coins |
| $7DC5 | 1 byte | Luigi's coins |
| $7D9F–$7DA1 | 3 bytes | Mario's score (÷10) |
| $7DC2–$7DC4 | 3 bytes | Luigi's score (÷10) |

### Level Completion

| Address | Size | Description |
|---------|------|-------------|
| $7D00–$7D3F | 64 bytes | Mario's level completion flags |
| $7D40–$7D7F | 64 bytes | Luigi's level completion flags |

**Completion flag byte format:** `0abbccc`
- a = player (0=Mario, 1=Luigi)
- b = page/screen (0–3)
- c = column (0–15)

**Row indexing within each byte:** bits 7–1 = rows 0–6; bit 0 = row 8 (row 7 is skipped).
Each entry represents a 16x16 metatile column on the world map.

---

## Sound & Music

### Music Trigger Addresses (RAM)

| Address | Description |
|---------|-------------|
| $04F1 | Sound effect trigger 1 (jump, blocks, swimming) |
| $04F2 | Sound effect trigger 2 (coins, power-ups, items) |
| $04F3 | Sound effect trigger 3 (bricks, fire, airship) |
| $04F4 | Fanfare music trigger (death, victory) |
| $04F5 | Music change (world maps, themes) |
| $04F6 | Sound effect trigger 4 (map movement, level entry) |
| $04F7 | Pause control (0x01=pause, 0x02=resume) |

### Music Values ($04F5)

| Value | Music |
|-------|-------|
| 0x01–0x08 | World 1–8 map music |
| 0x09 | World 9 / Coin Heaven |
| 0x0A | Star power |
| 0x0B | Warp whistle |
| 0x0C | Music box |
| 0x0D | Wand return |
| 0x10 | Plains |
| 0x20 | Underground |
| 0x30 | Water |
| 0x40 | Dungeon |
| 0x50 | Boss battle |
| 0x60 | Doomship |
| 0x70 | Hammer Bros stage |
| 0x80 | Mushroom house |
| 0x90 | Hilly theme |
| 0xA0 | P-switch |
| 0xB0 | Bowser fight |

### Title Menu Music

Vanilla SMB3 leaves the 1P/2P select menu silent (the only title-screen music is the brief intro cutscene snippet, which the seed-hash patch skips). To add menu music, the intro-skip routine in PRG031 free space appends `LDA #music / STA $04F5` after setting `Title_State = 6`. The music engine picks up the change on the next frame and loops the track for as long as the player stays on the menu; pressing Start advances to the world map, which queues its own music as normal.

The track is chosen deterministically from the seed via a curated 16-entry table (world map themes 1–9, plus level themes 0x10/0x20/0x30/0x40/0x60/0x80/0x90). See `src/randomize/title_screen.rs::MENU_MUSIC_TRACKS` and `pick_menu_music`. When `starting_items` is active it overwrites the lives-init hook, so `qol::write_starting_items` mirrors the same `STA $04F5` inside its own trampoline.

---

## Autoscroll Disable

Disabling autoscrollers requires far more than removing the D3 autoscroll objects. The reference patch `Super_Mario_Bros_3_NoAutoscrolls(Except 5-9).ips` (65 records, 662 bytes) makes changes across five ROM regions:

### 1. Enemy/Object Data (0x0BFD8–0x0E00D)

**D3 autoscroll removals** — 14 offsets set to 0x00:
`0x0CA74, 0x0CB63, 0x0CC44, 0x0CD28, 0x0CDD3, 0x0CF51, 0x0D6B7, 0x0D72D, 0x0D768, 0x0D7A9, 0x0D878, 0x0D92D, 0x0D980, 0x0DA15`

**NOT removed:** Level 5-9 parabeetle ride at `0x0CECE` (required for level to function — no ground without autoscroll).

**Airship enemy data rewrites** — enemies repositioned/replaced for free-scroll play. Large multi-byte patches at: `0x0CC6C` (4B), `0x0CDE7` (7B), `0x0CE9A` (10B), `0x0D6DB` (9B), `0x0D6EA` (18B), `0x0D789` (6B), `0x0D7B3` (5B), `0x0D7CA` (45B), `0x0D7FD` (5B), `0x0D825` (18B), `0x0D849` (6B), `0x0D858` (6B), and others. These provide new cannon, fire jet, and enemy configurations designed for player-controlled scrolling.

**Autoscroll type change:** `0x0D8DF` changed to `0x50` (converts one airship-path autoscroll to horizontal).

**Segment terminators:** `0x0CFE3`, `0x0D038`, `0x0D103` — restructure enemy data segments.

### 2. Level Pointer Table Redirects (PRG012: 0x18010–0x1A00F)

Each W1–W7 airship gets three pointer table changes (ByRowType, ObjSets, LevelLayouts) to load the rewritten enemy/layout data:

| World | ByRowType | ObjSets | LevelLayouts |
|-------|-----------|---------|--------------|
| W1 | 0x19449 = 0x8A | 0x19484 = [0xEA, 0xD6] | 0x194AE = [0xB7, 0xAD] |
| W2 | 0x194DE = 0x6A | 0x19560 = [0x1C, 0xD7] | 0x195BE = [0xAB, 0xAE] |
| W3 | 0x19609 = 0x8A | 0x196A2 = [0x57, 0xD7] | 0x1970A = [0x09, 0xB0] |
| W4 | 0x1971A = 0x6A | 0x19764 = [0x98, 0xD7] | 0x197A8 = [0x3A, 0xB1] |
| W5 | 0x19807 = 0xAA | 0x1987E = [0xA6, 0xD6] | 0x198D2 = [0x97, 0xAC] |
| W6 | 0x19919 = 0x6A | 0x199C0 = [0xE5, 0xD7] | 0x19A32 = [0xB3, 0xB2] |
| W7 | 0x19A69 = 0x9A | 0x19AF0 = [0x14, 0xD8] | 0x19B4C = [0x89, 0xB4] |

### 3. Level Layout Data (Pipe/Water region)

New tile generator data written for reworked airship geometry:
- `0x24DE0` — 28 bytes: repeated metatile pattern (airship deck sections)
- `0x24E6A` — 85 bytes: platform/geometry data (repositioned platforms and structures)

### 4. Airship Level Headers (Ship data: 0x2EC07–0x30005)

For each W1–W7 airship, byte4 (Y-start) and byte5 (X-start) are patched:

| World | Offset | byte4→ | byte5→ | Notes |
|-------|--------|--------|--------|-------|
| W1 | 0x2ECAD | 0xAA | 0x0A | Y-start=5, X-start=0 |
| W2 | 0x2EDCD | 0xAA | 0x0A | |
| W3 | 0x2EEC1 | 0xAA | 0x0A | |
| W4 | 0x2F01F | 0xAA | 0x0A | |
| W5 | 0x2F150 | 0xAA | 0x0A | |
| W6 | 0x2F2C9 | 0xAA | 0x0A | |
| W7 | 0x2F49F | 0xAA | 0x0A | |

**Extra headers:**
- `0x23162` = 0xAC, `0x23B00` = 0xAC — fortress sub-area byte4 (set Y-start bit 5)
- `0x2F62E` = 0x0A, `0x2FC2C` = 0x0A — ship sub-area byte5 (clear X-start bits)

### 5. PRG030 Code Patch

`0x3D7AD` = 0x80 — disables scroll-path camera logic in the game engine.

### Key Insight

Simply removing D3 objects and patching headers is insufficient. Without the enemy data rewrites and pointer redirects, airship levels exhibit broken behavior: the camera scrolls right with no Mario on screen and visual glitches occur. The header patches (byte4/byte5) only work correctly in conjunction with the full set of enemy repositioning and level layout changes. W8 auto-scrolling levels (tanks, battleship, airship) work with D3 removal alone because the PRG030 code patch at `0x3D7AD` handles their scroll behavior separately.

---

## Pipe Destination Tables & Pipe Shuffle

### Pipe Destination Tables (PRG002)

Four parallel tables of 24 bytes each, used by `OBJ_PIPEWAYCONTROLLER` (enemy object 0x25) to determine where Mario exits a pipe transit level on the overworld map.

| Table | ROM Offset | Contents |
|-------|-----------|----------|
| MapXHi | `0x046AA` | Screen number (packed nibbles) |
| MapX | `0x046C2` | Column within screen (packed nibbles) |
| MapY | `0x046DA` | Row nibble = grid_row + 2 (packed nibbles) |
| MapScrlXHi | `0x046F2` | Scroll screen (packed nibbles); bit 3 = center flag (adds 128px to `Horz_Scroll`). Vanilla: A=0, B=1 always. Pipe shuffle sets equal to MapXHi (no center) — the 128px offset assumes hand-tuned positions and causes camera misalignment when pipes land at screen boundaries |

Each byte packs **two** endpoint values as nibbles:
- **Upper nibble** = "A" endpoint (left pipe in transit level)
- **Lower nibble** = "B" endpoint (right pipe in transit level)

The game selects which nibble to use based on Mario's position within the pipe transit level (left half → upper nibble, right half → lower nibble).

### Dest Index → World Mapping

| Dest | World | Dest | World | Dest | World |
|------|-------|------|-------|------|-------|
| 0x01 | W2 | 0x08 | W7 | 0x0F | W8 |
| 0x02 | W6 | 0x09 | W7 | 0x10 | W8 |
| 0x03 | W6 | 0x0A | W7 | 0x11 | W8 |
| 0x04 | W7 | 0x0B | W7 | 0x12 | W3 |
| 0x05 | W7 | 0x0C | W8 | 0x13 | W3 |
| 0x06 | W7 | 0x0D | W8 | 0x14 | W3 |
| 0x07 | W7 | 0x0E | W8 | 0x15-0x16 | W4 |
| | | | | 0x17 | W5 |

Pipe pair counts: W1=0, W2=1, W3=3, W4=2, W5=1, W6=2, W7=8, W8=6.

### Pipe Transit Levels

Pipe transit levels use **tileset 14** and are identified by `entry.tileset == 14` in the pointer tables. Each transit level is a single-screen underground passage. The pipe controller object is stored in enemy data as: `01 25 02 XX FF` where XX = dest index.

Transit level layout data is chained: entry A's area2 = entry B's area1, creating a bidirectional connection. Layout byte5 bit 6 controls pipe direction (0x04 = left-to-right, 0x44 = right-to-left).

### Pipe Shuffle Algorithm

The pipe shuffle randomizes where pipe endpoints appear on each world's overworld map while ensuring all critical locations remain reachable.

**Progressive placement:**

1. **Open gaps**: Simulate post-fortress state by replacing gap tiles (locks → $46, bridges → $45, water gaps → $B3, sky gaps → $45)
2. **Remove pipes**: Replace all pipe tiles with junction tile ($47)
3. **Walk**: BFS from START tile to find initial reachable nodes
4. **Place pairs**: For each pipe pair (in random order):
   - If all nodes reachable → place both endpoints randomly among available reachable positions
   - Otherwise → place one endpoint in a reachable position, the other in an unreachable position (prioritizing components containing must-reach positions like airships and Bowser's castle)
5. **Re-walk** after each placement to update reachable set

**ROM patching:**

After placement, the algorithm patches the ROM:

1. **Entry swaps**: For each pipe that moved, swap its pointer table entry (ByRowType, ByScrCol, ObjSets, LevelLayouts) with the entry at the target position. The tileset nibble stays with its entry (travels with the level data); only the row/screen/col position is swapped.
2. **Dest table updates**: Write new endpoint positions as packed nibbles to all 4 destination tables.
3. **Pointer table re-sort**: Re-sort all entries by (screen, row_nib, col) because the game's lookup scans entries sequentially from `InitIndex[screen]`, matching row first then column. Also rebuilds the InitIndex table with correct per-screen offsets.

### InitIndex Table

The InitIndex master table at `0x193DA` contains 9 word pointers (8 worlds + warp zone). Each points to a per-world sub-table stored just before ByRowType in ROM. Each sub-table has one byte per screen, giving the offset into ByRowType where that screen's entries begin.

To compute the InitIndex file offset for a world:
```
init_ptr = read_word(rom, 0x193DA + world_idx * 2)
init_file = 0x18010 + (init_ptr - 0xA000)
```

**Important:** PRG012 is loaded at CPU `$A000-$BFFF` during the map screen, so the CPU addresses in the master table are in the `$A000+` range. The file offset formula uses `- 0xA000`, NOT `- 0x8000`. Each sub-table is always 4 bytes; unused screens (beyond the world's actual screen count) should be set to N (entry count) as a sentinel.

### Map Walker Movement Model

The game moves the player 2 tiles at a time on the overworld: from a **node** tile, through an intermediate **path** tile, to the next **node** tile. The path tile must be valid for the movement direction:

- **Horizontal** (left/right): `{$45, $B2, $B3, $AC, $B7, $B8, $DA, $B9, $E6}`
- **Vertical** (up/down): `{$46, $B1, $AA, $AB, $B0, $DB, $BA}`

Background tiles `{$B4, $FF, $02}` block movement to the destination node.

Pipes create **bidirectional teleport edges** between two node positions, bypassing the path tile check.

---

## Big ? Block Bonus Rooms

### Vanilla Behavior

When Mario hits a Big ? Block (objects 0x94–0x9A), the game transfers to a bonus room
via `LevelJct_BigQuestionBlock` at ROM **0x349F9** (PRG026, CPU $A9E9). This routine
uses `LDY World_Num` to select from per-world bonus room pointer tables.

**Per-world bonus room tables (PRG026, 8 entries each, indexed by World_Num 0–7):**

| Table | ROM Offset | Contents |
|-------|-----------|----------|
| Layout pointers | 0x3491B | 8 words: level layout CPU addresses per world |
| Enemy pointers | 0x3492B | 8 words: enemy data CPU addresses per world |
| Tileset IDs | 0x3493B | 8 bytes: tileset for each world's bonus room |

### Problem with Level Shuffle

When levels are shuffled across worlds, the Big ? Block bonus room selection breaks.
A level originally from W3 that gets shuffled into W6 will load W6's bonus room instead
of W3's, because the game indexes by the current `World_Num` ($0727), not by the level's
identity.

Two complications:
1. `Level_ObjPtrOrig` ($7EBB/$7EBC) gets overwritten by `Level_JctInit` during sub-area
   junction processing. By the time `LevelJct_BigQuestionBlock` runs, the original entry
   obj_ptr is gone.
2. PRG030's level init at $8948 checks `CPY #$07` (W8) and for W8 specifically overwrites
   `$65/$66` and `$7EBB/$7EBC` with a hardcoded `$C033`, destroying the real obj_ptr
   before any bank-specific save code can run.

### Two-Part Patch

**Part A — Save entry obj_ptr to scratch RAM (PRG030, fixed bank)**

During level init in PRG030 (always loaded), save the real obj_ptr from `$65/$66` to
scratch RAM at $7EB4/$7EB5 before the W8-specific overwrite can destroy it. Using the
fixed bank ensures this fires for ALL entry paths — normal tile entry, army sprite
encounters, and any other mechanism.

Hook point: ROM **0x3C958** — replaces `CPY #$07; BNE +$18` (4 bytes) with `JMP $9F2C` + NOP.

Trampoline at ROM **0x3DF3C** (PRG030 free space, CPU $9F2C), 20 bytes:

```
LDA $65            ; real obj_lo (before W8 overwrite)
STA $7EB4          ; scratch: saved entry obj_lo
LDA $66            ; real obj_hi
STA $7EB5          ; scratch: saved entry obj_hi
CPY #$07           ; (displaced: W8 check)
BNE +3             ; non-W8: skip to JMP $8964
JMP $894C          ; W8 path: continue with overwrite
JMP $8964          ; non-W8 path: skip overwrite
```

**Part B — Lookup routine replaces World_Num indexing (PRG026)**

Hook point: ROM **0x349F9** — replaces `LDY $0727` (3 bytes) with `JSR $B520`.

Lookup routine at ROM **0x35530** (PRG026 free space, CPU $B520), 66 bytes:

The routine reads the saved entry obj_ptr from $7EB4/$7EB5 (not $7EBB/$7EBC which may
have been overwritten by junctions). It searches an 11-entry table of obj_ptr values.
On match, it loads the corresponding room index into Y and returns. On no match (levels
that don't use Big ? Blocks, like W1/W2 levels), it falls back to `LDY $0727`
(World_Num).

**Obj_ptr → room index mapping table (11 levels that use Big ? Blocks):**

| Level | Obj Hi | Obj Lo | Room Index | Vanilla World |
|-------|--------|--------|------------|---------------|
| 3-5 | $CD | $EB | 2 | W3 |
| 3-9 | $C3 | $8F | 2 | W3 |
| 4-F2 | $D5 | $08 | 3 | W4 |
| 5-2 | $C8 | $BE | 4 | W5 |
| 5-5 | $CB | $0A | 4 | W5 |
| 6-3 | $CA | $8E | 5 | W6 |
| 6-9 | $CD | $2D | 5 | W6 |
| 6-10 | $CC | $E8 | 5 | W6 |
| 7-F1 | $D4 | $E4 | 6 | W7 |
| 7-8 | $C3 | $2D | 6 | W7 |
| 8-1 | $C4 | $24 | 7 | W8 |

Room indices are 0-indexed (matching World_Num values 0–7). W1 and W2 have no levels
with Big ? Blocks, so they are not in the table and use the World_Num fallback.

### Bonus Room Enemy Data (PRG006)

The 8 per-world bonus room enemy/object data segments are stored **inside** the main
enemy data region (PRG006, file offsets 0x0BFD8–0x0E00D). The enemy pointer table at
0x3492B contains CPU addresses in the $C9xx range (PRG006, CPU $C000–$DFFF), which map
to file offsets in the 0x0C9xx range.

**Per-world bonus room enemy data offsets:**

| World | CPU Addr | File Offset | Notes |
|-------|----------|-------------|-------|
| W1 | $C976 | 0x0C986 | |
| W2 | $C978 | 0x0C988 | |
| W3 | $C97D | 0x0C98D | |
| W4 | $C988 | 0x0C998 | |
| W5 | $C990 | 0x0C9A0 | |
| W6 | $C998 | 0x0C9A8 | |
| W7 | $C9A3 | 0x0C9B3 | |
| W8 | $C9AB | 0x0C9BB | |

Each bonus room's enemy data contains Big ? Block IDs (0x94–0x9A) that determine the
powerup the player receives. The visual block ID placed in the level is cosmetic only —
the actual powerup comes from this bonus room data.

**Critical**: The entire range 0x0C986–0x0C9C2 must be excluded from Big ? Block
randomization. If the randomizer scans the enemy data range for Big ? Block IDs to
shuffle, it will find and corrupt these bonus room entries, scrambling which powerup
each world's bonus room gives.

---

## Sources

- [Data Crystal ROM Map](https://datacrystal.tcrf.net/wiki/Super_Mario_Bros._3/ROM_map)
- [Data Crystal RAM Map](https://datacrystal.tcrf.net/wiki/Super_Mario_Bros._3/RAM_map)
- [Data Crystal Notes](https://datacrystal.tcrf.net/wiki/Super_Mario_Bros._3/Notes)
- [Southbird SMB3 Disassembly](https://sonicepoch.com/sm3mix/disassembly.html)
- [captainsouthbird/smb3 GitHub](https://github.com/captainsouthbird/smb3)
- [esc0rtd3w hacking_notes.txt](https://github.com/esc0rtd3w/nes-rom-tools/blob/master/super-mario-bros-3/docs/hacking_notes.txt)
