with open('roms/Super Mario Bros. 3 (USA) (Rev 1).nes', 'rb') as f:
    rom = f.read()

PAGE_A000 = [11, 15, 21, 16, 17, 19, 18, 18, 18, 20, 23, 19, 17, 19, 13, 26, 26, 26, 9]

# Look at levels that DON'T have autoscroll but DO have vertical scrolling
# to understand what header byte 6 values enable free vertical scroll
WORLDS = [
    (1, 0x19438, 21), (2, 0x194BA, 47), (3, 0x195D8, 52), (4, 0x19714, 34),
    (5, 0x197E4, 42), (6, 0x198E4, 57), (7, 0x19A3E, 46), (8, 0x19B56, 41),
]

# First, find all obj_ptrs that have autoscroll in their segment
enemy_start = 0x0BFD8
enemy_end = 0x0E00D
autoscroll_cpus = set()
offset = enemy_start
while offset < enemy_end:
    while offset < enemy_end and rom[offset] == 0xFF:
        offset += 1
    if offset >= enemy_end:
        break
    seg_start = offset
    seg_cpu = 0xC000 + (seg_start - 0x0C010)
    offset += 1
    has_autoscroll = False
    while offset < enemy_end and rom[offset] != 0xFF:
        if rom[offset] == 0xD3:
            has_autoscroll = True
        offset += 3
    if has_autoscroll:
        autoscroll_cpus.add(seg_cpu)

# Now show ALL levels with their byte6, noting which have autoscroll
print("All levels - header byte 6 analysis:")
print("%-25s %-6s %-8s %-4s %-4s %-10s" % ("level", "byte6", "bin", "vs", "sd", "autoscroll"))

for wnum, rt_off, count in WORLDS:
    sc_off = rt_off + count
    obj_off = sc_off + count
    lay_off = obj_off + count * 2
    for i in range(count):
        o = (rom[obj_off + i*2 + 1] << 8) | rom[obj_off + i*2]
        l = (rom[lay_off + i*2 + 1] << 8) | rom[lay_off + i*2]
        brt = rom[rt_off + i]
        ts = brt & 0x0F

        if o < 0xC000 or o >= 0xD000 or l == 0:
            continue  # skip non-levels

        if ts >= len(PAGE_A000) or l < 0xA000:
            continue

        bank = PAGE_A000[ts]
        lay_file = bank * 0x2000 + 0x10 + (l - 0xA000)
        if lay_file + 9 >= len(rom):
            continue

        byte6 = rom[lay_file + 6]
        vs = (byte6 >> 5) & 0x03
        sd = (byte6 >> 4) & 0x01
        has_as = "YES" if o in autoscroll_cpus else "no"

        name = "W%d-idx%d (ts=%d)" % (wnum, i, ts)
        print("%-25s 0x%02X   %s  %d    %d    %s" % (
            name, byte6, format(byte6, '08b'), vs, sd, has_as))
