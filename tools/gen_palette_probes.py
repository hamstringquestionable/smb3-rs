#!/usr/bin/env python3
"""Generate one test ROM per candidate palette table, with that table painted a
vivid canary color (NES 0x24 hot magenta). Run each in an emulator to identify
which level/area/sprite uses that table.

Each probe also has the patches/smb3practice_SE.ips open-movement subset (PRG010-011 only)
applied so Mario can walk freely over locks/forts/levels without entering them —
this lets us scan the entire overworld map for color changes.

Outputs to test_roms/palette_probe_<name>.nes
"""

import argparse
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ROM_PATH = ROOT / "roms/Super Mario Bros. 3 (USA) (Rev 1).nes"
PRACTICE_IPS = ROOT / "patches/smb3practice_SE.ips"
OPEN_MOVE_RANGE = (0x14010, 0x18010)  # PRG010-011 only; safe vs PRG012-013 palette tables
START_WORLD_OFFSET = 0x30CC3            # operand of LDA #$00 — sets starting world index 0..7
OUT_DIR = ROOT / "test_roms"
OUT_DIR.mkdir(exist_ok=True)

# When --full-practice is on, we apply EVERY practice IPS record except those that
# would overlap a painted palette table. Verified: vanilla practice IPS records in
# PRG13 land in the gaps between palette tables (0x34000-0x36BE4), not on them.
PRACTICE_IPS_DESC = (
    "Sebastian Mihai's smb3practice_SE: warp whistles in inventory, level select, "
    "infinite lives, etc. (~3 KB, 193 records)."
)

CANARY = 0x24  # NES bright pink-magenta — extremely rare in vanilla SMB3


def parse_ips(path: Path) -> list[tuple[int, bytes]]:
    data = path.read_bytes()
    if data[:5] != b"PATCH":
        raise SystemExit(f"bad IPS header in {path}")
    pos = 5
    records: list[tuple[int, bytes]] = []
    while pos < len(data):
        if data[pos : pos + 3] == b"EOF":
            break
        offset = (data[pos] << 16) | (data[pos + 1] << 8) | data[pos + 2]
        pos += 3
        size = (data[pos] << 8) | data[pos + 1]
        pos += 2
        if size == 0:
            n = (data[pos] << 8) | data[pos + 1]
            v = data[pos + 2]
            pos += 3
            records.append((offset, bytes([v]) * n))
        else:
            records.append((offset, data[pos : pos + size]))
            pos += size
    return records


def apply_ips_subset(rom: bytearray, records: list[tuple[int, bytes]], lo: int, hi: int) -> int:
    applied = 0
    for off, payload in records:
        if lo <= off < hi:
            rom[off : off + len(payload)] = payload
            applied += 1
    return applied

# Each candidate table is described by:
#   start, end           — file offsets (exclusive end)
#   kind                 — 'ppu_script' (skip 4-byte PPU sentinels)
#                       OR 'quartet' (every 4 bytes is [bgmirror, c1, c2, outline])
#                       OR 'raw' (paint every byte that isn't 0x00 or 0x0F)
#   description          — what this probe is testing
TABLES = [
    # PPU upload scripts: structure `00 3F dd LL <LL color bytes>` repeated
    ("33046_full_palette_uploads",   0x33046, 0x331A3, "ppu_script",
        "8 × 32-byte BG+sprite palette uploads"),
    ("33410_bg_only_uploads",        0x33410, 0x33497, "ppu_script",
        "4 × 16-byte BG-palette uploads"),
    ("334C4_sprite_only_uploads",    0x334C4, 0x33531, "ppu_script",
        "5 × 16-byte sprite-palette uploads"),

    # Static palette quartet tables — quartet painter assumes outline at byte 3, but
    # alignment varies across these tables. Use 'raw' painter (paint every byte that
    # isn't 0x00 or 0x0F) so we don't miss color bytes due to phase mismatch.
    ("36BE4_static_quartets_a",      0x36BE4, 0x36DA6, "raw",
        "Quartet table A in PRG013 — confirmed BG palette table"),
    ("36E20_quartet_table_b",        0x36E20, 0x36EBE, "raw",
        "Quartet table B in PRG013 (~40 four-byte palettes)"),
    ("36EE2_master_pool_a",          0x36EE2, 0x37000, "raw",
        "First slice of master pool — CONFIRMED contains sky palettes AND HUD"),
    # Sub-probes of 36EE2 to find the HUD/sky boundary. Each ~72 bytes.
    ("36EE2_sub1_first72",           0x36EE2, 0x36F2A, "raw",
        "Sub-probe: first ~72 B of 36EE2"),
    ("36EE2_sub2_second72",          0x36F2A, 0x36F72, "raw",
        "Sub-probe: second ~72 B of 36EE2"),
    ("36EE2_sub3_third72",           0x36F72, 0x36FBA, "raw",
        "Sub-probe: third ~72 B of 36EE2"),
    ("36EE2_sub4_fourth72",          0x36FBA, 0x37000, "raw",
        "Sub-probe: fourth ~70 B of 36EE2"),
    ("37000_master_pool_b",          0x37000, 0x37200, "raw",
        "Second slice of master quartet pool"),
    ("37200_master_pool_c",          0x37200, 0x37400, "raw",
        "Third slice of master quartet pool"),
    ("37400_master_pool_d",          0x37400, 0x37600, "raw",
        "Fourth slice of master quartet pool"),
    ("37600_master_pool_e",          0x37600, 0x37846, "raw",
        "Fifth slice of master quartet pool"),

    # Known palettes (sanity probes — we already know what these are)
    ("10539_character_palettes",     0x10539, 0x10555, "quartet",
        "Mario/Luigi power-up palettes (sanity probe)"),
    ("36DAA_lava_palette",           0x36DAA, 0x36DAE, "quartet",
        "Lava/Rotodisc palette (sanity probe)"),
    ("36DFE_bowser_palette",         0x36DFE, 0x36E02, "quartet",
        "Bowser/Donut palette (sanity probe)"),
]


def paint_ppu_script(buf: bytearray, start: int, end: int, canary: int) -> int:
    """Walk a PPU upload script. For each `00 3F dd LL` header found, leave the
    4 header bytes alone, then paint the next LL bytes (skipping 0x00 and 0x0F)
    with the canary color. Returns count of bytes painted."""
    painted = 0
    i = start
    # If the cluster starts before the first PPU sentinel, scan forward.
    while i < end - 3:
        if buf[i] == 0x00 and buf[i + 1] == 0x3F and buf[i + 3] != 0:
            # Looks like PPU header: 00 3F <addr_lo> <count>
            addr_lo = buf[i + 2]
            count = buf[i + 3]
            body_start = i + 4
            body_end = min(body_start + count, end)
            for j in range(body_start, body_end):
                if buf[j] not in (0x00, 0x0F):
                    buf[j] = canary
                    painted += 1
            i = body_end
        else:
            i += 1
    return painted


def paint_quartet(buf: bytearray, start: int, end: int, canary: int) -> int:
    """Treat every 4-byte run as a palette quartet. Paint bytes 1 and 2,
    skip byte 0 (bg mirror) and byte 3 (outline)."""
    painted = 0
    for i in range(start, end - 3, 4):
        for off in (1, 2):
            if buf[i + off] not in (0x00, 0x0F):
                buf[i + off] = canary
                painted += 1
    return painted


def paint_raw(buf: bytearray, start: int, end: int, canary: int) -> int:
    painted = 0
    for i in range(start, end):
        if buf[i] not in (0x00, 0x0F):
            buf[i] = canary
            painted += 1
    return painted


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--world", type=int, default=1, choices=range(1, 9),
        help="starting world (1-8). Default 1. Output filenames are suffixed _wN.",
    )
    ap.add_argument(
        "--only", action="append", metavar="NAME",
        help="generate only probes whose name contains this substring (repeatable)",
    )
    ap.add_argument(
        "--practice", choices=("open-move", "full"), default="full",
        help=(
            "what subset of patches/smb3practice_SE.ips to apply. "
            "'open-move' = PRG010-011 only (~85 B, walk freely over locks/forts). "
            "'full' (default) = everything except records that would overwrite a "
            "painted palette table (gives warp whistles, level select, infinite lives, etc.)."
        ),
    )
    args = ap.parse_args()

    if not ROM_PATH.exists():
        raise SystemExit(f"vanilla ROM not found: {ROM_PATH}")
    vanilla = bytes(ROM_PATH.read_bytes())

    practice_records: list[tuple[int, bytes]] = []
    if PRACTICE_IPS.exists():
        all_records = parse_ips(PRACTICE_IPS)
        if args.practice == "open-move":
            lo, hi = OPEN_MOVE_RANGE
            practice_records = [(o, p) for o, p in all_records if lo <= o < hi]
            print(
                f"Practice IPS: open-move subset only — {len(practice_records)} records "
                f"({lo:#x}-{hi:#x}, PRG010-011)"
            )
        else:
            # Full IPS, but skip any record that would overwrite a painted palette table.
            painted_ranges = [(t[1], t[2]) for t in TABLES]

            def overlaps_painted(off: int, length: int) -> bool:
                return any(off < end and off + length > start for start, end in painted_ranges)

            kept = [(o, p) for o, p in all_records if not overlaps_painted(o, len(p))]
            dropped = len(all_records) - len(kept)
            practice_records = kept
            print(
                f"Practice IPS: full ({PRACTICE_IPS_DESC}) — "
                f"{len(kept)} records applied, {dropped} dropped to protect painted palette tables"
            )
    else:
        print(f"WARN: {PRACTICE_IPS.name} not found — probes will not have practice patches applied")
    print(f"Painting candidate palette tables with NES color 0x{CANARY:02X} (hot magenta)")
    print(f"Starting world: W{args.world}  (writes {args.world - 1:#04x} to {START_WORLD_OFFSET:#06x})")
    print(f"Output dir: {OUT_DIR}")
    print()
    suffix = f"_w{args.world}"
    written = 0
    for name, start, end, kind, desc in TABLES:
        if args.only and not any(s in name for s in args.only):
            continue
        rom = bytearray(vanilla)
        if kind == "ppu_script":
            painted = paint_ppu_script(rom, start, end, CANARY)
        elif kind == "quartet":
            painted = paint_quartet(rom, start, end, CANARY)
        elif kind == "raw":
            painted = paint_raw(rom, start, end, CANARY)
        else:
            raise ValueError(kind)
        # Apply practice IPS records (full or open-move subset depending on --practice)
        for off, payload in practice_records:
            rom[off : off + len(payload)] = payload
        # Set starting world (file offset 0x30CC3, value = world index 0..7)
        rom[START_WORLD_OFFSET] = args.world - 1
        out = OUT_DIR / f"palette_probe_{name}{suffix}.nes"
        out.write_bytes(rom)
        written += 1
        print(f"  {name:<40s} {start:#08x}-{end:#08x} ({end-start:>4d} B span, {painted:>3d} painted) — {desc}")

    print()
    print(f"Wrote {written} probe ROMs to {OUT_DIR}/  (suffix {suffix})")
    print()
    print("How to use:")
    print("  1. Load each test_roms/palette_probe_*.nes in an emulator.")
    print("  2. Walk Mario across the overworld map — locks/forts won't block you.")
    print("  3. Enter levels you can reach via warp pipes; check world map AND in-level.")
    print("  4. Note which area/level/sprite turns hot pink — that's what the table drives.")
    print("  5. Cross-reference with docs/smb3_rom_reference.md palette tables section.")


if __name__ == "__main__":
    main()
