// CHR-from-ROM sprite renderer.
//
// SMB3 USA Rev 1 layout:
//   iNES header   = 16 bytes
//   PRG-ROM       = 256 KB at offset 0x10
//   CHR-ROM       = 128 KB at offset 0x40010 (16 pages × 8 KB = 512 tiles × 16 B)
//
// CHR tile encoding (NES standard):
//   16 bytes per 8×8 tile.
//   Bytes 0-7   = bit-plane 0 (LSB of color index)
//   Bytes 8-15  = bit-plane 1 (MSB of color index)
//   Pixel (x, y) color = ((p1[y] >> (7-x)) & 1) << 1 | ((p0[y] >> (7-x)) & 1)
//   Color 0 is the universal background and renders transparent here.

export const CHR_BASE = 0x40010;
export const TILE_BYTES = 16;

// NES 2C02 master palette. 64 RGB triples indexed by NES color byte 0x00-0x3F.
// Bytes 0x40-0xFF mirror 0x00-0x3F. Source: well-known FCEU/Nestopia palette.
// prettier-ignore
export const NES_PALETTE = [
	[0x7C,0x7C,0x7C],[0x00,0x00,0xFC],[0x00,0x00,0xBC],[0x44,0x28,0xBC],
	[0x94,0x00,0x84],[0xA8,0x00,0x20],[0xA8,0x10,0x00],[0x88,0x14,0x00],
	[0x50,0x30,0x00],[0x00,0x78,0x00],[0x00,0x68,0x00],[0x00,0x58,0x00],
	[0x00,0x40,0x58],[0x00,0x00,0x00],[0x00,0x00,0x00],[0x00,0x00,0x00],
	[0xBC,0xBC,0xBC],[0x00,0x78,0xF8],[0x00,0x58,0xF8],[0x68,0x44,0xFC],
	[0xD8,0x00,0xCC],[0xE4,0x00,0x58],[0xF8,0x38,0x00],[0xE4,0x5C,0x10],
	[0xAC,0x7C,0x00],[0x00,0xB8,0x00],[0x00,0xA8,0x00],[0x00,0xA8,0x44],
	[0x00,0x88,0x88],[0x00,0x00,0x00],[0x00,0x00,0x00],[0x00,0x00,0x00],
	[0xF8,0xF8,0xF8],[0x3C,0xBC,0xFC],[0x68,0x88,0xFC],[0x98,0x78,0xF8],
	[0xF8,0x78,0xF8],[0xF8,0x58,0x98],[0xF8,0x78,0x58],[0xFC,0xA0,0x44],
	[0xF8,0xB8,0x00],[0xB8,0xF8,0x18],[0x58,0xD8,0x54],[0x58,0xF8,0x98],
	[0x00,0xE8,0xD8],[0x78,0x78,0x78],[0x00,0x00,0x00],[0x00,0x00,0x00],
	[0xFC,0xFC,0xFC],[0xA4,0xE4,0xFC],[0xB8,0xB8,0xF8],[0xD8,0xB8,0xF8],
	[0xF8,0xB8,0xF8],[0xF8,0xA4,0xC0],[0xF0,0xD0,0xB0],[0xFC,0xE0,0xA8],
	[0xF8,0xD8,0x78],[0xD8,0xF8,0x78],[0xB8,0xF8,0xB8],[0xB8,0xF8,0xD8],
	[0x00,0xFC,0xFC],[0xF8,0xD8,0xF8],[0x00,0x00,0x00],[0x00,0x00,0x00],
];

// Resolve a 4-entry palette of NES color indices → 4 [r,g,b] triples.
// First entry is always treated as transparent regardless of value.
export function resolvePalette(indices) {
	return indices.map((i) => NES_PALETTE[i & 0x3F]);
}

// Decode a single 8×8 CHR tile to an ImageData. paletteRgb = 4 [r,g,b] tuples.
// Color 0 → fully transparent. Colors 1-3 → opaque.
export function decodeTile(romBytes, tileId, paletteRgb) {
	const base = CHR_BASE + tileId * TILE_BYTES;
	const data = new Uint8ClampedArray(8 * 8 * 4);
	for (let y = 0; y < 8; y++) {
		const p0 = romBytes[base + y];
		const p1 = romBytes[base + 8 + y];
		for (let x = 0; x < 8; x++) {
			const bit = 7 - x;
			const idx = (((p1 >> bit) & 1) << 1) | ((p0 >> bit) & 1);
			const o = (y * 8 + x) * 4;
			if (idx === 0) {
				data[o + 3] = 0; // transparent
			} else {
				const [r, g, b] = paletteRgb[idx];
				data[o] = r;
				data[o + 1] = g;
				data[o + 2] = b;
				data[o + 3] = 255;
			}
		}
	}
	return new ImageData(data, 8, 8);
}

// Blit a single 8×8 tile to the canvas (no scaling — caller controls size via CSS).
export function renderTileToCanvas(canvas, romBytes, tileId, paletteRgb) {
	canvas.width = 8;
	canvas.height = 8;
	const ctx = canvas.getContext("2d");
	ctx.clearRect(0, 0, 8, 8);
	ctx.putImageData(decodeTile(romBytes, tileId, paletteRgb), 0, 0);
}

// Render a 2×2 metasprite (16×16 px) from four tile IDs in [tl, tr, bl, br] order.
// Native 16×16 pixels — caller scales via CSS for crisp rendering with
// `image-rendering: pixelated`.
export function renderMetatile(canvas, romBytes, tileIds, paletteRgb) {
	canvas.width = 16;
	canvas.height = 16;
	const ctx = canvas.getContext("2d");
	ctx.clearRect(0, 0, 16, 16);
	const positions = [
		[0, 0],   // tl
		[8, 0],   // tr
		[0, 8],   // bl
		[8, 8],   // br
	];
	for (let i = 0; i < 4; i++) {
		const tid = tileIds[i];
		if (tid == null) continue;
		const [x, y] = positions[i];
		ctx.putImageData(decodeTile(romBytes, tid, paletteRgb), x, y);
	}
}

// Convenience: render an icon spec (from the schema) into a canvas.
// spec = { tiles: [tl, tr, bl, br], palette: [c0, c1, c2, c3] }
export function renderIcon(canvas, romBytes, spec) {
	if (!canvas || !romBytes || !spec) return;
	renderMetatile(canvas, romBytes, spec.tiles, resolvePalette(spec.palette));
}
