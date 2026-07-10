//! Randomizer configuration: the `Options` struct, its enums, and the serde
//! defaults that back them.

/// Sentinel: resolve to any random item (1–13).
pub const ITEM_RANDOM: u8 = 14;

/// Sentinel: resolve to any random item except Whistle (1–11, 13).
pub const ITEM_RANDOM_NO_WHISTLE: u8 = 15;

/// Sentinel: resolve to a random suit/powerup (1–6).
pub const ITEM_RANDOM_SUIT_ONLY: u8 = 16;

/// Returns default starting lives (5).
pub(super) fn default_starting_lives() -> u8 { 5 }

/// The four valid starting-lives counts (matches the flag-key encoding
/// and the WASM pill-toggle options).
pub const STARTING_LIVES_VALUES: [u8; 4] = [1, 5, 20, 99];

/// Map a 2-bit flag-key index to the corresponding lives count.
pub(super) fn idx_to_lives(idx: u8) -> u8 {
    STARTING_LIVES_VALUES[(idx & 0x3) as usize]
}

/// Map a lives count to its 2-bit flag-key index. Non-canonical values
/// are binned to the nearest canonical choice so CLI/JSON inputs that
/// predate this layout still round-trip cleanly.
pub(super) fn lives_to_idx(lives: u8) -> u8 {
    match lives {
        n if n <= 2 => 0,   // → 1
        n if n <= 12 => 1,  // → 5
        n if n <= 59 => 2,  // → 20
        _ => 3,             // → 99
    }
}

/// Returns default world count (7 — all worlds before Dark Land).
pub(super) fn default_world_count() -> u8 { 7 }

/// Per-class enemy randomization mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnemyMode {
    #[default]
    Off,
    Shuffle,
    Wild,
}

pub(super) fn default_shuffle() -> EnemyMode { EnemyMode::Shuffle }

pub(super) fn default_off() -> EnemyMode { EnemyMode::Off }

/// Random Fire Flower mode (issue #22). Collecting an in-level Fire Flower
/// grants a power state derived deterministically from the world and the
/// flower's level position, instead of always Fire. `On` substitutes among the
/// four big-form suits (Fire/Frog/Tanooki/Hammer); `Wild` adds the Small/Big
/// downgrade outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FireFlowerMode {
    #[default]
    Off,
    On,
    Wild,
}

/// Piranha shuffle mode. The two W7 piranha plant levels (7-P1/7-P2) are
/// normally pinned to their vanilla map spots. `On` releases them into the
/// global level pool and their plant sprites follow them to wherever they
/// land. `Wild` also releases them (as plain numbered levels), and instead
/// scatters plant sprites onto ~1 random level slot per world — stepping on
/// a plant auto-starts the level under it, vanilla W7 style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiranhaMode {
    #[default]
    Off,
    On,
    Wild,
}

/// Tri-state toggle for player-hidden flags: forced `Off`, forced `On`, or
/// left to the seed (`Maybe`). A `Maybe` flag is resolved to a concrete
/// on/off at generation time from a dedicated RNG substream (see
/// [`Tri::resolve`]), so the same seed + same flags always produce the same
/// ROM — the player just can't tell from the flag key which way a `Maybe`
/// landed, so it can't be planned around.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tri {
    #[default]
    Off,
    On,
    Maybe,
}

impl Tri {
    /// Collapse to a concrete bool. `Off`/`On` pass through; `Maybe` flips a
    /// coin on the provided RNG.
    pub(super) fn resolve<R: rand::Rng>(self, rng: &mut R) -> bool {
        match self {
            Tri::Off => false,
            Tri::On => true,
            Tri::Maybe => rng.random_bool(0.5),
        }
    }
    /// True only for the explicit `On` state — drives the value bit in the flag key.
    pub(super) fn is_on(self) -> bool { matches!(self, Tri::On) }
    /// True only for the `Maybe` state — drives the maybe bit in the flag key.
    pub(super) fn is_maybe(self) -> bool { matches!(self, Tri::Maybe) }
}

pub(super) fn default_tri_on() -> Tri { Tri::On }

/// Options controlling which randomizations to apply.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Options {
    #[serde(default = "default_true")]
    pub powerups: bool,
    /// Player colors: recolor the character wardrobe (random or picked via
    /// `player_color`). Off = vanilla outfits. Cosmetic — not in the flag key.
    #[serde(default = "default_true")]
    pub palettes: bool,
    /// World colors: themed palette randomization of levels, enemies, and
    /// overworld maps. Independent of `palettes`. Cosmetic — not encoded in
    /// the flag key, so flipping this never changes level content.
    #[serde(default)]
    pub palette_themed: bool,
    /// Player-chosen NES color byte anchoring the character wardrobe scheme
    /// (Mario's body gets this color; Luigi and the power-up suits are derived
    /// from it). Must be chromatic (hue nibble 1-C, value <= 0x3C). None =
    /// random color. Cosmetic — not encoded in the flag key.
    /// Only takes effect while `palettes` is on.
    #[serde(default)]
    pub player_color: Option<u8>,
    #[serde(default)]
    pub world_order: bool,
    /// Number of worlds before Dark Land (1–7, default 7).
    #[serde(default = "default_world_count")]
    pub world_count: u8,
    #[serde(default)]
    pub big_q_blocks: bool,
    /// Shuffle pipe endpoint positions during the overworld rebuild.
    #[serde(default = "default_true")]
    pub shuffle_pipes: bool,
    /// Shuffle airship levels across worlds 1-7.
    #[serde(default = "default_true")]
    pub shuffle_airships: bool,
    /// Redistribute the wandering Hammer Bro encounters across all worlds
    /// (random 1-3 per world, 15 total) instead of keeping their vanilla spots.
    #[serde(default = "default_true")]
    pub shuffle_hammer_bros: bool,
    #[serde(default = "default_true")]
    pub disable_autoscroll: bool,
    /// Set starting lives for both Mario and Luigi (1–99).
    #[serde(default = "default_starting_lives")]
    pub starting_lives: u8,
    /// Up to 3 items to start with in inventory (item IDs, e.g. 0x03 = Leaf).
    #[serde(default)]
    pub starting_items: Vec<u8>,
    /// Randomize chest and reward items (Hammer Bros, Toad House, Princess letter, treasure chests).
    #[serde(default = "default_true")]
    pub chest_items: bool,
    /// Remove warp whistles and replace with random items.
    #[serde(default = "default_true")]
    pub remove_whistles: bool,
    /// Add extra hammer-breakable rocks: the W1 (6,5) decoration (between
    /// hammer-bro 14 and toad house 20) and the W8 (3,37) screen-2 decoration.
    /// Each becomes a horizontal path when broken/cleared. Off keeps the
    /// vanilla non-removable rocks.
    ///
    /// Tri-state: `Maybe` lets the seed decide (hidden from the flag key).
    #[serde(default)]
    pub more_hammer_rocks: Tri,
    /// `8s are Wild`: enable the W8 (Dark World) canoe on screen 0 and the
    /// extra paths on screen 2. Off keeps W8 without the canoe shortcut.
    /// (The screen-3 bridges are always present.)
    ///
    /// Tri-state: `Maybe` lets the seed decide (hidden from the flag key).
    #[serde(default)]
    pub eights_are_wild: Tri,
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
    /// Randomize per-fortress Boom-Boom stomp counts (each gets 1–5 hits).
    #[serde(default = "default_true")]
    pub boomboom_hits: bool,
    /// Make Koopalings vulnerable to thrown hammers (clears invulnerability flag).
    #[serde(default)]
    pub hammer_vulnerable_koopalings: bool,
    /// Randomize which Koopaling appears in each world (shuffle boss identity).
    #[serde(default)]
    pub random_koopalings: bool,
    /// Hammer item also breaks fortress lock tiles on the overworld map.
    ///
    /// Tri-state: `Maybe` lets the seed decide (hidden from the flag key).
    #[serde(default)]
    pub hammer_breaks_locks: Tri,
    /// Hammer item also breaks water gap (bridge) tiles on the overworld map.
    ///
    /// Tri-state: `Maybe` lets the seed decide (hidden from the flag key).
    #[serde(default)]
    pub hammer_breaks_bridges: Tri,
    /// Angry Sun begins swooping immediately on spawn instead of waiting
    /// for the vanilla pre-attack delay. (MaCobra52's "Early Sun" patch.)
    #[serde(default)]
    pub early_sun: bool,
    /// Restrict wandering Hammer Bros to overworld path tiles by converting
    /// the map-object landing-tile blacklist into a path-tile whitelist.
    /// ("SMB3 - Limit Bro Movement" patch.)
    #[serde(default)]
    pub limit_bro_movement: bool,
    /// Damage drops the player straight to Small Mario regardless of
    /// current power-up, instead of demoting tier-by-tier. (MaCobra52's
    /// "Japanese damage system (fixed)" patch.)
    #[serde(default)]
    pub japanese_damage: bool,
    /// Toad / Mushroom Houses stay on the map after entering and can be
    /// visited any number of times. (MaCobra52's "Infinite use Mushroom
    /// Houses" patch.)
    #[serde(default)]
    pub infinite_mushroom_houses: bool,
    /// Skip the entry-input-lock and shorten the exit transition when
    /// using a Toad / Mushroom House. Combines MaCobra52's "Move Sooner
    /// in Mushroom House (Instant)" and "Exit Mushroom House Faster"
    /// patches under a single flag.
    #[serde(default)]
    pub fast_mushroom_house: bool,
    /// Reduce tail-swipe slowdown so the Raccoon / Tanooki tail is
    /// quicker to use mid-run. Bundles two compensating tweaks so the
    /// faster tail doesn't break level design: raccoon flight time is
    /// trimmed slightly (cancels a known 8-1 cheese the faster tail
    /// enables) and the 7-6 fly-strat wall is lowered so the intended
    /// route still clears at the new flight duration. (MaCobra52's
    /// "Faster Tail Speed" patch.)
    #[serde(default)]
    pub faster_tail_speed: bool,
    /// Game Over no longer wipes reserve inventory, world map progress,
    /// or card state. (MaCobra52's "No Game Over Penalty" patch.)
    #[serde(default)]
    pub no_game_over_penalty: bool,
    /// Speed up Frog-Suit swimming and running. ("SMB3 - Faster Frog
    /// (tail attack while swimming compatible)" — layers on top of the
    /// always-on tail-attack-while-swimming routine.)
    #[serde(default)]
    pub faster_frog: bool,
    /// Random Fire Flower (issue #22): an in-level Fire Flower grants a power
    /// state derived deterministically from the world + the flower's level
    /// position, instead of always Fire. `Off`/`On`/`Wild` (see
    /// [`FireFlowerMode`]). The flower sprite is unchanged.
    #[serde(default)]
    pub fire_flower: FireFlowerMode,
    /// When true, the 19 vanilla spade-game tiles are picked up by the overworld
    /// builder and re-placed at random HammerBro slots, freeing their original
    /// positions for level placement. When false, spade games stay at vanilla
    /// positions (and the overworld builder leaves those tiles untouched).
    #[serde(default = "default_true")]
    pub shuffle_spade_games: bool,
    /// When true, the 22 vanilla Toad Houses are picked up by the overworld
    /// builder and re-placed at random HammerBro slots (cross-world, so W8
    /// can receive one). Each entry preserves its vanilla obj_ptr, so reward
    /// pool identity is unchanged. When false, Toad Houses stay at vanilla
    /// positions.
    #[serde(default = "default_true")]
    pub shuffle_toad_houses: bool,
    /// Replace ~10% of regular-level slots with visible hand-trap tiles (0xE6).
    /// On arrival the player is grabbed (100%, no 50/50 roll) and pulled into
    /// the underlying level. After completion, vanilla rewrites the tile to a
    /// checkmark so subsequent visits don't re-grab.
    #[serde(default = "default_true")]
    pub hands_levels: bool,
    /// Disguise exactly one regular-level slot per world W2-W8 as a pipe
    /// (tile 0xBC). The player walks freely past the pipe; pressing A on
    /// it loads the underlying level (no pipe-transit, no destination
    /// table — uniform world-map dispatch enters the slot's pointer entry
    /// like any level number tile).
    ///
    /// Tri-state: `Maybe` lets the seed decide (hidden from the flag key).
    #[serde(default = "default_tri_on")]
    pub troll_pipes: Tri,
    /// Include ~9 unreferenced beta levels in the overworld shuffle pool.
    #[serde(default)]
    pub include_beta_stages: bool,
    /// Antechamber shuffle: the four levels that open with a small entry
    /// room (5-3, 6-6, 7-5, 7-7) get their interiors randomly permuted, so
    /// one level's entry pipe can drop into another's interior. The player
    /// then finishes through that level's vanilla ending; map completion
    /// still credits the tile they entered from.
    ///
    /// Tri-state: `Maybe` lets the seed decide (hidden from the flag key).
    #[serde(default)]
    pub antechamber_shuffle: Tri,
    /// Piranha shuffle: release the two W7 piranha plant levels into the
    /// level pool. `On` = plant sprites follow the levels; `Wild` = plants
    /// scatter onto ~1 random level slot per world instead (see
    /// [`PiranhaMode`]).
    #[serde(default)]
    pub piranha_shuffle: PiranhaMode,
    /// Per-world (W1-W7) coin flip: when on, each world independently rolls
    /// to swap Mario's start tile with the airship/castle tile. Mario spawns
    /// at the vanilla airship coords; the level objective lives at the
    /// vanilla start coords. W8 (Bowser's castle) never swaps.
    #[serde(default)]
    pub swap_start_airship: bool,
    /// Cosmetic: every inventory item displays as the Anchor sprite while
    /// keeping its original behavior. Covers the world-map reserve grid,
    /// Toad House chests, in-level treasure boxes, and the Princess letter
    /// cutscene.
    #[serde(default)]
    pub anchor_visuals: bool,
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
    /// Cannon fire — Shuffle stays within sub-class (LEFT-firing, RIGHT-firing,
    /// or BILLS = regular/homing Bullet Bills). Wild merges all cfire IDs
    /// (incl. goomba pipes and bob-omb launchers) so any cfire can become any
    /// other; rocky wrench / 4-way / laser remain fixed.
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
    /// Skip the SMB3 (USA) iNES header / page-count / size checks so that
    /// modded or translated ROMs can be loaded. When true, the title-screen
    /// seed hash is also skipped because its hooks rely on vanilla offsets.
    /// Not encoded in the flag key — a property of the input ROM, not the
    /// randomization seed.
    #[serde(default)]
    pub skip_rom_validation: bool,
}

pub(super) fn default_true() -> bool {
    true
}

impl Default for Options {
    fn default() -> Self {
        Options {
            powerups: true,
            palettes: true,
            palette_themed: false,
            player_color: None,
            world_order: false,
            world_count: default_world_count(),
            big_q_blocks: false,
            shuffle_pipes: true,
            shuffle_airships: true,
            shuffle_hammer_bros: true,
            disable_autoscroll: true,
            chest_items: true,
            remove_whistles: true,
            more_hammer_rocks: Tri::Off,
            eights_are_wild: Tri::Off,
            card_speed_clear: true,
            remove_n_cards: true,
            skip_wand_cutscene: true,
            adjust_boss_hitboxes: true,
            koopaling_hits: true,
            boomboom_hits: true,
            hammer_vulnerable_koopalings: false,
            random_koopalings: false,
            include_beta_stages: false,
            antechamber_shuffle: Tri::Off,
            piranha_shuffle: PiranhaMode::Off,
            hammer_breaks_locks: Tri::Off,
            hammer_breaks_bridges: Tri::Off,
            early_sun: false,
            limit_bro_movement: false,
            japanese_damage: false,
            infinite_mushroom_houses: false,
            fast_mushroom_house: false,
            faster_tail_speed: false,
            no_game_over_penalty: false,
            faster_frog: false,
            fire_flower: FireFlowerMode::Off,
            shuffle_spade_games: true,
            shuffle_toad_houses: true,
            hands_levels: true,
            troll_pipes: Tri::On,
            swap_start_airship: false,
            anchor_visuals: false,
            ground: EnemyMode::Shuffle,
            shell: EnemyMode::Shuffle,
            flying: EnemyMode::Shuffle,
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
            skip_rom_validation: false,
        }
    }
}
