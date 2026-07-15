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

import { NES_PALETTE } from "./chr.js";

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

// Off / On / Maybe pill for player-hidden flags. "Maybe" lets the seed
// decide on/off at generation time — deterministic, but unreadable from the
// flag key so the player can't plan around it. Values match the Rust `Tri`
// enum's serde representation.
const ON_OFF_MAYBE = [
	{ value: "off", label: "Off" },
	{ value: "on", label: "On" },
	{ value: "maybe", label: "Maybe" },
];

// Off / On / Wild pill for Random Fire Flower. "Wild" widens the pool to also
// include the Small/Big downgrade outcomes. Values match the Rust
// `FireFlowerMode` enum's serde representation.
const OFF_ON_WILD = [
	{ value: "off", label: "Off" },
	{ value: "on", label: "On" },
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
		tip: "Toad / Mushroom Houses don't disappear after entering — visit them any number of times.",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		group: "map", inFlagKey: true },
	{ id: "fast_mushroom_house", type: "bool", default: false,
		label: "Fast Mushroom House",
		tip: "Skip the entry animation and shorten the exit when using a Toad / Mushroom House.",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		group: "map", inFlagKey: true },
	{ id: "shuffle_airships", type: "bool", default: true,
		label: "Shuffle Airships",
		tip: "Shuffle airship levels across worlds 1-7",
		group: "map", inFlagKey: true },
	{ id: "shuffle_hammer_bros", type: "bool", default: true,
		label: "Shuffle HammerBro Locations",
		tip: "Spread the wandering Hammer Bros across all worlds (random spots, 1-3 per world) instead of their fixed vanilla locations.",
		group: "map", inFlagKey: true },
	{ id: "hands_levels", type: "bool", default: true,
		label: "Hand-Trap Levels", flavor: "It's a trap!",
		tip: "Add visible hand-trap tiles. Walking onto one grabs you and pulls you into a level.",
		group: "map", inFlagKey: true },
	{ id: "swap_start_airship", type: "bool", default: false,
		label: "Swap Start / Airship", flavor: "Beat the map backwards.",
		tip: "Each of Worlds 1-7 has a 50% chance to be played in reverse — Mario spawns where the airship usually lands.",
		group: "map", inFlagKey: true },
	{ id: "anchor_visuals", type: "bool", default: false,
		label: "Oops all Anchors", flavor: "Anchors aweigh.",
		tip: "Every item in your inventory looks like an Anchor. It still works the same — a mushroom still grows you.",
		group: "map", inFlagKey: true },
	{ id: "include_beta_stages", type: "bool", default: false,
		label: "Include Beta Stages",
		tip: "Adds 9 stages previously not included in the vanilla game.",
		group: "map", inFlagKey: true },
	{ id: "remove_n_cards", type: "bool", default: true,
		label: "Remove N-Cards",
		tip: "Remove the N-card (N-Spade) bonus games from the overworld map",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		group: "map", inFlagKey: true },
	{ id: "troll_pipes", type: "tri", options: ON_OFF_MAYBE, default: "on",
		label: "Troll Pipes", flavor: "Looks like a pipe…",
		tip: "Disguise one level per world (W2-W8) as a pipe. You can walk past freely, but pressing A loads the hidden level. Maybe: the seed secretly decides on or off, so you won't know until you play.",
		group: "map", inFlagKey: true },
	{ id: "more_hammer_rocks", type: "tri", options: ON_OFF_MAYBE, default: "off",
		label: "More hammer rocks",
		tip: "Add hammer-breakable rocks as shortcuts: one by the W1 toad house and one in W8. Maybe: the seed secretly decides on or off, so you won't know until you play.",
		group: "map", inFlagKey: true },
	{ id: "eights_are_wild", type: "tri", options: ON_OFF_MAYBE, default: "off",
		label: "8s are Wild",
		tip: "Open up World 8 with a canoe and extra paths. Maybe: the seed secretly decides on or off, so you won't know until you play.",
		group: "map", inFlagKey: true },
	{ id: "antechamber_shuffle", type: "tri", options: ON_OFF_MAYBE, default: "off",
		label: "Lobby Shuffle", flavor: "Wrong door…",
		tip: "Ten levels start with a pipe that leads into the level itself. Shuffle which of those levels each entrance drops you into — you finish through whichever level you land in. Maybe: the seed secretly decides on or off, so you won't know until you play.",
		group: "map", inFlagKey: true },
	{ id: "piranha_shuffle", type: "tri", options: OFF_ON_WILD, default: "off",
		label: "Piranha Shuffle",
		tip: "Free the two W7 piranha plant levels into the level shuffle. On: their plants travel with them, guarding wherever they land. Wild: the plants scatter instead — one lands on a random level in each world, and stepping on a plant starts the level under it.",
		group: "map", inFlagKey: true },
	{ id: "limit_bro_movement", type: "bool", default: false,
		label: "Limit Bro Movement",
		tip: "Gate Hammer Bro overworld Movements to increase race equality.",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
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
		tip: "Angry Sun starts attacking immediately on spawn.",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
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
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		icon: { x: 543, y: 364, w: 16, h: 16 },
		group: "bosses", inFlagKey: true },
	{ id: "adjust_boss_hitboxes", type: "bool", default: true,
		label: "Adjust Boss Hitboxes",
		tip: "Adjust Bowser and Koopaling hitboxes so they're easier to hit",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		icon: { x: 171, y: 511, w: 32, h: 44, sheet: "bosses" }, // Bowser
		group: "bosses", inFlagKey: true },
	{ id: "skip_wand_cutscene", type: "bool", default: true,
		label: "Skip Wand Cutscene", flavor: "Jump Up, Super Star!",
		tip: "Skip the wand falling cutscene after defeating a Koopaling — jump to grab the wand instead",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
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
	{ id: "fire_flower", type: "tri", options: OFF_ON_WILD, default: "off",
		label: "Random Fire Flower",
		tip: "Fire Flowers still look the same, but each one gives a different suit based on where it is. On: Fire, Frog, Tanooki, or Hammer. Wild: also lets it shrink you to Big or Small.",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		icon: { x: 453, y: 364, w: 16, h: 16 }, // fire flower
		group: "items", inFlagKey: true },
	{ id: "big_q_blocks", type: "bool", default: false,
		label: "Big ? Blocks",
		tip: "Randomize the contents of Big ? Blocks in bonus rooms",
		icon: { x: 435, y: 170, w: 32, h: 32 },
		group: "items", inFlagKey: true },
	{ id: "remove_whistles", type: "bool", default: true,
		label: "Remove Warp Whistles",
		tip: "Remove warp whistles so all worlds must be played",
		icon: { x: 561, y: 364, w: 16, h: 16 },
		group: "items", inFlagKey: true },
	{ id: "hammer_breaks_locks", type: "tri", options: ON_OFF_MAYBE, default: "off",
		label: "Hammer Breaks Locks",
		tip: "Hammer item also breaks fortress locks on the overworld map. Maybe: the seed secretly decides on or off, so you won't know until you play.",
		icon: { x: 615, y: 364, w: 16, h: 16 },
		group: "items", inFlagKey: true },
	{ id: "hammer_breaks_bridges", type: "tri", options: ON_OFF_MAYBE, default: "off",
		label: "Hammer Breaks Bridges",
		tip: "Hammer item builds bridges across water gaps on the overworld map. Maybe: the seed secretly decides on or off, so you won't know until you play.",
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
		tip: "Taking damage drops you straight to Small Mario instead of demoting one tier at a time.",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		group: "player", inFlagKey: true },
	{ id: "faster_tail_speed", type: "bool", default: false,
		label: "Faster Tail Speed",
		tip: "Speeds up the Raccoon / Tanooki tail swipe so you barely slow down using it. Slightly tweaks raccoon flight to keep level design intact.",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		group: "player", inFlagKey: true },
	{ id: "no_game_over_penalty", type: "bool", default: false,
		label: "No Game Over Penalty",
		tip: "Game Over no longer wipes your inventory, map progress, or cards — continue picks up where you left off.",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		group: "player", inFlagKey: true },
	{ id: "faster_frog", type: "bool", default: false,
		label: "Faster Frog",
		tip: "Speeds up swimming and running while wearing the Frog Suit.",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		group: "player", inFlagKey: true },
	{ id: "starting_items", type: "items",
		items: ITEM_OPTIONS, slots: 3,
		default: [],
		label: "Starting Items",
		note: "Choose up to 3 items to start with in your inventory.",
		group: "player", inFlagKey: true },

	// --- Cosmetic (does not affect seed or flag key) ---
	{ id: "palettes", type: "bool", default: true,
		label: "Player colors",
		tip: "Give Mario and Luigi new outfit colors. Off keeps the classic red and green.",
		group: "cosmetic", inFlagKey: false },
	{ id: "player_color", type: "nescolor", default: null,
		label: "Color",
		tip: "Pick Mario's color — Luigi and the power-up suits get matching colors built from your pick. Random rolls a new color every time.",
		group: "cosmetic", inFlagKey: false,
		enabledWhen: { palettes: true }, indent: true },
	{ id: "palette_themed", type: "bool", default: false,
		label: "World colors",
		tip: "Recolor levels, enemies, and world maps with a random color theme. Brightness stays the same, so everything stays easy to see.",
		group: "cosmetic", inFlagKey: false },
];

// Hardcoded fields sent to Rust that aren't user-facing.
const CONSTANT_FIELDS = {
	disable_autoscroll: true,
	card_speed_clear: true,
};

// --- Presets ---
//
// A preset is a curated recipe expressed as a *sparse* map of {field_id: value}
// listing only the gameplay fields that differ from the schema default.
// Applying one resets every flag-key field to its default, then overlays these
// overrides (cosmetic / ROM fields are left as the user set them — same fields
// applyOptions touches). Keyed by stable field ids rather than the bit-packed
// flag key, so a future flag-layout change can't silently corrupt a preset; an
// unknown id just no-ops (and assertPresetParity shouts about it on load).
//
// These override maps were generated by decoding the source flag keys once via
// Options::from_flag_key and diffing against Options::default(). To revise a
// preset, decode its new flag key and replace the overrides — don't store the
// flag key itself.
export const PRESETS = [
	{ id: "recommended", label: "Recommended",
		tip: "A balanced everyday ruleset: most enemies wild, beta stages, and quality-of-life conveniences.",
		overrides: {
			ground: "wild", shell: "wild", flying: "wild", piranhas: "wild",
			ghosts: "wild", water: "wild", cannons: "wild", hb_encounters: "wild",
			rotodiscs: "shuffle",
			wild_injections: true, early_sun: true,
			include_beta_stages: true, swap_start_airship: true,
			big_q_blocks: true, starting_items: [15],
			fast_mushroom_house: true, faster_frog: true, faster_tail_speed: true,
			limit_bro_movement: true,
			hammer_breaks_locks: "on", eights_are_wild: "on",
			world_order: true, random_koopalings: true,
			hammer_vulnerable_koopalings: true,
		} },
	{ id: "beginner", label: "Beginner Friendly",
		tip: "Gentler ruleset: extra lives and items, no game-over penalty, no hand traps or troll pipes.",
		overrides: {
			starting_lives: 20, starting_items: [1, 2, 3],
			infinite_mushroom_houses: true, fast_mushroom_house: true,
			no_game_over_penalty: true, faster_tail_speed: true,
			limit_bro_movement: true,
			hands_levels: false, troll_pipes: "off",
			shuffle_spade_games: false, more_hammer_rocks: "on",
			hammer_breaks_locks: "on", big_q_blocks: true,
			ghosts: "off", hb_encounters: "shuffle",
			world_order: true, random_koopalings: true,
			hammer_vulnerable_koopalings: true,
		} },
	{ id: "jet", label: "Jet",
		tip: "Shorter games — 5 worlds, wild enemies, quality-of-life speedups.",
		overrides: {
			world_order: true, world_count: 5,
			starting_lives: 20, starting_items: [15, 15, 11],
			ground: "wild", shell: "wild", flying: "wild", ghosts: "wild",
			hb_encounters: "wild", rotodiscs: "shuffle",
			infinite_mushroom_houses: true, fast_mushroom_house: true,
			no_game_over_penalty: true, faster_tail_speed: true, faster_frog: true,
			hands_levels: false, troll_pipes: "off", more_hammer_rocks: "on",
			hammer_breaks_locks: "on", big_q_blocks: true,
			random_koopalings: true, hammer_vulnerable_koopalings: true,
		} },
	{ id: "vanilla", label: "Vanilla Randomizer",
		tip: "Closer to a classic randomizer feel with beta stages and wild ground/flying enemies.",
		overrides: {
			ground: "wild", shell: "wild", flying: "wild",
			hb_encounters: "shuffle", rotodiscs: "shuffle",
			wild_injections: true, early_sun: true,
			include_beta_stages: true,
			shuffle_spade_games: false, shuffle_toad_houses: false,
			hands_levels: false, troll_pipes: "off",
			big_q_blocks: true, starting_items: [15, 15, 15],
			faster_tail_speed: true, faster_frog: true,
			world_order: true, random_koopalings: true,
			hammer_vulnerable_koopalings: true,
		} },
	{ id: "max_chaos", label: "Max Chaos",
		tip: "Everything wild: every enemy class, injections, swapped starts, beta stages, all maybes.",
		overrides: {
			ground: "wild", shell: "wild", flying: "wild", piranhas: "wild",
			ghosts: "wild", thwomps: "wild", rotodiscs: "wild", cannons: "wild",
			water: "wild", bros: "wild", hb_encounters: "wild",
			wild_injections: true, early_sun: true,
			include_beta_stages: true, swap_start_airship: true,
			big_q_blocks: true, starting_items: [15, 15, 15],
			faster_tail_speed: true, faster_frog: true,
			world_order: true, random_koopalings: true,
			hammer_vulnerable_koopalings: true,
			troll_pipes: "maybe", more_hammer_rocks: "maybe",
			eights_are_wild: "maybe",
			hammer_breaks_locks: "maybe", hammer_breaks_bridges: "maybe",
		} },
	{ id: "challenging", label: "Challenging",
		tip: "Wild enemies and beta stages with no quality-of-life crutches.",
		overrides: {
			ground: "wild", shell: "wild", flying: "wild", piranhas: "wild",
			ghosts: "wild", thwomps: "wild", rotodiscs: "wild", cannons: "wild",
			water: "wild", bros: "wild", hb_encounters: "wild",
			wild_injections: true, early_sun: true,
			include_beta_stages: true, swap_start_airship: true,
			big_q_blocks: true,
			world_order: true, random_koopalings: true,
		} },
];

// Apply a preset's overrides to the DOM. Resets every flag-key field to its
// schema default first, then writes the overrides on top, so the result is
// deterministic regardless of the user's prior toggles. Leaves cosmetic / ROM
// fields untouched (same fields applyOptions skips). Mirrors applyOptions so
// callers update the flag key + summary the same way afterward.
export function applyPreset(overrides) {
	for (const entry of SCHEMA) {
		if (!entry.inFlagKey) continue;
		const v = overrides && entry.id in overrides ? overrides[entry.id] : entry.default;
		writeValue(entry, v);
	}
	applyEnabledWhen();
	applyRowStates();
	saveSettings();
}

// Load-time sanity check: warn if any preset references a field id that isn't a
// flag-key schema entry (typo or a renamed/removed option). Catches drift the
// same way assertSchemaParity does for the Rust defaults.
export function assertPresetParity() {
	const flagKeyIds = new Set(SCHEMA.filter(s => s.inFlagKey).map(s => s.id));
	const bad = [];
	for (const preset of PRESETS) {
		for (const id of Object.keys(preset.overrides)) {
			if (!flagKeyIds.has(id)) bad.push({ preset: preset.id, field: id });
		}
	}
	if (bad.length) console.error("Preset references unknown flag-key fields", bad);
}

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
	let creditLine = null;
	if (entry.credit) {
		const { name, url } = entry.credit;
		const who = url
			? el("a", { href: url, target: "_blank", rel: "noopener noreferrer" }, name)
			: name;
		creditLine = el("div", { class: "option-credit" }, "Credit: ", who);
	}
	return el(
		"div",
		{ id: `tip-${entry.id}`, class: "option-tip", hidden: true },
		entry.tip,
		creditLine,
	);
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

// Bool entries render as a two-state Off/On pill group (same shape as `renderTri`)
// so checkboxes and pills share the same visual rhythm. The underlying state is
// still a real bool — `readValue` collapses the radio "on"/"off" back to true/false.
const BOOL_OPTIONS = [
	{ value: "off", label: "Off" },
	{ value: "on", label: "On" },
];

function renderBool(entry) {
	const wrap = el("label", { class: "select-label bool-row" + (entry.indent ? " sub-options" : "") });
	const icon = iconCanvas(entry);
	if (icon) wrap.appendChild(icon);
	wrap.appendChild(document.createTextNode(entry.label));
	if (entry.flavor) {
		wrap.appendChild(el("span", { class: "option-flavor" }, entry.flavor));
	}
	const btn = tipBtn(entry);
	if (btn) wrap.appendChild(btn);
	const group = el("div", { class: "pill-group" });
	for (const opt of BOOL_OPTIONS) {
		const inputId = `${domId(entry.id)}-${opt.value}`;
		const isChecked = (opt.value === "on") === !!entry.default;
		group.appendChild(el("input", {
			type: "radio", name: radioName(entry.id), id: inputId,
			value: opt.value, checked: isChecked,
		}));
		group.appendChild(el("label", { for: inputId }, opt.label));
	}
	wrap.appendChild(group);
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

const nesRgbCss = (byte) => {
	const [r, g, b] = NES_PALETTE[byte & 0x3F];
	return `rgb(${r},${g},${b})`;
};

// The pickable player colors: the 48 chromatic NES colors (luminance rows
// 0-3 x hue columns 1-C). Grays/blacks/whites are excluded — the palette
// scheme is derived by hue, so it needs a hue to anchor on.
function chromaticGridRows() {
	const rows = [];
	for (let row = 0; row < 4; row++) {
		const cols = [];
		for (let hue = 1; hue <= 0x0C; hue++) {
			cols.push((row << 4) | hue);
		}
		rows.push(cols);
	}
	return rows;
}

function renderNesColor(entry) {
	const wrap = el("div", { class: "nescolor-block" + (entry.indent ? " sub-options" : "") });
	const header = el("div", { class: "option-header" }, entry.label);
	const btn = tipBtn(entry);
	if (btn) header.appendChild(btn);
	wrap.appendChild(header);

	// "Random" tile first — the default.
	const randId = `${domId(entry.id)}-rand`;
	wrap.appendChild(el("input", {
		type: "radio", name: radioName(entry.id), id: randId,
		value: "rand", checked: entry.default == null, class: "nescolor-input",
	}));
	wrap.appendChild(el("label", { for: randId, class: "nescolor-random" }, "Random"));

	const grid = el("div", { class: "nescolor-grid" });
	for (const rowBytes of chromaticGridRows()) {
		for (const byte of rowBytes) {
			const hex = byte.toString(16).toUpperCase().padStart(2, "0");
			const inputId = `${domId(entry.id)}-${hex}`;
			grid.appendChild(el("input", {
				type: "radio", name: radioName(entry.id), id: inputId,
				value: String(byte), checked: entry.default === byte, class: "nescolor-input",
			}));
			grid.appendChild(el("label", {
				for: inputId, class: "nescolor-swatch",
				style: `background:${nesRgbCss(byte)}`,
				title: `$${hex}`,
			}));
		}
	}
	wrap.appendChild(grid);
	return wrap;
}

const RENDERERS = {
	bool: renderBool,
	tri: renderTri,
	select: renderSelect,
	radio: renderRadio,
	items: renderItems,
	nescolor: renderNesColor,
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
			const checked = document.querySelector(`input[name="${radioName(entry.id)}"]:checked`);
			return checked ? checked.value === "on" : entry.default;
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
		case "nescolor": {
			const checked = document.querySelector(`input[name="${radioName(entry.id)}"]:checked`);
			if (!checked || checked.value === "rand") return null;
			return Number(checked.value);
		}
	}
}

export function writeValue(entry, value) {
	if (value === undefined) return;
	switch (entry.type) {
		case "bool": {
			const target = !!value ? "on" : "off";
			const e = document.querySelector(`input[name="${radioName(entry.id)}"][value="${target}"]`);
			if (e) e.checked = true;
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
		case "nescolor": {
			const target = value == null ? "rand" : String(value);
			const e = document.querySelector(`input[name="${radioName(entry.id)}"][value="${target}"]`);
			if (e) e.checked = true;
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
		case "nescolor": {
			if (value == null) return "Random";
			return "$" + value.toString(16).toUpperCase().padStart(2, "0");
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
		case "select":
			return [domId(entry.id)];
		case "bool":
			return BOOL_OPTIONS.map(o => `${domId(entry.id)}-${o.value}`);
		case "tri":
		case "radio":
			return entry.options.map(o => `${domId(entry.id)}-${o.value}`);
		case "items":
			return Array.from({ length: entry.slots }, (_, i) => `${domId(entry.id)}-${i}`);
		case "nescolor": {
			const ids = [`${domId(entry.id)}-rand`];
			for (let row = 0; row < 4; row++) {
				for (let hue = 1; hue <= 0x0C; hue++) {
					const hex = ((row << 4) | hue).toString(16).toUpperCase().padStart(2, "0");
					ids.push(`${domId(entry.id)}-${hex}`);
				}
			}
			return ids;
		}
		default:
			return [];
	}
}

// Tag each bool / tri row with an `opt-on` (warm) or `opt-maybe` (cool) class
// so CSS can give the row a tinted background. For tris, "off" is neutral and
// every other state is `opt-on` except for "maybe" which gets its own variant.
export function applyRowStates() {
	for (const entry of SCHEMA) {
		if (entry.type !== "bool" && entry.type !== "tri") continue;
		const ids = entryDomIds(entry);
		const first = document.getElementById(ids[0]);
		if (!first) continue;
		const wrap = first.closest("label");
		if (!wrap) continue;
		const value = readValue(entry);
		let on = false, maybe = false;
		if (entry.type === "bool") {
			on = value === true;
		} else if (value === "maybe" || value === "wild") {
			// "wild" and "maybe" share the cool violet — both mean "the seed picks
			// something spicier than the plain shuffle / on baseline".
			maybe = true;
		} else if (value !== "off") {
			on = true;
		}
		wrap.classList.toggle("opt-on", on);
		wrap.classList.toggle("opt-maybe", maybe);
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
				applyRowStates();
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
			if (entry.type === "bool") {
				settings[`radio:${radioName(entry.id)}`] = v ? "on" : "off";
			} else if (entry.type === "tri" || entry.type === "radio") {
				settings[`radio:${radioName(entry.id)}`] = v;
			} else if (entry.type === "nescolor") {
				settings[`radio:${radioName(entry.id)}`] = v == null ? "rand" : String(v);
			} else if (entry.type === "items") {
				for (let i = 0; i < entry.slots; i++) {
					const node = document.getElementById(`${domId(entry.id)}-${i}`);
					if (node) settings[node.id] = node.value;
				}
			} else {
				settings[domId(entry.id)] = String(v);
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
				if (elNode) {
					if (elNode.type === "checkbox") elNode.checked = val === true || val === "true";
					else elNode.value = val;
					continue;
				}
				// Legacy: pre-pill bool settings stored under `domId(entry.id)` → true/false.
				// Route them through writeValue so the new radio UI picks them up.
				const legacy = SCHEMA.find(e => e.type === "bool" && domId(e.id) === key);
				if (legacy) writeValue(legacy, val === true || val === "true");
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
