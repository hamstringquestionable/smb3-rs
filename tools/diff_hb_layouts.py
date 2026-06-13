"""Diff HB encounter LAYOUT data between vanilla and a randomized ROM.

Walk each unique HB layout block (lay_ptr → layout_file_offset). SMB3 layout
streams are sequences of 3-byte tile-generator commands terminated by 0xFF.
Some commands are 4-byte (extra-byte dispatches per tileset).

For this diff we don't need to fully parse — we walk byte-by-byte to a robust
end and report all differences.
"""

import json

VANILLA = "/home/michio/git/SMB3R/roms/Super Mario Bros. 3 (USA) (Rev 1).nes"
RANDO = "/home/michio/git/SMB3R/smb3-rs_2889718939757641.nes"


def walk_layout(buf, dispatches, start, max_bytes=512):
    """Walk SMB3 layout stream starting at `start`, respecting 4-byte
    extra-byte dispatches in `dispatches`. Returns the (start, end) span
    including the 0xFF terminator.
    """
    i = start
    cap = start + max_bytes
    while i < cap:
        b0 = buf[i]
        if b0 == 0xFF:
            return start, i + 1
        # Compute dispatch index from b0 and b1: group * 15 + (b1 >> 4) - 1
        if i + 1 >= len(buf):
            return start, i
        b1 = buf[i + 1]
        group = b0 >> 5
        idx = group * 15 + (b1 >> 4) - 1
        size = 4 if idx in dispatches else 3
        i += size
    return start, cap


def main():
    with open(VANILLA, "rb") as f:
        v = f.read()
    with open(RANDO, "rb") as f:
        r = f.read()

    with open("tools/rom_map.json") as f:
        m = json.load(f)

    # Build (start..end, dispatches, tilesets) lookup
    regions = m["level_data_regions"]

    def region_for(off):
        for reg in regions:
            if reg["start"] <= off < reg["end"]:
                return reg
        return None

    # Collect (lay_ptr, layout_file_offset, tileset) per HB entry
    hb_entries = []
    for w in m["worlds"]:
        for e in w["entries"]:
            if e.get("type") == "hammer_bro":
                hb_entries.append({
                    "world": w["world"],
                    "lay_ptr": e["lay_ptr"],
                    "lay_off": e["layout_file_offset"],
                    "tileset": e["tileset"],
                })

    # De-dup by (lay_off, tileset)
    seen = set()
    unique = []
    for e in hb_entries:
        key = (e["lay_off"], e["tileset"])
        if key not in seen:
            seen.add(key)
            unique.append(e)

    print(f"{len(hb_entries)} HB entries, {len(unique)} unique (lay_off, tileset) pairs")
    print()

    for e in sorted(unique, key=lambda x: x["lay_off"]):
        off = e["lay_off"]
        ts = e["tileset"]
        reg = region_for(off)
        dispatches = set(reg["extra_byte_dispatches"]) if reg else set()
        s, end_v = walk_layout(v, dispatches, off)
        s, end_r = walk_layout(r, dispatches, off)
        end = max(end_v, end_r)
        v_data = v[off:end]
        r_data = r[off:end]
        diff = v_data != r_data
        print(f"lay_off=0x{off:06X} ts={ts} region={reg['region'] if reg else '?'}")
        print(f"  vanilla ({end_v - off}b): {v_data[:end_v-off].hex()}")
        print(f"  rando   ({end_r - off}b): {r_data[:end_r-off].hex()}")
        if diff:
            for i in range(min(len(v_data), len(r_data))):
                if v_data[i] != r_data[i]:
                    print(f"  >> diff @ +{i}: 0x{v_data[i]:02X} -> 0x{r_data[i]:02X}")
        print()


if __name__ == "__main__":
    main()
