use rand::SeedableRng;
use rand::seq::IndexedRandom;
use rand_chacha::ChaCha8Rng;

use crate::randomize;
use crate::rom::Rom;

/// Sentinel: resolve to any random item (1–13).
pub const ITEM_RANDOM: u8 = 14;
/// Sentinel: resolve to any random item except Whistle (1–11, 13).
pub const ITEM_RANDOM_NO_WHISTLE: u8 = 15;
/// Sentinel: resolve to a random suit/powerup (1–6).
pub const ITEM_RANDOM_SUIT_ONLY: u8 = 16;

/// Returns default starting lives (4).
fn default_starting_lives() -> u8 { 4 }

/// Returns default world count (7 — all worlds before Dark Land).
fn default_world_count() -> u8 { 7 }

/// Level shuffle mode.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelShuffle {
    Off,
    IntraWorld,
    CrossWorld,
}

impl Default for LevelShuffle {
    fn default() -> Self {
        LevelShuffle::Off
    }
}

/// Per-class enemy randomization mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnemyMode {
    Off,
    Shuffle,
    Wild,
}

impl Default for EnemyMode {
    fn default() -> Self {
        EnemyMode::Off
    }
}

fn default_shuffle() -> EnemyMode { EnemyMode::Shuffle }
fn default_off() -> EnemyMode { EnemyMode::Off }

/// Options controlling which randomizations to apply.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Options {
    pub powerups: bool,
    pub palettes: bool,
    pub world_order: bool,
    /// Number of worlds before Dark Land (1–7, default 7).
    #[serde(default = "default_world_count")]
    pub world_count: u8,
    #[serde(default = "default_false")]
    pub big_q_blocks: bool,
    /// Level shuffle under vanilla tile layout (off/intra/cross).
    /// Ignored when map_shuffle is true.
    #[serde(default)]
    pub level_shuffle: LevelShuffle,
    /// Enable overworld map shuffle (rebuilds tile layout, always cross-world).
    /// Mutually exclusive with level_shuffle (overrides it).
    #[serde(default = "default_true")]
    pub map_shuffle: bool,
    /// Shuffle pipe endpoint positions (only when map_shuffle is true).
    #[serde(default = "default_true")]
    pub shuffle_pipes: bool,
    /// Shuffle airship levels across worlds 1-7.
    #[serde(default = "default_true")]
    pub shuffle_airships: bool,
    #[serde(default = "default_true")]
    pub disable_autoscroll: bool,
    /// Set starting lives for both Mario and Luigi (1–99).
    #[serde(default = "default_starting_lives")]
    pub starting_lives: u8,
    /// Up to 3 items to start with in inventory (item IDs, e.g. 0x03 = Leaf).
    #[serde(default)]
    pub starting_items: Vec<u8>,
    /// Enable always-on airship lock (anchor effect, disables airship movement on death)
    #[serde(default = "default_true")]
    pub airship_lock: bool,
    /// Randomize chest and reward items (Hammer Bros, Toad House, Princess letter, treasure chests).
    #[serde(default = "default_true")]
    pub chest_items: bool,
    /// Remove warp whistles and replace with random items.
    #[serde(default = "default_true")]
    pub remove_whistles: bool,
    /// Fix W3 drawbridges so all paths are always passable.
    #[serde(default = "default_true")]
    pub fix_drawbridges: bool,
    /// Remove rocks blocking paths (W2 secret path, W3 boat dock).
    #[serde(default = "default_true", alias = "remove_w2_rock")]
    pub remove_rocks: bool,
    /// Clear cards instantly (no cutscene, no lives) when collecting one of each type.
    #[serde(default = "default_true")]
    pub card_speed_clear: bool,
    /// Remove N-card (N-Spade) panels from the overworld map.
    #[serde(default = "default_true")]
    pub remove_n_cards: bool,
    /// Skip the wand falling cutscene after defeating a Koopaling.
    #[serde(default = "default_true")]
    pub skip_wand_cutscene: bool,
    /// Adjust hitboxes for Bowser and Koopalings so they're easier to hit.
    #[serde(default = "default_true")]
    pub adjust_boss_hitboxes: bool,
    /// Randomize per-Koopaling stomp counts (each gets 1–5 hits independently).
    #[serde(default = "default_true")]
    pub koopaling_hits: bool,
    /// Make Koopalings vulnerable to thrown hammers (clears invulnerability flag).
    #[serde(default)]
    pub hammer_vulnerable_koopalings: bool,
    /// Randomize which Koopaling appears in each world (shuffle boss identity).
    #[serde(default)]
    pub random_koopalings: bool,
    /// Hammer item also breaks fortress lock tiles on the overworld map.
    #[serde(default)]
    pub hammer_breaks_locks: bool,
    /// Hammer item also breaks water gap (bridge) tiles on the overworld map.
    #[serde(default)]
    pub hammer_breaks_bridges: bool,
    /// Remove spade (card matching) games from the overworld, freeing map slots for levels.
    #[serde(default = "default_true")]
    pub remove_spade_games: bool,
    /// Include ~9 unreferenced beta levels in the overworld shuffle pool.
    #[serde(default)]
    pub include_beta_stages: bool,

    // --- Per-class enemy tri-state toggles ---
    /// Ground-walking enemies (Goomba, Spiny, Spike, etc.)
    #[serde(default = "default_shuffle")]
    pub ground: EnemyMode,
    /// Shell-producing enemies (Koopa, Buzzy Beetle, etc.)
    #[serde(default = "default_shuffle")]
    pub shell: EnemyMode,
    /// Flying/hopping enemies (Paratroopa, Paragoomba, etc.)
    #[serde(default = "default_shuffle")]
    pub flying: EnemyMode,
    /// Cheep cheep variants (overworld jumping types)
    #[serde(default = "default_shuffle")]
    pub cheeps: EnemyMode,
    /// Bullet Bill variants (standard and homing)
    #[serde(default = "default_shuffle")]
    pub bullet_bills: EnemyMode,
    /// Piranha plant variants (upward + ceiling)
    #[serde(default = "default_shuffle")]
    pub piranhas: EnemyMode,
    /// Ghost house enemies (Boo, Hot Foot)
    #[serde(default = "default_shuffle")]
    pub ghosts: EnemyMode,
    /// Thwomp movement variants
    #[serde(default = "default_off")]
    pub thwomps: EnemyMode,
    /// Rotodisc rotation variants
    #[serde(default = "default_off")]
    pub rotodiscs: EnemyMode,
    /// Cannon fire directions and types (5 sub-classes merge under Wild)
    #[serde(default = "default_off")]
    pub cannons: EnemyMode,
    /// Water enemies (Blooper, Big Bertha, etc.)
    #[serde(default = "default_shuffle")]
    pub water: EnemyMode,
    /// Hammer/Boomerang/Fire/Heavy Bros (only in non-HB segments)
    #[serde(default = "default_shuffle")]
    pub bros: EnemyMode,
    /// All enemies in Hammer Bro encounter segments
    #[serde(default = "default_off")]
    pub hb_encounters: EnemyMode,
    /// Inject Lakitu/Angry Sun/Boss Bass into ~15% of segments (CHR-compatible)
    #[serde(default)]
    pub wild_injections: bool,
}

fn default_false() -> bool {
    false
}

fn default_true() -> bool {
    true
}

const FLAG_KEY_VERSION: u8 = 15;
const FLAG_KEY_PREFIX: &str = "SMB3R-";

/// Crockford Base-32 alphabet (excludes I, L, O, U to avoid ambiguity).
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Encode a byte slice into a Crockford Base-32 string.
/// Pads the final group with zero bits as needed.
fn base32_encode(data: &[u8]) -> String {
    let bit_len = data.len() * 8;
    let out_len = (bit_len + 4) / 5; // ceil(bits / 5)
    let mut result = String::with_capacity(out_len);
    let mut buf: u64 = 0;
    let mut bits: u32 = 0;
    for &byte in data {
        buf = (buf << 8) | byte as u64;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            result.push(CROCKFORD[((buf >> bits) & 0x1F) as usize] as char);
        }
    }
    if bits > 0 {
        result.push(CROCKFORD[((buf << (5 - bits)) & 0x1F) as usize] as char);
    }
    result
}

/// Decode a Crockford Base-32 string back into bytes.
/// Accepts mixed case; normalizes I→1, L→1, O→0 per Crockford spec.
fn base32_decode(s: &str, expected_bytes: usize) -> Result<Vec<u8>, String> {
    let mut buf: u64 = 0;
    let mut bits: u32 = 0;
    let mut result = Vec::with_capacity(expected_bytes);
    for ch in s.chars() {
        let val = match ch.to_ascii_uppercase() {
            '0' | 'O' => 0,
            '1' | 'I' | 'L' => 1,
            '2' => 2, '3' => 3, '4' => 4, '5' => 5, '6' => 6, '7' => 7,
            '8' => 8, '9' => 9,
            'A' => 10, 'B' => 11, 'C' => 12, 'D' => 13, 'E' => 14, 'F' => 15,
            'G' => 16, 'H' => 17, 'J' => 18, 'K' => 19,
            'M' => 20, 'N' => 21, 'P' => 22, 'Q' => 23,
            'R' => 24, 'S' => 25, 'T' => 26, 'V' => 27,
            'W' => 28, 'X' => 29, 'Y' => 30, 'Z' => 31,
            c => return Err(format!("Invalid character in flag key: '{c}'")),
        };
        buf = (buf << 5) | val as u64;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            result.push((buf >> bits) as u8);
        }
    }
    if result.len() < expected_bytes {
        return Err(format!("Flag key too short (decoded {} bytes, expected {})", result.len(), expected_bytes));
    }
    result.truncate(expected_bytes);
    Ok(result)
}
/// Free space in PRG012 after the Big ? Block trampoline (0x19DD0 region).
/// The trampoline uses 0x19DD0–0x19DE1; we place the 16-byte stamp at 0x19DF0.
const STAMP_OFFSET: usize = 0x19DF0;

/// Resolve a starting item value: sentinels (14/15/16) become random concrete
/// items; concrete values (0–13) pass through unchanged.
pub fn resolve_starting_item(item: u8, rng: &mut ChaCha8Rng) -> u8 {
    match item {
        ITEM_RANDOM => {
            // Any item 1–13
            let pool: Vec<u8> = (1..=13).collect();
            *pool.choose(rng).unwrap()
        }
        ITEM_RANDOM_NO_WHISTLE => {
            // Any item 1–13 except whistle (0x0C)
            let pool: Vec<u8> = (1..=13).filter(|&v| v != 0x0C).collect();
            *pool.choose(rng).unwrap()
        }
        ITEM_RANDOM_SUIT_ONLY => {
            // Suits only: mushroom(1) through hammer suit(6)
            let pool: Vec<u8> = (1..=6).collect();
            *pool.choose(rng).unwrap()
        }
        _ => item,
    }
}

impl Options {
    /// Encode options into raw bytes.
    pub fn to_flag_bytes(&self) -> [u8; 11] {
        let level_shuffle_val = match self.level_shuffle {
            LevelShuffle::Off => 0u8,
            LevelShuffle::IntraWorld => 1,
            LevelShuffle::CrossWorld => 2,
        };

        let b0 = FLAG_KEY_VERSION;

        // b1: non-enemy flags
        let b1 = (self.powerups as u8) << 7
            | (self.hammer_breaks_locks as u8) << 6
            | (self.koopaling_hits as u8) << 5
            | (self.world_order as u8) << 4
            | (self.big_q_blocks as u8) << 3
            | (self.disable_autoscroll as u8) << 2
            | (self.airship_lock as u8) << 1
            | (self.chest_items as u8);

        let b2 = (self.remove_whistles as u8) << 7
            | (self.map_shuffle as u8) << 6
            | (self.shuffle_pipes as u8) << 5
            | (self.fix_drawbridges as u8) << 4
            | (self.remove_rocks as u8) << 3
            | (level_shuffle_val & 0x03) << 1
            | (self.shuffle_airships as u8);

        let b3 = ((self.hammer_breaks_bridges as u8) << 7)
            | (self.starting_lives.min(99).max(1) & 0x7F);

        let b4 = (self.card_speed_clear as u8) << 7
            | (self.remove_n_cards as u8) << 6
            | (self.skip_wand_cutscene as u8) << 5
            | (self.adjust_boss_hitboxes as u8) << 4
            | (self.remove_spade_games as u8) << 3
            | (self.hammer_vulnerable_koopalings as u8) << 2;
            // bits 1-0 free

        // Helper to encode EnemyMode as 2 bits
        fn em(m: EnemyMode) -> u8 {
            match m {
                EnemyMode::Off => 0,
                EnemyMode::Shuffle => 1,
                EnemyMode::Wild => 2,
            }
        }

        // b5: ground(7-6) shell(5-4) flying(3-2) cheeps(1-0)
        let b5 = em(self.ground) << 6
            | em(self.shell) << 4
            | em(self.flying) << 2
            | em(self.cheeps);

        // b6: bullet_bills(7-6) piranhas(5-4) ghosts(3-2) thwomps(1-0)
        let b6 = em(self.bullet_bills) << 6
            | em(self.piranhas) << 4
            | em(self.ghosts) << 2
            | em(self.thwomps);

        // b7: rotodiscs(7-6) cannons(5-4) water(3-2) bros(1-0)
        // But we also need hb_encounters(2 bits) and wild_injections(1 bit)
        // = 5 tri-states (10 bits) + 1 bool = 11 bits. We have 16 bits (b7+overflow).
        // Rearrange: put last 5 tri-states + injection across b7 and steal bits from b4.
        //
        // b7: rotodiscs(7-6) cannons(5-4) water(3-2) bros(1-0)
        let b7 = em(self.rotodiscs) << 6
            | em(self.cannons) << 4
            | em(self.water) << 2
            | em(self.bros);

        // Use b4 bits 2-0 for hb_encounters(2 bits) + wild_injections(1 bit)
        let b4 = b4
            | (em(self.hb_encounters) << 1)
            | (self.wild_injections as u8);

        // b8-b9: starting items (3 nibbles, 0 = none)
        // For sentinel values (>=14), store 0 in the nibble and encode
        // the random mode in b10 bits 5-0 instead.
        let items = &self.starting_items;
        fn item_nibble(item: u8) -> u8 {
            if item >= ITEM_RANDOM { 0 } else { item }
        }
        fn item_mode(item: u8) -> u8 {
            match item {
                ITEM_RANDOM => 1,
                ITEM_RANDOM_NO_WHISTLE => 2,
                ITEM_RANDOM_SUIT_ONLY => 3,
                _ => 0,
            }
        }
        let i0 = items.first().copied().unwrap_or(0);
        let i1 = items.get(1).copied().unwrap_or(0);
        let i2 = items.get(2).copied().unwrap_or(0);
        let b8 = (item_nibble(i0) << 4) | item_nibble(i1);
        let b9 = (item_nibble(i2) << 4) | (self.world_count.clamp(1, 7) & 0x0F);

        // b10: extra flags + per-slot random mode (2 bits each)
        let b10 = (self.random_koopalings as u8) << 7
            | (self.include_beta_stages as u8) << 6
            | (item_mode(i0) << 4)
            | (item_mode(i1) << 2)
            | item_mode(i2);

        [b0, b1, b2, b3, b4, b5, b6, b7, b8, b9, b10]
    }

    /// Encode options into a compact Crockford Base-32 flag key (e.g. "SMB3R-1S0G...").
    pub fn to_flag_key(&self) -> String {
        let bytes = self.to_flag_bytes();
        let mut key = String::with_capacity(6 + 18);
        key.push_str(FLAG_KEY_PREFIX);
        key.push_str(&base32_encode(&bytes));
        key
    }

    /// Decode a Crockford Base-32 flag key string into Options.
    pub fn from_flag_key(key: &str) -> Result<Options, String> {
        let encoded = key.strip_prefix(FLAG_KEY_PREFIX)
            .or_else(|| key.strip_prefix("smb3r-"))
            .unwrap_or(key);

        let bytes = base32_decode(encoded, 11)?;

        let version = bytes[0];
        if version != FLAG_KEY_VERSION {
            return Err(format!("Unsupported flag key version {version} (expected {FLAG_KEY_VERSION})"));
        }

        let b1 = bytes[1];
        let b2 = bytes[2];
        let b3 = bytes[3];
        let b4 = bytes[4];
        let b5 = bytes[5];
        let b6 = bytes[6];
        let b7 = bytes[7];
        let b8 = bytes[8];
        let b9 = bytes[9];
        let b10 = bytes[10];

        let level_shuffle_val = (b2 >> 1) & 0x03;
        let level_shuffle = match level_shuffle_val {
            1 => LevelShuffle::IntraWorld,
            2 => LevelShuffle::CrossWorld,
            _ => LevelShuffle::Off,
        };
        let starting_lives = b3 & 0x7F;
        let starting_lives = if starting_lives == 0 { 1 } else { starting_lives };

        fn dem(bits: u8) -> EnemyMode {
            match bits & 0x03 {
                1 => EnemyMode::Shuffle,
                2 => EnemyMode::Wild,
                _ => EnemyMode::Off,
            }
        }

        Ok(Options {
            powerups: (b1 >> 7) & 1 != 0,
            palettes: true, // cosmetic — not encoded in flag key
            hammer_breaks_locks: (b1 >> 6) & 1 != 0,
            koopaling_hits: (b1 >> 5) & 1 != 0,
            world_order: (b1 >> 4) & 1 != 0,
            big_q_blocks: (b1 >> 3) & 1 != 0,
            disable_autoscroll: (b1 >> 2) & 1 != 0,
            airship_lock: (b1 >> 1) & 1 != 0,
            chest_items: b1 & 1 != 0,
            remove_whistles: (b2 >> 7) & 1 != 0,
            map_shuffle: (b2 >> 6) & 1 != 0,
            shuffle_pipes: (b2 >> 5) & 1 != 0,
            shuffle_airships: b2 & 1 != 0,
            fix_drawbridges: (b2 >> 4) & 1 != 0,
            remove_rocks: (b2 >> 3) & 1 != 0,
            level_shuffle,
            starting_lives,
            card_speed_clear: (b4 >> 7) & 1 != 0,
            remove_n_cards: (b4 >> 6) & 1 != 0,
            skip_wand_cutscene: (b4 >> 5) & 1 != 0,
            adjust_boss_hitboxes: (b4 >> 4) & 1 != 0,
            remove_spade_games: (b4 >> 3) & 1 != 0,
            hammer_vulnerable_koopalings: (b4 >> 2) & 1 != 0,
            random_koopalings: (b10 >> 7) & 1 != 0,
            include_beta_stages: (b10 >> 6) & 1 != 0,
            hammer_breaks_bridges: (b3 >> 7) & 1 != 0,
            ground: dem(b5 >> 6),
            shell: dem(b5 >> 4),
            flying: dem(b5 >> 2),
            cheeps: dem(b5),
            bullet_bills: dem(b6 >> 6),
            piranhas: dem(b6 >> 4),
            ghosts: dem(b6 >> 2),
            thwomps: dem(b6),
            rotodiscs: dem(b7 >> 6),
            cannons: dem(b7 >> 4),
            water: dem(b7 >> 2),
            bros: dem(b7),
            hb_encounters: dem(b4 >> 1),
            wild_injections: b4 & 1 != 0,
            starting_items: {
                // Decode per-slot random mode from b10 bits 5-0
                fn mode_to_sentinel(mode: u8, nibble: u8) -> u8 {
                    match mode & 0x03 {
                        1 => ITEM_RANDOM,
                        2 => ITEM_RANDOM_NO_WHISTLE,
                        3 => ITEM_RANDOM_SUIT_ONLY,
                        _ => nibble,
                    }
                }
                let i0 = mode_to_sentinel((b10 >> 4) & 0x03, b8 >> 4);
                let i1 = mode_to_sentinel((b10 >> 2) & 0x03, b8 & 0x0F);
                let i2 = mode_to_sentinel(b10 & 0x03, b9 >> 4);
                let mut items = Vec::new();
                if i0 != 0 { items.push(i0); }
                if i1 != 0 { items.push(i1); }
                if i2 != 0 { items.push(i2); }
                items
            },
            world_count: {
                let wc = b9 & 0x0F;
                if wc == 0 { 7 } else { wc.clamp(1, 7) }
            },
        })
    }

    /// Returns true if any enemy class is enabled (not Off).
    pub fn any_enemies_active(&self) -> bool {
        self.ground != EnemyMode::Off || self.shell != EnemyMode::Off
            || self.flying != EnemyMode::Off || self.cheeps != EnemyMode::Off
            || self.bullet_bills != EnemyMode::Off || self.piranhas != EnemyMode::Off
            || self.ghosts != EnemyMode::Off || self.thwomps != EnemyMode::Off
            || self.rotodiscs != EnemyMode::Off || self.cannons != EnemyMode::Off
            || self.water != EnemyMode::Off || self.bros != EnemyMode::Off
            || self.hb_encounters != EnemyMode::Off || self.wild_injections
    }
}

impl Default for Options {
    fn default() -> Self {
        Options {
            powerups: true,
            palettes: true,
            world_order: false,
            world_count: default_world_count(),
            big_q_blocks: false,
            level_shuffle: LevelShuffle::Off,
            map_shuffle: true,
            shuffle_pipes: true,
            shuffle_airships: true,
            disable_autoscroll: true,
            airship_lock: true,
            chest_items: true,
            remove_whistles: true,
            fix_drawbridges: true,
            remove_rocks: true,
            card_speed_clear: true,
            remove_n_cards: true,
            skip_wand_cutscene: true,
            adjust_boss_hitboxes: true,
            koopaling_hits: true,
            hammer_vulnerable_koopalings: false,
            random_koopalings: false,
            include_beta_stages: false,
            hammer_breaks_locks: false,
            hammer_breaks_bridges: false,
            remove_spade_games: true,
            ground: EnemyMode::Shuffle,
            shell: EnemyMode::Shuffle,
            flying: EnemyMode::Shuffle,
            cheeps: EnemyMode::Shuffle,
            bullet_bills: EnemyMode::Shuffle,
            piranhas: EnemyMode::Shuffle,
            ghosts: EnemyMode::Shuffle,
            thwomps: EnemyMode::Off,
            rotodiscs: EnemyMode::Off,
            cannons: EnemyMode::Off,
            water: EnemyMode::Shuffle,
            bros: EnemyMode::Shuffle,
            hb_encounters: EnemyMode::Off,
            wild_injections: false,
            starting_lives: default_starting_lives(),
            starting_items: Vec::new(),
        }
    }
}

/// Apply all enabled randomizations to a ROM using the given seed.
pub fn randomize(rom: &mut Rom, seed: u64, options: &Options) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    // Resolve random starting items up front (deterministic from seed)
    let resolved_items: Vec<u8> = options.starting_items.iter()
        .map(|&item| resolve_starting_item(item, &mut rng))
        .collect();

    // QoL map patches run first so all subsequent overworld operations
    // (fortress redistribution, pipe shuffle, lock shuffle) see the final
    // map connectivity and store correct replacement tiles.
    if options.fix_drawbridges {
        rom.set_tag("qol/drawbridges");
        randomize::qol::fix_w3_drawbridges(rom);
    }
    if options.remove_rocks {
        rom.set_tag("qol/w2_rock");
        randomize::qol::remove_w2_rock(rom);
        rom.set_tag("qol/w3_boat_rock");
        randomize::qol::remove_w3_boat_rock(rom);
    }

    // Fix Big ? Block bonus rooms so they follow the level, not the world slot.
    // Always applied — needed whenever world_order or cross-world shuffle is active,
    // and harmless (identity mapping) when worlds aren't shuffled.
    rom.set_tag("qol/big_q_blocks");
    randomize::qol::fix_big_q_block_rooms(rom);

    // Autoscroll must run BEFORE powerups and the overworld builder:
    // it writes pre-baked replacement level data for airship levels, and
    // powerups/enemies need to randomize on top of that patched data.
    // It also writes airship pointer table redirects to hardcoded vanilla
    // offsets — the overworld builder's resort_pointer_table() rearranges
    // entries later, so autoscroll must go first.
    if options.disable_autoscroll {
        rom.set_tag("autoscroll");
        randomize::autoscroll::disable_autoscroll(rom);
    }
    if options.powerups {
        rom.set_tag("powerups");
        randomize::powerups::randomize(rom, &mut rng, options.hammer_vulnerable_koopalings);
    }
    if options.palettes {
        rom.set_tag("palettes");
        let mut palette_rng = ChaCha8Rng::from_os_rng();
        randomize::palettes::randomize(rom, &mut palette_rng);
    }
    if options.any_enemies_active() {
        rom.set_tag("enemies");
        randomize::enemies::randomize(rom, &mut rng, options);
    }
    if options.world_order {
        rom.set_tag("world_order");
        randomize::world_order::randomize(rom, &mut rng, options.world_count);
    }
    if options.big_q_blocks {
        rom.set_tag("enemies/big_q_blocks");
        randomize::enemies::randomize_big_q_blocks(rom, &mut rng);
    }
    // Airship shuffle runs after autoscroll (which patches airship pointer
    // entries at vanilla indices) and before the overworld builder (whose
    // resort_pointer_table re-sorts everything). shuffle_entries only moves
    // tileset + ObjSets + LevelLayouts, preserving row/col position, so
    // patched data travels correctly to its new world.
    if options.shuffle_airships {
        rom.set_tag("levels/airships");
        randomize::levels::randomize_airships(rom, &mut rng);
    }

    // Koopaling stability patches — needed whenever Koopalings may load in a
    // non-native world (airship shuffle, identity remap) or when the hammer
    // vulnerability patch is applied. Covers the softlock fix plus Fred's
    // three guards (phantom double-stomps, stale VRAM writes, Y wraparound).
    let koopalings_may_travel = options.shuffle_airships
        || options.hammer_vulnerable_koopalings
        || options.random_koopalings;
    if koopalings_may_travel {
        rom.set_tag("koopalings/fix_softlock");
        randomize::koopalings::fix_koopaling_softlock(rom);
        rom.set_tag("koopalings/collision_guard");
        randomize::koopalings::koopaling_collision_guard(rom);
        rom.set_tag("koopalings/vram_clear");
        randomize::koopalings::koopaling_vram_clear(rom);
        rom.set_tag("koopalings/y_clamp");
        randomize::koopalings::koopaling_y_clamp(rom);
    }

    // Make Koopalings vulnerable to thrown hammers (PRG000 $8302).
    if options.hammer_vulnerable_koopalings {
        rom.set_tag("koopalings/hammer_vulnerable");
        randomize::koopalings::hammer_vulnerable_koopalings(rom);
    }

    // Random Koopaling identity remap (Fred's Map_Unused7EEA hijack).
    if options.random_koopalings {
        rom.set_tag("koopalings/random_identity");
        randomize::koopalings::random_koopalings(rom, &mut rng);
    }

    // Two mutually exclusive modes:
    // 1. Map Shuffle: overworld builder rebuilds the map (always cross-world).
    // 2. Vanilla Layout: tiles stay in place, level entries shuffled underneath.
    if options.map_shuffle {
        rom.set_tag("overworld/builder");

        if options.include_beta_stages {
            randomize::qol::fix_beta_stages(rom);
        }

        let catalog = randomize::node_catalog::NodeCatalog::build(rom, options.include_beta_stages);
        let pickup = randomize::overworld_pickup::pick_up(rom, &catalog, options.remove_spade_games);
        let build = randomize::overworld_build::build(rom, &pickup, &catalog, &mut rng);
        randomize::overworld_writer::write_overworld(
            rom, &build, &pickup, &catalog, &mut rng, true,
        );
    } else {
        match options.level_shuffle {
            LevelShuffle::IntraWorld => {
                rom.set_tag("levels");
                randomize::levels::randomize_intra(rom, &mut rng);
            }
            LevelShuffle::CrossWorld => {
                rom.set_tag("levels");
                randomize::levels::randomize_cross(rom, &mut rng);
            }
            LevelShuffle::Off => {}
        }
    }
    if options.chest_items {
        rom.set_tag("items");
        randomize::items::randomize(rom, &mut rng, options.remove_whistles);
    } else if options.remove_whistles {
        rom.set_tag("items/whistles");
        randomize::items::remove_whistles_only(rom, &mut rng);
    }

    // Set starting lives (patched later by starting_items trampoline if items present)
    rom.set_tag("qol/starting_lives");
    randomize::qol::set_starting_lives(rom, options.starting_lives);

    // Airship lock (anchor effect always on): patch at 0x1FABC ("KXUUXZVG" / Game Genie)
    if options.airship_lock {
        rom.set_tag("airship_lock");
        // A9 01 EA = LDA #$01; NOP (forces anchor flag always set)
        rom.write_range(0x1FABC, &[0xA9, 0x01, 0xEA]);
        // Anchors stay in inventory as mystery items — patch the item-use
        // dispatch so using an anchor triggers a random powerup effect.
        rom.set_tag("items/mystery_anchor");
        randomize::items::write_mystery_anchor(rom, &mut rng);
    }

    // Patch double-digit level tiles (11–19) to show a "1" tens digit
    rom.set_tag("metatile/double_digit");
    randomize::overworld_writer::patch_double_digit_metatiles(rom);

    // Randomize king quotes (always on — cosmetic flavor text)
    rom.set_tag("king_quotes");
    randomize::king_quotes::randomize(rom, &mut rng);

    // Skip the wand falling cutscene after defeating a Koopaling.
    if options.skip_wand_cutscene {
        rom.set_tag("koopalings/skip_wand_cutscene");
        randomize::koopalings::skip_wand_cutscene(rom);
    }

    // Remove N-card (N-Spade) panels from the overworld map.
    if options.remove_n_cards {
        rom.set_tag("qol/remove_n_cards");
        randomize::qol::remove_n_cards(rom);
    }

    // Fix W3 canoe softlocks (needed when spade games are removed, since levels
    // can be placed on W3 island tiles that the canoe interacts with).
    if options.remove_spade_games {
        rom.set_tag("qol/fix_canoe_softlock");
        randomize::qol::fix_canoe_softlock(rom);
    }

    // Adjust Bowser and Koopaling hitboxes.
    if options.adjust_boss_hitboxes {
        rom.set_tag("koopalings/adjust_boss_hitboxes");
        randomize::koopalings::adjust_boss_hitboxes(rom);
    }

    // Per-Koopaling random stomp counts (1–5 hits each).
    if options.koopaling_hits {
        rom.set_tag("koopalings/random_hits");
        randomize::koopalings::randomize_koopaling_hits(rom, &mut rng);
    }

    // Hammer breaks tiles on the overworld map (locks, bridges, or both).
    if options.hammer_breaks_locks || options.hammer_breaks_bridges {
        rom.set_tag("qol/hammer_breaks_tiles");
        randomize::qol::hammer_breaks_tiles(rom, options.hammer_breaks_locks, options.hammer_breaks_bridges);
    }

    // Card speed clear: one-of-each clears cards with +1 life but no cutscene.
    if options.card_speed_clear {
        rom.set_tag("qol/card_speed_clear");
        randomize::qol::card_speed_clear(rom);
    }

    // Title screen seed hash icons (always on — cosmetic verification).
    // This hooks STA $0736 at 0x308E2 for intro skip.
    rom.set_tag("title_screen");
    randomize::title_screen::write_seed_hash(rom, seed, options);

    // Starting items trampoline — must run AFTER title_screen because both
    // write to the lives init region at 0x308E0. The trampoline incorporates
    // the intro skip (LDA #$06; STA $DE) so the title_screen hook is preserved.
    if !options.starting_items.is_empty() {
        rom.set_tag("qol/starting_items");
        randomize::qol::write_starting_items(rom, options.starting_lives, &resolved_items);
    }

    // Stamp flag key + seed into free space at STAMP_OFFSET (PRG012). 23 bytes:
    //   [0..4]   "S3R\xNN" magic + version
    //   [4..15]  flag key bytes (11 bytes in v12)
    //   [15..23] seed (little-endian u64)
    rom.set_tag("stamp");
    let flag_bytes = options.to_flag_bytes();
    let mut stamp = [0u8; 23];
    stamp[0..3].copy_from_slice(b"S3R");
    stamp[3] = FLAG_KEY_VERSION;
    stamp[4..15].copy_from_slice(&flag_bytes);
    stamp[15..23].copy_from_slice(&seed.to_le_bytes());
    rom.write_range(STAMP_OFFSET, &stamp);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::Rom;

    const ANCHOR_PATCH_OFFSET: usize = 0x1FABC;
    const PATCHED_BYTES: [u8; 3] = [0xA9, 0x01, 0xEA];
    const ANCHOR: u8 = 0x0A;

    // Item table offsets (must match items.rs)
    const HAMMER_BROS_ITEMS_OFFSET: usize = 0x16190;
    const TOAD_HOUSE_ITEMS_OFFSET: usize = 0x3B14B;

    /// Options safe for zeroed test ROMs (map_shuffle off — builder needs real ROM data).
    /// Palettes disabled because they use OS entropy (cosmetic, decoupled from seed).
    fn test_options() -> Options {
        let mut opts = Options::default();
        opts.map_shuffle = false;
        opts.shuffle_airships = false;
        opts.palettes = false;
        opts
    }

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        // iNES header
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn randomized_rom_has_anchor_lock_patch_by_default() {
        let mut rom = make_test_rom();
        let original_bytes = rom.read_range(ANCHOR_PATCH_OFFSET, 3).to_vec();
        let options = test_options();
        randomize(&mut rom, 0x12345678, &options);

        assert_eq!(
            rom.read_range(ANCHOR_PATCH_OFFSET, 3),
            &PATCHED_BYTES,
            "Anchor lock patch should be present by default"
        );
        // Sanity: the patch actually changed something
        assert_ne!(
            original_bytes, PATCHED_BYTES,
            "Test ROM should not already contain the patch bytes"
        );
    }

    #[test]
    fn anchor_lock_patch_can_be_disabled() {
        let mut rom = make_test_rom();
        let original_bytes = rom.read_range(ANCHOR_PATCH_OFFSET, 3).to_vec();
        let mut options = test_options();
        options.airship_lock = false;
        randomize(&mut rom, 0x12345678, &options);

        assert_eq!(
            rom.read_range(ANCHOR_PATCH_OFFSET, 3),
            &original_bytes[..],
            "Anchor lock patch must NOT be present when airship_lock = false"
        );
    }

    #[test]
    fn mystery_anchor_trampoline_when_airship_lock_on() {
        let mut rom = make_test_rom();
        // Place anchors in item tables — they should stay as 0x0A
        rom.write_byte(HAMMER_BROS_ITEMS_OFFSET + 2, ANCHOR);
        rom.write_byte(TOAD_HOUSE_ITEMS_OFFSET + 1, ANCHOR);

        let mut options = test_options();
        options.airship_lock = true;
        options.chest_items = false;
        options.remove_whistles = false;
        randomize(&mut rom, 0x12345678, &options);

        // Anchor items should remain in data tables (mystery behavior)
        assert_eq!(rom.read_byte(HAMMER_BROS_ITEMS_OFFSET + 2), ANCHOR,
            "Anchor should stay in item table (mystery item)");
        assert_eq!(rom.read_byte(TOAD_HOUSE_ITEMS_OFFSET + 1), ANCHOR,
            "Anchor should stay in item table (mystery item)");

        // Trampoline should be written at PRG026 free space
        const FS: usize = 0x35572;
        // Trampoline starts with LDX $7D80,Y (0xBE)
        assert_eq!(rom.read_byte(FS), 0xBE, "Trampoline LDX abs,Y opcode");
        // Target powerup is at offset +8 (LDX #imm operand)
        let target = rom.read_byte(FS + 8);
        assert!(target >= 0x01 && target <= 0x08,
            "Trampoline target 0x{target:02X} should be a valid mystery pool item (1-8)");

        // DynJump table entry at 0x34564: $A5B6 (Inv_UseItem_Powerup)
        assert_eq!(rom.read_range(0x34564, 2), &[0xB6, 0xA5]);
        // Hook at 0x345D8: JSR $B562
        assert_eq!(rom.read_range(0x345D8, 3), &[0x20, 0x62, 0xB5]);
    }

    #[test]
    fn mystery_anchor_not_written_when_airship_lock_off() {
        let mut rom = make_test_rom();
        rom.write_byte(HAMMER_BROS_ITEMS_OFFSET + 2, ANCHOR);

        let mut options = test_options();
        options.airship_lock = false;
        options.chest_items = false;
        options.remove_whistles = false;
        randomize(&mut rom, 0x12345678, &options);

        // Anchor should stay and no trampoline written
        assert_eq!(rom.read_byte(HAMMER_BROS_ITEMS_OFFSET + 2), ANCHOR,
            "Anchor should be preserved when airship_lock is off");
        // Dispatch should not be patched
        assert_ne!(rom.read_range(0x3E500, 3), &[0x4C, 0x40, 0xE2],
            "Dispatch should NOT be patched when airship_lock is off");
    }

    #[test]
    fn write_log_populated_after_randomize() {
        let mut rom = make_test_rom();
        let options = test_options();
        randomize(&mut rom, 0x12345678, &options);

        let log = rom.write_log();
        assert!(!log.is_empty(), "Write log should be non-empty after randomize");

        // Every write should have a proper tag (not "untagged")
        for record in log {
            assert_ne!(
                record.tag, "untagged",
                "Write at offset 0x{:05X} has no tag",
                record.offset
            );
        }
    }

    #[test]
    fn flag_key_round_trip_defaults() {
        let opts = Options::default();
        let key = opts.to_flag_key();
        assert!(key.starts_with("SMB3R-"));
        assert_eq!(key.len(), 24); // "SMB3R-" + 18 base32
        let decoded = Options::from_flag_key(&key).unwrap();
        assert_eq!(opts.powerups, decoded.powerups);
        assert_eq!(opts.palettes, decoded.palettes);
        assert_eq!(opts.world_order, decoded.world_order);
        assert_eq!(opts.world_count, decoded.world_count);
        assert_eq!(opts.big_q_blocks, decoded.big_q_blocks);
        assert_eq!(opts.disable_autoscroll, decoded.disable_autoscroll);
        assert_eq!(opts.airship_lock, decoded.airship_lock);
        assert_eq!(opts.chest_items, decoded.chest_items);
        assert_eq!(opts.remove_whistles, decoded.remove_whistles);
        assert_eq!(opts.map_shuffle, decoded.map_shuffle);
        assert_eq!(opts.shuffle_pipes, decoded.shuffle_pipes);
        assert_eq!(opts.shuffle_airships, decoded.shuffle_airships);
        assert_eq!(opts.fix_drawbridges, decoded.fix_drawbridges);
        assert_eq!(opts.remove_rocks, decoded.remove_rocks);
        assert_eq!(opts.level_shuffle, decoded.level_shuffle);
        assert_eq!(opts.starting_lives, decoded.starting_lives);
        assert_eq!(opts.card_speed_clear, decoded.card_speed_clear);
        assert_eq!(opts.remove_n_cards, decoded.remove_n_cards);
        assert_eq!(opts.skip_wand_cutscene, decoded.skip_wand_cutscene);
        assert_eq!(opts.adjust_boss_hitboxes, decoded.adjust_boss_hitboxes);
        assert_eq!(opts.ground, decoded.ground);
        assert_eq!(opts.shell, decoded.shell);
        assert_eq!(opts.flying, decoded.flying);
        assert_eq!(opts.cheeps, decoded.cheeps);
        assert_eq!(opts.bullet_bills, decoded.bullet_bills);
        assert_eq!(opts.piranhas, decoded.piranhas);
        assert_eq!(opts.ghosts, decoded.ghosts);
        assert_eq!(opts.thwomps, decoded.thwomps);
        assert_eq!(opts.rotodiscs, decoded.rotodiscs);
        assert_eq!(opts.cannons, decoded.cannons);
        assert_eq!(opts.water, decoded.water);
        assert_eq!(opts.bros, decoded.bros);
        assert_eq!(opts.hb_encounters, decoded.hb_encounters);
        assert_eq!(opts.wild_injections, decoded.wild_injections);
        assert_eq!(opts.starting_items, decoded.starting_items);
        assert_eq!(opts.hammer_breaks_locks, decoded.hammer_breaks_locks);
        assert_eq!(opts.hammer_breaks_bridges, decoded.hammer_breaks_bridges);
    }

    #[test]
    fn flag_key_round_trip_all_wild() {
        let opts = Options {
            powerups: true,
            palettes: true,
            world_order: true,
            world_count: 7,
            big_q_blocks: true,
            level_shuffle: LevelShuffle::CrossWorld,
            map_shuffle: true,
            shuffle_pipes: true,
            shuffle_airships: true,
            disable_autoscroll: true,
            airship_lock: true,
            chest_items: true,
            remove_whistles: true,
            fix_drawbridges: true,
            remove_rocks: true,
            starting_lives: 99,
            card_speed_clear: true,
            remove_n_cards: true,
            skip_wand_cutscene: true,
            adjust_boss_hitboxes: true,
            koopaling_hits: true,
            hammer_vulnerable_koopalings: true,
            random_koopalings: true,
            include_beta_stages: true,
            hammer_breaks_locks: true,
            hammer_breaks_bridges: true,
            remove_spade_games: true,
            ground: EnemyMode::Wild,
            shell: EnemyMode::Wild,
            flying: EnemyMode::Wild,
            cheeps: EnemyMode::Wild,
            bullet_bills: EnemyMode::Wild,
            piranhas: EnemyMode::Wild,
            ghosts: EnemyMode::Wild,
            thwomps: EnemyMode::Wild,
            rotodiscs: EnemyMode::Wild,
            cannons: EnemyMode::Wild,
            water: EnemyMode::Wild,
            bros: EnemyMode::Wild,
            hb_encounters: EnemyMode::Wild,
            wild_injections: true,
            starting_items: vec![0x05, 0x09, 0x03],
        };
        let key = opts.to_flag_key();
        let decoded = Options::from_flag_key(&key).unwrap();
        assert_eq!(opts.random_koopalings, decoded.random_koopalings);
        assert_eq!(opts.include_beta_stages, decoded.include_beta_stages);
        assert_eq!(opts.starting_items, decoded.starting_items);
        assert_eq!(opts.hammer_breaks_locks, decoded.hammer_breaks_locks);
        assert_eq!(opts.hammer_breaks_bridges, decoded.hammer_breaks_bridges);
        assert_eq!(opts.world_order, decoded.world_order);
        assert_eq!(opts.world_count, decoded.world_count);
        assert_eq!(opts.level_shuffle, decoded.level_shuffle);
        assert_eq!(opts.map_shuffle, decoded.map_shuffle);
        assert_eq!(opts.starting_lives, decoded.starting_lives);
        assert_eq!(opts.ground, decoded.ground);
        assert_eq!(opts.shell, decoded.shell);
        assert_eq!(opts.bullet_bills, decoded.bullet_bills);
        assert_eq!(opts.thwomps, decoded.thwomps);
        assert_eq!(opts.rotodiscs, decoded.rotodiscs);
        assert_eq!(opts.cannons, decoded.cannons);
        assert_eq!(opts.hb_encounters, decoded.hb_encounters);
        assert_eq!(opts.wild_injections, decoded.wild_injections);
    }

    #[test]
    fn flag_key_round_trip_all_off() {
        let opts = Options {
            powerups: false,
            palettes: false,
            world_order: false,
            world_count: 7,
            big_q_blocks: false,
            level_shuffle: LevelShuffle::Off,
            map_shuffle: false,
            shuffle_pipes: false,
            shuffle_airships: false,
            disable_autoscroll: false,
            airship_lock: false,
            chest_items: false,
            remove_whistles: false,
            fix_drawbridges: false,
            remove_rocks: false,
            starting_lives: 1,
            card_speed_clear: false,
            remove_n_cards: false,
            skip_wand_cutscene: false,
            adjust_boss_hitboxes: false,
            koopaling_hits: false,
            hammer_vulnerable_koopalings: false,
            random_koopalings: false,
            include_beta_stages: false,
            hammer_breaks_locks: false,
            hammer_breaks_bridges: false,
            remove_spade_games: false,
            ground: EnemyMode::Off,
            shell: EnemyMode::Off,
            flying: EnemyMode::Off,
            cheeps: EnemyMode::Off,
            bullet_bills: EnemyMode::Off,
            piranhas: EnemyMode::Off,
            ghosts: EnemyMode::Off,
            thwomps: EnemyMode::Off,
            rotodiscs: EnemyMode::Off,
            cannons: EnemyMode::Off,
            water: EnemyMode::Off,
            bros: EnemyMode::Off,
            hb_encounters: EnemyMode::Off,
            wild_injections: false,
            starting_items: vec![],
        };
        let key = opts.to_flag_key();
        let decoded = Options::from_flag_key(&key).unwrap();
        assert!(decoded.starting_items.is_empty());
        assert!(!decoded.powerups);
        assert!(!decoded.hammer_breaks_locks);
        assert!(!decoded.hammer_breaks_bridges);
        assert!(decoded.palettes); // palettes always true from flag key (cosmetic, not encoded)
        assert!(!decoded.disable_autoscroll);
        assert!(!decoded.map_shuffle);
        assert!(!decoded.shuffle_airships);
        assert!(!decoded.remove_spade_games);
        assert_eq!(decoded.ground, EnemyMode::Off);
        assert_eq!(decoded.bullet_bills, EnemyMode::Off);
        assert_eq!(decoded.thwomps, EnemyMode::Off);
        assert_eq!(decoded.hb_encounters, EnemyMode::Off);
        assert!(!decoded.wild_injections);
        assert_eq!(decoded.starting_lives, 1);
        assert_eq!(decoded.level_shuffle, LevelShuffle::Off);
    }

    #[test]
    fn flag_key_case_insensitive_prefix() {
        let opts = Options::default();
        let key = opts.to_flag_key();
        let lower = key.to_lowercase();
        let decoded = Options::from_flag_key(&lower).unwrap();
        assert_eq!(opts.powerups, decoded.powerups);
    }

    #[test]
    fn flag_key_without_prefix() {
        let opts = Options::default();
        let key = opts.to_flag_key();
        let b32 = key.strip_prefix("SMB3R-").unwrap();
        let decoded = Options::from_flag_key(b32).unwrap();
        assert_eq!(opts.powerups, decoded.powerups);
    }

    #[test]
    fn flag_key_invalid_version() {
        // Encode version 0xFF into base32 (first byte = 0xFF, rest zeros)
        let mut bad_bytes = [0u8; 11];
        bad_bytes[0] = 0xFF;
        let key = format!("SMB3R-{}", base32_encode(&bad_bytes));
        let result = Options::from_flag_key(&key);
        assert!(result.is_err());
    }

    #[test]
    fn flag_key_invalid_chars() {
        let result = Options::from_flag_key("SMB3R-!!!!!!!!!!!!!!!!!!!");
        assert!(result.is_err());
    }

    #[test]
    fn base32_round_trip() {
        // Test with various byte patterns
        for data in [
            vec![0u8; 11],
            vec![0xFF; 11],
            vec![0x0E, 0xFF, 0xFE, 0x63, 0xFC, 0xAA, 0xAA, 0xAA, 0x59, 0x37, 0xC0],
            (0..11).collect::<Vec<u8>>(),
        ] {
            let encoded = base32_encode(&data);
            let decoded = base32_decode(&encoded, data.len()).unwrap();
            assert_eq!(data, decoded, "round-trip failed for {data:?} (encoded: {encoded})");
        }
    }

    /// Inline FNV-1a hash — no external dependency needed.
    fn fnv1a(data: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for &b in data {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    /// Build an Options with everything disabled (exercises "skip everything" branches).
    fn all_off_options() -> Options {
        Options {
            powerups: false,
            palettes: false,
            world_order: false,
            world_count: 7,
            big_q_blocks: false,
            level_shuffle: LevelShuffle::Off,
            map_shuffle: false,
            shuffle_pipes: false,
            shuffle_airships: false,
            disable_autoscroll: false,
            airship_lock: false,
            chest_items: false,
            remove_whistles: false,
            fix_drawbridges: false,
            remove_rocks: false,
            starting_lives: 1,
            card_speed_clear: false,
            remove_n_cards: false,
            skip_wand_cutscene: false,
            adjust_boss_hitboxes: false,
            koopaling_hits: false,
            hammer_vulnerable_koopalings: false,
            random_koopalings: false,
            include_beta_stages: false,
            hammer_breaks_locks: false,
            hammer_breaks_bridges: false,
            remove_spade_games: false,
            ground: EnemyMode::Off,
            shell: EnemyMode::Off,
            flying: EnemyMode::Off,
            cheeps: EnemyMode::Off,
            bullet_bills: EnemyMode::Off,
            piranhas: EnemyMode::Off,
            ghosts: EnemyMode::Off,
            thwomps: EnemyMode::Off,
            rotodiscs: EnemyMode::Off,
            cannons: EnemyMode::Off,
            water: EnemyMode::Off,
            bros: EnemyMode::Off,
            hb_encounters: EnemyMode::Off,
            wild_injections: false,
            starting_items: vec![],
        }
    }

    /// Build an Options with all features cranked to max.
    /// Palettes disabled because they use OS entropy (cosmetic, decoupled from seed).
    fn all_on_options() -> Options {
        Options {
            powerups: true,
            palettes: false,
            world_order: true,
            world_count: 3,
            big_q_blocks: true,
            level_shuffle: LevelShuffle::CrossWorld,
            map_shuffle: false, // needs real ROM data
            shuffle_pipes: false,
            shuffle_airships: true,
            disable_autoscroll: true,
            airship_lock: true,
            chest_items: true,
            remove_whistles: true,
            fix_drawbridges: true,
            remove_rocks: true,
            starting_lives: 99,
            card_speed_clear: true,
            remove_n_cards: true,
            skip_wand_cutscene: true,
            adjust_boss_hitboxes: true,
            koopaling_hits: true,
            hammer_vulnerable_koopalings: true,
            random_koopalings: true,
            include_beta_stages: false,
            hammer_breaks_locks: true,
            hammer_breaks_bridges: true,
            remove_spade_games: true,
            ground: EnemyMode::Wild,
            shell: EnemyMode::Wild,
            flying: EnemyMode::Wild,
            cheeps: EnemyMode::Wild,
            bullet_bills: EnemyMode::Wild,
            piranhas: EnemyMode::Wild,
            ghosts: EnemyMode::Wild,
            thwomps: EnemyMode::Wild,
            rotodiscs: EnemyMode::Wild,
            cannons: EnemyMode::Wild,
            water: EnemyMode::Wild,
            bros: EnemyMode::Wild,
            hb_encounters: EnemyMode::Wild,
            wild_injections: true,
            starting_items: vec![0x05, 0x09, 0x03],
        }
    }

    /// Build an Options testing world_order in isolation (no enemy RNG consumption).
    fn world_order_only_options() -> Options {
        let mut opts = all_off_options();
        opts.world_order = true;
        opts.world_count = 5;
        opts
    }

    #[test]
    fn test_full_determinism() {
        let configs: Vec<(&str, Options)> = vec![
            ("defaults", test_options()),
            ("all_on", all_on_options()),
            ("all_off", all_off_options()),
            ("world_order_only", world_order_only_options()),
        ];

        let seed = 42u64;
        for (name, options) in &configs {
            // Run 1
            let mut rom1 = make_test_rom();
            randomize(&mut rom1, seed, options);

            // Run 2 (same seed, same options)
            let mut rom2 = make_test_rom();
            randomize(&mut rom2, seed, options);

            // Same-run determinism — find first differing byte for diagnostics
            let b1 = rom1.output_bytes();
            let b2 = rom2.output_bytes();
            if b1 != b2 {
                for i in 0..b1.len() {
                    if b1[i] != b2[i] {
                        panic!(
                            "{name}: non-determinism at offset 0x{i:05X}: \
                             run1=0x{:02X} run2=0x{:02X}",
                            b1[i], b2[i]
                        );
                    }
                }
            }

            // Verify hashes match (determinism, not pinned to a specific value)
            let hash1 = fnv1a(b1);
            let hash2 = fnv1a(b2);
            assert_eq!(
                hash1, hash2,
                "{name}: hash mismatch between runs (0x{hash1:016X} vs 0x{hash2:016X})"
            );
        }
    }

    #[test]
    fn write_log_tags_match_enabled_modules() {
        let mut rom = make_test_rom();
        let mut options = test_options();
        // Disable optional modules we can check for absence
        options.ground = EnemyMode::Off;
        options.shell = EnemyMode::Off;
        options.flying = EnemyMode::Off;
        options.cheeps = EnemyMode::Off;
        options.bullet_bills = EnemyMode::Off;
        options.piranhas = EnemyMode::Off;
        options.ghosts = EnemyMode::Off;
        options.water = EnemyMode::Off;
        options.bros = EnemyMode::Off;
        options.world_order = false;
        // Keep these on — they write to known offsets even on a zeroed ROM
        options.disable_autoscroll = true;
        options.airship_lock = true;
        randomize(&mut rom, 42, &options);

        let tags: Vec<&str> = rom.write_log().iter().map(|r| r.tag.as_str()).collect();
        // These modules write to fixed offsets that differ from zero
        assert!(tags.iter().any(|t| t.starts_with("autoscroll")));
        assert!(tags.iter().any(|t| t.starts_with("airship_lock")));
        // Disabled modules should not appear
        assert!(!tags.iter().any(|t| t.starts_with("enemies")));
        assert!(!tags.iter().any(|t| t.starts_with("world_order")));
    }

    #[test]
    fn flag_key_round_trip_all_random_items() {
        let mut opts = Options::default();
        opts.starting_items = vec![ITEM_RANDOM, ITEM_RANDOM_NO_WHISTLE, ITEM_RANDOM_SUIT_ONLY];
        let key = opts.to_flag_key();
        let decoded = Options::from_flag_key(&key).unwrap();
        assert_eq!(decoded.starting_items, vec![ITEM_RANDOM, ITEM_RANDOM_NO_WHISTLE, ITEM_RANDOM_SUIT_ONLY]);
    }

    #[test]
    fn flag_key_round_trip_mixed_random_and_concrete() {
        let mut opts = Options::default();
        opts.starting_items = vec![ITEM_RANDOM, 3];
        let key = opts.to_flag_key();
        let decoded = Options::from_flag_key(&key).unwrap();
        assert_eq!(decoded.starting_items, vec![ITEM_RANDOM, 3]);
    }

    #[test]
    fn resolve_starting_item_deterministic() {
        let mut rng1 = ChaCha8Rng::seed_from_u64(42);
        let mut rng2 = ChaCha8Rng::seed_from_u64(42);
        let a = resolve_starting_item(ITEM_RANDOM, &mut rng1);
        let b = resolve_starting_item(ITEM_RANDOM, &mut rng2);
        assert_eq!(a, b, "same seed must produce same item");
    }

    #[test]
    fn resolve_suit_only_in_range() {
        for seed in 0..100u64 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let item = resolve_starting_item(ITEM_RANDOM_SUIT_ONLY, &mut rng);
            assert!((1..=6).contains(&item), "suit-only produced {item}, expected 1-6");
        }
    }

    #[test]
    fn resolve_no_whistle_never_whistle() {
        for seed in 0..100u64 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let item = resolve_starting_item(ITEM_RANDOM_NO_WHISTLE, &mut rng);
            assert_ne!(item, 0x0C, "no-whistle produced a whistle on seed {seed}");
            assert!((1..=13).contains(&item), "no-whistle produced {item}, expected 1-13 (not 12)");
        }
    }

    #[test]
    fn resolve_concrete_passthrough() {
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        assert_eq!(resolve_starting_item(0, &mut rng), 0);
        assert_eq!(resolve_starting_item(5, &mut rng), 5);
        assert_eq!(resolve_starting_item(13, &mut rng), 13);
    }
}
