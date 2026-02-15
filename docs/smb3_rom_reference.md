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
| 0x28F3F–0x2A005 | ~4.3 KB | Desert |
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

**Tileset 1 (Plains) extra-byte dispatches:**

| Dispatch | Handler | Extra Byte Meaning |
|----------|---------|-------------------|
| 11, 12 | `LoadLevel_GroundRun` | Ground fill width |
| 35-42 | `LoadLevel_TopDecoBlocks` | Rectangle width |

**Other tilesets** have additional extra-byte routines (e.g., `LoadLevel_LavaRun`,
`LoadLevel_DecoGround`, `LoadLevel_DecoCeiling`). Each tileset's variable-size dispatch
table must be checked individually to identify which dispatches consume extra bytes.

The level simulator at `tools/level_sim.py` tracks extra-byte dispatches per tileset.

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
| 0x75 | OBJ_BOSSATTACK | Boss attack projectile |
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

### Title Screen

| File Offset | Description |
|------------|-------------|
| 0x30ABA–0x30AC1 | Title screen "3" flashing color sequence |
| 0x32AC2+ | Title screen background fade sequences |
| 0x32AFE | Title screen background final color |

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

| File Offset | Size | Description |
|------------|------|-------------|
| 0x185BA–0x19101 | ~2.9 KB | World map tile grids (raw tile data, all worlds) |

World maps are stored as raw tile grids (unlike levels which use generators).

### World Map Functionality (PRG010: 0x14010–0x1600F)

Key tables in PRG010 (indexed by World_Num 0–7):

| Label | Description |
|-------|-------------|
| `World_BGM_Arrival` | 9-byte table: music track per world (8 worlds + warp zone) |
| `FortressFXBase_ByWorld` | 8-byte table: fortress effect indices per world |
| `World_Map_Max_PanR` | 8-byte table: max rightward scroll per world (`10,20,30,30,00,30,20,00`) |
| `Map_EnterSpecialTiles` | Tile types that trigger level entry (see bug note below) |

**`Map_EnterSpecialTiles` list:** TOADHOUSE, SPADEBONUS, PIPE, ALTTOADHOUSE, CASTLEBOTTOM,
SPIRAL, ALTSPIRAL, PATHANDNUB, DANCINGFLOWER, HANDTRAP, BOWSERCASTLELL

**Known bug:** The tile entry check loop iterates up to index $1A instead of $0A,
causing subsequent palette data bytes to be incorrectly treated as enterable tile types.

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

### Airship Travel Data

| Label | Description |
|-------|-------------|
| `Map_Airship_Travel_BaseIdx` | Per-world base index (W1=0, W2=3, W3=6, ...) |
| `MAT_Y_W[1-8][A-C]` | Y destinations: 3 sets x 6 values per world |
| `MAT_X_W[1-8][A-C]` | X destinations: packed (lo=screen, hi=X pos) |

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

**ByRowType byte encoding:** upper nibble = row/position type, lower nibble = tileset ID.

**Entry type identification by ObjSets pointer value:**
- `obj >= 0xC000 && obj < 0xD000`: regular action level (shuffleable)
- `obj >= 0xD000`: fortress level
- `obj == 0x0700`: Toad House
- `obj == 0x0001` with `lay == 0x0000`: bonus game / N-Spade
- `obj < 0x1000` (other small values): hand traps, pipe junctions, special

**Level loading flow:** Player map position → match against ByRowType + ByScrCol →
extract tileset from lower nibble → load ObjSets pointer into `Level_ObjPtr_AddrL/H` →
load LevelLayouts pointer into `Level_LayPtr_AddrL/H` → bank-switch via
`PAGE_A000_ByTileset[Level_Tileset]` → execute level generators.

### World Map Starting Positions

| Label | Description |
|-------|-------------|
| `Map_Y_Starts` | Per-world initial Y coordinate |
| Fixed X = 0x20 | Same X start for all worlds |

### World Progression

World advancement is sequential via `INC World_Num` at file offset **0x3D0A1** (PRG030, CPU $9091).

Original bytes: `EE 27 07 4C A0 84` (INC $0727; JMP $84A0)

The code runs after the king's room cinematic (wand return) when a world boss is defeated. There is no "next world" lookup table in the original ROM — progression is always +1.

**Free space for patches:** PRG030 has unused space at **0x3DF20–0x3DF4F** (CPU $9F10–$9F3F), 48 bytes of $FF.

World BGM table (PRG030): file offset **0x3C424**, 9 bytes (worlds 1-8 + warp whistle): `01 02 03 04 05 06 07 08 0B`

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
| 0x309D5 | Debug mode toggle (0xCC=enable, 0x35=disable) |
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

## Sources

- [Data Crystal ROM Map](https://datacrystal.tcrf.net/wiki/Super_Mario_Bros._3/ROM_map)
- [Data Crystal RAM Map](https://datacrystal.tcrf.net/wiki/Super_Mario_Bros._3/RAM_map)
- [Data Crystal Notes](https://datacrystal.tcrf.net/wiki/Super_Mario_Bros._3/Notes)
- [Southbird SMB3 Disassembly](https://sonicepoch.com/sm3mix/disassembly.html)
- [captainsouthbird/smb3 GitHub](https://github.com/captainsouthbird/smb3)
- [esc0rtd3w hacking_notes.txt](https://github.com/esc0rtd3w/nes-rom-tools/blob/master/super-mario-bros-3/docs/hacking_notes.txt)
