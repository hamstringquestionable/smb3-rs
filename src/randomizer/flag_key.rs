//! Flag-key encode/decode: the shareable base32 string that round-trips an
//! `Options` set (Crockford alphabet, versioned).

use super::*;

pub(super) const FLAG_KEY_VERSION: u8 = 22;

pub(super) const FLAG_KEY_PREFIX: &str = "SMB3R-";

/// Salt mixed into the seed to derive the substream that resolves `Maybe`
/// flags. Keeping it on a separate stream means turning a flag to `Maybe`
/// never perturbs the main randomization RNG, so a seed with no `Maybe`
/// flags produces byte-identical output to before this feature existed.
pub(super) const MAYBE_SALT: u64 = 0x4D41_5942_455F_5631; // "MAYBE_V1"

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
pub(super) fn base32_decode(s: &str, expected_bytes: usize) -> Result<Vec<u8>, String> {
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

impl Options {
    /// Encode options into raw bytes.
    pub fn to_flag_bytes(&self) -> [u8; 12] {
        let b0 = FLAG_KEY_VERSION;

        // b1: non-enemy flags. hammer_breaks_locks is tri-state: its value bit
        // stores On vs (Off/Maybe); the Maybe bit lives in b11.
        // b1 bit 1 = shuffle_hammer_bros (reuses the slot formerly airship_lock,
        // now unconditionally on).
        let b1 = (self.powerups as u8) << 7
            | (self.hammer_breaks_locks.is_on() as u8) << 6
            | (self.koopaling_hits as u8) << 5
            | (self.world_order as u8) << 4
            | (self.big_q_blocks as u8) << 3
            | (self.disable_autoscroll as u8) << 2
            | (self.shuffle_hammer_bros as u8) << 1
            | (self.chest_items as u8);

        // b2 bit 4 = faster_frog (reuses the slot formerly fix_drawbridges,
        // now always-on).
        // b2 bit 3 = boomboom_hits (reuses the slot formerly remove_rocks).
        let b2 = (self.remove_whistles as u8) << 7
            | (self.hands_levels as u8) << 6
            | (self.shuffle_pipes as u8) << 5
            | (self.faster_frog as u8) << 4
            | (self.boomboom_hits as u8) << 3
            | (self.troll_pipes.is_on() as u8) << 2
            | (self.shuffle_toad_houses as u8) << 1
            | (self.shuffle_airships as u8);

        // b3: hammer_breaks_bridges(7) starting_lives(6-5) fast_mushroom_house(4)
        //     faster_tail_speed(3) no_game_over_penalty(2) swap_start_airship(1)
        //     more_hammer_rocks(0)
        // starting_lives shrank from a 7-bit clamped 1–99 to a 2-bit index
        // into {1, 5, 20, 99}, freeing bits 4-0 for future toggles.
        let b3 = ((self.hammer_breaks_bridges.is_on() as u8) << 7)
            | (lives_to_idx(self.starting_lives) << 5)
            | ((self.fast_mushroom_house as u8) << 4)
            | ((self.faster_tail_speed as u8) << 3)
            | ((self.no_game_over_penalty as u8) << 2)
            | ((self.swap_start_airship as u8) << 1)
            | (self.more_hammer_rocks.is_on() as u8);

        let b4 = (self.card_speed_clear as u8) << 7
            | (self.remove_n_cards as u8) << 6
            | (self.skip_wand_cutscene as u8) << 5
            | (self.adjust_boss_hitboxes as u8) << 4
            | (self.shuffle_spade_games as u8) << 3;
            // bits 2-0 used by hb_encounters and wild_injections below

        // Helper to encode EnemyMode as 2 bits
        fn em(m: EnemyMode) -> u8 {
            match m {
                EnemyMode::Off => 0,
                EnemyMode::Shuffle => 1,
                EnemyMode::Wild => 2,
            }
        }

        // b5: ground(7-6) shell(5-4) flying(3-2) hammer_vulnerable_koopalings(1) early_sun(0)
        let b5 = em(self.ground) << 6
            | em(self.shell) << 4
            | em(self.flying) << 2
            | (self.hammer_vulnerable_koopalings as u8) << 1
            | (self.early_sun as u8);

        // b6: japanese_damage(7) infinite_mushroom_houses(6) piranhas(5-4)
        //     ghosts(3-2) thwomps(1-0)
        // Bits 7-6 were the two `bullet_bills` bits before v17; now reused
        // for the two MaCobra52 player/map mechanic toggles.
        let b6 = (self.japanese_damage as u8) << 7
            | (self.infinite_mushroom_houses as u8) << 6
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
        // b9: i2 nibble (7-4) | limit_bro_movement (3) | world_count 1..7 (2-0)
        let b9 = (item_nibble(i2) << 4)
            | ((self.limit_bro_movement as u8) << 3)
            | (self.world_count.clamp(1, 7) & 0x07);

        // b10: extra flags + per-slot random mode (2 bits each)
        let b10 = (self.random_koopalings as u8) << 7
            | (self.include_beta_stages as u8) << 6
            | (item_mode(i0) << 4)
            | (item_mode(i1) << 2)
            | item_mode(i2);

        // Encode FireFlowerMode as 2 bits (off=0, on=1, wild=2).
        fn ffm(m: FireFlowerMode) -> u8 {
            match m {
                FireFlowerMode::Off => 0,
                FireFlowerMode::On => 1,
                FireFlowerMode::Wild => 2,
            }
        }

        // b11: "maybe" bits for the four player-hidden tri-state flags (bits
        // 0-3). When a maybe bit is set the flag is resolved from the seed at
        // generation time, and its value bit (in b1/b2/b3) is ignored on decode.
        // Bits 4-5 hold the Random Fire Flower mode. Bit 6 is the eights_are_wild
        // ON bit and bit 7 its Maybe bit (both live here since b1-b4 are full).
        let b11 = (self.hammer_breaks_locks.is_maybe() as u8)
            | (self.hammer_breaks_bridges.is_maybe() as u8) << 1
            | (self.troll_pipes.is_maybe() as u8) << 2
            | (self.more_hammer_rocks.is_maybe() as u8) << 3
            | ffm(self.fire_flower) << 4
            | (self.eights_are_wild.is_on() as u8) << 6
            | (self.eights_are_wild.is_maybe() as u8) << 7;

        [b0, b1, b2, b3, b4, b5, b6, b7, b8, b9, b10, b11]
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

        let bytes = base32_decode(encoded, 12)?;

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
        let b11 = bytes[11];

        let starting_lives = idx_to_lives((b3 >> 5) & 0x3);

        fn dem(bits: u8) -> EnemyMode {
            match bits & 0x03 {
                1 => EnemyMode::Shuffle,
                2 => EnemyMode::Wild,
                _ => EnemyMode::Off,
            }
        }

        // Decode a tri-state flag from its value bit (in b1/b2/b3) and its
        // maybe bit (in b11). Maybe wins; otherwise the value bit picks On/Off.
        fn dtri(value: bool, maybe: bool) -> Tri {
            if maybe { Tri::Maybe } else if value { Tri::On } else { Tri::Off }
        }

        // Decode the 2-bit Random Fire Flower mode (b11 bits 4-5).
        fn dffm(bits: u8) -> FireFlowerMode {
            match bits & 0x03 {
                1 => FireFlowerMode::On,
                2 => FireFlowerMode::Wild,
                _ => FireFlowerMode::Off,
            }
        }

        Ok(Options {
            powerups: (b1 >> 7) & 1 != 0,
            palettes: true,
            palette_themed: false, // cosmetic — not encoded in flag key
            hammer_breaks_locks: dtri((b1 >> 6) & 1 != 0, b11 & 1 != 0),
            koopaling_hits: (b1 >> 5) & 1 != 0,
            boomboom_hits: (b2 >> 3) & 1 != 0,
            world_order: (b1 >> 4) & 1 != 0,
            big_q_blocks: (b1 >> 3) & 1 != 0,
            disable_autoscroll: (b1 >> 2) & 1 != 0,
            shuffle_hammer_bros: (b1 >> 1) & 1 != 0,
            chest_items: b1 & 1 != 0,
            remove_whistles: (b2 >> 7) & 1 != 0,
            hands_levels: (b2 >> 6) & 1 != 0,
            shuffle_pipes: (b2 >> 5) & 1 != 0,
            faster_frog: (b2 >> 4) & 1 != 0,
            shuffle_airships: b2 & 1 != 0,
            shuffle_toad_houses: (b2 >> 1) & 1 != 0,
            troll_pipes: dtri((b2 >> 2) & 1 != 0, (b11 >> 2) & 1 != 0),
            starting_lives,
            card_speed_clear: (b4 >> 7) & 1 != 0,
            remove_n_cards: (b4 >> 6) & 1 != 0,
            skip_wand_cutscene: (b4 >> 5) & 1 != 0,
            adjust_boss_hitboxes: (b4 >> 4) & 1 != 0,
            shuffle_spade_games: (b4 >> 3) & 1 != 0,
            hammer_vulnerable_koopalings: (b5 >> 1) & 1 != 0,
            early_sun: b5 & 1 != 0,
            limit_bro_movement: (b9 >> 3) & 1 != 0,
            japanese_damage: (b6 >> 7) & 1 != 0,
            infinite_mushroom_houses: (b6 >> 6) & 1 != 0,
            fast_mushroom_house: (b3 >> 4) & 1 != 0,
            faster_tail_speed: (b3 >> 3) & 1 != 0,
            no_game_over_penalty: (b3 >> 2) & 1 != 0,
            swap_start_airship: (b3 >> 1) & 1 != 0,
            more_hammer_rocks: dtri(b3 & 1 != 0, (b11 >> 3) & 1 != 0),
            eights_are_wild: dtri((b11 >> 6) & 1 != 0, (b11 >> 7) & 1 != 0),
            random_koopalings: (b10 >> 7) & 1 != 0,
            include_beta_stages: (b10 >> 6) & 1 != 0,
            hammer_breaks_bridges: dtri((b3 >> 7) & 1 != 0, (b11 >> 1) & 1 != 0),
            fire_flower: dffm(b11 >> 4),
            ground: dem(b5 >> 6),
            shell: dem(b5 >> 4),
            flying: dem(b5 >> 2),
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
                let wc = b9 & 0x07;
                if wc == 0 { 7 } else { wc.clamp(1, 7) }
            },
            skip_rom_validation: false,
            anchor_visuals: false,
        })
    }

    /// Returns true if any enemy class is enabled (not Off).
    pub fn any_enemies_active(&self) -> bool {
        self.ground != EnemyMode::Off || self.shell != EnemyMode::Off
            || self.flying != EnemyMode::Off
            || self.piranhas != EnemyMode::Off
            || self.ghosts != EnemyMode::Off || self.thwomps != EnemyMode::Off
            || self.rotodiscs != EnemyMode::Off || self.cannons != EnemyMode::Off
            || self.water != EnemyMode::Off || self.bros != EnemyMode::Off
            || self.hb_encounters != EnemyMode::Off || self.wild_injections
    }
}
