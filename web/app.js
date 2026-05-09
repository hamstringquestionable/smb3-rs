import init, {
	generate_patch,
	generate_patched_rom,
	apply_visual_patch,
	build_ips_patch,
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
	getChangedFields,
	formatValue,
	applyOptions,
	saveSettings,
	restoreSettings,
	assertSchemaParity,
	selfTestRoundTrip,
} from "./options.js";

let wasmReady = false;
let romBytes = null;

// Curated, hand-vetted visual IPS patches shipped with the app. To add one:
// drop the file in web/visual-patches/ and add an entry here. The patch is
// fetched lazily at generate time and only applied when output is a Patched
// ROM (IPS output is the diff of the randomization itself).
const VISUAL_PATCHES = [
	{
		id: "super_luigi_35th",
		label: "Super Luigi Bros. 3",
		path: "./visual-patches/super-luigi-35th.ips",
		author: "Mario_GMD",
		url: "https://www.romhacking.net/hacks/5328/",
		color: "#6dce56", // Luigi green
	},
	{
		id: "super_princess_peach",
		label: "Super Princess Peach",
		path: "./visual-patches/super-princess-peach.ips",
		author: "Zynk Oxhyde",
		url: "https://www.romhacking.net/hacks/6284/",
		color: "#e07db4", // Peach pink
	},
	{
		id: "super_toad",
		label: "Super Toad (Blue)",
		path: "./visual-patches/super-toad-josuecr4ft.ips",
		author: "JosueCr4ft",
		url: "https://mfgg.net/index.php?act=resdb&param=02&c=7&id=38435",
		color: "#3a3aff", // Toad blue
	},
];

const visualPatchCache = new Map(); // id → Promise<Uint8Array>

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

// One-time cleanup: earlier versions cached uploaded visual-patch bytes
// in IndexedDB. Selection is now persisted via localStorage instead, so
// drop the orphan key on first run after upgrade.
async function cleanupOrphanVisualPatch() {
	const db = await openDb();
	const tx = db.transaction(DB_STORE, "readwrite");
	tx.objectStore(DB_STORE).delete("visual_patch");
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
const visualPatchPills = document.getElementById("visual-patch-pills");
const visualPatchCredit = document.getElementById("visual-patch-credit");
const skipValidationWarning = document.getElementById("skip-validation-warning");
const changesSummaryToggle = document.getElementById("changes-summary-toggle");
const changesSummaryText = document.getElementById("changes-summary-text");
const changesSummaryList = document.getElementById("changes-summary-list");

// --- Options form: render schema, restore, wire listeners ---

renderOptions(optionsRoot, { "rom-extras": romExtras });
renderVisualPatchPills();
restoreSettings();
applyEnabledWhen();
updateSkipValidationWarning();
updateChangesSummary();
wireListeners(() => {
	updateFlagKey();
	saveSettings();
	updateSkipValidationWarning();
	updateChangesSummary();
});

changesSummaryToggle.addEventListener("click", () => {
	if (changesSummaryToggle.disabled) return;
	const expanded = changesSummaryToggle.getAttribute("aria-expanded") === "true";
	changesSummaryToggle.setAttribute("aria-expanded", expanded ? "false" : "true");
	changesSummaryList.hidden = expanded;
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

// --- Visual patch (curated catalog rendered as a pill group) ---

function renderVisualPatchPills() {
	const opts = [{ id: "", label: "None" }, ...VISUAL_PATCHES];
	visualPatchPills.replaceChildren();
	for (const opt of opts) {
		const inputId = `vp-${opt.id || "none"}`;
		const input = document.createElement("input");
		input.type = "radio";
		input.name = "visual-patch";
		input.id = inputId;
		input.value = opt.id;
		if (opt.id === "") input.checked = true; // default to None
		input.addEventListener("change", () => {
			saveSettings();
			updateVisualPatchAccent();
			updateVisualPatchCredit();
		});
		const label = document.createElement("label");
		label.htmlFor = inputId;
		label.textContent = opt.label;
		visualPatchPills.append(input, label);
	}
}

function selectedVisualPatchId() {
	const checked = document.querySelector('input[name="visual-patch"]:checked');
	return checked?.value || "";
}

function updateVisualPatchAccent() {
	const id = selectedVisualPatchId();
	const entry = id ? VISUAL_PATCHES.find((p) => p.id === id) : null;
	if (entry?.color) {
		visualPatchPills.style.setProperty("--pill-active", entry.color);
	} else {
		visualPatchPills.style.removeProperty("--pill-active");
	}
}

function updateVisualPatchCredit() {
	const id = selectedVisualPatchId();
	const entry = id ? VISUAL_PATCHES.find((p) => p.id === id) : null;
	if (!entry || (!entry.author && !entry.url)) {
		visualPatchCredit.hidden = true;
		visualPatchCredit.replaceChildren();
		return;
	}
	visualPatchCredit.replaceChildren();
	visualPatchCredit.append(`${entry.label} by ${entry.author ?? "unknown"} — `);
	if (entry.url) {
		const a = document.createElement("a");
		a.href = entry.url;
		a.target = "_blank";
		a.rel = "noopener";
		a.textContent = entry.url;
		visualPatchCredit.append(a);
	}
	visualPatchCredit.hidden = false;
}

function fetchVisualPatch(id) {
	if (visualPatchCache.has(id)) return visualPatchCache.get(id);
	const entry = VISUAL_PATCHES.find((p) => p.id === id);
	if (!entry) return Promise.reject(new Error(`unknown visual patch: ${id}`));
	const promise = fetch(entry.path).then((r) => {
		if (!r.ok) throw new Error(`HTTP ${r.status}`);
		return r.arrayBuffer().then((buf) => new Uint8Array(buf));
	});
	visualPatchCache.set(id, promise);
	return promise;
}

updateVisualPatchAccent();
updateVisualPatchCredit();
cleanupOrphanVisualPatch().catch(() => {});

// --- Seed + generate ---

randomSeedBtn.addEventListener("click", () => {
	seedInput.value = Math.floor(Math.random() * Number.MAX_SAFE_INTEGER).toString();
});

generateBtn.addEventListener("click", async () => {
	if (!wasmReady || !romBytes) return;

	const seedStr = seedInput.value.trim();
	const seed = seedStr
		? BigInt(seedStr)
		: BigInt(Math.floor(Math.random() * Number.MAX_SAFE_INTEGER));

	const options = getOptionsJson();
	const outputFormat = document.querySelector('input[name="output-format"]:checked').value;
	const visualPatchId = selectedVisualPatchId();

	showStatus("Generating...", "loading");

	try {
		let result, filename;
		const mimeType = "application/octet-stream";
		let visualLabel = "";

		// Apply visual IPS to the input ROM first so randomization layers on top
		// (matches CLI ordering). For IPS output, diff against the *original*
		// vanilla input so the resulting .ips contains both the visual swap and
		// the randomization changes — applying it to a fresh ROM gives the same
		// result as the .nes output.
		let inputBytes = romBytes;
		if (visualPatchId) {
			const entry = VISUAL_PATCHES.find((p) => p.id === visualPatchId);
			try {
				const patch = await fetchVisualPatch(visualPatchId);
				inputBytes = apply_visual_patch(romBytes, patch);
				visualLabel = entry?.label ?? visualPatchId;
			} catch (err) {
				showStatus(`Visual patch '${entry?.label ?? visualPatchId}' failed to load: ${err}`, "error");
				return;
			}
		}

		if (outputFormat === "rom") {
			result = generate_patched_rom(inputBytes, seed, options);
			filename = `smb3-rs_${seed}.nes`;
		} else {
			const finalBytes = generate_patched_rom(inputBytes, seed, options);
			result = build_ips_patch(romBytes, finalBytes);
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

		const visualSuffix = visualLabel ? ` + visual: ${visualLabel}` : "";
		showStatus(`Generated ${filename} (${result.length} bytes, seed: ${seed})${visualSuffix}`, "success");
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

function updateChangesSummary() {
	const changes = getChangedFields();
	const n = changes.length;
	changesSummaryText.textContent = n === 0
		? "All defaults"
		: n === 1 ? "1 change from defaults" : `${n} changes from defaults`;
	changesSummaryToggle.disabled = n === 0;
	if (n === 0) {
		changesSummaryList.hidden = true;
		changesSummaryToggle.setAttribute("aria-expanded", "false");
	}
	changesSummaryList.replaceChildren();
	for (const { entry, current } of changes) {
		const row = document.createElement("div");
		row.className = "change-row";
		const labelSpan = document.createElement("span");
		labelSpan.className = "change-label";
		labelSpan.textContent = entry.label;
		const valueSpan = document.createElement("strong");
		valueSpan.className = "change-value";
		valueSpan.textContent = formatValue(entry, current);
		const defaultSpan = document.createElement("span");
		defaultSpan.className = "change-default";
		defaultSpan.textContent = `(default ${formatValue(entry, entry.default)})`;
		row.append(labelSpan, ": ", valueSpan, " ", defaultSpan);
		changesSummaryList.appendChild(row);
	}
}
