#!/usr/bin/env python3
"""Render every VariantGroup in src/randomize/palette_variants.rs as HTML swatches.

Parses the Rust source (so hand-added alternates show up too) and emits a
static HTML page with one card per VariantGroup: offset, then one row of
4 color swatches per variant (vanilla, recolored, any hand-curated alternates).

Use this to eyeball what's currently shipping and to decide which positions
would benefit from new curated variants.

Usage:
    nix-shell -p python3 --run "python3 tools/preview_palette_variants.py"
    # → palette_variants_preview.html at repo root
"""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "src/randomize/palette_variants.rs"
OUT = ROOT / "palette_variants_preview.html"

# FirebrandX Nostalgia-FBX palette, community-standard NES → sRGB.
NES_PALETTE_HEX = [
    "#616161","#0000C6","#1F04AA","#3C0081","#630054","#730028","#720600","#601500",
    "#402500","#1F3200","#003800","#003400","#002B54","#000000","#000000","#000000",
    "#AAAAAA","#104EDB","#4836C7","#772598","#A11E61","#B61F2E","#B1330A","#944E00",
    "#696D00","#3F8500","#1B8F00","#009227","#00896A","#000000","#000000","#000000",
    "#FFFFFF","#65A3FF","#998CFF","#D179FF","#FC6EF1","#FF7193","#FF8543","#EBA200",
    "#BDC100","#89D600","#5EDB3C","#3FD972","#3BCEBC","#616161","#000000","#000000",
    "#FFFFFF","#C3DBFF","#D3CCFF","#EBC3FF","#FFC5FC","#FFC8DC","#FFCFB7","#FFDB98",
    "#F1E68E","#D4EF8F","#BDF3A0","#ACF3BC","#ACEDE2","#AAAAAA","#000000","#000000",
]

VARIANT_LABELS = ["vanilla", "recolored"]


def nes_hex(byte):
    # 0xFF is SMB3's "terminator/flag" byte inside palette quartets — not a color.
    # Render as diagonal-hatched cell so it visually stands out.
    if byte == 0xFF:
        return None
    return NES_PALETTE_HEX[byte & 0x3F]


def parse_regions(src):
    """Yield (const_name, section_header_comment, [VariantGroup...])."""
    # Split on `pub const NAME: &[VariantGroup] = &[` ... `];`
    const_re = re.compile(
        r"pub const (\w+): &\[VariantGroup\] = &\[(.+?)\];",
        re.DOTALL,
    )
    # Section banner lives immediately above each const declaration as
    #   // ----------------------------------------------------------------
    #   // slot 3 (...) - ... description ...
    #   // N quartets changed by Recolored.
    #   // ----------------------------------------------------------------
    comment_re = re.compile(r"//\s*[-]{3,}\s*\n((?://[^\n]*\n)+)//\s*[-]{3,}", re.MULTILINE)

    # Walk consts in order with their preceding banner.
    out = []
    for m in const_re.finditer(src):
        const_name = m.group(1)
        body = m.group(2)
        # Find the comment banner that ends just before this const (search the
        # 300 chars right before match start for the last banner).
        pre = src[max(0, m.start() - 600):m.start()]
        banners = comment_re.findall(pre)
        desc = banners[-1].strip() if banners else const_name
        # Parse VariantGroup entries
        entries = []
        grp_re = re.compile(
            r"VariantGroup\s*\{\s*offset:\s*0x([0-9A-Fa-f]+)\s*,\s*variants:\s*&\[(.+?)\]\s*\}",
            re.DOTALL,
        )
        for g in grp_re.finditer(body):
            offset = int(g.group(1), 16)
            variants_body = g.group(2)
            variant_re = re.compile(r"\[\s*(0x[0-9A-Fa-f]{2}(?:\s*,\s*0x[0-9A-Fa-f]{2}){3})\s*\]")
            variants = []
            for v in variant_re.finditer(variants_body):
                bytes_ = [int(x, 16) for x in v.group(1).split(",")]
                variants.append(bytes_)
            entries.append((offset, variants))
        out.append((const_name, desc, entries))
    return out


def swatch_cell(byte):
    color = nes_hex(byte)
    label = f"{byte:02X}"
    if color is None:
        # 0xFF (terminator/flag). Hatched cell, no fill color.
        return (
            '<div class="cell flag">'
            '<div class="sw flag"></div>'
            f'<div class="code">{label}</div>'
            '</div>'
        )
    # Pick black/white text based on relative luminance for legibility.
    r, g, b = int(color[1:3], 16), int(color[3:5], 16), int(color[5:7], 16)
    lum = 0.299 * r + 0.587 * g + 0.114 * b
    txt = "#000" if lum > 140 else "#fff"
    return (
        f'<div class="cell">'
        f'<div class="sw" style="background:{color};color:{txt}">{label}</div>'
        f'</div>'
    )


def render_variant(bytes_, label):
    cells = "".join(swatch_cell(b) for b in bytes_)
    return f'<div class="variant"><div class="vlabel">{label}</div><div class="quartet">{cells}</div></div>'


def render_group(offset, variants):
    rows = []
    for i, v in enumerate(variants):
        label = VARIANT_LABELS[i] if i < len(VARIANT_LABELS) else f"variant {i}"
        rows.append(render_variant(v, label))
    return (
        f'<div class="group">'
        f'<div class="offset">0x{offset:05X}</div>'
        f'{"".join(rows)}'
        f'</div>'
    )


def render_region(const_name, desc, entries):
    groups = "\n".join(render_group(off, variants) for off, variants in entries)
    return (
        f'<section>'
        f'<h2>{const_name} <span class="count">({len(entries)} positions)</span></h2>'
        f'<pre class="desc">{desc}</pre>'
        f'<div class="grid">{groups}</div>'
        f'</section>'
    )


CSS = """
body { background: #181828; color: #ddd; font-family: -apple-system, sans-serif;
       margin: 0; padding: 1.5rem; }
h1 { font-size: 1.3rem; margin: 0 0 1rem; }
h2 { font-size: 1rem; margin: 1.5rem 0 0.3rem; color: #a0c4ff; }
h2 .count { color: #888; font-weight: normal; font-size: 0.8rem; }
.desc { color: #888; font-size: 0.75rem; white-space: pre-wrap; margin: 0 0 0.6rem; }
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
        gap: 0.6rem; }
.group { background: #22223a; border: 1px solid #333355; border-radius: 4px;
         padding: 0.4rem 0.5rem; }
.offset { font-family: monospace; font-size: 0.75rem; color: #888; margin-bottom: 0.25rem; }
.variant { display: flex; align-items: center; gap: 0.4rem; margin-bottom: 0.15rem; }
.vlabel { width: 4.8rem; font-size: 0.65rem; color: #aaa; text-align: right; }
.quartet { display: flex; gap: 2px; }
.cell { display: flex; flex-direction: column; align-items: center; }
.sw { width: 42px; height: 28px; font-family: monospace; font-size: 0.65rem;
      display: flex; align-items: center; justify-content: center; border-radius: 2px; }
.sw.flag { background: repeating-linear-gradient(45deg, #444, #444 4px, #222 4px, #222 8px);
           color: #ccc; }
"""


def main():
    src = SRC.read_text()
    regions = parse_regions(src)

    total = sum(len(e) for _, _, e in regions)
    html = [
        "<!DOCTYPE html><html><head><meta charset='utf-8'>",
        "<title>SMB3-RS palette variants</title>",
        f"<style>{CSS}</style></head><body>",
        f"<h1>SMB3-RS palette variants — {total} positions across {len(regions)} regions</h1>",
        "<p style='color:#888;font-size:0.75rem;max-width:700px'>",
        "Each position ships one or more 4-byte variants (vanilla + Recolored + any hand-curated alternates). ",
        "The randomizer picks one variant per position at random. Hatched cells (FF) are SMB3 palette-data format flags, not colors.",
        "</p>",
    ]
    for const_name, desc, entries in regions:
        html.append(render_region(const_name, desc, entries))
    html.append("</body></html>")

    OUT.write_text("\n".join(html))
    print(f"wrote {OUT} ({total} positions)")


if __name__ == "__main__":
    main()
