// Icon specs shared by more than one page.
//
// A spec names CHR tiles in the player's own ROM plus four NES colors; see
// renderIcon in chr.js for the fields. These live apart from options.js so a
// page that only draws icons (the maze tracker) does not load the whole option
// schema. Specs only one page uses stay with that page.

// World-map background tiles. Tile bytes are the map's own; tile indices are
// the metatile's quadrants on BG CHR pages $14-$17. `clear: 3` makes the
// ground transparent (see renderMapTiles in chr.js). The palette page is the
// tile byte's high two bits; colors are World 1's unless noted.
export const W1_PAL0 = [0x0F, 0x0F, 0x30, 0x3C];
export const W1_PAL1 = [0x0F, 0x36, 0x27, 0x37];
export const W1_PAL2 = [0x0F, 0x21, 0x2A, 0x37];
export const W1_PAL3 = [0x0F, 0x30, 0x16, 0x37];

// Maze hint marks: lock 0x54/0xE4 ($B6-$B9), fortress 0x67/0xEB ($C4-$C7),
// and 0x6A ($64-$67).
export const HINT_TILES = [
	{ tiles: [1462, 1464, 1463, 1465], cols: 2, clear: 3, palette: W1_PAL1 }, // 0x54
	{ tiles: [1462, 1464, 1463, 1465], cols: 2, clear: 3, palette: W1_PAL3 }, // 0xE4
	{ tiles: [1476, 1478, 1477, 1479], cols: 2, clear: 3, palette: W1_PAL1 }, // 0x67
	{ tiles: [1380, 1382, 1381, 1383], cols: 2, clear: 3, palette: W1_PAL1 }, // 0x6A
	{ tiles: [1476, 1478, 1477, 1479], cols: 2, clear: 3, palette: W1_PAL3 }, // 0xEB
];

// World Maze: path junctions on land 0x4A, water 0xAF and sky 0xDE (W5's
// palette 3), and the maze's own telepad 0xDF. The pad's quadrants are
// TELEPAD_QUADRANTS ($80-$83), which the randomizer writes over vanilla's
// alternate spiral — the player's ROM doesn't draw 0xDF this way.
export const MAZE_TILES = [
	{ tiles: [1534, 1472, 1505, 1485], cols: 2, clear: 3, palette: W1_PAL1 }, // 0x4A
	{ tiles: [1296, 1473, 1474, 1475], cols: 2, palette: W1_PAL2 }, // 0xAF
	{ tiles: [1534, 1472, 1505, 1485], cols: 2, clear: 3, palette: [0x0F, 0x36, 0x21, 0x30] }, // 0xDE
	{ tiles: [1408, 1409, 1410, 1411], cols: 2, palette: W1_PAL3 }, // 0xDF telepad
];

// World 8's tank, battleship and airship.
export const W8_MILITARY = [
	{ tiles: [2276, 2278, 2277, 2279], cols: 2, palette: [0x0F, 0x17, 0x27, 0x0F] },
	{ tiles: [2292, 2294, 2293, 2295], cols: 2, palette: [0x0F, 0x17, 0x27, 0x0F] },
	{ tiles: [2300, 2302, 2301, 2303], cols: 2, palette: [0x0F, 0x17, 0x36, 0x0F] },
];

// One scenery tile per world, each in that world's own palette. W3's water
// fills its square, so it has no ground to clear.
export const WORLD_TILES = [
	{ tiles: [1280, 1282, 1281, 1283], cols: 2, clear: 3, palette: W1_PAL2 }, // 0xB4, W1
	{ tiles: [1326, 1328, 1327, 1329], cols: 2, clear: 3, palette: [0x0F, 0x36, 0x27, 0x28] }, // 0x69, W2
	{ tiles: [1288, 1290, 1289, 1291], cols: 2, clear: 3, palette: [0x0F, 0x12, 0x2A, 0x28] }, // 0xBB, W2
	{ tiles: [1296, 1297, 1310, 1311], cols: 2, palette: W1_PAL2 }, // 0x8D, W3
	{ tiles: [1322, 1324, 1323, 1325], cols: 2, clear: 3, palette: [0x0F, 0x12, 0x2A, 0x3A] }, // 0xBD, W4
	{ tiles: [1376, 1378, 1377, 1379], cols: 2, clear: 3, palette: [0x0F, 0x36, 0x27, 0x3B] }, // 0x5F, W5
	{ tiles: [1298, 1300, 1299, 1301], cols: 2, clear: 3, palette: [0x0F, 0x30, 0x22, 0x30] }, // 0xEA, W6
	{ tiles: [1302, 1308, 1303, 1309], cols: 2, clear: 3, palette: [0x0F, 0x11, 0x1A, 0x2A] }, // 0xBE, W7
	{ tiles: [1313, 1316, 1315, 1350], cols: 2, clear: 3, palette: [0x0F, 0x35, 0x25, 0x17] }, // 0xE1, W8
];

const WAND = { tiles: [1982, 1983], cols: 1, palette: [0x0F, 0x28, 0x37, 0x03] }; // $1E, 8x16
// The jewel's upper facets are the only pixels in color 3, so swapping that one
// entry recolors the jewel alone: vanilla purple plus six others, one per wand.
export const WANDS = [0x03, 0x16, 0x2A, 0x21, 0x27, 0x30, 0x14].map((c) => ({ ...WAND, palette: [0x0F, 0x28, 0x37, c] }));

// The tab icon: a "?" block, Plains BG palette 1. Its quadrants are BG tiles
// $98-$9B in the first frame of the level's animated pattern bank ($60 of
// PT2_Anim); the other three frames are the block turning.
export const Q_BLOCK = { tiles: [6168, 6170, 6169, 6171], cols: 2, palette: [0x0F, 0x0F, 0x36, 0x27] };
