"""For each byte that differs between vanilla and randomized ROM in the
level data regions, identify which module write actually changed it.

The powerups module writes the entire region back as a single write_range,
so naively the write log attributes every byte to powerups. We need to look
inside each WriteRecord and check whether the byte ACTUALLY changed at that
position in that write.

Strategy: parse the write log file, build per-byte attribution by walking
records chronologically and tracking value at each offset.
"""

import re
import json
import sys

VANILLA = "/home/michio/git/SMB3R/Super Mario Bros. 3 (USA) (Rev 1).nes"
RANDO = "/tmp/smb3rs_repro.nes"


def main():
    with open(VANILLA, "rb") as f:
        v = f.read()
    with open(RANDO, "rb") as f:
        r = f.read()

    with open("tools/rom_map.json") as f:
        m = json.load(f)

    # The text write log doesn't include the actual bytes for ranges > 4 bytes.
    # We need a structured dump. Add a JSON write log dumper to main.rs?
    # For now, take a different approach: replay the modules.
    #
    # Even simpler: for each byte diff, check if it falls inside any of the
    # specific write ranges from beta_stages/autoscroll/hand_rooms (which are
    # tracked at byte granularity). If so, attribute to that. Otherwise,
    # attribute to powerups (the only module that writes layout regions in
    # bulk).

    # Parse write log for explicit byte-level writes (records with len <= 4)
    # by tag. Also record bulk write ranges per tag.
    explicit_writes = {}  # offset -> tag
    bulk_ranges = []      # (start, end, tag)

    with open("/tmp/smb3rs_writelog.txt") as f:
        current_tag = None
        for line in f:
            if line.startswith("["):
                m_match = re.match(r"\[([^\]]+)\]", line)
                if m_match:
                    current_tag = m_match.group(1)
                continue
            line = line.rstrip()
            if not line:
                continue
            # explicit byte write: "  0xOFFSET  HEX -> HEX"
            m_byte = re.match(r"\s+0x([0-9A-F]+)\s+([0-9A-F ]+)\s+->\s+([0-9A-F ]+)$", line)
            if m_byte:
                off = int(m_byte.group(1), 16)
                explicit_writes[off] = current_tag
                continue
            # bulk range: "  0xSTART..0xEND  (N bytes)"
            m_range = re.match(r"\s+0x([0-9A-F]+)\.\.0x([0-9A-F]+)\s+\((\d+) bytes\)", line)
            if m_range:
                start = int(m_range.group(1), 16)
                end = int(m_range.group(2), 16)
                bulk_ranges.append((start, end + 1, current_tag))
                continue

    # For each diff in level data regions, find responsible tag
    print(f"Layout region byte diff attribution\n{'='*60}")
    for region in m["level_data_regions"]:
        s, e = region["start"], region["end"]
        diffs = [(i, v[i], r[i]) for i in range(s, e) if v[i] != r[i]]
        if not diffs:
            continue
        # Attribute each diff
        attribs = {}  # tag -> count
        examples = {}  # tag -> first example (off, ov, nv)
        for off, ov, nv in diffs:
            # Explicit byte writes win over bulk
            tag = explicit_writes.get(off)
            if tag is None:
                # Find bulk range
                for bs, be, bt in bulk_ranges:
                    if bs <= off < be:
                        tag = bt
                        break
            if tag is None:
                tag = "<unknown>"
            attribs[tag] = attribs.get(tag, 0) + 1
            if tag not in examples:
                examples[tag] = (off, ov, nv)

        print(f"\n{region['region']}: {len(diffs)} byte diffs")
        for tag in sorted(attribs):
            off, ov, nv = examples[tag]
            ctx_v = v[off-3:off+4].hex(' ')
            ctx_r = r[off-3:off+4].hex(' ')
            print(f"  [{tag}] {attribs[tag]} bytes  e.g. 0x{off:06X} 0x{ov:02X}->0x{nv:02X}")
            print(f"    v=[{ctx_v}]  r=[{ctx_r}]")


if __name__ == "__main__":
    main()
