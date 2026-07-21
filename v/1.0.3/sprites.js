// Sprite-sheet renderer. Clips icons from a single bundled PNG.
//
// Sheet provenance: SMB3 sprite sheet from spriters-resource.com, contributors
// credited in the sheet itself (Fieepa, Rotodisco, ComputerBoi13,
// LBlueTheSpriter, Docvon Schmeltwick).
//
// The bundled sprites.png has been preprocessed to make the per-cell blue
// backgrounds transparent (RGBA colors (68,145,190), (35,50,62), (41,88,124)
// → alpha 0). If you re-source the sheet, redo the knockout — otherwise icons
// will render with a blue square around them on the dark site bg.
//
// Icon spec format on schema entries:
//   icon: { x: 0, y: 48, w: 16, h: 16 }     // basic 16x16
//   icon: { x: 0, y: 48 }                    // w/h default to 16
//   icon: { x: 0, y: 48, w: 16, h: 32 }      // tall sprites (Mario etc.)

// Available sprite sheets. Add new entries here when bundling a new sheet —
// the picker (sprite-picker.html) and schema icons (options.js `sheet:` field)
// look up by the same key.
export const SHEETS = {
	default: "./assets/sprites.png",
	bosses: "./assets/sprites-bosses.png",
	enemies: "./assets/sprites-enemies.png",
};

const sheetPromises = {};
function loadSheet(name = "default") {
	const path = SHEETS[name];
	if (!path) return Promise.reject(new Error(`unknown sprite sheet: ${name}`));
	if (sheetPromises[name]) return sheetPromises[name];
	sheetPromises[name] = new Promise((resolve, reject) => {
		const img = new Image();
		img.onload = () => resolve(img);
		img.onerror = (e) => reject(new Error(`failed to load sprite sheet ${name}: ${e}`));
		img.src = path;
	});
	return sheetPromises[name];
}

export async function ensureSheet(name = "default") {
	return loadSheet(name);
}

export function drawSpriteFromSheet(canvas, sheet, spec) {
	if (!canvas || !sheet || !spec) return;
	const w = spec.w ?? 16;
	const h = spec.h ?? 16;
	canvas.width = w;
	canvas.height = h;
	const ctx = canvas.getContext("2d");
	ctx.imageSmoothingEnabled = false;
	ctx.clearRect(0, 0, w, h);
	ctx.drawImage(sheet, spec.x, spec.y, w, h, 0, 0, w, h);
}

// Convenience: paint one icon canvas (looking up the sheet promise).
// Use renderAllIcons() when painting many at once — it loads the sheet once.
export async function renderIcon(canvas, spec) {
	if (!canvas || !spec) return;
	const sheet = await loadSheet();
	drawSpriteFromSheet(canvas, sheet, spec);
}
