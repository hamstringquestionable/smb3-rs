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

// Wild Injections is a `toggles` pill: "Off" is exclusive and the chasers
// toggle independently, so the value is the list of lit ones (empty = off).
// Values match the Rust `WildChaser` enum.
const WILD_INJECTION_TOGGLES = [
	{ value: "off", label: "Off" },
	{ value: "sun", label: "Sun" },
	{ value: "lakitu", label: "Lakitu" },
	{ value: "bass", label: "Bass" },
];

// Off / Some / All pill for Limit Hazards. Values match the Rust `HazardLimit`
// enum's serde representation ("some" is the `Sparse` variant, renamed there
// only so it never reads as `Option::Some`).
const OFF_SOME_ALL = [
	{ value: "off", label: "Off" },
	{ value: "some", label: "Some" },
	{ value: "all", label: "All" },
];

// Off / On / Wild pill for Random Fire Flower. "Wild" widens the pool to also
// include the Small/Big downgrade outcomes. Values match the Rust
// `FireFlowerMode` enum's serde representation.
const OFF_ON_WILD = [
	{ value: "off", label: "Off" },
	{ value: "on", label: "On" },
	{ value: "wild", label: "Wild" },
];

// Off / Double / Wild pill for Deja Vu. Values match the Rust `DejaVuMode`
// enum's serde representation.
const OFF_DOUBLE_WILD = [
	{ value: "off", label: "Off" },
	{ value: "double", label: "Double" },
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
// CHR icons: tiles are absolute CHR ROM tile indices, listed row-major and
// `cols` wide; palette is four NES color indices, entry 0 always transparent.
// Decoded from the player's own ROM at render time — pick them with
// web/chr-picker.html rather than by hand.
// Map inventory items, all on CHR page $05. The suits and the mushroom are
// stored as a left half only and drawn mirrored, hence `flipRight`.
const FROG_SUIT = { tiles: [320, 320, 321, 321], cols: 2, palette: [0x0F, 0x16, 0x0A, 0x0E], flipRight: true };
const TANOOKI_SUIT = { tiles: [322, 322, 323, 323], cols: 2, palette: [0x0F, 0x36, 0x08, 0x0E], flipRight: true };
const MUSHROOM = { tiles: [324, 324, 325, 325], cols: 2, palette: [0x0F, 0x36, 0x16, 0x0E], flipRight: true };
const FIRE_FLOWER = { tiles: [326, 326, 327, 327], cols: 2, palette: [0x0F, 0x30, 0x0B, 0x1D], flipRight: true };
const HAMMER_SUIT = { tiles: [330, 330, 331, 331], cols: 2, palette: [0x0F, 0x30, 0x38, 0x1D], flipRight: true };
const PWING = { tiles: [336, 338, 337, 339], cols: 2, palette: [0x00, 0x30, 0x28, 0x1D] };
const HAMMER = { tiles: [344, 346, 345, 347], cols: 2, palette: [0x00, 0x30, 0x08, 0x1D] };
const WHISTLE = { tiles: [352, 354, 353, 355], cols: 2, palette: [0x00, 0x30, 0x28, 0x1D] };
const CHEST = { tiles: [362, 364, 363, 365], cols: 2, palette: [0x0F, 0x1D, 0x18, 0x08] };

// Other pages — see "Enemy Sprite CHR Bank Switching" in the ROM reference for
// which page holds which object's art.
// $0B holds most of the ground/shell roster.
const SPINY = { tiles: [704, 706, 705, 707], cols: 2, palette: [0x0F, 0x1D, 0x20, 0x06] };
const BUZZY_BEETLE = { tiles: [720, 722, 721, 723], cols: 2, palette: [0x0F, 0x1D, 0x20, 0x27] };
const KURIBO_SHOE = { tiles: [744, 746, 745, 747], cols: 2, palette: [0x0F, 0x1D, 0x20, 0x09] };
const BOB_OMB = { tiles: [756, 758, 757, 759], cols: 2, palette: [0x0F, 0x1D, 0x20, 0x27] };

// Which option an enemy belongs to comes from the class tables in
// src/randomize/enemies/tables.rs, not from intuition — Dry Bones is in
// GHOST_ENEMIES, alongside Boo and Hot Foot.
const DRY_BONES = { // $13, 16x32 stacked from two picks
	tiles: [
		1216, 1218,
		1217, 1219,
		1220, 1222,
		1221, 1223,
	],
	cols: 2,
	palette: [0x0F, 0x1D, 0x10, 0x20],
};
const ROTODISCS = [ // $12, two rotation frames
	{ tiles: [1176, 1178, 1177, 1179], cols: 2, palette: [0x0F, 0x18, 0x21, 0x20] },
	{ tiles: [1180, 1182, 1181, 1183], cols: 2, palette: [0x0F, 0x18, 0x21, 0x20] },
];
const START_TILE = { tiles: [1488, 1490, 1489, 1491], cols: 2, palette: [0x0F, 0x38, 0x20, 0x04] }; // $17
const KOOPALING_RING = { tiles: [4762, 4762, 4763, 4763], cols: 2, palette: [0x0F, 0x1E, 0x20, 0x25], flipRight: true }; // $4A
const Q_ORB = { tiles: [4988, 4990, 4989, 4991], cols: 2, palette: [0x0F, 0x1D, 0x38, 0x20] }; // $4D, Boom-Boom's Q ball

// $3B. Bowser's own draw routine assembles him tile by tile, so no rectangle in
// CHR is "Bowser" — this is two picks joined. His top half is stored as a left
// half and mirrored; his bottom half is stored whole. That mix is why the tiles
// carry per-tile `flip` rather than the spec-wide `flipRight`.
const BOWSER = {
	tiles: [
		3780, 3782, { t: 3782, flip: true }, { t: 3780, flip: true },
		3781, 3783, { t: 3783, flip: true }, { t: 3781, flip: true },
		3802, 3804, 3806, 3808,
		3803, 3805, 3807, 3809,
	],
	cols: 4,
	palette: [0x0F, 0x1D, 0x28, 0x09],
};
const CANNON = { tiles: [6436, 6438, 6437, 6439], cols: 2, palette: [0x0F, 0x1D, 0x10, 0x20] }; // $64

// $4C. 32x32, and like Boss Bass its halves aren't adjacent in CHR, so they're
// picked separately and stacked into one grid.
const BIG_Q = {
	tiles: [
		4864, 4866, 4868, 4870,
		4865, 4867, 4869, 4871,
		4872, 4874, 4876, 4878,
		4873, 4875, 4877, 4879,
	],
	cols: 4,
	palette: [0x0F, 0x1D, 0x28, 0x20],
};
const SPADE = { tiles: [1448, 1448, 1449, 1449], cols: 2, palette: [0x0F, 0x20, 0x20, 0x1D], flipRight: true }; // $16
const WAND = { tiles: [1982, 1983], cols: 1, palette: [0x0F, 0x28, 0x37, 0x03] }; // $1E, 8x16
const N_CARD = { tiles: [2064, 2064, 2065, 2065], cols: 2, palette: [0x0F, 0x20, 0x20, 0x1D], flipRight: true }; // $20

// Water enemies, page $1A. Boss Bass is 24x32 — his two halves aren't adjacent
// in CHR, so they're picked separately and stacked here into one grid.
const BLOOPER = { tiles: [1712, 1712, 1713, 1713], cols: 2, palette: [0x0F, 0x1E, 0x37, 0x03], flipRight: true };
const MINI_CHEEP = { tiles: [1682, 1684, 1683, 1685], cols: 2, palette: [0x0F, 0x1E, 0x37, 0x16] };
const BOSS_BASS = {
	tiles: [
		1664, 1666, 1668,
		1665, 1667, 1669,
		1670, 1672, 1674,
		1671, 1673, 1675,
	],
	cols: 3,
	palette: [0x0F, 0x1E, 0x37, 0x16],
};

// Random pick on each page load — flavor for "what will you get?". Still short
// the super leaf and the starman.
const POWERUPS = [MUSHROOM, FIRE_FLOWER, FROG_SUIT, TANOOKI_SUIT, HAMMER_SUIT, PWING];

// Schema. Field names match the Rust Options struct; the load-time parity
// check guarantees they stay aligned. inFlagKey decides whether applying a
// shared key or a preset writes this field, and is checked at load against the
// list of fields Rust actually encodes (`flag_key_fields_json`) — the Rust
// encoder remains the authority, this just can't drift from it unnoticed.
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
	{ id: "shuffle_spade_games", type: "bool", default: true,
		label: "Shuffle Spade Games",
		tip: "Move spade (card-matching) games to random spots on the map",
		icon: SPADE,
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
		icon: START_TILE,
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
		icon: N_CARD,
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
		icon: [SPINY, BOB_OMB, KURIBO_SHOE],
		group: "enemies", inFlagKey: true },
	{ id: "shell", type: "tri", options: TRI, default: "shuffle",
		label: "Shell",
		tip: "Shelled enemies (Koopa, Buzzy Beetle, etc.)",
		icon: BUZZY_BEETLE,
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
		icon: DRY_BONES,
		group: "enemies", inFlagKey: true },
	{ id: "thwomps", type: "tri", options: TRI, default: "off",
		label: "Thwomps",
		tip: "Thwomp movement variants (diagonal slides, sideways, up-down)",
		group: "enemies", inFlagKey: true },
	{ id: "rotodiscs", type: "tri", options: TRI, default: "off",
		label: "Rotodiscs",
		tip: "Rotodisc rotation variants (single/dual, CW/CCW)",
		icon: ROTODISCS,
		group: "enemies", inFlagKey: true },
	{ id: "cannons", type: "tri", options: TRI, default: "off",
		label: "Cannons",
		tip: "Cannons, Bullet Bill launchers, goomba pipes, and bob-omb launchers. Shuffle keeps fire direction; Wild lets any cannon become any other.",
		icon: CANNON,
		group: "enemies", inFlagKey: true },
	{ id: "water", type: "tri", options: TRI, default: "shuffle",
		label: "Water",
		tip: "Water enemies (Blooper, Big Bertha, etc.)",
		icon: [BLOOPER, MINI_CHEEP],
		group: "enemies", inFlagKey: true },
	{ id: "bros", type: "tri", options: TRI, default: "shuffle",
		label: "Bros",
		tip: "Hammer / Boomerang / Fire Bros inside levels",
		group: "enemies", inFlagKey: true },
	{ id: "hb_encounters", type: "tri", options: TRI, default: "off",
		label: "HB Encounters",
		tip: "All enemies in overworld Hammer Bro mini-battles",
		group: "enemies", inFlagKey: true },
	{ id: "friendlier_levels", type: "bool", default: false,
		label: "Friendlier Levels",
		tip: "Keeps the roughest levels out of the shuffle — 2-3, 5-3, 6-6, 7-5, 7-8 and 8-1. Their slots go to beta stages if you have those on, otherwise to a second visit to a level already in the seed. Two fortresses, 7F2 and 8F1, are usually made optional rather than removed: still there, still beatable, just not in your way.",
		group: "map", inFlagKey: true },
	{ id: "deja_vu", type: "tri", options: OFF_DOUBLE_WILD, default: "off",
		label: "Deja Vu", flavor: "Haven't we been here?",
		tip: "Let the same level show up on more than one tile. Double: every level gets a second copy in the deck, so some show up twice and others sit the seed out. Wild: no limit — a level can turn up over and over, or never. Levels that hand you an item still appear exactly once.",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		group: "map", inFlagKey: true },
	{ id: "limit_hazards", type: "tri", options: OFF_SOME_ALL, default: "off",
		label: "Limit Hazards",
		tip: "Stops swaps from dropping nippers, Ptooies, thwomps, Hot Foots or Bros into levels that weren't built for them. Some allows the occasional one, All allows none. Hazards that were always there stay put.",
		group: "enemies", inFlagKey: true },
	{ id: "wild_injections", type: "toggles", options: WILD_INJECTION_TOGGLES,
		default: [],
		label: "Wild Injections",
		tip: "Drop a chaser into some levels that never had one — an Angry Sun, a Lakitu, or a leaping Big Bertha. Pick any combination.",
		icon: BOSS_BASS,
		group: "enemies", inFlagKey: true },
	{ id: "lakitu_stays_down", type: "bool", default: false,
		label: "Lakitu Stays Down",
		tip: "Beat a Lakitu and it stays down, instead of drifting back in a few seconds later.",
		group: "enemies", inFlagKey: true },
	{ id: "early_sun", type: "bool", default: false,
		label: "Early Sun",
		tip: "Angry Sun starts attacking immediately on spawn.",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		group: "enemies", inFlagKey: true },
	{ id: "bro_battle_timer", type: "bool", default: false,
		label: "Bro Battle Timer",
		tip: "Walk into a Hammer, Boomerang, Heavy or Fire Bro and the clock starts at 10. Clear the room fast or the fight clears you.",
		group: "enemies", inFlagKey: true },

	// --- Bosses ---
	{ id: "random_koopalings", type: "bool", default: false,
		label: "Random Koopalings",
		tip: "Shuffle which Koopaling appears in each world. Each keeps its own moves and abilities. Thanks to fcoughlin (Fred) for the patch.",
		icon: KOOPALING_RING,
		group: "bosses", inFlagKey: true },
	{ id: "koopaling_hits", type: "bool", default: true,
		label: "Random Koopaling Stomps",
		tip: "Each Koopaling takes a random number of stomps (1–5) instead of the usual 3",
		icon: KOOPALING_RING,
		group: "bosses", inFlagKey: true },
	{ id: "boomboom_hits", type: "bool", default: true,
		label: "Random Boom-Boom Stomps",
		tip: "Each fortress Boom-Boom takes a random number of stomps (1–5) instead of the usual 3",
		icon: Q_ORB,
		group: "bosses", inFlagKey: true },
	{ id: "hammer_vulnerable_koopalings", type: "bool", default: false,
		label: "Hammer Vulnerable Koopalings",
		tip: "Koopalings can be damaged by thrown hammers (normally hammers pass through them)",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		icon: HAMMER_SUIT,
		group: "bosses", inFlagKey: true },
	{ id: "adjust_boss_hitboxes", type: "bool", default: true,
		label: "Adjust Boss Hitboxes",
		tip: "Adjust Bowser and Koopaling hitboxes so they're easier to hit",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		icon: BOWSER,
		group: "bosses", inFlagKey: true },
	{ id: "skip_wand_cutscene", type: "bool", default: true,
		label: "Skip Wand Cutscene", flavor: "Jump Up, Super Star!",
		tip: "Skip the wand falling cutscene after defeating a Koopaling — jump to grab the wand instead",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		icon: WAND,
		group: "bosses", inFlagKey: true },

	// --- Items & Pickups ---
	// (sprite curation deferred — sprite CHR uses dynamic banking that requires
	//  per-object disassembly chasing. Map tiles below have known static banks.)
	{ id: "powerups", type: "bool", default: true,
		label: "Power-ups",
		tip: "Randomize ? block and brick block contents, keeping each roughly the same tier",
		icon: POWERUPS,
		group: "items", inFlagKey: true },
	{ id: "chest_items", type: "bool", default: true,
		label: "Chest Items",
		tip: "Randomize chest and Toad House reward items",
		icon: CHEST,
		group: "items", inFlagKey: true },
	{ id: "fire_flower", type: "tri", options: OFF_ON_WILD, default: "off",
		label: "Random Fire Flower",
		tip: "Fire Flowers still look the same, but each one gives a different suit based on where it is. On: Fire, Frog, Tanooki, or Hammer. Wild: also lets it shrink you to Big or Small.",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		icon: FIRE_FLOWER,
		group: "items", inFlagKey: true },
	{ id: "big_q_blocks", type: "bool", default: false,
		label: "Big ? Blocks",
		tip: "Randomize the contents of Big ? Blocks in bonus rooms",
		icon: BIG_Q,
		group: "items", inFlagKey: true },
	{ id: "shuffle_big_q_rooms", type: "bool", default: false,
		label: "Shuffle Big ? Rooms",
		tip: "Big ? pipes lead somewhere else — including eight bonus rooms that were left in the game and never used.",
		icon: BIG_Q,
		group: "items", inFlagKey: true },
	{ id: "remove_whistles", type: "bool", default: true,
		label: "Remove Warp Whistles",
		tip: "Remove warp whistles so all worlds must be played",
		icon: WHISTLE,
		group: "items", inFlagKey: true },
	{ id: "hammer_breaks_locks", type: "tri", options: ON_OFF_MAYBE, default: "off",
		label: "Hammer Breaks Locks",
		tip: "Hammer item also breaks fortress locks on the overworld map. Maybe: the seed secretly decides on or off, so you won't know until you play.",
		icon: HAMMER,
		group: "items", inFlagKey: true },
	{ id: "hammer_breaks_bridges", type: "tri", options: ON_OFF_MAYBE, default: "off",
		label: "Hammer Breaks Bridges",
		tip: "Hammer item builds bridges across water gaps on the overworld map. Maybe: the seed secretly decides on or off, so you won't know until you play.",
		icon: HAMMER,
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
		icon: FROG_SUIT,
		group: "player", inFlagKey: true },
	{ id: "modern_powerups", type: "bool", default: false,
		label: "Modern Power-Ups",
		tip: "Power-ups work like the newer Mario games — grab a Fire Flower or suit as Small Mario and get its power without turning Big first.",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		group: "player", inFlagKey: true },
	{ id: "poison_mushrooms", type: "bool", default: false,
		label: "Poison Mushrooms",
		tip: "Some 1-Up blocks hand out an upside-down poison mushroom that hurts you instead of a 1-Up. You can't tell which until you hit the block.",
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
	{ id: "remove_flashing", type: "bool", default: true,
		label: "Remove flashing",
		tip: "Stop the full-screen flashing and fading effects. On by default so the game is safer for players sensitive to flashing lights.",
		credit: { name: "MaCobra52", url: "https://github.com/macobra52" },
		group: "cosmetic", inFlagKey: false },
	{ id: "king_quotes", type: "bool", default: true,
		label: "King quotes",
		tip: "Give each rescued king a new thing to say. Turn this off and the kings say what they say in the original game.",
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
			wild_injections: ["sun", "lakitu", "bass"], early_sun: true,
			include_beta_stages: true, swap_start_airship: true,
			antechamber_shuffle: "on", piranha_shuffle: "wild",
			big_q_blocks: true, starting_items: [15, 15, 15],
			fast_mushroom_house: true, faster_frog: true, faster_tail_speed: true,
			no_game_over_penalty: true, limit_bro_movement: true,
			hammer_breaks_locks: "on", eights_are_wild: "on",
			more_hammer_rocks: "maybe",
			world_order: true, random_koopalings: true,
			hammer_vulnerable_koopalings: true,
		} },
	{ id: "beginner", label: "Beginner Friendly",
		tip: "Gentler ruleset: extra lives and items, no added hazards, the roughest levels sat out, no game-over penalty, no hand traps or troll pipes.",
		overrides: {
			starting_lives: 20, starting_items: [1, 2, 3],
			infinite_mushroom_houses: true, fast_mushroom_house: true,
			no_game_over_penalty: true, faster_tail_speed: true,
			modern_powerups: true,
			limit_bro_movement: true,
			hands_levels: false, troll_pipes: "off",
			shuffle_spade_games: false, more_hammer_rocks: "on",
			hammer_breaks_locks: "on", big_q_blocks: true,
			// `ghosts` was "off" purely to stop Boo -> Hot Foot, the only
			// hazard that class can produce in Shuffle. limit_hazards blocks
			// that directly, so the class goes back on and Boo <-> Dry Bones
			// variety comes with it.
			limit_hazards: "all", ghosts: "shuffle", hb_encounters: "shuffle",
			friendlier_levels: true,
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
			wild_injections: ["sun", "bass"], early_sun: true,
			include_beta_stages: true,
			shuffle_spade_games: false, shuffle_toad_houses: false,
			hands_levels: false, troll_pipes: "off",
			big_q_blocks: true, starting_items: [15, 15, 15],
			faster_frog: true,
			world_order: true, random_koopalings: true,
			hammer_vulnerable_koopalings: true,
		} },
	{ id: "max_chaos", label: "Max Chaos",
		tip: "Everything wild: every enemy class, all three chasers, wild Fire Flowers, poison mushrooms, beta stages, and every maybe.",
		overrides: {
			ground: "wild", shell: "wild", flying: "wild", piranhas: "wild",
			ghosts: "wild", thwomps: "wild", rotodiscs: "wild", cannons: "wild",
			water: "wild", bros: "wild", hb_encounters: "wild",
			wild_injections: ["sun", "lakitu", "bass"], early_sun: true,
			include_beta_stages: true, swap_start_airship: true,
			antechamber_shuffle: "maybe", piranha_shuffle: "wild",
			big_q_blocks: true, starting_items: [15, 15, 15],
			fire_flower: "wild", poison_mushrooms: true,
			faster_tail_speed: true, faster_frog: true,
			world_order: true, random_koopalings: true,
			hammer_vulnerable_koopalings: true,
			troll_pipes: "maybe", more_hammer_rocks: "maybe",
			eights_are_wild: "maybe",
			hammer_breaks_locks: "maybe", hammer_breaks_bridges: "maybe",
		} },
	{ id: "league_s7", label: "League Season 7",
		tip: "The Season 7 league ruleset: every enemy class wild, beta stages, shuffled lobbies and scattered piranhas, and the race conveniences.",
		overrides: {
			ground: "wild", shell: "wild", flying: "wild", piranhas: "wild",
			ghosts: "wild", thwomps: "wild", rotodiscs: "wild", cannons: "wild",
			water: "wild", bros: "wild", hb_encounters: "wild",
			wild_injections: ["sun", "lakitu"], early_sun: true,
			include_beta_stages: true, swap_start_airship: true,
			antechamber_shuffle: "on", piranha_shuffle: "wild",
			troll_pipes: "off", eights_are_wild: "maybe",
			hammer_breaks_locks: "on", limit_bro_movement: true,
			big_q_blocks: true, fire_flower: "on", starting_items: [15, 15, 15],
			fast_mushroom_house: true, faster_tail_speed: true, faster_frog: true,
			no_game_over_penalty: true,
			world_order: true, random_koopalings: true,
			hammer_vulnerable_koopalings: true,
		} },
	{ id: "challenging", label: "Challenging",
		tip: "Wild enemies, beta stages, poison mushrooms, and one hit back to Small Mario — no quality-of-life crutches.",
		overrides: {
			ground: "wild", shell: "wild", flying: "wild", piranhas: "wild",
			ghosts: "wild", thwomps: "wild", rotodiscs: "wild", cannons: "wild",
			water: "wild", bros: "wild", hb_encounters: "wild",
			wild_injections: ["sun", "lakitu", "bass"], early_sun: true,
			include_beta_stages: true, swap_start_airship: true,
			antechamber_shuffle: "on", piranha_shuffle: "wild",
			big_q_blocks: true, poison_mushrooms: true, japanese_damage: true,
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
// natural pixel size with a known DOM id; app.js decodes it from the player's
// own ROM. If entry.icon is unset, returns null and the caller skips the slot.
//
// To add an icon to an option:
//   1. Open web/chr-picker.html. It reads the ROM the randomizer page cached,
//      or you can load a .nes directly.
//   2. Find the sprite's CHR page. The enemy -> page table in
//      docs/smb3_rom_reference.md ("Enemy Sprite CHR Bank Switching") saves
//      hunting; map and inventory art is mostly on $05.
//   3. Drag a rectangle around it, tick "mirror" if only half of it exists in
//      CHR, then "Copy as JSON" and paste into the entry's `icon` field.
//   4. `icon` takes one spec or an array of them (random pick per page load,
//      for options covering several things — Power-ups, Ground, Water).
//
// Art the game assembles from scattered tiles (Bowser) needs two picks joined
// by hand; see BOWSER above for the shape that takes.
//
// Native pixel size of an icon spec: a CHR tile grid, 8px per tile.
function iconNativeSize(spec) {
	const cols = spec.cols ?? 2;
	return { w: cols * 8, h: Math.ceil(spec.tiles.length / cols) * 8 };
}

// Icons are pixel art, so they may only be scaled by a whole number — at a
// fractional scale the browser snaps some source pixels to two device pixels
// and others to one, and the sprite reads as squashed. Pick the largest integer
// scale that keeps the icon inside ICON_BOX; art already that big renders 1:1
// rather than being shrunk to fit.
const ICON_BOX = 32;

function iconScale({ w, h }) {
	return Math.max(1, Math.floor(ICON_BOX / Math.max(w, h)));
}

// Set an icon canvas's displayed size to its native size times a whole number.
// `iconCanvas` reserves space using the largest variant; the renderer calls
// this again with the variant actually drawn, since a smaller one displayed at
// the reserved size would be back to a fractional scale.
export function applyIconScale(canvas, spec) {
	if (!canvas || !spec) return;
	const native = iconNativeSize(spec);
	const k = iconScale(native);
	canvas.style.width = `${native.w * k}px`;
	canvas.style.height = `${native.h * k}px`;
}

function iconCanvas(entry) {
	if (!entry.icon) return null;
	// A random-per-load array can hold variants of differing size (the
	// Koopalings differ by a pixel), so reserve the largest.
	const specs = Array.isArray(entry.icon) ? entry.icon : [entry.icon];
	const sizes = specs.map(iconNativeSize);
	const native = {
		w: Math.max(...sizes.map((s) => s.w)),
		h: Math.max(...sizes.map((s) => s.h)),
	};
	const k = iconScale(native);
	// Hidden until something is actually drawn into it. An undrawn canvas is
	// transparent but still occupies its box, which would indent icon'd options
	// past icon-less ones and read as a set of broken images.
	return el("canvas", {
		class: "opt-icon",
		id: `icon-${entry.id}`,
		"data-icon": entry.id,
		width: native.w,
		height: native.h,
		style: `width:${native.w * k}px;height:${native.h * k}px`,
		hidden: true,
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

// A pill group where the non-"off" pills toggle independently: check Sun,
// check Bass, check both. The value is the array of lit pill values, in
// schema order; empty means off, which is what the exclusive "Off" pill sets.
function togglePills(entry) {
	return entry.options.filter(o => o.value !== "off");
}

function toggleNode(entry, value) {
	return document.getElementById(`${domId(entry.id)}-${value}`);
}

// Keep the group coherent after any click: "off" is exclusive, and clearing
// the last lit pill falls back to "off" so the row is never blank.
function syncToggles(entry, clicked) {
	const off = toggleNode(entry, "off");
	const pills = togglePills(entry).map(o => toggleNode(entry, o.value)).filter(Boolean);
	if (clicked === "off") {
		for (const node of pills) node.checked = false;
		if (off) off.checked = true; // clicking "off" while off keeps it off
		return;
	}
	if (off) off.checked = !pills.some(node => node.checked);
}

function renderToggles(entry) {
	const wrap = el("label", { class: "select-label" });
	const icon = iconCanvas(entry);
	if (icon) wrap.appendChild(icon);
	wrap.appendChild(document.createTextNode(entry.label));
	const btn = tipBtn(entry);
	if (btn) wrap.appendChild(btn);
	const group = el("div", { class: "pill-group" });
	for (const opt of entry.options) {
		const inputId = `${domId(entry.id)}-${opt.value}`;
		const lit = opt.value === "off"
			? entry.default.length === 0
			: entry.default.includes(opt.value);
		const input = el("input", {
			type: "checkbox", name: radioName(entry.id), id: inputId,
			value: opt.value, checked: lit,
		});
		// Registered before wireListeners' listener on the same node, so the
		// group is already coherent by the time readValue runs.
		input.addEventListener("change", () => syncToggles(entry, opt.value));
		group.appendChild(input);
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
	toggles: renderToggles,
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
		case "toggles": {
			const pills = togglePills(entry).map(o => toggleNode(entry, o.value));
			if (pills.every(node => !node)) return entry.default; // not rendered yet
			return togglePills(entry)
				.filter(o => toggleNode(entry, o.value)?.checked)
				.map(o => o.value);
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
		case "toggles": {
			// Tolerates a bare string as well as a list so a hand-edited or
			// older payload ("sun") still applies.
			const want = Array.isArray(value) ? value : [value];
			for (const opt of entry.options) {
				const node = toggleNode(entry, opt.value);
				if (node) node.checked = opt.value === "off"
					? want.length === 0
					: want.includes(opt.value);
			}
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
		case "toggles": {
			if (!Array.isArray(value) || value.length === 0) return "OFF";
			return value
				.map(v => entry.options.find(o => o.value === v)?.label ?? v)
				.join(" + ");
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
// fields (palettes, palette_themed, remove_flashing, skip_rom_validation) so applying a
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
		case "toggles":
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
		if (!["bool", "tri", "toggles"].includes(entry.type)) continue;
		const ids = entryDomIds(entry);
		const first = document.getElementById(ids[0]);
		if (!first) continue;
		const wrap = first.closest("label");
		if (!wrap) continue;
		const value = readValue(entry);
		let on = false, maybe = false;
		if (entry.type === "bool") {
			on = value === true;
		} else if (entry.type === "toggles") {
			on = Array.isArray(value) && value.length > 0;
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
			} else if (entry.type === "toggles") {
				// Own key prefix: the value is a list, which no single input
				// carries. Stored as JSON so restore gets an array back.
				settings[`toggles:${radioName(entry.id)}`] = JSON.stringify(v);
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
			if (key.startsWith("toggles:")) {
				const name = key.slice(8);
				const entry = SCHEMA.find(e => e.type === "toggles" && radioName(e.id) === name);
				if (!entry) continue;
				let parsed;
				try { parsed = JSON.parse(val); } catch (_) { parsed = val; }
				// Pre-Bass settings stored the combined value as "both".
				if (parsed === "both") parsed = ["sun", "lakitu"];
				writeValue(entry, parsed);
			} else if (key.startsWith("radio:")) {
				const name = key.slice(6);
				const elNode = document.querySelector(`input[name="${name}"][value="${val}"]`);
				if (elNode) elNode.checked = true;
				if (elNode) continue;
				// Legacy: a field that used to be a bool and is now a toggle
				// group (wild_injections). "on" meant the pool as it stood
				// then — sun and lakitu, before Boss Bass joined.
				const promoted = SCHEMA.find(e => e.type === "toggles" && radioName(e.id) === name);
				if (promoted) writeValue(promoted, val === "on" ? ["sun", "lakitu"] : []);
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

export function assertSchemaParity(wasmDefaultsJson, flagKeyFieldsJson) {
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

	// `inFlagKey` used to be documentation — nothing checked it, so an option
	// marked shareable that Rust didn't actually encode would have been
	// invisible. It drives applyOptions and applyPreset, so a wrong marking
	// means a shared key silently doesn't apply that option. Rust reports what
	// it encodes; anything that disagrees is a bug on one side or the other.
	if (!flagKeyFieldsJson) return;
	let encoded;
	try {
		encoded = new Set(JSON.parse(flagKeyFieldsJson));
	} catch (e) {
		console.error("Schema parity: could not parse the flag-key field list", e);
		return;
	}
	// Truthiness, not `!== false`, because that's what applyOptions and
	// applyPreset test — an entry that just forgets the marking is skipped by
	// them, so it should be reported here too.
	const claimedButNotEncoded = SCHEMA
		.filter(e => e.inFlagKey && !encoded.has(e.id))
		.map(e => e.id);
	const encodedButNotClaimed = SCHEMA
		.filter(e => !e.inFlagKey && encoded.has(e.id))
		.map(e => e.id);
	if (claimedButNotEncoded.length || encodedButNotClaimed.length) {
		console.error("inFlagKey drift detected", { claimedButNotEncoded, encodedButNotClaimed });
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
