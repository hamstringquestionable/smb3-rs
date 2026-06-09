#!/usr/bin/env python3
"""Find ROM offset literals that duplicate a named constant in rom_data.rs.

The codebase keeps all ROM offsets as named constants in `rom_data.rs` (the
"single source of truth" rule in CLAUDE.md). This scan flags the two ways that
rule gets broken:

  PRIMARY (bypass): a hex literal that has a `const NAME: usize = 0x...;`
    definition in rom_data.rs but is *also* hardcoded somewhere else. Whoever
    wrote the second site should have imported the constant. These are almost
    always worth fixing.

  SECONDARY (--all): a 5-digit file offset (>= 0x10000) with no named constant
    at all, hardcoded in two or more files. Candidate for a new constant.

Comments are stripped before scanning, so address-arithmetic in comments
(e.g. "$A000 + (0x355B1 - 0x34010)") does not trip the scan. Bank-window base
addresses (0x2000, 0xa000, 0xc000, ...) are ignored — they legitimately recur.

Usage:
    nix-shell -p python3 --run "python3 tools/offset_dups.py"           # primary only
    nix-shell -p python3 --run "python3 tools/offset_dups.py --all"     # + secondary
    nix-shell -p python3 --run "python3 tools/offset_dups.py --exit-code"  # CI advisory: exit 1 on any bypass
"""
import os
import re
import sys

SRC_DIR = "src"
ROM_DATA = os.path.join("src", "randomize", "rom_data.rs")

# CPU/MMC3 bank-window base addresses. These are not file offsets; they recur in
# pointer arithmetic across many modules by design, so duplication is expected.
DENYLIST = {0x0000, 0x2000, 0x4000, 0x6000, 0x8000, 0xA000, 0xC000, 0xE000}

# File pairs whose shared offsets are a deliberate split, not a smell.
IGNORE_PAIRS = [
    {"palettes.rs", "palette_variants.rs"},
]

HEX = re.compile(r"0x[0-9A-Fa-f]{4,5}\b")
CONST_DEF = re.compile(r"\bconst\s+([A-Z0-9_]+)\s*:\s*usize\s*=\s*0x([0-9A-Fa-f]+)")


def code_lines(text):
    """Yield (lineno, code) with // line and /* */ block comments blanked out.

    Line numbers are preserved so reported locations match the source. String
    literals are not specially handled (a hex offset inside a string is rare
    enough not to matter here)."""
    in_block = False
    for lineno, line in enumerate(text.split("\n"), 1):
        out = []
        j = 0
        while j < len(line):
            if in_block:
                end = line.find("*/", j)
                if end == -1:
                    j = len(line)
                else:
                    in_block = False
                    j = end + 2
            else:
                slash = line.find("//", j)
                block = line.find("/*", j)
                if slash != -1 and (block == -1 or slash < block):
                    out.append(line[j:slash])
                    j = len(line)
                elif block != -1:
                    out.append(line[j:block])
                    in_block = True
                    j = block + 2
                else:
                    out.append(line[j:])
                    j = len(line)
        yield lineno, "".join(out)


def rs_files(root):
    for dirpath, _, names in os.walk(root):
        for name in names:
            if name.endswith(".rs"):
                yield os.path.join(dirpath, name)


def main():
    show_all = "--all" in sys.argv
    exit_code = "--exit-code" in sys.argv

    # value -> {const_name, line} for offsets defined in rom_data.rs
    consts = {}
    # value -> list of (path, lineno) occurrences across all source (comment-free)
    occ = {}

    for path in rs_files(SRC_DIR):
        with open(path, encoding="utf-8") as f:
            text = f.read()
        is_rom_data = os.path.normpath(path) == os.path.normpath(ROM_DATA)
        for lineno, code in code_lines(text):
            if is_rom_data:
                m = CONST_DEF.search(code)
                if m:
                    val = int(m.group(2), 16)
                    consts.setdefault(val, (m.group(1), lineno))
            for hm in HEX.finditer(code):
                val = int(hm.group(0), 16)
                if val in DENYLIST:
                    continue
                occ.setdefault(val, []).append((path, lineno))

    def basename(p):
        return os.path.basename(p)

    def ignored_pair(paths):
        names = {basename(p) for p in paths}
        return any(names <= pair for pair in IGNORE_PAIRS)

    # PRIMARY: named const in rom_data.rs, hardcoded somewhere outside rom_data.rs
    bypasses = []
    for val, (name, def_line) in sorted(consts.items()):
        if val in DENYLIST:
            continue
        sites = occ.get(val, [])
        outside = [(p, ln) for (p, ln) in sites
                   if os.path.normpath(p) != os.path.normpath(ROM_DATA)]
        if outside:
            bypasses.append((val, name, def_line, outside))

    print("ROM offset duplication scan")
    print("=" * 60)
    print()
    if bypasses:
        print("Named constants in rom_data.rs that are ALSO hardcoded elsewhere:")
        print("(import the constant instead of repeating the literal)")
        print()
        for val, name, def_line, outside in bypasses:
            print(f"  0x{val:X}  {name}")
            print(f"    {ROM_DATA}:{def_line}  (definition)")
            for p, ln in outside:
                print(f"    {p}:{ln}")
            print()
    else:
        print("No named constants are being bypassed. ✓")
        print()

    if show_all:
        print("-" * 60)
        print("5-digit file offsets duplicated across files with NO named constant:")
        print("(candidates for a new rom_data.rs constant)")
        print()
        found_secondary = False
        for val, sites in sorted(occ.items()):
            if val in consts or val < 0x10000:
                continue
            files = {os.path.normpath(p) for (p, _) in sites}
            if len(files) < 2:
                continue
            if ignored_pair([p for (p, _) in sites]):
                continue
            found_secondary = True
            print(f"  0x{val:X}")
            for p, ln in sites:
                print(f"    {p}:{ln}")
            print()
        if not found_secondary:
            print("  (none)")
            print()

    if exit_code and bypasses:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
