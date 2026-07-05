Generate a BizHawk / EmuHawk RAM Watch (`.wch`) file for a set of memory addresses.

## Usage
`/wch <addr:label> [addr:label ...] [--system NES] [-o path.wch]`

You can also just describe what you want to watch in plain English (e.g. "watch Mario's
world-map position and the camera scroll") and pick addresses from the reference below.

Examples:
- `/wch 0079:World_Map_X 0077:World_Map_XHi 0075:World_Map_Y`
- `/wch 7982:Map_Previous_X 7980:Map_Previous_XHi 797E:Map_Previous_Y -o roms/debug.wch`
- `/wch --preset overworld` — emit the full overworld-position watch set below

## Instructions

1. **Resolve addresses.** Each argument is `HEXADDR:label` (address is a CPU/System-Bus
   address, hex, no `0x`). If the user described things in words, map them via the
   reference tables below. If they ask for a preset, emit that whole set.

2. **Write the file with a script** (real tab characters are mandatory — do not hand-type
   tabs, they get mangled). Default domain is **System Bus** (literal CPU addresses, no
   offset math — works for zero-page, RAM `$0000-$07FF`, and cartridge WRAM `$6000-$7FFF`
   alike). Default output `roms/debug.wch` unless `-o` given.

   ```sh
   nix-shell -p python3 --run 'python3 - <<"PY"
   rows = [
       ("0079", "World_Map_X"),
       ("0077", "World_Map_XHi"),
       # (addr, label) ...
   ]
   system = "NES"          # SystemID header
   lines = ["SystemID " + system]
   for addr, note in rows:
       # addr \t size(b=byte,w=word,d=dword) \t type(h=hex,u,s,b) \t bigendian(0/1) \t domain \t notes
       lines.append("\t".join([addr, "b", "h", "0", "System Bus", note]))
   open("roms/debug.wch", "w").write("\n".join(lines) + "\n")
   print("\n".join(lines))
   PY'
   ```

3. **Report** the path and remind the user to load it via **Tools → RAM Watch → File →
   Open** (or **Append** to add to an existing watch list). All entries are 1-byte hex on
   System Bus; for 2-byte values pass `w` instead of `b` in the row.

## `.wch` file format (verified against BizHawk `WatchList`/`Watch` source)
- Line 1: `SystemID <SYS>` (e.g. `SystemID NES`).
- One line per watch, **tab-separated, 6 fields**:
  `ADDRESS \t SIZE \t TYPE \t BIGENDIAN \t DOMAIN \t NOTES`
  - `ADDRESS` — hex, no `0x` (leading zeros optional; loader parses hex regardless).
  - `SIZE` — `b` byte, `w` word, `d` dword.
  - `TYPE` — `h` hex, `u` unsigned, `s` signed, `b` binary.
  - `BIGENDIAN` — `0` little, `1` big.
  - `DOMAIN` — `System Bus` (recommended: literal CPU addresses). Alternatives on NES:
    `RAM` (2 KB internal, address as-is `$0000-$07FF`), `WRAM` (cart SRAM, address =
    CPU − `$6000`), `PRG ROM`, `CHR VROM`. If System Bus rejects an address, switch that
    row's domain and adjust the address accordingly.
  - `NOTES` — the label (must contain no tab).

## SMB3 overworld reference (World_Num = `$0727`, `01` = World 2)
Mario/Luigi are interleaved: the address shown is Mario; Luigi is `+1`.

| Addr | Name | Meaning |
|------|------|---------|
| `0075` | World_Map_Y | live map row (ZP) |
| `0077` | World_Map_XHi | live map page (ZP) |
| `0079` | World_Map_X | live map column low-pixel (ZP) |
| `7976` | Map_Entered_Y | tile you entered a level from (row) |
| `7978` | Map_Entered_XHi | …page |
| `797A` | Map_Entered_X | …column |
| `797E` | Map_Previous_Y | "safe" tile you skid back to on death (row) |
| `7980` | Map_Previous_XHi | …page |
| `7982` | Map_Previous_X | …column |
| `7986` | Map_Prev_XOff2 | secondary camera-scroll backup (low) — afar-skid target |
| `7988` | Map_Prev_XHi2 | secondary camera-scroll backup (page) — afar-skid target |
| `0722` | Map_Prev_XOff | primary camera-scroll backup (low) |
| `0724` | Map_Prev_XHi | primary camera-scroll backup (page) |
| `00FD` | Horz_Scroll | live camera scroll low |
| `0012` | Horz_Scroll_Hi | live camera scroll page |

## Preset: `overworld`
Emit World_Num + the live position (`0075/0077/0079`), Map_Entered, Map_Previous, the two
secondary backups (`7986/7988`), the primary backups (`0722/0724`), and live scroll
(`00FD/0012`) — the full set used to debug the SAS death-respawn / scroll-desync.
