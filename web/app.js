import init, {
	generate_patch,
	generate_patched_rom,
	apply_visual_patch,
	encode_flag_key,
	decode_flag_key,
	default_options_json,
	version,
} from "./pkg/smb3_rs.js";
import {
	renderOptions,
	wireListeners,
	applyEnabledWhen,
	getOptionsJson,
	applyOptions,
	saveSettings,
	restoreSettings,
	assertSchemaParity,
	selfTestRoundTrip,
} from "./options.js";

let wasmReady = false;
let romBytes = null;
let visualPatchBytes = null;

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

// --- DOM lookups (static, non-schema elements) ---

const romInput = document.getElementById("rom-input");
const romLabel = document.getElementById("rom-label");
const romExtras = document.getElementById("rom-extras");
const optionsRoot = document.getElementById("options-root");
const seedInput = document.getElementById("seed-input");
const randomSeedBtn = document.getElementById("random-seed-btn");
const generateBtn = document.getElementById("generate-btn");
const statusDiv = document.getElementById("status");
const flagKeyInput = document.getElementById("flag-key-input");
const flagKeyCopyBtn = document.getElementById("flag-key-copy-btn");
const flagKeyApplyBtn = document.getElementById("flag-key-apply-btn");
const shareUrlBtn = document.getElementById("share-url-btn");
const visualPatchInput = document.getElementById("visual-patch-input");
const visualPatchLabel = document.getElementById("visual-patch-label");
const visualPatchClear = document.getElementById("visual-patch-clear");
const skipValidationWarning = document.getElementById("skip-validation-warning");

// --- Options form: render schema, restore, wire listeners ---

renderOptions(optionsRoot, { "rom-extras": romExtras });
restoreSettings();
applyEnabledWhen();
updateSkipValidationWarning();
wireListeners(() => {
	updateFlagKey();
	saveSettings();
	updateSkipValidationWarning();
});

// Output-format radios live in static HTML, outside the schema.
for (const radio of document.querySelectorAll('input[name="output-format"]')) {
	radio.addEventListener("change", saveSettings);
}

// --- WASM init ---

init()
	.then(() => {
		wasmReady = true;
		const versionEl = document.getElementById("version");
		if (versionEl) versionEl.textContent = `v${version()}`;
		updateGenerateButton();
		updateFlagKey();
		assertSchemaParity(default_options_json());
		selfTestRoundTrip(encode_flag_key, decode_flag_key);
		applyUrlParams();
	})
	.catch((err) => {
		showStatus(`Failed to load WASM module: ${err}`, "error");
	});

// --- ROM file selection ---

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
	reader.onerror = () => showStatus("Failed to read ROM file", "error");
	reader.readAsArrayBuffer(file);
});

loadRom().then((bytes) => {
	if (bytes) {
		romBytes = bytes;
		romLabel.textContent = "ROM loaded from cache";
		romLabel.classList.add("loaded");
		updateGenerateButton();
	}
}).catch(() => {});

// --- Visual patch ---

visualPatchInput.addEventListener("change", (e) => {
	const file = e.target.files[0];
	if (!file) return;

	const reader = new FileReader();
	reader.onload = () => {
		visualPatchBytes = new Uint8Array(reader.result);
		visualPatchLabel.textContent = file.name;
		visualPatchLabel.classList.add("loaded");
		visualPatchClear.hidden = false;
	};
	reader.onerror = () => showStatus("Failed to read visual patch file", "error");
	reader.readAsArrayBuffer(file);
});

visualPatchClear.addEventListener("click", () => {
	visualPatchBytes = null;
	visualPatchInput.value = "";
	visualPatchLabel.textContent = "Select IPS file...";
	visualPatchLabel.classList.remove("loaded");
	visualPatchClear.hidden = true;
});

// --- Seed + generate ---

randomSeedBtn.addEventListener("click", () => {
	seedInput.value = Math.floor(Math.random() * Number.MAX_SAFE_INTEGER).toString();
});

generateBtn.addEventListener("click", () => {
	if (!wasmReady || !romBytes) return;

	const seedStr = seedInput.value.trim();
	const seed = seedStr
		? BigInt(seedStr)
		: BigInt(Math.floor(Math.random() * Number.MAX_SAFE_INTEGER));

	const options = getOptionsJson();
	const outputFormat = document.querySelector('input[name="output-format"]:checked').value;

	showStatus("Generating...", "loading");

	try {
		let result, filename;
		const mimeType = "application/octet-stream";

		if (outputFormat === "rom") {
			result = generate_patched_rom(romBytes, seed, options);
			if (visualPatchBytes) result = apply_visual_patch(result, visualPatchBytes);
			filename = `smb3-rs_${seed}.nes`;
		} else {
			result = generate_patch(romBytes, seed, options);
			filename = `smb3-rs_${seed}.ips`;
		}

		const blob = new Blob([result], { type: mimeType });
		const url = URL.createObjectURL(blob);
		const a = document.createElement("a");
		a.href = url;
		a.download = filename;
		document.body.appendChild(a);
		a.click();
		document.body.removeChild(a);
		URL.revokeObjectURL(url);

		if (window.goatcounter?.count) {
			const v = version();
			const fk = flagKeyInput.value;
			goatcounter.count({ path: `/generate/${v}/${fk}`, event: true });
		}

		showStatus(`Generated ${filename} (${result.length} bytes, seed: ${seed})`, "success");
	} catch (err) {
		showStatus(`Error: ${err}`, "error");
	}
});

// --- Flag Key ---

function updateFlagKey() {
	if (!wasmReady) return;
	try {
		flagKeyInput.value = encode_flag_key(getOptionsJson());
	} catch (_) {}
}

function applyFlagKey(key) {
	if (!wasmReady) return;
	try {
		const json = decode_flag_key(key.trim());
		applyOptions(JSON.parse(json));
		applyEnabledWhen();
		saveSettings();
		updateFlagKey();
		showStatus("Flag key applied!", "success");
	} catch (err) {
		showStatus(`Invalid flag key: ${err}`, "error");
	}
}

flagKeyCopyBtn.addEventListener("click", () => {
	updateFlagKey();
	navigator.clipboard.writeText(flagKeyInput.value).then(() => {
		showStatus("Flag key copied!", "success");
	});
});

flagKeyApplyBtn.addEventListener("click", () => applyFlagKey(flagKeyInput.value));

shareUrlBtn.addEventListener("click", () => {
	updateFlagKey();
	const params = new URLSearchParams();
	const seedStr = seedInput.value.trim();
	if (seedStr) params.set("seed", seedStr);
	if (flagKeyInput.value) params.set("flags", flagKeyInput.value);
	const url = `${location.origin}${location.pathname}?${params.toString()}`;
	navigator.clipboard.writeText(url).then(() => {
		showStatus("Share URL copied!", "success");
	});
});

function applyUrlParams() {
	const params = new URLSearchParams(location.search);
	const seed = params.get("seed");
	const flags = params.get("flags");
	if (seed) seedInput.value = seed;
	if (flags) {
		flagKeyInput.value = flags;
		applyFlagKey(flags);
	}
}

// --- Misc ---

function updateGenerateButton() {
	generateBtn.disabled = !(wasmReady && romBytes);
}

function showStatus(message, type) {
	statusDiv.textContent = message;
	statusDiv.className = `status ${type}`;
	statusDiv.hidden = false;
}

function updateSkipValidationWarning() {
	const skip = document.getElementById("opt-skip-rom-validation");
	if (skipValidationWarning && skip) {
		skipValidationWarning.hidden = !skip.checked;
	}
}
