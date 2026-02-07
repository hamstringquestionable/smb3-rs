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

| Byte | Contents |
|------|----------|
| 0–1 | Transition scenery address (pointer) |
| 2–3 | Transition actor/enemy address (pointer) |
| 4 | Y-start properties + course end page |
| 5 | X-start properties, object/background palettes |
| 6 | Transition type, scroll mode, course type |
| 7 | Friction factor + CHR banks |
| 8 | Timer seed + music selection |

### Level Tile Format

Levels use "tile generators" (variable/fixed-size construction routines), not raw tile grids.
World maps are stored as raw tile grids instead.

---

## Enemy / Object Data

| File Offset | Size | Description |
|------------|------|-------------|
| 0x0BFD8–0x0E00D | ~8.2 KB | Enemy-to-level definitions (all levels) |

### Enemy IDs (Partial List)

**Ground enemies:**
| ID | Enemy |
|----|-------|
| 0x00 | Green Goomba (walks off ledges) |
| 0x01 | Red Goomba (turns at ledges) |
| 0x06 | Green Koopa Troopa |
| 0x07 | Red Koopa Troopa |
| 0x08 | Buzzy Beetle |
| 0x0A | Spiny |
| 0x11 | Bob-omb |
| 0x15 | Green Koopa Paratroopa (hops) |

**Flying enemies:**
| ID | Enemy |
|----|-------|
| 0x03 | Red Para-goomba (hops) |
| 0x16 | Red Koopa Paratroopa (flies up/down) |
| 0x17 | Green Koopa Paratroopa (flies left/right) |

**Water enemies:**
| ID | Enemy |
|----|-------|
| 0x19 | Blooper |
| 0x1A | Blooper with babies |
| 0x1B | Cheep Cheep (slow) |
| 0x1C | Cheep Cheep (fast) |
| 0x1E | Big Bertha |

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

### Block / Power-Up Offsets

| File Offset | Size | Description |
|------------|------|-------------|
| 0x02611–0x02618 | 8 bytes | Bumped block attribute data (item types from ? blocks) |
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
| `Map_EnterSpecialTiles` | Tile types that trigger level entry |

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

### World Map Starting Positions

| Label | Description |
|-------|-------------|
| `Map_Y_Starts` | Per-world initial Y coordinate |
| Fixed X = 0x20 | Same X start for all worlds |

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

## Sources

- [Data Crystal ROM Map](https://datacrystal.tcrf.net/wiki/Super_Mario_Bros._3/ROM_map)
- [Data Crystal RAM Map](https://datacrystal.tcrf.net/wiki/Super_Mario_Bros._3/RAM_map)
- [Data Crystal Notes](https://datacrystal.tcrf.net/wiki/Super_Mario_Bros._3/Notes)
- [Southbird SMB3 Disassembly](https://sonicepoch.com/sm3mix/disassembly.html)
- [captainsouthbird/smb3 GitHub](https://github.com/captainsouthbird/smb3)
- [esc0rtd3w hacking_notes.txt](https://github.com/esc0rtd3w/nes-rom-tools/blob/master/super-mario-bros-3/docs/hacking_notes.txt)
