#!/usr/bin/env python3
"""
palette_inspect.py — Reverse-engineer the "Super Mario Bros. 3 Recolored" IPS.

Loads the vanilla USA Rev 1 ROM, parses the Recolored IPS, applies it in memory,
and produces a structured report classifying every patched offset against known
ROM regions (level headers, character palettes, lava/Bowser palettes, CHR data,
and unknown regions that need further investigation).

Usage:
  python3 tools/palette_inspect.py                     # full report
  python3 tools/palette_inspect.py --clusters          # only cluster summary
  python3 tools/palette_inspect.py --headers           # only level header diffs
  python3 tools/palette_inspect.py --tables            # only candidate palette tables
  python3 tools/palette_inspect.py --chr               # only CHR diffs
  python3 tools/palette_inspect.py --json out.json     # write structured JSON
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
ROM_PATH = ROOT / "roms/Super Mario Bros. 3 (USA) (Rev 1).nes"
IPS_PATH = ROOT / "patches/Super Mario Bros. 3 Recolored v1.0.ips"
ROM_MAP_PATH = ROOT / "tools" / "rom_map.json"

INES_HEADER = 0x10
PRG_BANK_SIZE = 0x4000
CHR_START = INES_HEADER + 32 * PRG_BANK_SIZE  # 0x40010

# Known palette regions (file offsets) from docs/smb3_rom_reference.md
KNOWN_PALETTES = {
    "mario_normal":   (0x10539, 4),
    "luigi_normal":   (0x1053D, 4),
    "fire":           (0x10541, 4),  # 0x10545-0x10548 is between fire and frog (1up?)
    "frog":           (0x10549, 4),
    "tanooki":        (0x1054D, 4),
    "hammer":         (0x10551, 4),
    "lava_rotodisc":  (0x36DAA, 4),
    "bowser_donut":   (0x36DFE, 4),
    "title_pal3":     (0x31976, 16),  # documented as 4 palettes x 4 bytes
}


# ----------------------------------------------------------------------------
# IPS parsing
# ----------------------------------------------------------------------------

@dataclass
class IpsRecord:
    offset: int
    size: int
    payload: bytes  # for RLE this is rle_count copies of rle_byte
    is_rle: bool

    def end(self) -> int:
        return self.offset + self.size


def parse_ips(path: Path) -> list[IpsRecord]:
    data = path.read_bytes()
    if data[:5] != b"PATCH":
        sys.exit(f"ERROR: {path} is not an IPS file (header={data[:5]!r})")

    records: list[IpsRecord] = []
    i = 5
    while True:
        chunk = data[i : i + 3]
        if chunk == b"EOF":
            break
        offset = (chunk[0] << 16) | (chunk[1] << 8) | chunk[2]
        i += 3
        size = (data[i] << 8) | data[i + 1]
        i += 2
        if size == 0:
            rle_count = (data[i] << 8) | data[i + 1]
            rle_byte = data[i + 2]
            i += 3
            records.append(
                IpsRecord(
                    offset=offset,
                    size=rle_count,
                    payload=bytes([rle_byte]) * rle_count,
                    is_rle=True,
                )
            )
        else:
            payload = data[i : i + size]
            i += size
            records.append(
                IpsRecord(offset=offset, size=size, payload=payload, is_rle=False)
            )
    return records


def apply_records(rom: bytearray, records: list[IpsRecord]) -> None:
    for r in records:
        rom[r.offset : r.offset + r.size] = r.payload


# ----------------------------------------------------------------------------
# Region classification helpers
# ----------------------------------------------------------------------------

def file_offset_to_prg(off: int) -> tuple[str, int]:
    """Return ('PRGNN' or 'CHR', offset_within_bank)."""
    if off < INES_HEADER:
        return ("HEADER", off)
    if off < CHR_START:
        bank = (off - INES_HEADER) // PRG_BANK_SIZE
        within = (off - INES_HEADER) % PRG_BANK_SIZE
        # CPU $A000 base for level/tileset banks; we report the raw file offset.
        return (f"PRG{bank:03d}", within)
    return ("CHR", off - CHR_START)


def load_rom_map() -> dict[str, Any]:
    if not ROM_MAP_PATH.exists():
        sys.exit(
            f"ERROR: rom_map.json not found at {ROM_MAP_PATH}.\n"
            f"Generate it first: nix-shell -p python3 --run 'python3 tools/rom_map.py'"
        )
    return json.loads(ROM_MAP_PATH.read_text())


def build_level_header_index(rom_map: dict[str, Any]) -> dict[int, dict[str, Any]]:
    """Map every level-header file offset (and its 9 byte slots) to level metadata.

    Returns: {file_offset: {'level': lvl_dict, 'header_byte_index': 0..8}}
    """
    idx: dict[int, dict[str, Any]] = {}
    for region in rom_map.get("level_data_regions", []):
        for lvl in region.get("levels", []):
            base = lvl["header_offset"]
            for b in range(9):
                idx[base + b] = {
                    "level": lvl,
                    "region": region["region"],
                    "header_byte_index": b,
                }
    return idx


# ----------------------------------------------------------------------------
# Cluster grouping
# ----------------------------------------------------------------------------

@dataclass
class Cluster:
    start: int
    end: int  # exclusive
    records: list[IpsRecord]

    @property
    def size(self) -> int:
        return self.end - self.start

    @property
    def patched_bytes(self) -> int:
        return sum(r.size for r in self.records)


def cluster_records(records: list[IpsRecord], gap_threshold: int = 8) -> list[Cluster]:
    """Group records that are within `gap_threshold` bytes of each other."""
    if not records:
        return []
    sorted_recs = sorted(records, key=lambda r: r.offset)
    clusters: list[Cluster] = []
    current = [sorted_recs[0]]
    for r in sorted_recs[1:]:
        if r.offset - current[-1].end() <= gap_threshold:
            current.append(r)
        else:
            clusters.append(
                Cluster(
                    start=current[0].offset,
                    end=current[-1].end(),
                    records=current,
                )
            )
            current = [r]
    clusters.append(
        Cluster(
            start=current[0].offset,
            end=current[-1].end(),
            records=current,
        )
    )
    return clusters


# ----------------------------------------------------------------------------
# Classification
# ----------------------------------------------------------------------------

def classify_offset(
    off: int,
    header_idx: dict[int, dict[str, Any]],
    vanilla: bytes | None = None,
    recolored: bytes | None = None,
) -> str:
    """Return a short classification tag for a file offset.

    If vanilla/recolored are given, also detect the `JSR $FE99 -> JSR $FE92`
    redirect pattern: the low byte of a JSR target that was 0x99 in vanilla
    and is 0x92 in recolored, with the preceding byte being the JSR opcode
    0x20 and the following byte being 0xFE.
    """
    for name, (start, size) in KNOWN_PALETTES.items():
        if start <= off < start + size:
            return f"PALETTE:{name}+{off - start}"
    if off in header_idx:
        meta = header_idx[off]
        return f"LEVEL_HEADER[{meta['header_byte_index']}]"
    if off >= CHR_START:
        return "CHR"
    if vanilla is not None and recolored is not None and off >= 1:
        if (
            vanilla[off] == 0x99
            and recolored[off] == 0x92
            and vanilla[off - 1] == 0x20
            and off + 1 < len(vanilla)
            and vanilla[off + 1] == 0xFE
        ):
            return "JSR_PALLOAD_HOOK"  # redirect JSR $FE99 -> JSR $FE92
    return "UNKNOWN"


def looks_like_palette_table(data: bytes) -> tuple[bool, str]:
    """Heuristic: a palette table contains lots of bytes <= 0x3F and often
    PPU register sentinels like 003F, 003F00, 003F10."""
    if not data:
        return (False, "empty")
    sentinels = []
    for marker in (b"\x00\x3f\x00", b"\x00\x3f\x10", b"\x00\x3f\x11", b"\x00\x3f\x12"):
        if marker in data:
            sentinels.append(marker.hex())
    in_range = sum(1 for b in data if b <= 0x3F)
    pct = in_range / len(data)
    if sentinels:
        return (True, f"contains PPU $3F sentinels: {sentinels}; {pct:.0%} bytes ≤ 0x3F")
    if pct >= 0.8 and len(data) >= 8:
        return (True, f"dense palette-range bytes ({pct:.0%} ≤ 0x3F)")
    return (False, f"only {pct:.0%} bytes ≤ 0x3F")


# ----------------------------------------------------------------------------
# Reporting
# ----------------------------------------------------------------------------

def section(title: str) -> None:
    print()
    print("=" * 76)
    print(title)
    print("=" * 76)


def fmt_bytes(b: bytes, max_len: int = 64) -> str:
    h = b.hex()
    if len(h) <= max_len * 2:
        return h
    return h[: max_len * 2] + f"...({len(b)} bytes)"


def report_level_headers(records: list[IpsRecord], header_idx: dict[int, dict[str, Any]],
                         vanilla: bytes, recolored: bytes) -> list[dict[str, Any]]:
    """Show every patch that lands inside a level header byte. Decode header byte 5
    (palette indices) before/after."""
    rows: list[dict[str, Any]] = []
    for r in records:
        for k in range(r.size):
            off = r.offset + k
            if off in header_idx:
                meta = header_idx[off]
                lvl = meta["level"]
                bidx = meta["header_byte_index"]
                v = vanilla[off]
                n = recolored[off]
                row = {
                    "file_offset": off,
                    "level_offset": lvl["header_offset"],
                    "region": meta["region"],
                    "header_byte_index": bidx,
                    "vanilla": v,
                    "recolored": n,
                }
                if bidx == 5:
                    row["vanilla_decoded"] = {
                        "bg_palette": v & 0b111,
                        "obj_palette": (v >> 3) & 0b11,
                        "x_start": (v >> 5) & 0b11,
                    }
                    row["recolored_decoded"] = {
                        "bg_palette": n & 0b111,
                        "obj_palette": (n >> 3) & 0b11,
                        "x_start": (n >> 5) & 0b11,
                    }
                rows.append(row)
    return rows


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--clusters", action="store_true", help="cluster summary only")
    ap.add_argument("--headers", action="store_true", help="level header diffs only")
    ap.add_argument("--tables", action="store_true", help="candidate palette tables only")
    ap.add_argument("--chr", action="store_true", help="CHR diffs only")
    ap.add_argument("--unknown", action="store_true", help="show unclassified clusters")
    ap.add_argument("--hooks", action="store_true", help="JSR $FE99 -> JSR $FE92 redirects")
    ap.add_argument("--engine", action="store_true", help="dump the palette engine area ($FE92-$FFFF)")
    ap.add_argument("--json", metavar="OUT", help="write structured JSON report to OUT")
    args = ap.parse_args()

    show_all = not any([args.clusters, args.headers, args.tables, args.chr, args.unknown, args.hooks, args.engine])

    if not ROM_PATH.exists():
        sys.exit(f"ERROR: vanilla ROM not found: {ROM_PATH}")
    if not IPS_PATH.exists():
        sys.exit(f"ERROR: Recolored IPS not found: {IPS_PATH}")

    vanilla = bytearray(ROM_PATH.read_bytes())
    recolored = bytearray(vanilla)
    records = parse_ips(IPS_PATH)
    apply_records(recolored, records)

    rom_map = load_rom_map()
    header_idx = build_level_header_index(rom_map)

    # ------------------------------------------------------------------------
    # Top-level summary
    # ------------------------------------------------------------------------
    section("OVERVIEW")
    total_bytes = sum(r.size for r in records)
    print(f"vanilla ROM:       {ROM_PATH.name} ({len(vanilla)} bytes)")
    print(f"Recolored IPS:     {IPS_PATH.name}")
    print(f"records:           {len(records)} ({sum(1 for r in records if r.is_rle)} RLE)")
    print(f"total bytes patched: {total_bytes}")

    # ------------------------------------------------------------------------
    # Per-byte classification histogram
    # ------------------------------------------------------------------------
    counts: dict[str, int] = {}
    for r in records:
        for k in range(r.size):
            tag = classify_offset(r.offset + k, header_idx, bytes(vanilla), bytes(recolored))
            # collapse PALETTE:* and LEVEL_HEADER[N] subkeys
            key = tag.split("+")[0].split("[")[0]
            counts[key] = counts.get(key, 0) + 1
    if show_all or args.clusters:
        section("BYTE CLASSIFICATION")
        for k, v in sorted(counts.items(), key=lambda kv: -kv[1]):
            print(f"  {k:<24s} {v:>5d} bytes")

    # ------------------------------------------------------------------------
    # Cluster summary
    # ------------------------------------------------------------------------
    clusters = cluster_records(records, gap_threshold=4)
    if show_all or args.clusters:
        section(f"CLUSTERS  (gap ≤ 4 bytes;  {len(clusters)} clusters)")
        print(f"{'#':>3} {'start':>8} {'end':>8} {'span':>6} {'patched':>7}  classification")
        for i, c in enumerate(clusters):
            tags = []
            for r in c.records:
                for k in range(r.size):
                    t = classify_offset(r.offset + k, header_idx, bytes(vanilla), bytes(recolored))
                    base = t.split("+")[0].split("[")[0]
                    if base not in tags:
                        tags.append(base)
            print(
                f"{i:>3} {c.start:#08x} {c.end:#08x} {c.size:>6d} {c.patched_bytes:>7d}  {', '.join(tags)}"
            )

    # ------------------------------------------------------------------------
    # JSR $FE99 -> JSR $FE92 hook redirects
    # ------------------------------------------------------------------------
    hook_sites: list[tuple[int, str]] = []
    for r in records:
        for k in range(r.size):
            off = r.offset + k
            if classify_offset(off, header_idx, bytes(vanilla), bytes(recolored)) == "JSR_PALLOAD_HOOK":
                bank, within = file_offset_to_prg(off)
                hook_sites.append((off, bank))
    if show_all or args.hooks:
        section(f"PALETTE-LOAD HOOK CALLSITES  ({len(hook_sites)} JSR $FE99 -> JSR $FE92)")
        # Group by bank
        by_bank: dict[str, list[int]] = {}
        for off, bank in hook_sites:
            by_bank.setdefault(bank, []).append(off)
        for bank in sorted(by_bank):
            offs = by_bank[bank]
            print(f"  {bank}: {len(offs)} sites  ({offs[0]:#08x}..{offs[-1]:#08x})")

    # ------------------------------------------------------------------------
    # Palette engine landing zone ($FE92-$FFFF in fixed bank)
    # ------------------------------------------------------------------------
    if show_all or args.engine:
        section("PALETTE ENGINE AREA  (fixed bank $E000-$FFFF)")
        FIXED_BASE = 0x10 + 0x3E000  # $E000 in CPU
        # Find all changed bytes in fixed bank
        engine_changes = []
        for r in records:
            for k in range(r.size):
                off = r.offset + k
                if FIXED_BASE <= off < FIXED_BASE + 0x2000:
                    engine_changes.append(off)
        if engine_changes:
            lo = min(engine_changes)
            hi = max(engine_changes) + 1
            cpu_lo = 0xE000 + (lo - FIXED_BASE)
            cpu_hi = 0xE000 + (hi - FIXED_BASE)
            print(f"  changes span: file {lo:#08x}-{hi:#08x}  CPU ${cpu_lo:04X}-${cpu_hi:04X}  ({hi-lo} bytes)")
            print(f"  total changed bytes in fixed bank: {len(engine_changes)}")
            # Dump 16-byte rows of vanilla vs recolored over the changed span
            print(f"  diff dump (vanilla / recolored, 16 bytes/row):")
            row_lo = lo & ~0xF
            row_hi = (hi + 0xF) & ~0xF
            for r in range(row_lo, row_hi, 16):
                cpu = 0xE000 + (r - FIXED_BASE)
                v_row = bytes(vanilla[r:r + 16]).hex()
                n_row = bytes(recolored[r:r + 16]).hex()
                if v_row != n_row:
                    # Mark changed bytes with brackets
                    print(f"    ${cpu:04X}: V  {v_row}")
                    print(f"           R  {n_row}")

    # ------------------------------------------------------------------------
    # Level header diffs
    # ------------------------------------------------------------------------
    header_rows = report_level_headers(records, header_idx, bytes(vanilla), bytes(recolored))
    if show_all or args.headers:
        section(f"LEVEL HEADER PATCHES  ({len(header_rows)} bytes)")
        # Aggregate by header_byte_index
        by_idx: dict[int, list[dict[str, Any]]] = {}
        for row in header_rows:
            by_idx.setdefault(row["header_byte_index"], []).append(row)
        for bidx in sorted(by_idx):
            rows = by_idx[bidx]
            print(f"\n  Header byte [{bidx}]: {len(rows)} levels patched")
            if bidx == 5:
                # Show distribution of new bg/obj palette indices
                bg_dist: dict[int, int] = {}
                obj_dist: dict[int, int] = {}
                for r in rows:
                    bg_dist[r["recolored_decoded"]["bg_palette"]] = bg_dist.get(r["recolored_decoded"]["bg_palette"], 0) + 1
                    obj_dist[r["recolored_decoded"]["obj_palette"]] = obj_dist.get(r["recolored_decoded"]["obj_palette"], 0) + 1
                print(f"    new bg_palette distribution:  {dict(sorted(bg_dist.items()))}")
                print(f"    new obj_palette distribution: {dict(sorted(obj_dist.items()))}")
                # Show vanilla vs new for a sample
                print("    sample (first 20):")
                for r in rows[:20]:
                    v = r["vanilla_decoded"]
                    n = r["recolored_decoded"]
                    print(
                        f"      {r['region']:<35s}  off={r['file_offset']:#08x}  "
                        f"bg {v['bg_palette']}→{n['bg_palette']}  obj {v['obj_palette']}→{n['obj_palette']}"
                    )
            else:
                for r in rows[:10]:
                    print(
                        f"    {r['region']:<35s}  off={r['file_offset']:#08x}  "
                        f"vanilla={r['vanilla']:#04x} recolored={r['recolored']:#04x}"
                    )

    # ------------------------------------------------------------------------
    # Candidate palette tables (large clusters with PPU $3F sentinels)
    # ------------------------------------------------------------------------
    if show_all or args.tables:
        section("CANDIDATE PALETTE TABLES  (clusters with PPU $3F structure or dense palette bytes)")
        for i, c in enumerate(clusters):
            data = bytes(recolored[c.start : c.end])
            looks, why = looks_like_palette_table(data)
            if not looks:
                continue
            van = bytes(vanilla[c.start : c.end])
            bank, within = file_offset_to_prg(c.start)
            print(
                f"\n  Cluster #{i}: {c.start:#08x}-{c.end:#08x} "
                f"({c.size} bytes, {c.patched_bytes} patched)  [{bank}]"
            )
            print(f"    heuristic: {why}")
            # Find $3F00, $3F10, etc. sentinel positions
            sentinels = []
            for s_off in range(len(data) - 1):
                if data[s_off] == 0x00 and data[s_off + 1] == 0x3F:
                    sentinels.append((c.start + s_off, data[s_off:s_off + 8].hex()))
            if sentinels:
                print(f"    PPU $3F sentinels at: ", end="")
                print(", ".join(f"{o:#08x} ({h})" for o, h in sentinels[:6]))
            # Print first 64 bytes of vanilla vs recolored
            print(f"    vanilla:   {fmt_bytes(van)}")
            print(f"    recolored: {fmt_bytes(data)}")

    # ------------------------------------------------------------------------
    # CHR diffs
    # ------------------------------------------------------------------------
    if show_all or args.chr:
        section("CHR DIFFS  (tile pixel edits)")
        chr_recs = [r for r in records if r.offset >= CHR_START]
        if not chr_recs:
            print("  (none)")
        for r in chr_recs:
            chr_off = r.offset - CHR_START
            tile_idx = chr_off // 16  # 16 bytes per 8x8 tile
            tile_byte = chr_off % 16
            page = chr_off // 0x1000  # 4KB CHR pages (NES PPU-side)
            within_page_tile = (chr_off % 0x1000) // 16
            print(
                f"  {r.offset:#08x}  CHR+{chr_off:#06x}  page={page} tile={within_page_tile:#04x} byte={tile_byte}  "
                f"size={r.size}  {fmt_bytes(r.payload, 32)}"
            )

    # ------------------------------------------------------------------------
    # Unknown clusters (gold for further investigation)
    # ------------------------------------------------------------------------
    if show_all or args.unknown:
        section("UNCLASSIFIED CLUSTERS  (no known structure here yet)")
        for i, c in enumerate(clusters):
            tags = set()
            for r in c.records:
                for k in range(r.size):
                    t = classify_offset(r.offset + k, header_idx, bytes(vanilla), bytes(recolored))
                    base = t.split("+")[0].split("[")[0]
                    tags.add(base)
            if tags - {"UNKNOWN"} and "UNKNOWN" not in tags:
                continue
            if not tags or tags == {"CHR"}:
                continue
            van = bytes(vanilla[c.start : c.end])
            data = bytes(recolored[c.start : c.end])
            bank, within = file_offset_to_prg(c.start)
            print(
                f"\n  Cluster #{i}: {c.start:#08x}-{c.end:#08x} "
                f"({c.size} bytes, {c.patched_bytes} patched)  [{bank}]"
            )
            print(f"    vanilla:   {fmt_bytes(van)}")
            print(f"    recolored: {fmt_bytes(data)}")

    # ------------------------------------------------------------------------
    # JSON output
    # ------------------------------------------------------------------------
    if args.json:
        out = {
            "rom": str(ROM_PATH.name),
            "ips": str(IPS_PATH.name),
            "record_count": len(records),
            "total_patched_bytes": total_bytes,
            "byte_classification": counts,
            "clusters": [
                {
                    "index": i,
                    "start": c.start,
                    "end": c.end,
                    "span": c.size,
                    "patched_bytes": c.patched_bytes,
                    "record_count": len(c.records),
                    "tags": sorted(
                        {
                            classify_offset(r.offset + k, header_idx, bytes(vanilla), bytes(recolored)).split("+")[0].split("[")[0]
                            for r in c.records
                            for k in range(r.size)
                        }
                    ),
                    "vanilla": bytes(vanilla[c.start : c.end]).hex(),
                    "recolored": bytes(recolored[c.start : c.end]).hex(),
                }
                for i, c in enumerate(clusters)
            ],
            "level_header_patches": header_rows,
        }
        Path(args.json).write_text(json.dumps(out, indent=2))
        print(f"\n  wrote JSON report -> {args.json}")


if __name__ == "__main__":
    main()
