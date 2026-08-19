//! Flag-key encode/decode: the shareable base32 string that round-trips an
//! `Options` set (Crockford alphabet, versioned).
//!
//! # Format (v29)
//!
//! ```text
//! byte 0   version
//! byte 1   checksum over byte 0 and the payload as transmitted
//! byte 2+  payload — a bitfield, trailing zero bytes stripped
//! ```
//!
//! Three properties fall out of that shape, and each one is load-bearing:
//!
//! - **Bit collisions are unrepresentable.** The payload is a
//!   [`modular_bitfield`] struct, so bit positions are assigned by declaration
//!   order rather than written out by hand. Two options cannot land on the same
//!   bit, which is a class of bug this format used to catch one incident at a
//!   time.
//! - **Adding an option does not break existing keys.** The payload has spare
//!   capacity ([`FLAG_PAYLOAD_BYTES`]); a new option spends reserve bits, an
//!   older key decodes them as zero, and zero is "off" for every bool and the
//!   default for every enum here. So an addition needs **no version bump** —
//!   only *repurposing* an allocated bit does.
//! - **Mistyped keys are rejected instead of silently decoding to a different
//!   ruleset.** Before the checksum, ~90% of single-character typos produced a
//!   valid key for different settings with nothing on screen to say so.
//!
//! The version and checksum sit in a fixed two-byte envelope at the front on
//! purpose: both stay readable without knowing the payload layout, so a future
//! format can still be classified as "older"/"newer" rather than "garbage".

use super::*;
use modular_bitfield::prelude::*;

pub(super) const FLAG_KEY_VERSION: u8 = 29;

pub(super) const FLAG_KEY_PREFIX: &str = "SMB3R-";

/// Salt mixed into the seed to derive the substream that resolves `Maybe`
/// flags. Keeping it on a separate stream means turning a flag to `Maybe`
/// never perturbs the main randomization RNG, so a seed with no `Maybe`
/// flags produces byte-identical output to before this feature existed.
pub(super) const MAYBE_SALT: u64 = 0x4D41_5942_455F_5631; // "MAYBE_V1"

/// Bytes of payload the format can address, past the two-byte envelope.
///
/// 93 bits are spent today, leaving 147 in reserve — years of headroom at the
/// rate this project has actually added options (the layout was bumped six
/// times in the 38 days to 2026-08-06). It is deliberately generous rather than
/// "one more byte than we need": running out of reserve is the one thing that
/// forces a breaking version bump, and unused capacity is free. Trailing zero
/// bytes never leave the encoder, so raising this number does not lengthen a
/// single key.
///
/// Must match the `bytes` argument on `FlagBits` — the compiler enforces it,
/// since `from_bytes` takes the generated array type.
const FLAG_PAYLOAD_BYTES: usize = 30;

/// Version byte + checksum byte.
const ENVELOPE_BYTES: usize = 2;

/// Every pre-v29 key was exactly this many bytes: a version byte and 12 bytes
/// of hand-packed flags, with no checksum anywhere.
const LEGACY_KEY_BYTES: usize = 13;

/// Last format version that used the pre-v29 fixed-width layout. Nothing past
/// this will ever be added, which is what keeps [`legacy_version_of`] exact.
const LAST_LEGACY_VERSION: u8 = 28;

/// Crockford Base-32 alphabet (excludes I, L, O, U to avoid ambiguity).
pub(super) const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Encode a byte slice into a Crockford Base-32 string.
/// Pads the final group with zero bits as needed.
pub(super) fn base32_encode(data: &[u8]) -> String {
    let bit_len = data.len() * 8;
    let out_len = bit_len.div_ceil(5);
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
///
/// Length is not fixed — a key carries only as many payload bytes as it needs
/// — but it is not free either: N bytes always encode to exactly `ceil(N*8/5)`
/// characters, and anything else is rejected. That catches a character appended
/// to or lost from the tail, which the checksum on its own can miss, since those
/// spare bits are padding that never reaches a byte.
pub(super) fn base32_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut buf: u64 = 0;
    let mut bits: u32 = 0;
    let mut result = Vec::new();
    let mut chars = 0usize;
    for ch in s.chars() {
        chars += 1;
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
    if chars != (result.len() * 8).div_ceil(5) {
        return Err(format!("Flag key is the wrong length ({chars} characters)"));
    }
    Ok(result)
}

/// CRC-8 (polynomial `0x07`, init 0) over the version byte and the payload as
/// transmitted.
///
/// **Position-weighted on purpose.** A plain sum — or worse, an XOR — is blind
/// to transposed bytes, so swapping two characters of a pasted key would sail
/// through. A CRC catches every single-byte error, every transposition, and
/// burst errors, for one byte of key.
///
/// This function is part of the wire format and must not change: a build reads
/// the envelope of a key from *any* version before it knows whether it can
/// decode the payload, which is what lets a mistyped key ("invalid") be told
/// apart from a key built by a different release ("older"/"newer").
pub(super) fn checksum(version: u8, payload: &[u8]) -> u8 {
    let mut crc: u8 = 0;
    for &byte in std::iter::once(&version).chain(payload) {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
        }
    }
    crc
}

/// Trim surrounding whitespace and strip the "SMB3R-" prefix
/// case-insensitively if present. Compares as bytes so a non-ASCII key can't
/// panic on a char-boundary slice.
///
/// Shared by `from_flag_key` and `flag_key_version_of` so the two always see
/// the same string — otherwise a key one accepts and the other rejects would be
/// classified with the wrong reason.
fn strip_flag_key_prefix(key: &str) -> &str {
    let key = key.trim();
    let prefix_len = FLAG_KEY_PREFIX.len();
    if key.len() >= prefix_len
        && key.as_bytes()[..prefix_len].eq_ignore_ascii_case(FLAG_KEY_PREFIX.as_bytes())
    {
        &key[prefix_len..]
    } else {
        key
    }
}

/// Decode a prefix-stripped key into its raw bytes, rejecting anything that
/// fails the envelope check. On success `bytes[0]` is the version and
/// `bytes[ENVELOPE_BYTES..]` is the payload as transmitted.
fn decode_envelope(stripped: &str) -> Result<Vec<u8>, String> {
    let bytes = base32_decode(stripped)?;
    if bytes.len() < ENVELOPE_BYTES {
        return Err("Flag key is too short".to_string());
    }
    if bytes[1] != checksum(bytes[0], &bytes[ENVELOPE_BYTES..]) {
        return Err("Flag key failed its checksum — it looks mistyped or truncated".to_string());
    }
    Ok(bytes)
}

/// The flag-key format version this build reads.
pub fn current_flag_key_version() -> u8 {
    FLAG_KEY_VERSION
}

/// The format version a key claims, read without interpreting the rest of it.
///
/// Lets a caller tell "this key is from an older/newer build" apart from "this
/// isn't a flag key at all" — two failures that need different things from the
/// user.
///
/// The checksum is what makes that split honest: a key only gets as far as
/// reporting a version if its bytes are intact, so a typo is reported as a bad
/// key rather than sending the user hunting for a build that never existed.
pub fn flag_key_version_of(key: &str) -> Result<u8, String> {
    let stripped = strip_flag_key_prefix(key);
    match decode_envelope(stripped) {
        Ok(bytes) => Ok(bytes[0]),
        Err(e) => legacy_version_of(stripped).ok_or(e),
    }
}

/// Read the version off a pre-v29 key.
///
/// Those keys carry no checksum, so [`decode_envelope`] rejects every one of
/// them as damaged. Without this fallback, everyone still holding a v26 or v28
/// key — which is everyone, the day v29 ships — would be told their key is
/// mistyped and sent hunting for a typo that isn't there, instead of "this is
/// from an older version".
///
/// Exact rather than heuristic, and permanently bounded: the old layout was
/// always 13 bytes with the version in byte 0, and no version past 28 ever used
/// it. A v29+ key can also be 13 bytes, but its version byte is 29 or higher, so
/// the two can't be confused.
fn legacy_version_of(stripped: &str) -> Option<u8> {
    let bytes = base32_decode(stripped).ok()?;
    // `first()` rather than `bytes[0]`: an empty key decodes to no bytes, and
    // `then_some` would evaluate the index either way.
    let version = *bytes.first()?;
    let is_legacy =
        bytes.len() == LEGACY_KEY_BYTES && (1..=LAST_LEGACY_VERSION).contains(&version);
    is_legacy.then_some(version)
}

/// The message for a key whose format this build doesn't read.
fn unsupported_version(version: u8) -> String {
    format!("Unsupported flag key version {version} (expected {FLAG_KEY_VERSION})")
}

/// `Options` fields deliberately absent from the flag key, with the reason.
///
/// This is the *only* hand-maintained part of the mapping: everything else is
/// forced by the destructure in [`Options::to_flag_bits`], which names every
/// field and so fails to compile when `Options` grows one. Adding a name here
/// is a conscious decision, not an oversight.
pub(super) const NOT_ENCODED: &[&str] = &[
    "palettes",            // cosmetic; uses OS randomness, not seed-derived
    "palette_themed",      // cosmetic
    "player_color",        // cosmetic
    "remove_flashing",     // cosmetic/accessibility; static patch, no RNG
    "skip_rom_validation", // operational (CLI/WASM input handling), not randomization
];

/// The `Options` field names this build encodes into the flag key.
///
/// Derived — the serde field set minus [`NOT_ENCODED`] — so the web app's
/// `inFlagKey` markings can be checked against what Rust actually persists
/// instead of being documentation nobody verifies.
pub fn flag_key_fields() -> Vec<String> {
    let value = serde_json::to_value(Options::default())
        .expect("Options is always serializable");
    value
        .as_object()
        .expect("Options serializes to an object")
        .keys()
        .filter(|name| !NOT_ENCODED.contains(&name.as_str()))
        .cloned()
        .collect()
}

/// Wraps the generated bitfield so the whole macro expansion can be exempted
/// from `dead_code` in one place.
///
/// Reason: the macro emits a getter, a checked getter, a setter, a checked
/// setter and a builder method per field, in an `impl` block of its own. Decode
/// uses the checked getters and encode uses the builders; the rest are
/// unreachable by construction. The attribute has to sit on a module because it
/// doesn't survive onto the generated `impl` from the struct.
mod payload {
    #![allow(dead_code)]
    use super::*;

    /// The flag-key payload.
    ///
    /// **Declaration order is the wire format.** Fields are packed from bit 0
    /// of byte 0 upward in the order written here, so:
    ///
    /// - New options are **appended** immediately before the reserve. Never
    ///   reorder, never resize an existing field, never insert in the middle —
    ///   any of those silently repurposes bits that keys in circulation are
    ///   using, which is the one change that forces a version bump.
    /// - Removing an option leaves its bits behind as a named `_dead_*` hole
    ///   rather than closing the gap, for the same reason.
    #[bitfield(bytes = 30)]
    #[derive(Debug, Clone, Copy)]
    pub(super) struct FlagBits {
        // --- Bools (32) ---
        pub(super) powerups: bool,
        pub(super) world_order: bool,
        pub(super) big_q_blocks: bool,
        pub(super) chest_items: bool,
        pub(super) remove_whistles: bool,
        pub(super) shuffle_airships: bool,
        pub(super) shuffle_hammer_bros: bool,
        pub(super) shuffle_spade_games: bool,
        pub(super) shuffle_toad_houses: bool,
        pub(super) hands_levels: bool,
        pub(super) disable_autoscroll: bool,
        pub(super) include_beta_stages: bool,
        pub(super) swap_start_airship: bool,
        pub(super) limit_bro_movement: bool,
        pub(super) remove_n_cards: bool,
        pub(super) card_speed_clear: bool,
        pub(super) skip_wand_cutscene: bool,
        pub(super) adjust_boss_hitboxes: bool,
        pub(super) koopaling_hits: bool,
        pub(super) boomboom_hits: bool,
        pub(super) hammer_vulnerable_koopalings: bool,
        pub(super) random_koopalings: bool,
        pub(super) early_sun: bool,
        pub(super) japanese_damage: bool,
        pub(super) infinite_mushroom_houses: bool,
        pub(super) fast_mushroom_house: bool,
        pub(super) faster_tail_speed: bool,
        pub(super) faster_frog: bool,
        pub(super) no_game_over_penalty: bool,
        pub(super) poison_mushrooms: bool,
        pub(super) modern_powerups: bool,
        pub(super) anchor_visuals: bool,

        // --- Player-hidden tri-state flags (2 bits each) ---
        // One field each now. The old layout split these into a value bit and a
        // "maybe" bit in a different byte, purely because no byte had two free
        // bits left.
        pub(super) hammer_breaks_locks: Tri,
        pub(super) hammer_breaks_bridges: Tri,
        pub(super) more_hammer_rocks: Tri,
        pub(super) eights_are_wild: Tri,
        pub(super) troll_pipes: Tri,
        pub(super) antechamber_shuffle: Tri,

        // --- Per-class enemy modes (2 bits each) ---
        pub(super) ground: EnemyMode,
        pub(super) shell: EnemyMode,
        pub(super) flying: EnemyMode,
        pub(super) piranhas: EnemyMode,
        pub(super) ghosts: EnemyMode,
        pub(super) thwomps: EnemyMode,
        pub(super) rotodiscs: EnemyMode,
        pub(super) cannons: EnemyMode,
        pub(super) water: EnemyMode,
        pub(super) bros: EnemyMode,
        pub(super) hb_encounters: EnemyMode,

        // --- Mode enums (2 bits each) ---
        pub(super) fire_flower: FireFlowerMode,
        pub(super) piranha_shuffle: PiranhaMode,

        // --- Wild-injection chaser set: one bit each, in WildChaser::ALL order ---
        pub(super) wild_sun: bool,
        pub(super) wild_lakitu: bool,
        pub(super) wild_bass: bool,

        // --- Numbers ---
        /// Index into [`STARTING_LIVES_VALUES`].
        pub(super) starting_lives: B2,
        /// 1–7; 0 is a dead pattern that decodes to the default of 7.
        pub(super) world_count: B3,
        /// Item IDs directly (0 = empty slot), including the `ITEM_RANDOM*`
        /// sentinels — 5 bits covers all of them, where the old layout needed a
        /// nibble plus a separate 2-bit mode.
        pub(super) starting_item_0: B5,
        pub(super) starting_item_1: B5,
        pub(super) starting_item_2: B5,

        // --- Appended after v29 shipped; older keys read these as zero/off ---
        pub(super) lakitu_stays_dead: bool,
        pub(super) koopaling_grab_height: bool,

        // --- Reserve ---
        // 146 bits. Adding an option is: declare it immediately above this
        // block, then take the same number of bits off `B19`. An older key
        // simply has those bits zero, which is "off" for a bool and the default
        // for every enum here, so it stays a correct key for the settings it
        // named — no version bump, and keys already in circulation keep working.
        //
        // Forgetting to shrink the reserve is a compile error, but a cryptic
        // one: the crate reports `OneMod8: TotalSizeIsMultipleOfEightBits is not
        // satisfied` against the struct. It means the widths no longer add up to
        // `bytes * 8`, nothing more.
        //
        // Two fields because the crate's widths stop at B128.
        #[skip] __: B128,
        #[skip] __: B17,
    }
}

use payload::FlagBits;

/// Reduce a starting-item slot to a value the rest of the app knows: item IDs
/// `1..=13` and the three `ITEM_RANDOM*` sentinels, or 0 for an empty slot.
///
/// Applied on both sides. On encode it keeps a hand-written JSON or CLI input
/// inside the 5-bit field; on decode it drops the 17–31 patterns, which are
/// reachable from a corrupt or newer key but mean nothing to the inventory
/// writer.
fn sanitize_item(raw: u8) -> u8 {
    if raw <= ITEM_RANDOM_SUIT_ONLY { raw } else { 0 }
}

impl Options {
    /// Pack options into the flag-key payload.
    #[rustfmt::skip]
    fn to_flag_bits(&self) -> FlagBits {
        // Destructured rather than field-accessed on purpose: adding a field to
        // `Options` is a compile error here until it is either encoded below or
        // bound as `field: _` and listed in `NOT_ENCODED`. That is what makes
        // "did we forget to encode this one?" a build failure instead of a test
        // that only ever covered the types someone remembered to list.
        let Options {
            powerups, world_order, big_q_blocks, chest_items, remove_whistles,
            shuffle_airships, shuffle_hammer_bros, shuffle_spade_games,
            shuffle_toad_houses, hands_levels, disable_autoscroll,
            include_beta_stages, swap_start_airship, limit_bro_movement,
            remove_n_cards, card_speed_clear, skip_wand_cutscene,
            adjust_boss_hitboxes, koopaling_hits, boomboom_hits,
            hammer_vulnerable_koopalings, random_koopalings, early_sun,
            japanese_damage, infinite_mushroom_houses, fast_mushroom_house,
            faster_tail_speed, faster_frog, lakitu_stays_dead, koopaling_grab_height,
            no_game_over_penalty,
            poison_mushrooms, modern_powerups, anchor_visuals,
            hammer_breaks_locks, hammer_breaks_bridges, more_hammer_rocks,
            eights_are_wild, troll_pipes, antechamber_shuffle,
            ground, shell, flying, piranhas, ghosts, thwomps, rotodiscs,
            cannons, water, bros, hb_encounters,
            fire_flower, piranha_shuffle, wild_injections,
            starting_lives, world_count, starting_items,
            // Not encoded — see NOT_ENCODED for the reason on each.
            palettes: _, palette_themed: _, player_color: _,
            remove_flashing: _, skip_rom_validation: _,
        } = self;

        let item = |i: usize| starting_items.get(i).copied().unwrap_or(0);
        let has = |c: WildChaser| wild_injections.contains(&c);

        FlagBits::new()
            .with_powerups(*powerups)
            .with_world_order(*world_order)
            .with_big_q_blocks(*big_q_blocks)
            .with_chest_items(*chest_items)
            .with_remove_whistles(*remove_whistles)
            .with_shuffle_airships(*shuffle_airships)
            .with_shuffle_hammer_bros(*shuffle_hammer_bros)
            .with_shuffle_spade_games(*shuffle_spade_games)
            .with_shuffle_toad_houses(*shuffle_toad_houses)
            .with_hands_levels(*hands_levels)
            .with_disable_autoscroll(*disable_autoscroll)
            .with_include_beta_stages(*include_beta_stages)
            .with_swap_start_airship(*swap_start_airship)
            .with_limit_bro_movement(*limit_bro_movement)
            .with_remove_n_cards(*remove_n_cards)
            .with_card_speed_clear(*card_speed_clear)
            .with_skip_wand_cutscene(*skip_wand_cutscene)
            .with_adjust_boss_hitboxes(*adjust_boss_hitboxes)
            .with_koopaling_hits(*koopaling_hits)
            .with_boomboom_hits(*boomboom_hits)
            .with_hammer_vulnerable_koopalings(*hammer_vulnerable_koopalings)
            .with_random_koopalings(*random_koopalings)
            .with_early_sun(*early_sun)
            .with_japanese_damage(*japanese_damage)
            .with_infinite_mushroom_houses(*infinite_mushroom_houses)
            .with_fast_mushroom_house(*fast_mushroom_house)
            .with_faster_tail_speed(*faster_tail_speed)
            .with_faster_frog(*faster_frog)
            .with_lakitu_stays_dead(*lakitu_stays_dead)
            .with_koopaling_grab_height(*koopaling_grab_height)
            .with_no_game_over_penalty(*no_game_over_penalty)
            .with_poison_mushrooms(*poison_mushrooms)
            .with_modern_powerups(*modern_powerups)
            .with_anchor_visuals(*anchor_visuals)
            .with_hammer_breaks_locks(*hammer_breaks_locks)
            .with_hammer_breaks_bridges(*hammer_breaks_bridges)
            .with_more_hammer_rocks(*more_hammer_rocks)
            .with_eights_are_wild(*eights_are_wild)
            .with_troll_pipes(*troll_pipes)
            .with_antechamber_shuffle(*antechamber_shuffle)
            .with_ground(*ground)
            .with_shell(*shell)
            .with_flying(*flying)
            .with_piranhas(*piranhas)
            .with_ghosts(*ghosts)
            .with_thwomps(*thwomps)
            .with_rotodiscs(*rotodiscs)
            .with_cannons(*cannons)
            .with_water(*water)
            .with_bros(*bros)
            .with_hb_encounters(*hb_encounters)
            .with_fire_flower(*fire_flower)
            .with_piranha_shuffle(*piranha_shuffle)
            .with_wild_sun(has(WildChaser::Sun))
            .with_wild_lakitu(has(WildChaser::Lakitu))
            .with_wild_bass(has(WildChaser::Bass))
            .with_starting_lives(lives_to_idx(*starting_lives))
            .with_world_count((*world_count).clamp(1, 7))
            .with_starting_item_0(sanitize_item(item(0)))
            .with_starting_item_1(sanitize_item(item(1)))
            .with_starting_item_2(sanitize_item(item(2)))
    }

    /// Unpack the flag-key payload back into options.
    ///
    /// Every enum is read through the crate's *checked* accessor. Three-variant
    /// enums in two bits leave a dead fourth pattern, and a key with reserve
    /// bits set by a newer build can land on it — a panicking accessor there
    /// would take the whole web app down, where falling back to the default is
    /// exactly the "an unknown value is off" rule the reserve already relies on.
    fn from_flag_bits(f: FlagBits) -> Options {
        let chasers: Vec<WildChaser> = [
            (WildChaser::Sun, f.wild_sun()),
            (WildChaser::Lakitu, f.wild_lakitu()),
            (WildChaser::Bass, f.wild_bass()),
        ]
        .into_iter()
        .filter(|&(_, set)| set)
        .map(|(c, _)| c)
        .collect();

        let items: Vec<u8> = [f.starting_item_0(), f.starting_item_1(), f.starting_item_2()]
            .into_iter()
            .map(sanitize_item)
            .filter(|&i| i != 0)
            .collect();

        Options {
            powerups: f.powerups(),
            world_order: f.world_order(),
            big_q_blocks: f.big_q_blocks(),
            chest_items: f.chest_items(),
            remove_whistles: f.remove_whistles(),
            shuffle_airships: f.shuffle_airships(),
            shuffle_hammer_bros: f.shuffle_hammer_bros(),
            shuffle_spade_games: f.shuffle_spade_games(),
            shuffle_toad_houses: f.shuffle_toad_houses(),
            hands_levels: f.hands_levels(),
            disable_autoscroll: f.disable_autoscroll(),
            include_beta_stages: f.include_beta_stages(),
            swap_start_airship: f.swap_start_airship(),
            limit_bro_movement: f.limit_bro_movement(),
            remove_n_cards: f.remove_n_cards(),
            card_speed_clear: f.card_speed_clear(),
            skip_wand_cutscene: f.skip_wand_cutscene(),
            adjust_boss_hitboxes: f.adjust_boss_hitboxes(),
            koopaling_hits: f.koopaling_hits(),
            boomboom_hits: f.boomboom_hits(),
            hammer_vulnerable_koopalings: f.hammer_vulnerable_koopalings(),
            random_koopalings: f.random_koopalings(),
            early_sun: f.early_sun(),
            japanese_damage: f.japanese_damage(),
            infinite_mushroom_houses: f.infinite_mushroom_houses(),
            fast_mushroom_house: f.fast_mushroom_house(),
            faster_tail_speed: f.faster_tail_speed(),
            faster_frog: f.faster_frog(),
            lakitu_stays_dead: f.lakitu_stays_dead(),
            koopaling_grab_height: f.koopaling_grab_height(),
            no_game_over_penalty: f.no_game_over_penalty(),
            poison_mushrooms: f.poison_mushrooms(),
            modern_powerups: f.modern_powerups(),
            anchor_visuals: f.anchor_visuals(),
            hammer_breaks_locks: f.hammer_breaks_locks_or_err().unwrap_or_default(),
            hammer_breaks_bridges: f.hammer_breaks_bridges_or_err().unwrap_or_default(),
            more_hammer_rocks: f.more_hammer_rocks_or_err().unwrap_or_default(),
            eights_are_wild: f.eights_are_wild_or_err().unwrap_or_default(),
            troll_pipes: f.troll_pipes_or_err().unwrap_or_default(),
            antechamber_shuffle: f.antechamber_shuffle_or_err().unwrap_or_default(),
            ground: f.ground_or_err().unwrap_or_default(),
            shell: f.shell_or_err().unwrap_or_default(),
            flying: f.flying_or_err().unwrap_or_default(),
            piranhas: f.piranhas_or_err().unwrap_or_default(),
            ghosts: f.ghosts_or_err().unwrap_or_default(),
            thwomps: f.thwomps_or_err().unwrap_or_default(),
            rotodiscs: f.rotodiscs_or_err().unwrap_or_default(),
            cannons: f.cannons_or_err().unwrap_or_default(),
            water: f.water_or_err().unwrap_or_default(),
            bros: f.bros_or_err().unwrap_or_default(),
            hb_encounters: f.hb_encounters_or_err().unwrap_or_default(),
            fire_flower: f.fire_flower_or_err().unwrap_or_default(),
            piranha_shuffle: f.piranha_shuffle_or_err().unwrap_or_default(),
            wild_injections: chasers,
            starting_lives: idx_to_lives(f.starting_lives()),
            // 0 is unreachable from the encoder (it clamps to 1–7) but reachable
            // from a corrupt or newer key; take the default rather than a world
            // count the builder can't satisfy.
            world_count: match f.world_count() { 0 => default_world_count(), n => n },
            starting_items: items,
            // Not encoded: fixed to the values a shared key should never
            // override. The web app skips these fields when applying a decoded
            // key, so a racer's cosmetic and ROM choices survive.
            palettes: true,
            palette_themed: false,
            player_color: None,
            remove_flashing: true,
            skip_rom_validation: false,
        }
    }

    /// Encode options into the raw key bytes: version, checksum, payload.
    pub fn to_flag_bytes(&self) -> Vec<u8> {
        let payload = self.to_flag_bits().into_bytes();
        // Trailing zero bytes carry nothing, so they are not transmitted. That
        // is what decouples the format's capacity from the key's length: the
        // reserve can be generous without anyone ever typing it.
        let used = payload.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
        let mut bytes = Vec::with_capacity(ENVELOPE_BYTES + used);
        bytes.push(FLAG_KEY_VERSION);
        bytes.push(checksum(FLAG_KEY_VERSION, &payload[..used]));
        bytes.extend_from_slice(&payload[..used]);
        bytes
    }

    /// Encode options into a compact Crockford Base-32 flag key (e.g. "SMB3R-1S0G...").
    pub fn to_flag_key(&self) -> String {
        let mut key = String::with_capacity(FLAG_KEY_PREFIX.len() + 24);
        key.push_str(FLAG_KEY_PREFIX);
        key.push_str(&base32_encode(&self.to_flag_bytes()));
        key
    }

    /// Decode a Crockford Base-32 flag key string into Options.
    pub fn from_flag_key(key: &str) -> Result<Options, String> {
        let stripped = strip_flag_key_prefix(key);
        let bytes = match decode_envelope(stripped) {
            Ok(bytes) => bytes,
            // A pre-v29 key fails the envelope check for want of a checksum, not
            // because it is damaged. Say which it is — the CLI shows this message
            // verbatim, and "mistyped" would be a lie.
            Err(checksum_err) => {
                return Err(legacy_version_of(stripped).map_or(checksum_err, unsupported_version));
            }
        };

        let version = bytes[0];
        if version != FLAG_KEY_VERSION {
            return Err(unsupported_version(version));
        }

        let body = &bytes[ENVELOPE_BYTES..];
        if body.len() > FLAG_PAYLOAD_BYTES {
            return Err(format!(
                "Flag key payload is too long ({} bytes, capacity {FLAG_PAYLOAD_BYTES})",
                body.len(),
            ));
        }
        // Zero-fill back to capacity: a key made before some later option
        // existed simply doesn't carry the bytes that option lives in.
        let mut payload = [0u8; FLAG_PAYLOAD_BYTES];
        payload[..body.len()].copy_from_slice(body);

        Ok(Options::from_flag_bits(FlagBits::from_bytes(payload)))
    }

    /// Returns true if any enemy class is enabled (not Off).
    pub fn any_enemies_active(&self) -> bool {
        self.ground != EnemyMode::Off || self.shell != EnemyMode::Off
            || self.flying != EnemyMode::Off
            || self.piranhas != EnemyMode::Off
            || self.ghosts != EnemyMode::Off || self.thwomps != EnemyMode::Off
            || self.rotodiscs != EnemyMode::Off || self.cannons != EnemyMode::Off
            || self.water != EnemyMode::Off || self.bros != EnemyMode::Off
            || self.hb_encounters != EnemyMode::Off || !self.wild_injections.is_empty()
    }
}
