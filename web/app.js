import init, { generate_patch, generate_patched_rom, encode_flag_key, decode_flag_key, version } from "./pkg/smb3_rs.js";

let wasmReady = false;
let romBytes = null;

// --- IndexedDB ROM persistence ---

const DB_NAME = "smb3-rs";
const DB_STORE = "rom";

function openDb() {
	return new Promise((resolve, reject) => {
		const req = indexedDB.open(DB_NAME, 1);
		req.onupgradeneeded = () => req.result.createObjectStore(DB_STORE);
		req.onsuccess = () => resolve(req.result);
		req.onerror = () => reject(req.error);
	});
}

async function saveRom(bytes) {
	const db = await openDb();
	const tx = db.transaction(DB_STORE, "readwrite");
	tx.objectStore(DB_STORE).put(bytes, "data");
}

async function loadRom() {
	const db = await openDb();
	return new Promise((resolve) => {
		const tx = db.transaction(DB_STORE, "readonly");
		const req = tx.objectStore(DB_STORE).get("data");
		req.onsuccess = () => resolve(req.result || null);
		req.onerror = () => resolve(null);
	});
}

const romInput = document.getElementById("rom-input");
const romLabel = document.getElementById("rom-label");
const seedInput = document.getElementById("seed-input");
const randomSeedBtn = document.getElementById("random-seed-btn");
const generateBtn = document.getElementById("generate-btn");
const statusDiv = document.getElementById("status");

const optPowerups = document.getElementById("opt-powerups");
const optPalettes = document.getElementById("opt-palettes");
const optWorldOrder = document.getElementById("opt-world-order");
const optBigQBlocks = document.getElementById("opt-big-q-blocks");
const optShufflePipes = document.getElementById("opt-shuffle-pipes");
const optShuffleAirships = document.getElementById("opt-shuffle-airships");
const optChestItems = document.getElementById("opt-chest-items");
const optRemoveWhistles = document.getElementById("opt-remove-whistles");
const optAirshipLock = document.getElementById("opt-airship-lock");
const optFixDrawbridges = document.getElementById("opt-fix-drawbridges");
const optRemoveRocks = document.getElementById("opt-remove-rocks");
const optRemoveNCards = document.getElementById("opt-remove-n-cards");
const optRemoveSpadeGames = document.getElementById("opt-remove-spade-games");
const optSkipWandCutscene = document.getElementById("opt-skip-wand-cutscene");
const optAdjustBossHitboxes = document.getElementById("opt-adjust-boss-hitboxes");
const optKoopalingHits = document.getElementById("opt-koopaling-hits");
// Pill group helpers (tri-state radio buttons)
function getPill(name) {
	return document.querySelector(`input[name="${name}"]:checked`)?.value || "off";
}
function setPill(name, val) {
	const el = document.querySelector(`input[name="${name}"][value="${val}"]`);
	if (el) el.checked = true;
}
const optWildInjections = document.getElementById("opt-wild-injections");
const optStartingLives = document.getElementById("opt-starting-lives");
const optStartItems = [
	document.getElementById("opt-start-item-0"),
	document.getElementById("opt-start-item-1"),
	document.getElementById("opt-start-item-2"),
];
const flagKeyInput = document.getElementById("flag-key-input");
const flagKeyCopyBtn = document.getElementById("flag-key-copy-btn");
const flagKeyApplyBtn = document.getElementById("flag-key-apply-btn");

const colVanilla = document.getElementById("col-vanilla");
const colMapShuffle = document.getElementById("col-map-shuffle");

// --- Overworld mode helpers ---

function getOverworldMode() {
	return document.querySelector('input[name="overworld-mode"]:checked')?.value || "map_shuffle";
}
function setOverworldMode(val) {
	const el = document.querySelector(`input[name="overworld-mode"][value="${val}"]`);
	if (el) el.checked = true;
	updateOverworldColumns();
}
function getLevelShuffle() {
	return document.querySelector('input[name="level-shuffle"]:checked')?.value || "off";
}
function setLevelShuffle(val) {
	const el = document.querySelector(`input[name="level-shuffle"][value="${val}"]`);
	if (el) el.checked = true;
}

function updateOverworldColumns() {
	const mode = getOverworldMode();
	colVanilla.classList.toggle("disabled", mode !== "vanilla");
	colMapShuffle.classList.toggle("disabled", mode !== "map_shuffle");
}

// Dynamically populate Starting Lives dropdown (5, 10, 15, ... 99)
if (optStartingLives) {
	for (let i = 5; i <= 99; i += 5) {
		const option = document.createElement("option");
		option.value = i;
		option.textContent = i;
		if (i === 5) option.selected = true;
		optStartingLives.appendChild(option);
	}
}

// Initialize WASM
init()
	.then(() => {
		wasmReady = true;
		const versionEl = document.getElementById("version");
		if (versionEl) versionEl.textContent = `v${version()}`;
		updateGenerateButton();
		updateOverworldColumns();
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
		saveRom(romBytes).catch(() => {});
	};
	reader.onerror = () => {
		showStatus("Failed to read ROM file", "error");
	};
	reader.readAsArrayBuffer(file);
});

// Try to restore ROM from IndexedDB
loadRom().then((bytes) => {
	if (bytes) {
		romBytes = bytes;
		romLabel.textContent = "ROM loaded from cache";
		romLabel.classList.add("loaded");
		updateGenerateButton();
	}
}).catch(() => {});

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

	const options = getCurrentOptionsJson();

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
			filename = `smb3-rs_${seed}.nes`;
		} else {
			result = generate_patch(romBytes, seed, options);
			filename = `smb3-rs_${seed}.ips`;
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
	const isMapShuffle = getOverworldMode() === "map_shuffle";
	return JSON.stringify({
		powerups: optPowerups.checked,
		palettes: optPalettes.checked,
		world_order: optWorldOrder.checked,
		big_q_blocks: optBigQBlocks.checked,
		map_shuffle: isMapShuffle,
		level_shuffle: isMapShuffle ? "off" : getLevelShuffle(),
		shuffle_pipes: optShufflePipes.checked,
		shuffle_airships: optShuffleAirships.checked,
		chest_items: optChestItems.checked,
		remove_whistles: optRemoveWhistles.checked,
		airship_lock: optAirshipLock.checked,
		fix_drawbridges: optFixDrawbridges.checked,
		remove_rocks: optRemoveRocks.checked,
		remove_n_cards: optRemoveNCards.checked,
		remove_spade_games: optRemoveSpadeGames.checked,
		skip_wand_cutscene: optSkipWandCutscene.checked,
		adjust_boss_hitboxes: optAdjustBossHitboxes.checked,
		koopaling_hits: optKoopalingHits.checked,
		ground: getPill("opt-ground"),
		shell: getPill("opt-shell"),
		flying: getPill("opt-flying"),
		cheeps: getPill("opt-cheeps"),
		bullet_bills: getPill("opt-bullet-bills"),
		piranhas: getPill("opt-piranhas"),
		ghosts: getPill("opt-ghosts"),
		thwomps: getPill("opt-thwomps"),
		rotodiscs: getPill("opt-rotodiscs"),
		cannons: getPill("opt-cannons"),
		water: getPill("opt-water"),
		bros: getPill("opt-bros"),
		hb_encounters: getPill("opt-hb-encounters"),
		wild_injections: optWildInjections.checked,
		starting_lives: Number(optStartingLives.value),
		starting_items: optStartItems.map(s => Number(s.value)).filter(v => v > 0),
		disable_autoscroll: true,
		card_speed_clear: true,
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
		// palettes is cosmetic — not controlled by flag key, leave user's choice
		optWorldOrder.checked = opts.world_order;
		optBigQBlocks.checked = opts.big_q_blocks;
		setOverworldMode(opts.map_shuffle ? "map_shuffle" : "vanilla");
		setLevelShuffle(opts.level_shuffle);
		optShufflePipes.checked = opts.shuffle_pipes;
		if (opts.shuffle_airships !== undefined) optShuffleAirships.checked = opts.shuffle_airships;
		optChestItems.checked = opts.chest_items;
		optRemoveWhistles.checked = opts.remove_whistles;
		optAirshipLock.checked = opts.airship_lock;
		optFixDrawbridges.checked = opts.fix_drawbridges;
		optRemoveRocks.checked = opts.remove_rocks;
		if (opts.remove_n_cards !== undefined) optRemoveNCards.checked = opts.remove_n_cards;
		if (opts.remove_spade_games !== undefined) optRemoveSpadeGames.checked = opts.remove_spade_games;
		if (opts.skip_wand_cutscene !== undefined) optSkipWandCutscene.checked = opts.skip_wand_cutscene;
		if (opts.adjust_boss_hitboxes !== undefined) optAdjustBossHitboxes.checked = opts.adjust_boss_hitboxes;
		if (opts.koopaling_hits !== undefined) optKoopalingHits.checked = opts.koopaling_hits;
		if (opts.ground !== undefined) setPill("opt-ground", opts.ground);
		if (opts.shell !== undefined) setPill("opt-shell", opts.shell);
		if (opts.flying !== undefined) setPill("opt-flying", opts.flying);
		if (opts.cheeps !== undefined) setPill("opt-cheeps", opts.cheeps);
		if (opts.bullet_bills !== undefined) setPill("opt-bullet-bills", opts.bullet_bills);
		if (opts.piranhas !== undefined) setPill("opt-piranhas", opts.piranhas);
		if (opts.ghosts !== undefined) setPill("opt-ghosts", opts.ghosts);
		if (opts.thwomps !== undefined) setPill("opt-thwomps", opts.thwomps);
		if (opts.rotodiscs !== undefined) setPill("opt-rotodiscs", opts.rotodiscs);
		if (opts.cannons !== undefined) setPill("opt-cannons", opts.cannons);
		if (opts.water !== undefined) setPill("opt-water", opts.water);
		if (opts.bros !== undefined) setPill("opt-bros", opts.bros);
		if (opts.hb_encounters !== undefined) setPill("opt-hb-encounters", opts.hb_encounters);
		if (opts.wild_injections !== undefined) optWildInjections.checked = opts.wild_injections;
		if (opts.starting_lives) optStartingLives.value = opts.starting_lives;
		const items = opts.starting_items || [];
		for (let i = 0; i < 3; i++) {
			optStartItems[i].value = items[i] || 0;
		}
		updateOverworldColumns();
		showStatus("Flag key applied!", "success");
	} catch (err) {
		showStatus(`Invalid flag key: ${err}`, "error");
	}
}

// Update flag key whenever any option changes
const allOptionElements = [
	optPowerups, optWorldOrder, optBigQBlocks,
	optShufflePipes, optShuffleAirships, optChestItems, optRemoveWhistles,
	optAirshipLock,
	optFixDrawbridges, optRemoveRocks, optRemoveNCards, optRemoveSpadeGames, optSkipWandCutscene, optAdjustBossHitboxes, optKoopalingHits,
	optWildInjections,
	optStartingLives,
	...optStartItems,
];
for (const el of allOptionElements) {
	el.addEventListener("change", updateFlagKey);
}
// Pill group radios
for (const radio of document.querySelectorAll('.pill-group input[type="radio"]')) {
	radio.addEventListener("change", updateFlagKey);
}
// Radio groups
for (const name of ["overworld-mode", "level-shuffle"]) {
	for (const radio of document.querySelectorAll(`input[name="${name}"]`)) {
		radio.addEventListener("change", () => {
			updateOverworldColumns();
			updateFlagKey();
		});
	}
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
