import init, { generate_patch, generate_patched_rom } from "../pkg/smb3r.js";

let wasmReady = false;
let romBytes = null;

const romInput = document.getElementById("rom-input");
const romLabel = document.getElementById("rom-label");
const seedInput = document.getElementById("seed-input");
const randomSeedBtn = document.getElementById("random-seed-btn");
const generateBtn = document.getElementById("generate-btn");
const statusDiv = document.getElementById("status");

const optPowerups = document.getElementById("opt-powerups");
const optPalettes = document.getElementById("opt-palettes");
const optEnemies = document.getElementById("opt-enemies");
const optWorldOrder = document.getElementById("opt-world-order");
const optBigQBlocks = document.getElementById("opt-big-q-blocks");
const optLevelShuffle = document.getElementById("opt-level-shuffle");
const optChestItems = document.getElementById("opt-chest-items");
const optRemoveWhistles = document.getElementById("opt-remove-whistles");
const optShuffleFortresses = document.getElementById("opt-shuffle-fortresses");
const optAirshipLock = document.getElementById("opt-airship-lock");
const optFixDrawbridges = document.getElementById("opt-fix-drawbridges");
const optStartingLives = document.getElementById("opt-starting-lives");

// Dynamically populate Starting Lives dropdown (4–99)
if (optStartingLives) {
	for (let i = 4; i <= 99; i++) {
		const option = document.createElement("option");
		option.value = i;
		option.textContent = i;
		if (i === 4) option.selected = true;
		optStartingLives.appendChild(option);
	}
}

// Initialize WASM
init()
	.then(() => {
		wasmReady = true;
		updateGenerateButton();
	})
	.catch((err) => {
		showStatus(`Failed to load WASM module: ${err}`, "error");
	});

// ROM file selection
romInput.addEventListener("change", (e) => {
	const file = e.target.files[0];
	if (!file) return;

	const reader = new FileReader();
	reader.onload = () => {
		romBytes = new Uint8Array(reader.result);
		romLabel.textContent = file.name;
		romLabel.classList.add("loaded");
		updateGenerateButton();
	};
	reader.onerror = () => {
		showStatus("Failed to read ROM file", "error");
	};
	reader.readAsArrayBuffer(file);
});

// Random seed button
randomSeedBtn.addEventListener("click", () => {
	seedInput.value = Math.floor(
		Math.random() * Number.MAX_SAFE_INTEGER,
	).toString();
});

// Generate button
generateBtn.addEventListener("click", () => {
	if (!wasmReady || !romBytes) return;

	const seedStr = seedInput.value.trim();
	const seed = seedStr
		? BigInt(seedStr)
		: BigInt(Math.floor(Math.random() * Number.MAX_SAFE_INTEGER));

	const options = JSON.stringify({
		powerups: optPowerups.checked,
		palettes: optPalettes.checked,
		enemies: optEnemies.checked,
		world_order: optWorldOrder.checked,
		big_q_blocks: optBigQBlocks.checked,
		level_shuffle: optLevelShuffle.value,
		chest_items: optChestItems.checked,
		remove_whistles: optRemoveWhistles.checked,
		shuffle_fortresses: optShuffleFortresses.checked,
		airship_lock: optAirshipLock.checked,
		fix_drawbridges: optFixDrawbridges.checked,
		starting_lives: Number(optStartingLives.value),
	});

	const outputFormat = document.querySelector(
		'input[name="output-format"]:checked',
	).value;

	showStatus("Generating...", "loading");

	try {
		let result;
		let filename;
		const mimeType = "application/octet-stream";

		if (outputFormat === "rom") {
			result = generate_patched_rom(romBytes, seed, options);
			filename = `smb3r_${seed}.nes`;
		} else {
			result = generate_patch(romBytes, seed, options);
			filename = `smb3r_${seed}.ips`;
		}

		// Trigger download
		const blob = new Blob([result], { type: mimeType });
		const url = URL.createObjectURL(blob);
		const a = document.createElement("a");
		a.href = url;
		a.download = filename;
		document.body.appendChild(a);
		a.click();
		document.body.removeChild(a);
		URL.revokeObjectURL(url);

		showStatus(
			`Generated ${filename} (${result.length} bytes, seed: ${seed})`,
			"success",
		);
	} catch (err) {
		showStatus(`Error: ${err}`, "error");
	}
});

function updateGenerateButton() {
	generateBtn.disabled = !(wasmReady && romBytes);
}

function showStatus(message, type) {
	statusDiv.textContent = message;
	statusDiv.className = `status ${type}`;
	statusDiv.hidden = false;
}
