"""For each unique HB encounter layout offset, walk the layout span and report:
- Which bytes changed (vanilla vs rando)
- Which module the write log attributes each change to
"""

import re
import json

VANILLA = "/home/michio/git/SMB3R/roms/Super Mario Bros. 3 (USA) (Rev 1).nes"
RANDO = "/tmp/smb3rs_repro.nes"


def walk_layout_end(buf, dispatches, start, max_bytes=512):
    """Walk SMB3 layout commands from `start` to first 0xFF terminator."""
    i = start
    cap = start + max_bytes
    while i < cap:
        if buf[i] == 0xFF:
            return i + 1
        b0 = buf[i]
        b2 = buf[i + 2] if i + 2 < len(buf) else 0
        is_fixed = (b2 & 0xF0) == 0
        size = 3
        if not is_fixed:
            grp = b0 >> 5
            disp = grp * 15 + (b2 >> 4) - 1
            if disp in dispatches:
                size = 4
        i += size
    return cap


def parse_writelog(path):
    """Return (explicit_writes: dict offset->tag, bulk_ranges: list of (start, end_exclusive, tag))."""
    explicit = {}
    bulk = []
    current_tag = None
    with open(path) as f:
        for line in f:
            if line.startswith("["):
                m = re.match(r"\[([^\]]+)\]", line)
                if m:
                    current_tag = m.group(1)
                continue
            line = line.rstrip()
            if not line:
                continue
            m_byte = re.match(r"\s+0x([0-9A-F]+)\s+([0-9A-F ]+)\s+->\s+([0-9A-F ]+)$", line)
            if m_byte:
                explicit[int(m_byte.group(1), 16)] = current_tag
                continue
            m_range = re.match(r"\s+0x([0-9A-F]+)\.\.0x([0-9A-F]+)\s+\((\d+) bytes\)", line)
            if m_range:
                bulk.append((int(m_range.group(1), 16), int(m_range.group(2), 16) + 1, current_tag))
    return explicit, bulk


def attribute(off, explicit, bulk):
    if off in explicit:
        return explicit[off]
    for s, e, t in bulk:
        if s <= off < e:
            return t
    return "<none>"


def main():
    with open(VANILLA, "rb") as f:
        v = f.read()
    with open(RANDO, "rb") as f:
        r = f.read()
    with open("tools/rom_map.json") as f:
        m = json.load(f)

    explicit, bulk = parse_writelog("/tmp/smb3rs_writelog.txt")

    # All unique HB lay_off + tileset
    hbs = []
    for w in m["worlds"]:
        for e in w["entries"]:
            if e.get("type") == "hammer_bro":
                hbs.append((e["layout_file_offset"], e["tileset"], w["world"]))
    seen = set()
    unique = []
    for x in hbs:
        if (x[0], x[1]) not in seen:
            seen.add((x[0], x[1]))
            unique.append(x)
    unique.sort(key=lambda x: x[0])

    regions = m["level_data_regions"]
    def region_for(off):
        for reg in regions:
            if reg["start"] <= off < reg["end"]:
                return reg
        return None

    for lay_off, ts, w in unique:
        reg = region_for(lay_off)
        disps = set(reg["extra_byte_dispatches"]) if reg else set()
        end = walk_layout_end(v, disps, lay_off)
        diffs = [(i, v[i], r[i], attribute(i, explicit, bulk))
                 for i in range(lay_off, end) if v[i] != r[i]]
        if not diffs:
            continue
        print(f"lay_off=0x{lay_off:06X} ts={ts} (used by W{w}) span=[{lay_off:#x}..{end:#x}]")
        for off, ov, nv, tag in diffs:
            ctx_v = v[off-3:off+4].hex(' ')
            print(f"  0x{off:06X} 0x{ov:02X}->0x{nv:02X}  by [{tag}]  v=[{ctx_v}]")
        print()


if __name__ == "__main__":
    main()
