#!/usr/bin/env python3
"""POC: swap start tile ↔ airship tile per world, optionally +practice IPS.

Default output: SMB3R_POC.nes (with practice IPS applied, for emulator testing)

Use `--no-practice` to skip the practice IPS, producing SMB3R_POC_swaponly.nes.
That swap-only ROM is intended as input to the Rust randomizer for the
integration smoke test (`autoscroll::disable_autoscroll` substitutes for
practice IPS's airship pointer redirects).
"""
import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VANILLA = ROOT / "Super Mario Bros. 3 (USA) (Rev 1).nes"
PRACTICE_IPS = ROOT / "smb3practice_SE.ips"
AUTOSCROLL_JSON = Path("/tmp/autoscroll_patches.json")
OUT_DEFAULT = ROOT / "SMB3R_POC.nes"
OUT_SWAPONLY = ROOT / "SMB3R_POC_swaponly.nes"

# --- World map metadata (from rom_map.json / rom_data.rs) ---

# (file_offset of tile grid, columns)
MAP_TILE_GRIDS = [
    (0x185BA, 16),  # W1
    (0x1864B, 32),  # W2
    (0x1876C, 48),  # W3
    (0x1891D, 32),  # W4
    (0x18A3E, 32),  # W5
    (0x18B5F, 48),  # W6
    (0x18D10, 32),  # W7
    (0x18E31, 64),  # W8
]

# (rowtype_offset, entry_count)
WORLDS = [
    (0x19438, 21),  # W1
    (0x194BA, 47),  # W2
    (0x195D8, 52),  # W3
    (0x19714, 34),  # W4
    (0x197E4, 42),  # W5
    (0x198E4, 57),  # W6
    (0x19A3E, 46),  # W7
    (0x19B56, 41),  # W8
]

# Airship entry indices per world (W8 has no airship)
AIRSHIP_ENTRY = [17, 36, 49, 6, 35, 53, 43, None]

# Vanilla start position per world (row, col)
VANILLA_START = [
    (2, 2),   # W1
    (8, 2),   # W2
    (8, 2),   # W3
    (2, 2),   # W4
    (6, 2),   # W5
    (4, 2),   # W6
    (1, 2),   # W7
    (3, 2),   # W8 (unchanged)
]

# Vanilla airship position per world (row, col) — verified against rom_map.json
VANILLA_AIRSHIP = [
    (6, 12),  # W1 — screen 0
    (4, 18),  # W2 — screen 1
    (6, 41),  # W3 — screen 2
    (4, 8),   # W4 — screen 0
    (8, 18),  # W5 — screen 1
    (4, 44),  # W6 — screen 2
    (7, 24),  # W7 — screen 1
    None,     # W8
]

# --- Free-space allocations (PRG031, always-mapped @ CPU $E000-$FFFF) ---
# Practice IPS already claims PRG030 free space, so we route through PRG031
# (mapped at all times by MMC3 on this ROM).
FS_X_HELPER          = 0x3E250  # 10 bytes — CPU $E240  (X low → $797A,X / $7982,X)
FS_X_TABLE           = 0x3E25A  # 8 bytes  — CPU $E24A
FS_XHI_HELPER        = 0x3E262  # 19 bytes — CPU $E252  (XHi/scroll seeds)
FS_XHI_TABLE         = 0x3E275  # 8 bytes  — CPU $E265  (Mario's screen index → $7978)
FS_SCRL_L_TABLE      = 0x3E27D  # 8 bytes  — CPU $E26D  (camera scroll low → $0722)
FS_SCRL_H_TABLE      = 0x3E285  # 8 bytes  — CPU $E275  (camera scroll high → $0724)
# End = 0x3E28D; PRG031 free run extends to 0x3E2D0.

def cpu_addr(file_off):
    # PRG031 file base = 0x3E010, mapped to CPU $E000.
    return 0xE000 + (file_off - 0x3E010)

# --- Patch sites in Map_Init (PRG011 @ CPU $A237, file 0x16247) ---
MAP_Y_STARTS_OFF = 0x3C39A      # 8-byte Map_Y_Starts table (vanilla)
INIT_X_LOW_SITE  = 0x16257      # `A9 20 9D 7A 79 9D 82 79` (8 bytes)
INIT_XHI_SITE    = 0x16269      # `9D 78 79` (3 bytes) — STA Map_Entered_XHi,X
# `9D 24 07` (3 bytes) — STA Map_Prev_XHi,X. This feeds Horz_Scroll_Hi (ZP $12)
# on world entry via PRG030_8634. Vanilla zeroes it (start always on screen 0);
# non-zero-screen starts need this to match Mario's XHi. We NOP this site out
# because the XHi-helper at INIT_XHI_SITE already writes $0724,X correctly.
# Leave $0722 (Map_Prev_XOff) at vanilla 0 — that's Horz_Scroll low and must
# stay 0 for the camera to align to the screen Mario starts on.
INIT_X0724_SITE = 0x1627E       # `9D 24 07` (3 bytes)

# Master InitIndex pointer table (PRG012 @ $A000-$BFFF, file base 0x18010).
INIT_INDEX_MASTER = 0x193DA

# Map_Object slot 1 is the airship per world (per southbird disasm:
# "NOTE: Assumes Index 1 is the Airship!"). Updating this is what makes the
# airship sprite spawn at the new map coordinates — the airship is entered
# via sprite collision, not tile-walk.
AIRSHIP_OBJ_SLOT     = 1
MAP_OBJ_YS_MASTER    = 0x16020
MAP_OBJ_XHIS_MASTER  = 0x16030
MAP_OBJ_XLOS_MASTER  = 0x16040


def map_tile_offset(world_idx, row, col):
    base, ncols = MAP_TILE_GRIDS[world_idx]
    assert col < ncols, f"W{world_idx+1} col {col} out of range (cols={ncols})"
    screen = col // 16
    col_in_screen = col % 16
    return base + screen * 144 + row * 16 + col_in_screen


def apply_autoscroll(buf):
    patches = json.load(open(AUTOSCROLL_JSON))
    for off, bs in patches:
        for i, b in enumerate(bs):
            buf[off + i] = b
    print(f"  autoscroll: applied {len(patches)} patches")


def apply_start_airship_swap(buf):
    y_tbl    = bytearray([0] * 8)
    x_tbl    = bytearray([0] * 8)
    xhi_tbl  = bytearray([0] * 8)
    scrl_l   = bytearray([0] * 8)
    scrl_h   = bytearray([0] * 8)

    for w in range(8):
        ai = AIRSHIP_ENTRY[w]
        if ai is None:
            # W8: keep vanilla start
            r, c = VANILLA_START[w]
            y_tbl[w] = (r * 0x10) + 0x20
            x_tbl[w] = (c % 16) * 0x10
            xhi_tbl[w] = c // 16
            # Page-aligned: load Mario's screen into the (horizontally-mirrored)
            # nametable. $0722 stays 0 (camera at left of loaded content);
            # $0724 = screen index drives Scroll_Update_Ranges to load the
            # right 16 columns.
            scrl_l[w] = 0
            scrl_h[w] = c // 16
            continue

        r0, c0 = VANILLA_START[w]
        r1, c1 = VANILLA_AIRSHIP[w]

        # Sanity: read airship's grid_pos from ROM and confirm
        rowtype_off, n_entries = WORLDS[w]
        rt_byte = buf[rowtype_off + ai]
        sc_byte = buf[rowtype_off + n_entries + ai]
        rom_row = (rt_byte >> 4) - 2
        rom_col = (sc_byte >> 4) * 16 + (sc_byte & 0x0F)
        assert (rom_row, rom_col) == (r1, c1), (
            f"W{w+1} airship ROM pos ({rom_row},{rom_col}) != expected ({r1},{c1})"
        )

        # Sanity: tile bytes match
        start_off = map_tile_offset(w, r0, c0)
        air_off   = map_tile_offset(w, r1, c1)
        assert buf[start_off] == 0xE5, (
            f"W{w+1} expected 0xE5 at start ({r0},{c0}), got 0x{buf[start_off]:02X}"
        )
        assert buf[air_off]   == 0xC9, (
            f"W{w+1} expected 0xC9 at airship ({r1},{c1}), got 0x{buf[air_off]:02X}"
        )

        # 1. Swap tile bytes (base + the tile DIRECTLY ABOVE each, since the
        #    airship sprite is 2 tiles tall; otherwise its top half stays at
        #    the original airship position as a stray graphic).
        buf[start_off], buf[air_off] = buf[air_off], buf[start_off]
        if r0 > 0 and r1 > 0:
            start_above_off = map_tile_offset(w, r0 - 1, c0)
            air_above_off   = map_tile_offset(w, r1 - 1, c1)
            buf[start_above_off], buf[air_above_off] = (
                buf[air_above_off], buf[start_above_off]
            )

        # 2. Update airship entry's rowtype/scrcol to point at OLD start position
        preserved = rt_byte & 0x0F
        new_rt = (((r0 + 2) & 0x0F) << 4) | preserved
        buf[rowtype_off + ai] = new_rt
        new_sc = ((c0 // 16) & 0x0F) << 4 | (c0 % 16)
        buf[rowtype_off + n_entries + ai] = new_sc

        # 2a. ALSO relocate the vanilla "Start" entry. Every world has a
        #     pointer-table entry sitting on its start tile — its obj/lay
        #     bytes are dummies (the engine treats start tiles specially
        #     based on the tile byte, not the entry). After our tile-byte
        #     swap, the start tile (0xE5) lives at the OLD airship position,
        #     and the airship tile (0xC9) lives at the OLD start position.
        #     If we leave the Start entry at the old start coords, two
        #     entries (Start + airship) share that grid position; the game's
        #     sorted-scan finds the Start entry first and uses ITS garbage
        #     obj_ptr — so pressing A on the airship tile loads junk.
        #     Relocate the Start entry to the new start position so each
        #     position has exactly one entry.
        start_entry_idx = find_start_entry_idx(buf, w, r0, c0, exclude=ai)
        if start_entry_idx is not None:
            se_rt = buf[rowtype_off + start_entry_idx]
            preserved_se = se_rt & 0x0F
            buf[rowtype_off + start_entry_idx] = (
                ((r1 + 2) & 0x0F) << 4
            ) | preserved_se
            buf[rowtype_off + n_entries + start_entry_idx] = (
                ((c1 // 16) & 0x0F) << 4
            ) | (c1 % 16)

        # 2b. Move the airship SPRITE (Map_Object slot 1). The airship is
        #     entered by sprite collision, not tile-walk, so its world-map
        #     sprite spawn must move with the tile or stepping onto the new
        #     0xC9 does nothing.
        move_map_obj_slot1(buf, w, r0, c0)

        # 3. Record per-world spawn + camera values.
        # Mario's logical position: row+col in his actual screen.
        y_tbl[w]   = (r1 * 0x10) + 0x20
        x_tbl[w]   = (c1 % 16) * 0x10
        xhi_tbl[w] = c1 // 16

        # Camera: page-aligned. World map uses HORIZONTAL mirroring, so only
        # 16 cols of world data are loaded into the visible nametable at a
        # time. Scroll_Update_Ranges reads Map_Prev_XHi ($0724) to decide
        # WHICH 16 cols to load — setting it to Mario's screen index loads
        # cols (screen*16)..(screen*16+15). $0722 stays 0 so camera shows
        # the left edge of the loaded content (Mario appears at viewport
        # col = c1 % 16, same visual layout as vanilla col-2 starts).
        scrl_l[w] = 0
        scrl_h[w] = c1 // 16

        print(f"  W{w+1}: start ({r0},{c0}) ↔ airship ({r1},{c1})  "
              f"Mario[Y=0x{y_tbl[w]:02X} X=0x{x_tbl[w]:02X} XHi=0x{xhi_tbl[w]:02X}]  "
              f"cam[$0722=0x{scrl_l[w]:02X} $0724=0x{scrl_h[w]:02X} -> "
              f"loads cols {(c1//16)*16}-{(c1//16)*16+15}]")

    # 4. Write tables
    buf[MAP_Y_STARTS_OFF:MAP_Y_STARTS_OFF + 8] = y_tbl
    buf[FS_X_TABLE:FS_X_TABLE + 8] = x_tbl
    buf[FS_XHI_TABLE:FS_XHI_TABLE + 8] = xhi_tbl
    buf[FS_SCRL_L_TABLE:FS_SCRL_L_TABLE + 8] = scrl_l
    buf[FS_SCRL_H_TABLE:FS_SCRL_H_TABLE + 8] = scrl_h

    # 5. X-low helper at $9FB6
    x_tbl_cpu = cpu_addr(FS_X_TABLE)
    x_helper = bytes([
        0xB9, x_tbl_cpu & 0xFF, x_tbl_cpu >> 8,  # LDA Map_X_Starts,Y
        0x9D, 0x7A, 0x79,                        # STA $797A,X
        0x9D, 0x82, 0x79,                        # STA $7982,X
        0x60,                                    # RTS
    ])
    buf[FS_X_HELPER:FS_X_HELPER + 10] = x_helper

    # 6. XHi-and-scroll helper. Writes Mario's logical screen ($7978,X) plus
    #    the camera viewport-left scroll ($0722,X low / $0724,X high). The
    #    camera scroll is centered on Mario (clamped at world edges), not
    #    Mario's screen index — that's the key correction over the previous
    #    iteration. JSR'd from the FINAL site in Map_Init's per-player loop
    #    so its writes override any earlier inline zero-stores at the same
    #    targets.
    xhi_tbl_cpu  = cpu_addr(FS_XHI_TABLE)
    scrll_tbl_cpu = cpu_addr(FS_SCRL_L_TABLE)
    scrlh_tbl_cpu = cpu_addr(FS_SCRL_H_TABLE)
    xhi_helper = bytes([
        0xB9, xhi_tbl_cpu & 0xFF, xhi_tbl_cpu >> 8,    # LDA Map_XHi_Starts,Y
        0x9D, 0x78, 0x79,                              # STA $7978,X (Mario XHi)
        0xB9, scrll_tbl_cpu & 0xFF, scrll_tbl_cpu >> 8, # LDA Map_ScrL_Starts,Y
        0x9D, 0x22, 0x07,                              # STA $0722,X (Horz_Scroll)
        0xB9, scrlh_tbl_cpu & 0xFF, scrlh_tbl_cpu >> 8, # LDA Map_ScrH_Starts,Y
        0x9D, 0x24, 0x07,                              # STA $0724,X (Horz_Scroll_Hi)
        0x60,                                          # RTS
    ])
    assert len(xhi_helper) == 19, len(xhi_helper)
    buf[FS_XHI_HELPER:FS_XHI_HELPER + 19] = xhi_helper

    # 6b. Map_Player_SkidBack one-frame gate for the left-edge auto-pan.
    # Setting $073E,X to nonzero makes the auto-pan check at the top of
    # Map_DoPlayer_Edge_Scroll early-exit. Vanilla auto-pan would otherwise
    # fire on world entry for cross-screen starts (Mario sprite X = 32 on
    # screen 1+ triggers the LEFT pan, which scrolls the camera the wrong
    # way). Multiple engine paths clear $073E,X to 0 (Mario state
    # transitions, death respawn, etc.) so this should auto-clear within
    # a few frames and leave normal in-game pan navigation working.
    #
    # The byte is set inside the XHi-helper below so it runs per player
    # at the end of Map_Init's loop body.

    # 7. Patch Map_Init inline sites
    x_helper_cpu   = cpu_addr(FS_X_HELPER)
    xhi_helper_cpu = cpu_addr(FS_XHI_HELPER)
    # Replace 8-byte X-low immediate-store block with JSR + 5 NOPs
    buf[INIT_X_LOW_SITE:INIT_X_LOW_SITE + 8] = bytes([
        0x20, x_helper_cpu & 0xFF, x_helper_cpu >> 8,
        0xEA, 0xEA, 0xEA, 0xEA, 0xEA,
    ])
    # 0x16269 (STA $7978,X) — restore to vanilla. The XHi helper at the
    # $0724 site below now handles all three writes correctly at the end of
    # the loop body.
    buf[INIT_XHI_SITE:INIT_XHI_SITE + 3] = bytes([0x9D, 0x78, 0x79])
    # 0x1627E (STA $0724,X, last byte writes before DEX) → JSR helper. Vanilla
    # stores 0 here; replacing with our JSR makes the helper's writes to
    # $7978/$0722/$0724 the LAST values per-player before the loop iterates.
    buf[INIT_X0724_SITE:INIT_X0724_SITE + 3] = bytes([
        0x20, xhi_helper_cpu & 0xFF, xhi_helper_cpu >> 8,
    ])
    # $0722 (Map_Prev_XOff) inline site at 0x1627B is LEFT ALONE — its STA
    # writes 0, which our helper overwrites a few cycles later.

    print(f"  Map_Y_Starts:  {' '.join(f'{b:02X}' for b in y_tbl)}")
    print(f"  Map_X_Starts:  {' '.join(f'{b:02X}' for b in x_tbl)}")
    print(f"  Map_XHi_Starts:{' '.join(f'{b:02X}' for b in xhi_tbl)}")

    # 8. Re-sort each affected world's pointer table by (screen, row_nib, col)
    #    and rebuild its InitIndex sub-table. Without this, the game's lookup
    #    won't find the airship entry at its new (low) row/col because entries
    #    stored after it (in sorted order) won't be scanned past the row break.
    for w in range(7):  # W1-W7 (W8 unchanged)
        resort_pointer_table(buf, w)
    print("  pointer tables re-sorted (W1-W7)")


def find_start_entry_idx(buf, world_idx, start_row, start_col, exclude=None):
    """Locate the pointer-table entry whose CURRENT grid position is the
    vanilla start position (row, col). Excludes a given index (e.g. the
    airship entry, in case it's already been mutated to land there)."""
    rt_off, n = WORLDS[world_idx]
    sc_off = rt_off + n
    target_rt_hi = (start_row + 2) & 0x0F
    target_screen = (start_col // 16) & 0x0F
    target_col_in_scr = start_col % 16
    for i in range(n):
        if i == exclude:
            continue
        rt = buf[rt_off + i]
        sc = buf[sc_off + i]
        if (rt >> 4) == target_rt_hi and (sc >> 4) == target_screen and (sc & 0x0F) == target_col_in_scr:
            return i
    return None


def move_map_obj_slot1(buf, world_idx, grid_row, grid_col):
    """Write slot-1 (airship) position into the per-world Map_Object tables.

    Format (matches `write_map_sprite_position` in rom_data.rs):
        Y    = (grid_row + 2) * 16
        XHi  = grid_col / 16
        XLo  = (grid_col % 16) * 16
    """
    def slot_offset(master_table):
        cpu_lo = buf[master_table + world_idx * 2]
        cpu_hi = buf[master_table + world_idx * 2 + 1]
        cpu = cpu_lo | (cpu_hi << 8)
        # PRG011 is bank 11 → file = 0x16010 + (cpu - 0xA000) + slot
        return 0x16010 + (cpu - 0xA000) + AIRSHIP_OBJ_SLOT

    y_off   = slot_offset(MAP_OBJ_YS_MASTER)
    xhi_off = slot_offset(MAP_OBJ_XHIS_MASTER)
    xlo_off = slot_offset(MAP_OBJ_XLOS_MASTER)

    buf[y_off]   = ((grid_row + 2) * 16) & 0xFF
    buf[xhi_off] = (grid_col // 16) & 0xFF
    buf[xlo_off] = ((grid_col % 16) * 16) & 0xFF


def resort_pointer_table(buf, world_idx):
    """Mirror of `pipe_helpers::resort_pointer_table` in Rust."""
    rt_off, n = WORLDS[world_idx]
    sc_off = rt_off + n
    obj_off = sc_off + n
    lay_off = obj_off + n * 2

    # InitIndex sub-table CPU pointer → file offset
    master = INIT_INDEX_MASTER + world_idx * 2
    init_ptr = buf[master] | (buf[master + 1] << 8)
    init_file = 0x18010 + (init_ptr - 0xA000)

    entries = []
    for i in range(n):
        rt = buf[rt_off + i]
        sc = buf[sc_off + i]
        entries.append({
            "rowtype": rt,
            "scrcol": sc,
            "obj_lo": buf[obj_off + i * 2],
            "obj_hi": buf[obj_off + i * 2 + 1],
            "lay_lo": buf[lay_off + i * 2],
            "lay_hi": buf[lay_off + i * 2 + 1],
            "screen": (sc >> 4) & 0x0F,
            "row_nib": (rt >> 4) & 0x0F,
            "col": sc & 0x0F,
        })
    entries.sort(key=lambda e: (e["screen"], e["row_nib"], e["col"]))

    for i, e in enumerate(entries):
        buf[rt_off + i] = e["rowtype"]
        buf[sc_off + i] = e["scrcol"]
        buf[obj_off + i * 2]     = e["obj_lo"]
        buf[obj_off + i * 2 + 1] = e["obj_hi"]
        buf[lay_off + i * 2]     = e["lay_lo"]
        buf[lay_off + i * 2 + 1] = e["lay_hi"]

    # Rebuild InitIndex (4 bytes per world, one per screen; unused = n).
    for s in range(4):
        offset = next((i for i, e in enumerate(entries) if e["screen"] == s), n)
        buf[init_file + s] = offset


def apply_ips(buf, ips_path):
    data = open(ips_path, "rb").read()
    assert data[:5] == b"PATCH"
    i = 5
    n_records = 0
    while True:
        chunk = data[i:i + 3]
        if chunk == b"EOF":
            break
        off = int.from_bytes(chunk, "big")
        sz = int.from_bytes(data[i + 3:i + 5], "big")
        if sz == 0:
            # RLE record: 16-bit length + 8-bit value
            rle_sz = int.from_bytes(data[i + 5:i + 7], "big")
            val = data[i + 7]
            # IPS offsets are ROM-file relative — for NES iNES, header
            # adds 16 bytes; this practice IPS is for vanilla iNES file.
            for j in range(rle_sz):
                buf[off + j] = val
            i += 8
        else:
            for j in range(sz):
                buf[off + j] = data[i + 5 + j]
            i += 5 + sz
        n_records += 1
    print(f"  IPS: {ips_path.name} ({n_records} records)")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--no-practice", action="store_true",
                    help="Skip practice IPS — produce swap-only ROM "
                         "(intended as input to Rust randomizer for "
                         "integration smoke test).")
    args = ap.parse_args()

    buf = bytearray(open(VANILLA, "rb").read())
    print(f"Loaded {VANILLA.name} ({len(buf)} bytes)")

    if args.no_practice:
        out = OUT_SWAPONLY
        print("Step 1: SKIPPED practice IPS (--no-practice)")
    else:
        out = OUT_DEFAULT
        # Practice patch must come FIRST: it claims PRG030 free-space at the
        # same offsets SMB3R uses, and rewrites the airship pointer redirects
        # at the same bytes autoscroll uses. Our swap then runs on top, so its
        # tile-byte writes and pointer-table position updates win.
        print("Step 1: practice IPS (base)")
        apply_ips(buf, PRACTICE_IPS)

    print("Step 2: start ↔ airship swap")
    apply_start_airship_swap(buf)

    open(out, "wb").write(buf)
    print(f"\nWrote {out} ({len(buf)} bytes)")


if __name__ == "__main__":
    main()
