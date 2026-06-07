// Single source of truth for the options form. Drives:
//   - HTML rendering
//   - JSON serialization for the WASM API
//   - Flag-key apply (writing options back to the DOM)
//   - Settings persistence (localStorage)
//   - Live flag-key updates (universal change listener)
//   - Sub-option visibility (enabledWhen)
//
// Adding a new option = one schema entry. The renderer, serializer,
// applier, and listener pick it up automatically. A load-time parity
// check against the Rust source-of-truth (default_options_json) flags
// drift in either direction via console.error.

const ITEM_OPTIONS = [
	{ value: 0, label: "None" },
	{ value: 1, label: "Mushroom" },
	{ value: 2, label: "Fire Flower" },
	{ value: 3, label: "Super Leaf" },
	{ value: 4, label: "Frog Suit" },
	{ value: 5, label: "Tanooki Suit" },
	{ value: 6, label: "Hammer Suit" },
	{ value: 7, label: "Cloud" },
	{ value: 8, label: "P-Wing" },
	{ value: 9, label: "Starman" },
	{ value: 10, label: "Anchor" },
	{ value: 11, label: "Hammer" },
	{ value: 12, label: "Whistle" },
	{ value: 13, label: "Music Box" },
	{ value: 14, label: "Random" },
	{ value: 15, label: "Random - No Whistle" },
	{ value: 16, label: "Random - Suit Only" },
];

// Starting-lives pill options — Mario power-up state names with the
// actual life count in brackets. Encodes to 2 bits in the flag key.
const STARTING_LIVES_OPTIONS = [
	{ value: 1,  label: "Small [1]" },
	{ value: 5,  label: "Super [5]" },
	{ value: 20, label: "Fire [20]" },
	{ value: 99, label: "Hammer [99]" },
];

const TRI = [
	{ value: "off", label: "Off" },
	{ value: "shuffle", label: "Shuffle" },
	{ value: "wild", label: "Wild" },
];

// Categories rendered as <fieldset> sections, in order.
export const GROUPS = [
	{ id: "map", label: "Map" },
	{ id: "enemies", label: "Enemies" },
	{ id: "bosses", label: "Bosses" },
	{ id: "items", label: "Items & Pickups" },
	{ id: "player", label: "Player" },
	{ id: "cosmetic", label: "Cosmetic", note: "Cosmetic — does not affect the seed or flag key." },
];

// Reused icon sets — referenced from multiple SCHEMA entries.
// All seven Koopalings; bound to every boss option that doesn't have a more
// specific icon, randomized per-entry per-page-load for visual variety.
const KOOPALINGS = [
	{ x: 1, y: 273, w: 24, h: 32, sheet: "bosses" },
	{ x: 1, y: 307, w: 24, h: 32, sheet: "bosses" },
	{ x: 1, y: 341, w: 24, h: 32, sheet: "bosses" },
	{ x: 1, y: 375, w: 24, h: 33, sheet: "bosses" },
	{ x: 1, y: 409, w: 24, h: 32, sheet: "bosses" },
	{ x: 1, y: 443, w: 24, h: 32, sheet: "bosses" },
	{ x: 1, y: 477, w: 24, h: 32, sheet: "bosses" },
];

// Schema. Field names match the Rust Options struct; the load-time parity
// check guarantees they stay aligned. inFlagKey is informational/UX only;
// the Rust flag-key encoder is what actually decides what's persisted.
//
// Within each group, entries render in SCHEMA order, so keep each group's
// entries contiguous and ordered for display.
export const SCHEMA = [
	// --- ROM section (rendered into a separate host above the fieldsets) ---
	{ id: "skip_rom_validation", type: "bool", default: false,
		label: "Skip ROM validation (advanced)",
		tip: "Allow modded or translated ROMs by skipping integrity checks. Disables the seed verification icons on the title screen.",
		group: "rom-extras", host: "rom-extras", inFlagKey: false },

	// --- Map ---
	// Icons are clipped from web/assets/sprites.png. Coordinates picked via
	// web/sprite-picker.html. Format: { x, y, w, h } in sprite-sheet pixels.
	{ id: "shuffle_pipes", type: "bool", default: true,
		label: "Pipe Shuffle",
		tip: "Shuffle pipe endpoint positions on the overworld map",
		group: "map", inFlagKey: true },
	{ id: "shuffle_spade_games", type: "bool", default: true,
		label: "Shuffle Spade Games",
		tip: "Move spade (card-matching) games to random spots on the map",
		group: "map", inFlagKey: true },
	{ id: "shuffle_toad_houses", type: "bool", default: true,
		label: "Shuffle Toad Houses",
		tip: "Move Toad Houses to random spots across all worlds. Items inside are still randomized.",
		group: "map", inFlagKey: true },
	{ id: "infinite_mushroom_houses", type: "bool", default: false,
		label: "Infinite Mushroom Houses",
		tip: "Toad / Mushroom Houses don't disappear after entering — visit them any number of times. Credit: MaCobra52.",
		group: "map", inFlagKey: true },
	{ id: "fast_mushroom_house", type: "bool", default: false,
		label: "Fast Mushroom House",
		tip: "Skip the entry animation and shorten the exit when using a Toad / Mushroom House. Credit: MaCobra52.",
		group: "map", inFlagKey: true },
	{ id: "shuffle_airships", type: "bool", default: true,
		label: "Shuffle Airships",
		tip: "Shuffle airship levels across worlds 1-7",
		group: "map", inFlagKey: true },
	{ id: "hands_levels", type: "bool", default: true,
		label: "Hand-Trap Levels", flavor: "It's a trap!",
		tip: "Add visible hand-trap tiles. Walking onto one grabs you and pulls you into a level.",
		group: "map", inFlagKey: true },
	{ id: "troll_pipes", type: "bool", default: true,
		label: "Troll Pipes", flavor: "Looks like a pipe…",
		tip: "Disguise one level per world (W2-W8) as a pipe. You can walk past freely, but pressing A loads the hidden level.",
		group: "map", inFlagKey: true },
	{ id: "swap_start_airship", type: "bool", default: false,
		label: "Swap Start / Airship", flavor: "Beat the map backwards.",
		tip: "Each of Worlds 1-7 has a 50% chance to be played in reverse — Mario spawns where the airship usually lands.",
		group: "map", inFlagKey: true },
	{ id: "anchor_visuals", type: "bool", default: false,
		label: "Oops all Anchors", flavor: "Anchors aweigh.",
		tip: "Every item in your inventory looks like an Anchor. It still works the same — a mushroom still grows you.",
		group: "map", inFlagKey: false },
	{ id: "include_beta_stages", type: "bool", default: false,
		label: "Include Beta Stages",
		tip: "Adds 9 stages previously not included in the vanilla game.",
		group: "map", inFlagKey: true },
	{ id: "remove_rocks", type: "bool", default: true,
		label: "Remove Rocks",
		tip: "Remove rocks blocking paths in W2 (secret path), W3 (boat dock), and W4 (pipe shortcut)",
		group: "map", inFlagKey: true },
	{ id: "w1_hammer_rock", type: "bool", default: false,
		label: "W1 Hammer Rock",
		tip: "Place a hammer-breakable rock next to the W1 toad house to provide a shortcut",
		group: "map", inFlagKey: true },
	{ id: "remove_n_cards", type: "bool", default: true,
		label: "Remove N-Cards",
		tip: "Remove the N-card (N-Spade) bonus games from the overworld map",
		group: "map", inFlagKey: true },
	{ id: "world_order", type: "bool", default: false,
		label: "World Order",
		tip: "Shuffle the order you progress through Worlds 1-8",
		group: "map", inFlagKey: true },
	{ id: "world_count", type: "tri", numeric: true,
		options: [1,2,3,4,5,6,7].map(n => ({ value: n, label: String(n) })),
		default: 7,
		label: "World Count",
		tip: "Number of worlds before Dark Land (fewer = shorter game)",
		group: "map", inFlagKey: true,
		enabledWhen: { world_order: true } },

	// --- Enemies ---
	{ id: "ground", type: "tri", options: TRI, default: "shuffle",
		label: "Ground",
		tip: "Ground-walking enemies (Goomba, Spiny, Spike, etc.)",
		group: "enemies", inFlagKey: true },
	{ id: "shell", type: "tri", options: TRI, default: "shuffle",
		label: "Shell",
		tip: "Shelled enemies (Koopa, Buzzy Beetle, etc.)",
		group: "enemies", inFlagKey: true },
	{ id: "flying", type: "tri", options: TRI, default: "shuffle",
		label: "Flying",
		tip: "Flying/hopping enemies (Paratroopa, Paragoomba, etc.)",
		group: "enemies", inFlagKey: true },
	{ id: "piranhas", type: "tri", options: TRI, default: "shuffle",
		label: "Piranhas",
		tip: "Piranha plant variants (upward and ceiling)",
		group: "enemies", inFlagKey: true },
	{ id: "ghosts", type: "tri", options: TRI, default: "shuffle",
		label: "Ghosts",
		tip: "Ghost house enemies (Boo, Hot Foot)",
		group: "enemies", inFlagKey: true },
	{ id: "thwomps", type: "tri", options: TRI, default: "off",
		label: "Thwomps",
		tip: "Thwomp movement variants (diagonal slides, sideways, up-down)",
		group: "enemies", inFlagKey: true },
	{ id: "rotodiscs", type: "tri", options: TRI, default: "off",
		label: "Rotodiscs",
		tip: "Rotodisc rotation variants (single/dual, CW/CCW)",
		group: "enemies", inFlagKey: true },
	{ id: "cannons", type: "tri", options: TRI, default: "off",
		label: "Cannons",
		tip: "Cannons, Bullet Bill launchers, goomba pipes, and bob-omb launchers. Shuffle keeps fire direction; Wild lets any cannon become any other.",
		group: "enemies", inFlagKey: true },
	{ id: "water", type: "tri", options: TRI, default: "shuffle",
		label: "Water",
		tip: "Water enemies (Blooper, Big Bertha, etc.)",
		group: "enemies", inFlagKey: true },
	{ id: "bros", type: "tri", options: TRI, default: "shuffle",
		label: "Bros",
		tip: "Hammer / Boomerang / Fire Bros inside levels",
		group: "enemies", inFlagKey: true },
	{ id: "hb_encounters", type: "tri", options: TRI, default: "off",
		label: "HB Encounters",
		tip: "All enemies in overworld Hammer Bro mini-battles",
		group: "enemies", inFlagKey: true },
	{ id: "wild_injections", type: "bool", default: false,
		label: "Wild Injections",
		tip: "Spawn Lakitu, Angry Sun, or Boss Bass in ~15% of levels",
		group: "enemies", inFlagKey: true },
	{ id: "early_sun", type: "bool", default: false,
		label: "Early Sun",
		tip: "Angry Sun starts attacking immediately on spawn. Credit: MaCobra52.",
		group: "enemies", inFlagKey: true },

	// --- Bosses ---
	{ id: "random_koopalings", type: "bool", default: false,
		label: "Random Koopalings",
		tip: "Shuffle which Koopaling appears in each world. Each keeps its own moves and abilities. Thanks to fcoughlin (Fred) for the patch.",
		icon: KOOPALINGS,
		group: "bosses", inFlagKey: true },
	{ id: "koopaling_hits", type: "bool", default: true,
		label: "Random Koopaling Stomps",
		tip: "Each Koopaling takes a random number of stomps (1–5) instead of the usual 3",
		icon: KOOPALINGS,
		group: "bosses", inFlagKey: true },
	{ id: "hammer_vulnerable_koopalings", type: "bool", default: false,
		label: "Hammer Vulnerable Koopalings",
		tip: "Koopalings can be damaged by thrown hammers (normally hammers pass through them)",
		icon: { x: 543, y: 364, w: 16, h: 16 },
		group: "bosses", inFlagKey: true },
	{ id: "adjust_boss_hitboxes", type: "bool", default: true,
		label: "Adjust Boss Hitboxes",
		tip: "Adjust Bowser and Koopaling hitboxes so they're easier to hit",
		icon: { x: 171, y: 511, w: 32, h: 44, sheet: "bosses" }, // Bowser
		group: "bosses", inFlagKey: true },
	{ id: "skip_wand_cutscene", type: "bool", default: true,
		label: "Skip Wand Cutscene", flavor: "Jump Up, Super Star!",
		tip: "Skip the wand falling cutscene after defeating a Koopaling — jump to grab the wand instead",
		icon: { x: 435, y: 328, w: 16, h: 16 },
		group: "bosses", inFlagKey: true },

	// --- Items & Pickups ---
	// (sprite curation deferred — sprite CHR uses dynamic banking that requires
	//  per-object disassembly chasing. Map tiles below have known static banks.)
	{ id: "powerups", type: "bool", default: true,
		label: "Power-ups",
		tip: "Randomize ? block and brick block contents, keeping each roughly the same tier",
		// Random pick on each page load — flavor for "what will you get?"
		icon: [
			{ x: 435, y: 364, w: 16, h: 16 }, // mushroom
			{ x: 453, y: 364, w: 16, h: 16 }, // fire flower
			{ x: 471, y: 364, w: 16, h: 16 }, // super leaf
			{ x: 489, y: 364, w: 16, h: 16 }, // star
			{ x: 507, y: 364, w: 16, h: 16 }, // frog suit
			{ x: 525, y: 364, w: 16, h: 16 }, // tanooki suit
			{ x: 543, y: 364, w: 16, h: 16 }, // hammer suit
		],
		group: "items", inFlagKey: true },
	{ id: "chest_items", type: "bool", default: true,
		label: "Chest Items",
		tip: "Randomize chest and Toad House reward items",
		icon: { x: 525, y: 292, w: 16, h: 16 },
		group: "items", inFlagKey: true },
	{ id: "big_q_blocks", type: "bool", default: false,
		label: "Big ? Blocks",
		tip: "Randomize the contents of Big ? Blocks in bonus rooms",
		icon: { x: 435, y: 170, w: 32, h: 32 },
		group: "items", inFlagKey: true },
	{ id: "airship_lock", type: "bool", default: true,
		label: "Remove Anchor",
		tip: "Anchors become random power-ups, and airships stay put after losing instead of moving.",
		icon: { x: 651, y: 364, w: 16, h: 16 },
		group: "items", inFlagKey: true },
	{ id: "remove_whistles", type: "bool", default: true,
		label: "Remove Warp Whistles",
		tip: "Remove warp whistles so all worlds must be played",
		icon: { x: 561, y: 364, w: 16, h: 16 },
		group: "items", inFlagKey: true },
	{ id: "hammer_breaks_locks", type: "bool", default: false,
		label: "Hammer Breaks Locks",
		tip: "Hammer item also breaks fortress locks on the overworld map",
		icon: { x: 615, y: 364, w: 16, h: 16 },
		group: "items", inFlagKey: true },
	{ id: "hammer_breaks_bridges", type: "bool", default: false,
		label: "Hammer Breaks Bridges",
		tip: "Hammer item builds bridges across water gaps on the overworld map",
		icon: { x: 615, y: 364, w: 16, h: 16 },
		group: "items", inFlagKey: true },

	// --- Player ---
	{ id: "starting_lives", type: "tri", numeric: true,
		options: STARTING_LIVES_OPTIONS, default: 5,
		label: "Starting Lives",
		tip: "Number of lives you start with. The label is Mario's power-up state; the bracketed number is the actual count.",
		group: "player", inFlagKey: true },
	{ id: "japanese_damage", type: "bool", default: false,
		label: "Japanese Damage System",
		tip: "Taking damage drops you straight to Small Mario instead of demoting one tier at a time. Credit: MaCobra52.",
		group: "player", inFlagKey: true },
	{ id: "faster_tail_speed", type: "bool", default: false,
		label: "Faster Tail Speed",
		tip: "Speeds up the Raccoon / Tanooki tail swipe so you barely slow down using it. Slightly tweaks raccoon flight to keep level design intact. Credit: MaCobra52.",
		group: "player", inFlagKey: true },
	{ id: "no_game_over_penalty", type: "bool", default: false,
		label: "No Game Over Penalty",
		tip: "Game Over no longer wipes your inventory, map progress, or cards — continue picks up where you left off. Credit: MaCobra52.",
		group: "player", inFlagKey: true },
	{ id: "faster_frog", type: "bool", default: false,
		label: "Faster Frog",
		tip: "Speeds up swimming and running while wearing the Frog Suit.",
		group: "player", inFlagKey: true },
	{ id: "starting_items", type: "items",
		items: ITEM_OPTIONS, slots: 3,
		default: [],
		label: "Starting Items",
		note: "Choose up to 3 items to start with in your inventory.",
		group: "player", inFlagKey: true },

	// --- Cosmetic (does not affect seed or flag key) ---
	{ id: "palettes", type: "bool", default: true,
		label: "Palettes",
		tip: "Randomize character, lava, and Bowser color palettes",
		group: "cosmetic", inFlagKey: false },
	{ id: "palette_themed", type: "bool", default: false,
		label: "Themed per-tileset",
		tip: "Also randomize background and enemy colors per area, using palettes from Super Mario Bros. 3 Recolored. Re-rolls freely (doesn't affect the seed).",
		group: "cosmetic", inFlagKey: false,
		enabledWhen: { palettes: true }, indent: true },
];

// Hardcoded fields sent to Rust that aren't user-facing.
const CONSTANT_FIELDS = {
	disable_autoscroll: true,
	card_speed_clear: true,
};

// --- DOM helpers ---

const DOM_PREFIX = "opt-";
const domId = (id) => `${DOM_PREFIX}${id.replaceAll("_", "-")}`;
const radioName = (id) => `${DOM_PREFIX}${id.replaceAll("_", "-")}`;

function el(tag, attrs = {}, ...children) {
	const node = document.createElement(tag);
	for (const [k, v] of Object.entries(attrs)) {
		if (v === false || v == null) continue;
		if (k === "class") node.className = v;
		else if (k === "html") node.innerHTML = v;
		else if (k === "for") node.htmlFor = v;
		else if (k === "checked") node.checked = !!v;
		else if (k === "selected") node.selected = !!v;
		else if (k === "value") node.value = v;
		else if (k === "hidden" && v) node.hidden = true;
		else if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
		else node.setAttribute(k, v);
	}
	for (const c of children) {
		if (c == null) continue;
		node.appendChild(typeof c === "string" ? document.createTextNode(c) : c);
	}
	return node;
}

// --- Tip helpers ---

function tipBtn(entry) {
	if (!entry.tip) return null;
	return el("button", {
		type: "button",
		class: "tip-btn",
		"aria-label": "Show description",
		"aria-expanded": "false",
		"aria-controls": `tip-${entry.id}`,
		onclick: (e) => {
			e.preventDefault();
			e.stopPropagation();
			const tip = document.getElementById(`tip-${entry.id}`);
			const target = e.currentTarget;
			const expanded = target.getAttribute("aria-expanded") === "true";
			target.setAttribute("aria-expanded", expanded ? "false" : "true");
			if (tip) tip.hidden = expanded;
		},
	}, "?");
}

function tipBlock(entry) {
	if (!entry.tip) return null;
	return el("div", { id: `tip-${entry.id}`, class: "option-tip", hidden: true }, entry.tip);
}

// Optional sprite icon next to an option. Returns a canvas at the icon's
// natural pixel size with a known DOM id; app.js paints it from the bundled
// sprite sheets at startup. If entry.icon is unset, returns null and the
// caller skips the slot.
//
// Sheets are declared in web/sprites.js (see SHEETS). The default sheet is
// web/assets/sprites.png — to use a different one, add `sheet: "bosses"` (or
// "enemies") to the icon spec.
//
// To add an icon to an option:
//   1. Open web/sprite-picker.html in the browser.
//   2. Pick the sheet from the dropdown, then click-and-drag a tight
//      rectangle around the sprite.
//   3. Click "Copy as JSON" and paste the {x,y,w,h} (with `sheet` if not the
//      default) into the schema entry's `icon` field below.
//   4. `icon` accepts either a single {x,y,w,h[,sheet]} object or an array of
//      them (random pick at page load — used by Power-ups for variety).
function iconCanvas(entry) {
	if (!entry.icon) return null;
	const first = Array.isArray(entry.icon) ? entry.icon[0] : entry.icon;
	return el("canvas", {
		class: "opt-icon",
		id: `icon-${entry.id}`,
		"data-icon": entry.id,
		width: first.w ?? 16,
		height: first.h ?? 16,
	});
}

// --- Renderers per type ---

function renderBool(entry) {
	const wrap = el("label", { class: "checkbox-label" + (entry.indent ? " sub-options" : "") });
	wrap.appendChild(el("input", { type: "checkbox", id: domId(entry.id), checked: entry.default }));
	const icon = iconCanvas(entry);
	if (icon) wrap.appendChild(icon);
	wrap.appendChild(document.createTextNode(" " + entry.label));
	if (entry.flavor) {
		wrap.appendChild(el("span", { class: "option-flavor" }, entry.flavor));
	}
	const btn = tipBtn(entry);
	if (btn) wrap.appendChild(btn);
	return wrap;
}

function renderTri(entry) {
	const wrap = el("label", { class: "select-label" });
	const icon = iconCanvas(entry);
	if (icon) wrap.appendChild(icon);
	wrap.appendChild(document.createTextNode(entry.label));
	const btn = tipBtn(entry);
	if (btn) wrap.appendChild(btn);
	const group = el("div", { class: "pill-group" });
	for (const opt of entry.options) {
		const inputId = `${domId(entry.id)}-${opt.value}`;
		group.appendChild(el("input", {
			type: "radio", name: radioName(entry.id), id: inputId,
			value: opt.value, checked: opt.value === entry.default,
		}));
		group.appendChild(el("label", { for: inputId }, opt.label));
	}
	wrap.appendChild(group);
	return wrap;
}

function renderSelect(entry) {
	const wrap = el("label", {
		class: "select-label" + (entry.indent ? " sub-options" : ""),
		id: `${domId(entry.id)}-label`,
	});
	wrap.appendChild(document.createTextNode(entry.label));
	const btn = tipBtn(entry);
	if (btn) wrap.appendChild(btn);
	const select = el("select", { id: domId(entry.id) });
	for (const opt of entry.options) {
		select.appendChild(el("option", {
			value: opt.value,
			selected: opt.value === entry.default,
		}, opt.label));
	}
	wrap.appendChild(select);
	return wrap;
}

function renderRadio(entry) {
	const wrap = el("div", { class: "radio-group-vertical" + (entry.indent ? " sub-options" : "") });
	if (entry.label) {
		const header = el("div", { class: "option-header" }, entry.label);
		const btn = tipBtn(entry);
		if (btn) header.appendChild(btn);
		wrap.appendChild(header);
	}
	for (const opt of entry.options) {
		const inputId = `${domId(entry.id)}-${opt.value}`;
		const label = el("label", { class: "radio-label" });
		label.appendChild(el("input", {
			type: "radio", name: radioName(entry.id), id: inputId,
			value: opt.value, checked: opt.value === entry.default,
		}));
		label.appendChild(document.createTextNode(" " + opt.label));
		if (opt.desc) {
			label.appendChild(el("span", { class: "option-desc" }, opt.desc));
		}
		wrap.appendChild(label);
	}
	return wrap;
}

function renderItems(entry) {
	const frag = document.createDocumentFragment();
	if (entry.note) {
		frag.appendChild(el("p", { class: "note", style: "margin-bottom:0.5rem" }, entry.note));
	}
	for (let i = 0; i < entry.slots; i++) {
		const wrap = el("label", { class: "select-label" });
		wrap.appendChild(document.createTextNode(`Slot ${i + 1}`));
		const select = el("select", { id: `${domId(entry.id)}-${i}` });
		for (const opt of entry.items) {
			select.appendChild(el("option", { value: opt.value }, opt.label));
		}
		wrap.appendChild(select);
		frag.appendChild(wrap);
	}
	return frag;
}

const RENDERERS = {
	bool: renderBool,
	tri: renderTri,
	select: renderSelect,
	radio: renderRadio,
	items: renderItems,
};

function renderEntry(entry) {
	const r = RENDERERS[entry.type];
	if (!r) throw new Error(`Unknown schema type: ${entry.type}`);
	const node = r(entry);
	const block = tipBlock(entry);
	if (!block) return node;
	const frag = document.createDocumentFragment();
	frag.appendChild(node);
	frag.appendChild(block);
	return frag;
}

// --- Public API ---

export function renderOptions(rootEl, hosts = {}) {
	for (const group of GROUPS) {
		const fieldset = el("fieldset", { class: "section", id: `group-${group.id}` });
		fieldset.appendChild(el("legend", {}, group.label));
		if (group.note) {
			fieldset.appendChild(el("p", { class: "note group-note" }, group.note));
		}
		const entries = SCHEMA.filter(s => s.group === group.id && !s.host);
		for (const entry of entries) {
			fieldset.appendChild(renderEntry(entry));
		}
		rootEl.appendChild(fieldset);
	}
	for (const entry of SCHEMA) {
		if (!entry.host) continue;
		const host = hosts[entry.host];
		if (!host) {
			console.warn(`Schema entry ${entry.id} expects host ${entry.host}, none provided`);
			continue;
		}
		host.appendChild(renderEntry(entry));
	}
}

export function readValue(entry) {
	switch (entry.type) {
		case "bool": {
			return document.getElementById(domId(entry.id))?.checked ?? entry.default;
		}
		case "tri":
		case "radio": {
			const checked = document.querySelector(`input[name="${radioName(entry.id)}"]:checked`);
			const v = checked?.value ?? entry.default;
			return entry.numeric ? Number(v) : v;
		}
		case "select": {
			const v = document.getElementById(domId(entry.id))?.value ?? entry.default;
			return entry.numeric ? Number(v) : v;
		}
		case "items": {
			const out = [];
			for (let i = 0; i < entry.slots; i++) {
				const v = Number(document.getElementById(`${domId(entry.id)}-${i}`)?.value ?? 0);
				if (v > 0) out.push(v);
			}
			return out;
		}
	}
}

export function writeValue(entry, value) {
	if (value === undefined) return;
	switch (entry.type) {
		case "bool": {
			const e = document.getElementById(domId(entry.id));
			if (e) e.checked = !!value;
			break;
		}
		case "tri":
		case "radio": {
			const e = document.querySelector(`input[name="${radioName(entry.id)}"][value="${value}"]`);
			if (e) e.checked = true;
			break;
		}
		case "select": {
			const e = document.getElementById(domId(entry.id));
			if (e) e.value = String(value);
			break;
		}
		case "items": {
			const arr = Array.isArray(value) ? value : [];
			for (let i = 0; i < entry.slots; i++) {
				const e = document.getElementById(`${domId(entry.id)}-${i}`);
				if (e) e.value = String(arr[i] ?? 0);
			}
			break;
		}
	}
}

export function getOptions() {
	const out = { ...CONSTANT_FIELDS };
	for (const entry of SCHEMA) {
		out[entry.id] = readValue(entry);
	}
	return out;
}

// Walk the schema, return the entries whose current value differs from
// the schema default. Used by the changes-summary UI in the control panel.
export function getChangedFields() {
	const changed = [];
	for (const entry of SCHEMA) {
		const current = readValue(entry);
		if (!valuesEqual(current, entry.default)) {
			changed.push({ entry, current });
		}
	}
	return changed;
}

function valuesEqual(a, b) {
	if (Array.isArray(a) && Array.isArray(b)) {
		return a.length === b.length && a.every((v, i) => v === b[i]);
	}
	return a === b;
}

// Human-readable rendering of a field value for the changes summary.
export function formatValue(entry, value) {
	switch (entry.type) {
		case "bool": return value ? "ON" : "OFF";
		case "tri":
		case "radio":
		case "select": {
			const opt = entry.options.find(o => o.value === value);
			return opt ? opt.label : String(value);
		}
		case "items": {
			if (!Array.isArray(value) || value.length === 0) return "(none)";
			return value.map(v => {
				const opt = entry.items.find(o => o.value === v);
				return opt ? opt.label : String(v);
			}).join(", ");
		}
		default: return String(value);
	}
}

export function getOptionsJson() {
	return JSON.stringify(getOptions());
}

// Apply a decoded flag-key payload back to the DOM. Skips non-flag-key
// fields (palettes, palette_themed, skip_rom_validation) so applying a
// shared key doesn't clobber the user's local cosmetic / ROM choices.
export function applyOptions(opts) {
	for (const entry of SCHEMA) {
		if (!entry.inFlagKey) continue;
		writeValue(entry, opts[entry.id]);
	}
}

export function applyEnabledWhen() {
	for (const entry of SCHEMA) {
		if (!entry.enabledWhen) continue;
		const enabled = Object.entries(entry.enabledWhen).every(
			([id, want]) => {
				const e = SCHEMA.find(s => s.id === id);
				return e && readValue(e) === want;
			},
		);
		applyEntryEnabled(entry, enabled);
	}
}

function applyEntryEnabled(entry, enabled) {
	const ids = entryDomIds(entry);
	for (const id of ids) {
		const elNode = document.getElementById(id);
		if (!elNode) continue;
		elNode.disabled = !enabled;
		// Walk up to the wrapping label/div so the visual styling matches today
		const wrap = elNode.closest("label, .radio-group-vertical, .pill-group");
		if (wrap) wrap.classList.toggle("disabled", !enabled);
	}
}

function entryDomIds(entry) {
	switch (entry.type) {
		case "bool":
		case "select":
			return [domId(entry.id)];
		case "tri":
		case "radio":
			return entry.options.map(o => `${domId(entry.id)}-${o.value}`);
		case "items":
			return Array.from({ length: entry.slots }, (_, i) => `${domId(entry.id)}-${i}`);
		default:
			return [];
	}
}

// Wire one universal change listener that fires on every schema-driven input.
export function wireListeners(onChange) {
	for (const entry of SCHEMA) {
		for (const id of entryDomIds(entry)) {
			const node = document.getElementById(id);
			if (!node) continue;
			node.addEventListener("change", () => {
				applyEnabledWhen();
				onChange(entry);
			});
		}
	}
}

// --- Persistence (localStorage) ---
//
// Uses DOM ids as keys so existing user settings written by the pre-refactor
// version still restore. Only writes/reads schema-driven inputs; non-schema
// state (output format, ROM, visual patch) is handled by app.js.

const SETTINGS_KEY = "smb3r-settings";

export function saveSettings() {
	try {
		const settings = {};
		for (const entry of SCHEMA) {
			const v = readValue(entry);
			if (entry.type === "tri" || entry.type === "radio") {
				settings[`radio:${radioName(entry.id)}`] = v;
			} else if (entry.type === "items") {
				for (let i = 0; i < entry.slots; i++) {
					const node = document.getElementById(`${domId(entry.id)}-${i}`);
					if (node) settings[node.id] = node.value;
				}
			} else {
				settings[domId(entry.id)] = entry.type === "bool" ? !!v : String(v);
			}
		}
		// Static radios that live outside the schema (rendered/managed by app.js).
		for (const name of ["output-format", "visual-patch"]) {
			const el = document.querySelector(`input[name="${name}"]:checked`);
			if (el) settings[`radio:${name}`] = el.value;
		}
		localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
	} catch (_) {}
}

export function restoreSettings() {
	try {
		const raw = localStorage.getItem(SETTINGS_KEY);
		if (!raw) return;
		const settings = JSON.parse(raw);
		for (const [key, val] of Object.entries(settings)) {
			if (key.startsWith("radio:")) {
				const name = key.slice(6);
				const elNode = document.querySelector(`input[name="${name}"][value="${val}"]`);
				if (elNode) elNode.checked = true;
			} else {
				const elNode = document.getElementById(key);
				if (!elNode) continue;
				if (elNode.type === "checkbox") elNode.checked = val === true || val === "true";
				else elNode.value = val;
			}
		}
	} catch (_) {}
}

// --- Parity check ---
//
// At load time, compare schema field ids against the Rust source-of-truth
// (via wasm `default_options_json`). Any drift is shouted via console.error
// so the developer notices on the next refresh.

export function assertSchemaParity(wasmDefaultsJson) {
	let defaults;
	try {
		defaults = JSON.parse(wasmDefaultsJson);
	} catch (e) {
		console.error("Schema parity: could not parse wasm defaults", e);
		return;
	}
	const schemaIds = new Set(SCHEMA.map(s => s.id));
	const wasmIds = new Set(Object.keys(defaults));
	// Hardcoded fields are ours to set, not user-facing — exclude from the diff
	for (const c of Object.keys(CONSTANT_FIELDS)) wasmIds.delete(c);
	const missingInJs = [...wasmIds].filter(id => !schemaIds.has(id));
	const missingInRust = [...schemaIds].filter(id => !wasmIds.has(id));
	if (missingInJs.length || missingInRust.length) {
		console.error("Options schema drift detected", { missingInJs, missingInRust });
	}
}

// --- Round-trip self-test ---
//
// Take the current options, encode to flag key via WASM, decode back, and
// diff. Catches "I added a JS schema entry but forgot the Rust flag-key
// bits" or vice versa, without anyone having to run cargo test.

export function selfTestRoundTrip(encode, decode) {
	try {
		const before = getOptions();
		const onlyFlagKey = Object.fromEntries(
			Object.entries(before).filter(([k]) => {
				const e = SCHEMA.find(s => s.id === k);
				return !e || e.inFlagKey !== false;
			}),
		);
		const key = encode(JSON.stringify(before));
		const decoded = JSON.parse(decode(key));
		const drift = [];
		for (const [k, v] of Object.entries(onlyFlagKey)) {
			if (k in decoded && JSON.stringify(decoded[k]) !== JSON.stringify(v)) {
				drift.push({ field: k, before: v, after: decoded[k] });
			}
		}
		if (drift.length) {
			console.error("Flag-key round-trip drift", drift);
		}
	} catch (e) {
		console.error("Self-test failed", e);
	}
}
