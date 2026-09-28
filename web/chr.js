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

// Color index (0-3) of pixel (x, y) in a CHR tile.
function tilePixel(romBytes, tileId, x, y) {
	const base = CHR_BASE + tileId * TILE_BYTES;
	const bit = 7 - x;
	return (((romBytes[base + 8 + y] >> bit) & 1) << 1) | ((romBytes[base + y] >> bit) & 1);
}

// Decode a single 8×8 CHR tile to an ImageData. paletteRgb = 4 [r,g,b] tuples.
// Color 0 → fully transparent. Colors 1-3 → opaque.
// flipX mirrors horizontally, the way OAM attribute bit 6 does.
export function decodeTile(romBytes, tileId, paletteRgb, flipX = false) {
	const data = new Uint8ClampedArray(8 * 8 * 4);
	for (let y = 0; y < 8; y++) {
		for (let x = 0; x < 8; x++) {
			const idx = tilePixel(romBytes, tileId, x, y);
			const o = (y * 8 + (flipX ? 7 - x : x)) * 4;
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

// Render an arbitrary grid of 8×8 tiles, listed row-major and `cols` wide.
//
// Each entry is a CHR tile index, `null` for a transparent cell (so a
// non-rectangular sprite can skip tiles it doesn't use), or `{ t, flip: true }`
// to h-flip that one tile. Per-tile flips are for composites whose parts differ:
// Bowser's top half is stored as a left half and mirrored, while his bottom half
// is stored whole, so no whole-grid rule covers both. `{ t, palette: [...] }`
// gives one tile its own four NES colors, for sprites the engine assembles from
// parts in different palettes (a Troopa's head and feet vs. its shell).
//
// The canvas is sized to the grid in native pixels — callers scale via CSS with
// `image-rendering: pixelated`.
// flipRight h-flips the right half of the grid, for symmetric art stored as one
// half and drawn twice (the title-screen seed hash does this). Paired with a
// tile list whose right half repeats the left in reverse, it mirrors the whole
// sprite; for the common 2-column case that's just "flip the right column".
//
// `over` places extra `{ t, x, y, flip?, palette? }` tiles at pixel positions
// after the grid, for parts the engine overlaps rather than tiles (a
// Paratroopa's wing sits 8px into its shell). Tiles are composited: a tile's
// transparent pixels leave whatever is under them.
export function renderTiles(canvas, romBytes, tileIds, cols, paletteRgb, flipRight = false, over = []) {
	const w = cols * 8;
	const h = Math.ceil(tileIds.length / cols) * 8;
	canvas.width = w;
	canvas.height = h;
	const img = new ImageData(w, h);
	const place = (entry, px, py, flip) => {
		const pal = entry.palette ? resolvePalette(entry.palette) : paletteRgb;
		const tile = decodeTile(romBytes, entry.t, pal, flip).data;
		for (let y = 0; y < 8; y++) {
			for (let x = 0; x < 8; x++) {
				const s = (y * 8 + x) * 4;
				if (!tile[s + 3] || px + x < 0 || px + x >= w || py + y < 0 || py + y >= h) continue;
				img.data.set(tile.subarray(s, s + 4), ((py + y) * w + px + x) * 4);
			}
		}
	};
	tileIds.forEach((entry, i) => {
		if (entry == null) return;
		const tile = typeof entry === "object" ? entry : { t: entry };
		const col = i % cols;
		const flip = typeof entry === "object" ? !!entry.flip : flipRight && col >= cols / 2;
		place(tile, col * 8, Math.floor(i / cols) * 8, flip);
	});
	for (const o of over) place(o, o.x, o.y, !!o.flip);
	canvas.getContext("2d").putImageData(img, 0, 0);
}

// Render a 2×2 metasprite (16×16 px) from four tile IDs in [tl, tr, bl, br] order.
export function renderMetatile(canvas, romBytes, tileIds, paletteRgb, flipRight = false) {
	renderTiles(canvas, romBytes, tileIds, 2, paletteRgb, flipRight);
}

// Render world-map background tiles, whose art is the reverse of a sprite's:
// the ground fills the square in color `clear` and the outline is color 0.
// Ground pixels reachable from the edge go transparent — a flood fill, not a
// color swap, because a bush or flower reuses the ground color inside its own
// outline — and color 0 draws opaque. Tiles are plain indices (no flips).
function renderMapTiles(canvas, romBytes, tileIds, cols, paletteRgb, clear) {
	const w = cols * 8;
	const h = Math.ceil(tileIds.length / cols) * 8;
	const idx = new Int8Array(w * h);
	tileIds.forEach((tid, i) => {
		const ox = (i % cols) * 8;
		const oy = Math.floor(i / cols) * 8;
		for (let y = 0; y < 8; y++) {
			for (let x = 0; x < 8; x++) idx[(oy + y) * w + ox + x] = tilePixel(romBytes, tid, x, y);
		}
	});
	const stack = [];
	for (let x = 0; x < w; x++) stack.push(x, (h - 1) * w + x);
	for (let y = 0; y < h; y++) stack.push(y * w, y * w + w - 1);
	while (stack.length) {
		const p = stack.pop();
		if (idx[p] !== clear) continue;
		idx[p] = -1;
		const x = p % w;
		if (x > 0) stack.push(p - 1);
		if (x < w - 1) stack.push(p + 1);
		if (p >= w) stack.push(p - w);
		if (p < w * (h - 1)) stack.push(p + w);
	}
	canvas.width = w;
	canvas.height = h;
	const img = new ImageData(w, h);
	for (let p = 0; p < w * h; p++) {
		if (idx[p] >= 0) img.data.set([...paletteRgb[idx[p]], 255], p * 4);
	}
	canvas.getContext("2d").putImageData(img, 0, 0);
}

// Convenience: render an icon spec (from the schema) into a canvas.
// spec = { tiles: [...row-major], cols?: 2, palette: [c0, c1, c2, c3], flipRight?, clear? }
// `clear` marks a world-map tile and names its ground color; see renderMapTiles.
export function renderIcon(canvas, romBytes, spec) {
	if (!canvas || !romBytes || !spec) return;
	const cols = spec.cols ?? 2;
	const pal = resolvePalette(spec.palette);
	if (spec.clear != null) renderMapTiles(canvas, romBytes, spec.tiles, cols, pal, spec.clear);
	else renderTiles(canvas, romBytes, spec.tiles, cols, pal, !!spec.flipRight, spec.over);
}

// Draw an icon spec into a `box`-sized square: trimmed to its opaque pixels,
// scaled by the largest whole number that fits (never above `maxScale`, so small
// art keeps the same pixel size as the rest), and centred. Art that is bigger
// than the box at 1x grows the canvas instead of being shrunk. The canvas is
// displayed 1:1 — the scaling is in its pixels, not in CSS.
export function renderIconBox(canvas, romBytes, spec, box, maxScale) {
	const art = document.createElement("canvas");
	renderIcon(art, romBytes, spec);
	const { data, width, height } = art.getContext("2d").getImageData(0, 0, art.width, art.height);
	let x0 = width, y0 = height, x1 = -1, y1 = -1;
	for (let y = 0; y < height; y++) {
		for (let x = 0; x < width; x++) {
			if (!data[(y * width + x) * 4 + 3]) continue;
			x0 = Math.min(x0, x);
			x1 = Math.max(x1, x);
			y0 = Math.min(y0, y);
			y1 = Math.max(y1, y);
		}
	}
	if (x1 < 0) return;
	const w = x1 - x0 + 1;
	const h = y1 - y0 + 1;
	const k = Math.max(1, Math.min(maxScale, Math.floor(box / Math.max(w, h))));
	canvas.width = Math.max(box, w * k);
	canvas.height = Math.max(box, h * k);
	canvas.style.width = `${canvas.width}px`;
	canvas.style.height = `${canvas.height}px`;
	const ctx = canvas.getContext("2d");
	ctx.imageSmoothingEnabled = false;
	ctx.clearRect(0, 0, canvas.width, canvas.height);
	const dx = Math.floor((canvas.width - w * k) / 2);
	const dy = Math.floor((canvas.height - h * k) / 2);
	ctx.drawImage(art, x0, y0, w, h, dx, dy, w * k, h * k);
}
