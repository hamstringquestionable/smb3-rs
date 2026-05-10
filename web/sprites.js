// Sprite-sheet renderer. Clips icons from a single bundled PNG.
//
// Sheet provenance: SMB3 sprite sheet from spriters-resource.com, contributors
// credited in the sheet itself (Fieepa, Rotodisco, ComputerBoi13,
// LBlueTheSpriter, Docvon Schmeltwick).
//
// Icon spec format on schema entries:
//   icon: { x: 0, y: 48, w: 16, h: 16 }     // basic 16x16
//   icon: { x: 0, y: 48 }                    // w/h default to 16
//   icon: { x: 0, y: 48, w: 16, h: 32 }      // tall sprites (Mario etc.)

const SHEET_PATH = "./assets/sprites.png";

let sheetPromise = null;
function loadSheet() {
	if (sheetPromise) return sheetPromise;
	sheetPromise = new Promise((resolve, reject) => {
		const img = new Image();
		img.onload = () => resolve(img);
		img.onerror = (e) => reject(new Error("failed to load sprite sheet: " + e));
		img.src = SHEET_PATH;
	});
	return sheetPromise;
}

export async function ensureSheet() {
	return loadSheet();
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
