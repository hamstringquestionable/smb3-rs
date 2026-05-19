"""Verify the powerups randomizer walker stays aligned in each level data region.

Re-implement powerups.rs walker exactly, then for every byte it modifies in the
randomized ROM, confirm it's a legitimate powerup-shape byte (group 1 fixed +
QBLOCK/BRICK or group 2 fixed + NOTE/WOOD in randomize_note_wood regions).

Any byte modified that DOESN'T match these criteria = walker desync (bug).
"""

import json

VANILLA = "/home/michio/git/SMB3R/Super Mario Bros. 3 (USA) (Rev 1).nes"
RANDO = "/home/michio/git/SMB3R/smb3-rs_2889718939757641.nes"

QBLOCK = {0x00, 0x01, 0x02}
BRICK = {0x06, 0x07, 0x08, 0x0B}
NOTE = {0x01, 0x02, 0x03}
WOOD = {0x04, 0x05, 0x06}

GEN_GROUP_MASK = 0xE0
GEN_GROUP_POWERBLOCK = 0x20
GEN_GROUP_EXTENDED = 0x40
LEVEL_HEADER = 9


def walk(buf, region):
    """Yield (file_offset, b0, b1, b2, is_fixed, modified_legitimately)."""
    start = region["start"]
    end = region["end"]
    dispatches = set(region["extra_byte_dispatches"])
    rnw = region["randomize_note_wood"]
    i = LEVEL_HEADER
    while i + 2 < (end - start):
        b0 = buf[start + i]
        if b0 == 0xFF:
            i += 1 + LEVEL_HEADER
            continue
        b1 = buf[start + i + 1]
        b2 = buf[start + i + 2]
        is_fixed = (b2 & 0xF0) == 0
        legit = False
        if is_fixed:
            shape = b2 & 0x0F
            grp = b0 & GEN_GROUP_MASK
            if grp == GEN_GROUP_POWERBLOCK and (shape in QBLOCK or shape in BRICK):
                legit = True
            elif grp == GEN_GROUP_EXTENDED and rnw and (shape in NOTE or shape in WOOD):
                legit = True
        cmd_size = 3
        if not is_fixed:
            grp = b0 >> 5
            disp = grp * 15 + (b2 >> 4) - 1
            if disp in dispatches:
                cmd_size = 4
        yield (start + i, b0, b1, b2, is_fixed, legit, cmd_size)
        i += cmd_size


def main():
    with open(VANILLA, "rb") as f:
        v = f.read()
    with open(RANDO, "rb") as f:
        r = f.read()
    with open("tools/rom_map.json") as f:
        m = json.load(f)

    # Need randomize_note_wood per region — pull from rust source via constant lookup
    # For brevity, hardcode per-tileset based on rom_data.rs:
    rnw_map = {
        "Underground (TS14)": True,
        "Plains (TS1)": True,
        "Hilly (TS3)": True,
        "Ice/Sky (TS4/12)": True,
        "Pipe/Water (TS7)": True,
        "Cloudy/Giant/Plant (TS5/11/13)": True,
        "Desert (TS9)": False,
        "Dungeon (TS2)": False,
        "Ship (TS10)": True,
    }

    # First — for each region, find ALL modified bytes (vanilla != rando) and check
    # whether they were modified by powerups walker (i.e. byte is at file_offset = i+2
    # of a "legitimate" command), or by something else.
    for region in m["level_data_regions"]:
        # Inject randomize_note_wood
        region["randomize_note_wood"] = rnw_map.get(region["region"], True)

        # Build map of (file_off, expected_byte_position_within_cmd)
        cmd_at = {}  # file_offset of byte+2 -> (b0, b2, is_fixed, legit)
        for fo, b0, b1, b2, fx, lg, cz in walk(v, region):
            cmd_at[fo + 2] = (b0, b2, fx, lg, cz)

        # Find all byte diffs in this region
        diffs = [(i, v[i], r[i]) for i in range(region["start"], region["end"]) if v[i] != r[i]]
        if not diffs:
            continue

        legit_diffs = 0
        suspect_diffs = []
        for fo, ov, nv in diffs:
            if fo in cmd_at:
                b0, b2, fx, lg, cz = cmd_at[fo]
                if lg:
                    legit_diffs += 1
                else:
                    # Walker visited this position but didn't intend to modify it.
                    # That means the modification came from some OTHER source.
                    suspect_diffs.append((fo, ov, nv, b0, b2, fx, "walker-saw-but-not-legit"))
            else:
                # Walker never landed on this byte position. Modification source unknown.
                suspect_diffs.append((fo, ov, nv, None, None, None, "walker-did-not-visit"))

        print(f"=== {region['region']} ===")
        print(f"  total byte diffs: {len(diffs)}")
        print(f"  legitimate powerup randomizations: {legit_diffs}")
        print(f"  suspect (not legit / unvisited): {len(suspect_diffs)}")
        for fo, ov, nv, b0, b2, fx, reason in suspect_diffs[:10]:
            ctx_v = v[fo-3:fo+4].hex(' ')
            ctx_r = r[fo-3:fo+4].hex(' ')
            extra = f"cmd b0=0x{b0:02X} b2=0x{b2:02X} fixed={fx}" if b0 is not None else ""
            print(f"  0x{fo:06X} 0x{ov:02X}->0x{nv:02X} {reason} {extra}")
            print(f"    v=[{ctx_v}]  r=[{ctx_r}]")
        print()


if __name__ == "__main__":
    main()
