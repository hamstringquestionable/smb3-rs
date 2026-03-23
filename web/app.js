import init, { generate_patch, generate_patched_rom, encode_flag_key, decode_flag_key } from "./pkg/smb3r.js";

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
const optShufflePipes = document.getElementById("opt-shuffle-pipes");
const optChestItems = document.getElementById("opt-chest-items");
const optRemoveWhistles = document.getElementById("opt-remove-whistles");
const optShuffleFortresses = document.getElementById("opt-shuffle-fortresses");
const optFortressRedistribute = document.getElementById(
	"opt-fortress-redistribute",
);
const optAirshipLock = document.getElementById("opt-airship-lock");
const optFixDrawbridges = document.getElementById("opt-fix-drawbridges");
const optRemoveW2Rock = document.getElementById("opt-remove-w2-rock");
const optStartingLives = document.getElementById("opt-starting-lives");
const flagKeyInput = document.getElementById("flag-key-input");
const flagKeyCopyBtn = document.getElementById("flag-key-copy-btn");
const flagKeyApplyBtn = document.getElementById("flag-key-apply-btn");

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
		updateFlagKey();
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
		shuffle_pipes: optShufflePipes.checked,
		chest_items: optChestItems.checked,
		remove_whistles: optRemoveWhistles.checked,
		shuffle_fortresses: optShuffleFortresses.checked,
		fortress_redistribute: optFortressRedistribute.value,
		airship_lock: optAirshipLock.checked,
		fix_drawbridges: optFixDrawbridges.checked,
		remove_w2_rock: optRemoveW2Rock.checked,
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

// --- Flag Key ---

function getCurrentOptionsJson() {
	return JSON.stringify({
		powerups: optPowerups.checked,
		palettes: optPalettes.checked,
		enemies: optEnemies.checked,
		world_order: optWorldOrder.checked,
		big_q_blocks: optBigQBlocks.checked,
		level_shuffle: optLevelShuffle.value,
		shuffle_pipes: optShufflePipes.checked,
		chest_items: optChestItems.checked,
		remove_whistles: optRemoveWhistles.checked,
		shuffle_fortresses: optShuffleFortresses.checked,
		fortress_redistribute: optFortressRedistribute.value,
		airship_lock: optAirshipLock.checked,
		fix_drawbridges: optFixDrawbridges.checked,
		remove_w2_rock: optRemoveW2Rock.checked,
		starting_lives: Number(optStartingLives.value),
		disable_autoscroll: true, // always on in web UI
	});
}

function updateFlagKey() {
	if (!wasmReady) return;
	try {
		flagKeyInput.value = encode_flag_key(getCurrentOptionsJson());
	} catch (_) {
		// ignore before WASM ready
	}
}

function applyFlagKey(key) {
	if (!wasmReady) return;
	try {
		const json = decode_flag_key(key.trim());
		const opts = JSON.parse(json);
		optPowerups.checked = opts.powerups;
		optPalettes.checked = opts.palettes;
		optEnemies.checked = opts.enemies;
		optWorldOrder.checked = opts.world_order;
		optBigQBlocks.checked = opts.big_q_blocks;
		optLevelShuffle.value = opts.level_shuffle;
		optShufflePipes.checked = opts.shuffle_pipes;
		optChestItems.checked = opts.chest_items;
		optRemoveWhistles.checked = opts.remove_whistles;
		optShuffleFortresses.checked = opts.shuffle_fortresses;
		optFortressRedistribute.value = opts.fortress_redistribute;
		optAirshipLock.checked = opts.airship_lock;
		optFixDrawbridges.checked = opts.fix_drawbridges;
		optRemoveW2Rock.checked = opts.remove_w2_rock;
		if (opts.starting_lives) optStartingLives.value = opts.starting_lives;
		showStatus("Flag key applied!", "success");
	} catch (err) {
		showStatus(`Invalid flag key: ${err}`, "error");
	}
}

// Update flag key whenever any option changes
const allOptionElements = [
	optPowerups, optPalettes, optEnemies, optWorldOrder, optBigQBlocks,
	optLevelShuffle, optShufflePipes, optChestItems, optRemoveWhistles,
	optShuffleFortresses, optFortressRedistribute, optAirshipLock,
	optFixDrawbridges, optRemoveW2Rock, optStartingLives,
];
for (const el of allOptionElements) {
	el.addEventListener("change", updateFlagKey);
}

flagKeyCopyBtn.addEventListener("click", () => {
	updateFlagKey();
	navigator.clipboard.writeText(flagKeyInput.value).then(() => {
		showStatus("Flag key copied!", "success");
	});
});

flagKeyApplyBtn.addEventListener("click", () => {
	applyFlagKey(flagKeyInput.value);
});

function updateGenerateButton() {
	generateBtn.disabled = !(wasmReady && romBytes);
}

function showStatus(message, type) {
	statusDiv.textContent = message;
	statusDiv.className = `status ${type}`;
	statusDiv.hidden = false;
}
