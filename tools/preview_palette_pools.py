#!/usr/bin/env python3
"""Render candidate themed palette pools as a static HTML file with color swatches.

Open the output file in a browser to visually judge whether the color choices
"read as" their theme (plains as grassy, water as blue, etc.) before baking
them into the Rust randomizer.

Edit the POOLS dict below and re-run to iterate. This tool exists so we can
tweak pool choices quickly without touching Rust code.

Usage:
    python3 tools/preview_palette_pools.py                 # writes palette_pools_preview.html
    python3 tools/preview_palette_pools.py --out foo.html
"""

import argparse
from pathlib import Path

# FirebrandX Nostalgia-FBX palette (community-standard NES → sRGB mapping).
# 64 entries indexed 0x00-0x3F. Values from:
# https://www.firebrandx.com/downloads/fbx-nes-palettes.zip
NES_PALETTE_HEX = [
    # 0x00-0x0F
    "#616161", "#0000C6", "#1F04AA", "#3C0081", "#630054", "#730028", "#720600", "#601500",
    "#402500", "#1F3200", "#003800", "#003400", "#002B54", "#000000", "#000000", "#000000",
    # 0x10-0x1F
    "#AAAAAA", "#104EDB", "#4836C7", "#772598", "#A11E61", "#B61F2E", "#B1330A", "#944E00",
    "#696D00", "#3F8500", "#1B8F00", "#009227", "#00896A", "#000000", "#000000", "#000000",
    # 0x20-0x2F
    "#FFFFFF", "#65A3FF", "#998CFF", "#D179FF", "#FC6EF1", "#FF7193", "#FF8543", "#EBA200",
    "#BDC100", "#89D600", "#5EDB3C", "#3FD972", "#3BCEBC", "#616161", "#000000", "#000000",
    # 0x30-0x3F
    "#FFFFFF", "#C3DBFF", "#D3CCFF", "#EBC3FF", "#FFC5FC", "#FFC8DC", "#FFCFB7", "#FFDB98",
    "#F1E68E", "#D4EF8F", "#BDF3A0", "#ACF3BC", "#ACEDE2", "#AAAAAA", "#000000", "#000000",
]

# Initial pool proposal. Each pool: NES color indices that "read as" a theme.
# Order is suggestive (dark→light or hue progression), not required for use.
POOLS = {
    "plains":   [0x0B, 0x1A, 0x2A, 0x3A, 0x08, 0x18, 0x28, 0x38],
    "water":    [0x01, 0x11, 0x21, 0x31, 0x02, 0x12, 0x22, 0x32],
    "desert":   [0x07, 0x17, 0x27, 0x37, 0x08, 0x18, 0x28, 0x38],
    "ice":      [0x01, 0x11, 0x21, 0x31, 0x30, 0x3C, 0x20, 0x10],
    "sky":      [0x12, 0x22, 0x32, 0x35, 0x36, 0x26, 0x16, 0x06],
    "fortress": [0x00, 0x06, 0x16, 0x26, 0x07, 0x17, 0x27, 0x37],
    "airship":  [0x00, 0x04, 0x14, 0x24, 0x07, 0x17, 0x27, 0x37],
    "giant":    [0x06, 0x16, 0x26, 0x36, 0x09, 0x19, 0x29, 0x39],
    "hud_safe": [0x00, 0x01, 0x02, 0x05, 0x06, 0x0C, 0x11, 0x12],
}

# Example 4-color palettes (what a level might actually end up using after randomization).
# byte0 = $3F00 mirror (must be dark/HUD-safe); byte1-2 = hue/highlight; byte3 = outline (always $0F).
def sample_palette(pool, seed):
    """Deterministically build a 4-byte example palette from a pool."""
    import random
    r = random.Random(seed)
    # byte 0 always from hud_safe subset of pool
    hud_safe = [c for c in pool if c <= 0x1C]
    b0 = r.choice(hud_safe) if hud_safe else 0x0F
    remaining = [c for c in pool if c != b0]
    b1 = r.choice(remaining)
    b2 = r.choice([c for c in remaining if c != b1] or remaining)
    return (b0, b1, b2, 0x0F)


HTML_HEAD = """<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>SMB3-RS palette pool preview</title>
<style>
  body { font-family: -apple-system, system-ui, sans-serif; background: #222; color: #eee; padding: 24px; }
  h1 { font-weight: 500; margin-bottom: 4px; }
  .subtitle { color: #888; margin-bottom: 32px; }
  .pool { margin-bottom: 40px; border-top: 1px solid #333; padding-top: 16px; }
  .pool-name { font-weight: 600; font-size: 18px; margin-bottom: 8px; }
  .swatches { display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 12px; }
  .swatch { width: 96px; height: 96px; border-radius: 4px; display: flex; flex-direction: column;
            justify-content: flex-end; padding: 6px; font-size: 11px; font-family: ui-monospace, monospace;
            color: rgba(0,0,0,0.65); text-shadow: 0 1px 2px rgba(255,255,255,0.5); }
  .swatch.dark { color: rgba(255,255,255,0.9); text-shadow: 0 1px 2px rgba(0,0,0,0.5); }
  .sample-row { display: flex; gap: 16px; flex-wrap: wrap; margin-top: 8px; }
  .sample { display: flex; align-items: center; gap: 4px; padding: 6px 10px; background: #2a2a2a;
            border-radius: 4px; font-family: ui-monospace, monospace; font-size: 11px; }
  .sample .chip { width: 24px; height: 24px; border-radius: 2px; border: 1px solid #444; }
  .label { color: #777; margin-right: 4px; }
  .full-nes { margin-top: 64px; border-top: 1px solid #333; padding-top: 16px; }
  .full-nes .row { display: flex; gap: 2px; margin-bottom: 2px; }
  .full-nes .cell { width: 32px; height: 32px; font-size: 9px; color: rgba(0,0,0,0.6);
                    display: flex; align-items: flex-end; justify-content: flex-end; padding: 2px;
                    font-family: ui-monospace, monospace; }
  .full-nes .cell.dark { color: rgba(255,255,255,0.8); }
</style>
</head>
<body>
<h1>SMB3-RS palette pool preview</h1>
<p class="subtitle">Candidate themed palette pools for the randomizer. Edit
<code>tools/preview_palette_pools.py</code> and re-run to iterate.</p>
"""

HTML_TAIL = """</body></html>"""


def is_dark(hex_str):
    r = int(hex_str[1:3], 16); g = int(hex_str[3:5], 16); b = int(hex_str[5:7], 16)
    # Perceptual luminance
    return (0.299 * r + 0.587 * g + 0.114 * b) < 128


def render(out_path: Path):
    parts = [HTML_HEAD]
    for name, pool in POOLS.items():
        parts.append('<div class="pool">')
        parts.append(f'<div class="pool-name">{name}</div>')
        # Swatches
        parts.append('<div class="swatches">')
        for idx in pool:
            hexv = NES_PALETTE_HEX[idx]
            cls = "swatch dark" if is_dark(hexv) else "swatch"
            parts.append(f'<div class="{cls}" style="background:{hexv}">0x{idx:02X}<br>{hexv}</div>')
        parts.append('</div>')
        # 3 sample palettes from this pool
        parts.append('<div class="sample-row">')
        for seed in range(3):
            palette = sample_palette(pool, seed + hash(name))
            parts.append('<div class="sample"><span class="label">seed '+str(seed)+'</span>')
            for b in palette:
                hexv = NES_PALETTE_HEX[b]
                parts.append(f'<div class="chip" title="0x{b:02X}" style="background:{hexv}"></div>')
            parts.append('</div>')
        parts.append('</div>')
        parts.append('</div>')

    # Full NES palette grid for reference
    parts.append('<div class="full-nes"><h2 style="font-weight:500;">Full NES palette (for reference)</h2>')
    for row in range(4):
        parts.append('<div class="row">')
        for col in range(16):
            idx = row * 16 + col
            hexv = NES_PALETTE_HEX[idx]
            cls = "cell dark" if is_dark(hexv) else "cell"
            parts.append(f'<div class="{cls}" style="background:{hexv}" title="0x{idx:02X} {hexv}">{idx:02X}</div>')
        parts.append('</div>')
    parts.append('</div>')

    parts.append(HTML_TAIL)
    out_path.write_text("".join(parts))
    print(f"Wrote {out_path}")
    print(f"Open in browser to preview {len(POOLS)} pools ({sum(len(p) for p in POOLS.values())} total colors).")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", type=Path, default=Path("palette_pools_preview.html"))
    args = ap.parse_args()
    render(args.out)


if __name__ == "__main__":
    main()
