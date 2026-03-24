use rand::seq::IndexedRandom;
use rand_chacha::ChaCha8Rng;

use crate::rom::Rom;

/// Text encoding for SMB3 main text table.
fn encode_char(c: char) -> u8 {
    match c {
        'A'..='Z' => 0xB0 + (c as u8 - b'A'),
        'a'..='p' => 0xD0 + (c as u8 - b'a'),
        'q' => 0xCA,
        'r' => 0xCB,
        's' => 0xCC,
        't' => 0xCD,
        'u' => 0xCE,
        'v' => 0xCF,
        'w' => 0x81,
        'x' => 0x88,
        'y' => 0x8C,
        'z' => 0x8F,
        ' ' => 0xFE,
        ',' => 0x9A,
        '.' => 0xE9,
        '\'' => 0xAB,
        '!' => 0xEA,
        '?' => 0xEB,
        _ => 0xFE, // unknown → space
    }
}

/// Encode a quote (6 lines, each up to 20 chars) into 120 ROM bytes.
fn encode_quote(lines: &[&str; 6]) -> [u8; 120] {
    let mut buf = [0xFE; 120]; // fill with spaces
    for (i, line) in lines.iter().enumerate() {
        for (j, c) in line.chars().enumerate() {
            if j < 20 {
                buf[i * 20 + j] = encode_char(c);
            }
        }
    }
    buf
}

/// Pool of quotes the king can say. Each is 6 lines x 20 chars max.
/// Characters available: A-Z, a-z, space, comma, period, apostrophe, !, ?
const QUOTES: &[[&str; 6]] = &[
    [
        "Hey, why don't I",
        "just go eat some",
        "hay, make things",
        "out of clay, lay by",
        "the bay? I just",
        "may! What'd ya say?",
    ],
    [
        "I just saved a bunch",
        "of coins by getting",
        "a new plumber.",
        "",
        "Here is a letter",
        "from the Princess.",
    ],
    [
        "You're late!",
        "I've been a dog",
        "for three days.",
        "Do you know how many",
        "fire hydrants there",
        "are in this kingdom?",
    ],
    [
        "The wizard turned me",
        "into a newt!",
        "",
        "I got better.",
        "",
        "Here is your letter.",
    ],
    [
        "Thank you,brave one!",
        "Please accept this",
        "lukewarm coffee as",
        "a token of my",
        "gratitude.",
        "",
    ],
    [
        "I was told there",
        "would be cake.",
        "",
        "There is no cake.",
        "",
        "Here is a letter.",
    ],
    [
        "Before you go,",
        "have you considered",
        "a career in",
        "castle security?",
        "We clearly need it.",
        "",
    ],
    [
        "You know,being a",
        "dog wasn't all bad.",
        "Free belly rubs.",
        "No meetings.",
        "",
        "Anyway,thanks.",
    ],
    [
        "My therapist says I",
        "need to stop getting",
        "kidnapped.",
        "",
        "Here is a letter",
        "from the Princess.",
    ],
    [
        "Fun fact!",
        "I am the fourth king",
        "you've rescued and",
        "not one of us knows",
        "how to fight.",
        "",
    ],
    [
        "Please don't tell",
        "anyone you found me",
        "hiding in a closet.",
        "",
        "Here is a letter",
        "from the Princess.",
    ],
    [
        "That wizard turned",
        "me into a frog.",
        "A princess kissed me",
        "but I'm still a",
        "king. Awkward.",
        "",
    ],
    [
        "I tried to fight",
        "the wizard myself.",
        "It did not go well.",
        "",
        "Please take this",
        "letter and go.",
    ],
    [
        "Between you and me,",
        "being a king is just",
        "waving and signing",
        "things all day.",
        "",
        "Thanks for the wand!",
    ],
    [
        "Oh good,you're here.",
        "The royal plumber",
        "finally shows up.",
        "",
        "Here is a letter",
        "from the Princess.",
    ],
    [
        "I've been stuck as",
        "a bug for a week.",
        "On the bright side,",
        "I can now see in",
        "every direction.",
        "",
    ],
    [
        "Wonderful!",
        "Now if you could",
        "also fix the roof,",
        "unclog the moat,and",
        "mow the lawn...",
        "No? Just the wand?",
    ],
    [
        "Thank you,hero!",
        "The kingdom owes",
        "you a great debt.",
        "And by debt I mean",
        "this letter.",
        "Budgets are tight.",
    ],
    [
        "Word of advice.",
        "Never trust a wizard",
        "who offers you a",
        "free makeover.",
        "",
        "Take this letter.",
    ],
    [
        "I thought you'd",
        "never get here!",
        "Did you stop for",
        "coins on the way?",
        "",
        "Here is a letter.",
    ],
    [
        "You missed the feast",
        "but I saved you a",
        "mushroom.",
        "",
        "Also,a letter from",
        "the Princess.",
    ],
    [
        "The kingdom thanks",
        "you!",
        "",
        "The kingdom also",
        "has no money.",
        "Here is a letter.",
    ],
    [
        "Legend says a hero",
        "in red would save",
        "us. I expected",
        "someone taller.",
        "",
        "Anyway,here. Letter.",
    ],
    [
        "I was a cat for two",
        "weeks. I knocked",
        "everything off every",
        "table in the castle.",
        "No regrets.",
        "",
    ],
    [
        "Thank you so much!",
        "I would knight you",
        "but I lost my sword",
        "when I was a frog.",
        "",
        "Take this letter.",
    ],
    [
        "While I was gone",
        "my advisors voted",
        "to replace me with",
        "a potted plant.",
        "It passed.",
        "Here is your letter.",
    ],
    [
        "Oh,it's you!",
        "The princess said",
        "you'd come.",
        "She also said you'd",
        "be faster.",
        "",
    ],
    [
        "Do you know what",
        "it's like being",
        "turned into a shoe?",
        "Nobody wants to",
        "talk about it.",
        "",
    ],
    [
        "Started making it,",
        "had a breakdown,",
        "bon appetit!",
        "",
        "",
        "",
    ],
    [
        "Soylent Green",
        "is...",
        "Toads.",
        "",
        "",
        "",
    ],
    [
        "Luigi, I love you,",
        "but sooner or later,",
        "you're going to have",
        "to face the fact",
        "you're a goddamn",
        "moron.",
    ],
    [
        "Fireplants.",
        "Lots of fireplants",
        "",
        "",
        "",
        "",
    ],
    [
        "I'd rather not spend",
        "the rest of this",
        "seed",
        "TIED TO THIS",
        "F'ING THRONE!",
        "",
    ],
    [
        "Good night,Westley.",
        "Good work.",
        "Sleep well.",
        "I'll most likely",
        "kill you in the",
        "morning.",
    ],
    [
        "The Dude is not in.",
        "Leave a message",
        "after the beep.",
        "",
        "It takes a minute",
        "",
    ],
    [
        "Somehow,",
        "Bowser returned.",
        "",
        "",
        "",
        "",
    ],
    [
        "What a horrible",
        "night to have",
        "a curse.",
        "",
        "",
        "",
    ],
    [
        "It's time for",
        "revenge.",
        "Let's attack",
        "aggressively!",
        "",
        "",
    ],
    [
        "Mario?",
        "",
        "Mario?",
        "",
        "MAAAAAAAARIOOO!!!",
        "",
    ],
    [
        "Do a barrel roll!",
        "",
        "",
        "",
        "",
        "",
    ],
    [
        "Praise the",
        "Angry Sun!",
        "",
        "",
        "",
        "",
    ],
    [
        "The right toad",
        "in the wrong place",
        "can make all the",
        "difference in",
        "the world.",
        "",
    ],
    [
        "What is a plumber?",
        "",
        "A miserable little",
        "pile of secrets.",
        "",
        "",
    ],
    [
        "SMB III.",
        "",
        "SMB III never",
        "changes.",
        "Unless its",
        "randomizer...",
    ],
    [
        "You and your",
        "friends are dead.",
        "",
        "Game Over",
        "",
        "",
    ],
    [
        "I am",
        "Error.",
        "",
        "",
        "",
        "",
    ],
    [
        "Somebody set up us",
        "the Bobomb.",
        "",
        "",
        "",
        "",
    ],
    [
        "Mario's name is",
        "Mario Mario,",
        "Luigi's name is",
        "Luigi Mario,",
        "oh!",
        "the Mario Bros.",
    ],
    [
        "Do you know what I",
        "love about mud?",
        "It's clean and",
        "it's dirty at the",
        "same time.",
        "",
    ],
    [
        "Do the words",
        "'doo hoo hoo'",
        "mean anything",
        "to you?",
        "",
        "",
    ],
    [
        "I picked a hell",
        "of a day to quit",
        "drinkin'.",
        "",
        "",
        "",
    ],
    [
        "Ten years later, my",
        "niece is getting",
        "married.",
        "My biological clock",
        "is...",
        "ticking",
    ],
];

/// Suit-specific quotes: shown when Mario visits the king wearing frog suit.
const FROG_QUOTES: &[[&str; 6]] = &[
    [
        "Is that a frog suit?",
        "I was just turned",
        "INTO a frog.",
        "Read the room,",
        "plumber.",
        "",
    ],
    [
        "Nice frog suit!",
        "You know,I was a",
        "frog once too.",
        "Small world.",
        "",
        "Here is your letter.",
    ],
    [
        "A frog! At last,",
        "someone who",
        "understands what",
        "I've been through.",
        "",
        "Take this letter.",
    ],
    [
        "we got literally",
        "every girls costume",
        "in the entire",
        "goddamn universe...",
        "and frog",
        "",
    ],
];

/// Suit-specific quotes: shown when Mario visits the king as raccoon/tanooki.
const RACCOON_QUOTES: &[[&str; 6]] = &[
    [
        "Thank you,kind",
        "raccoon.",
        "",
        "Please tell me your",
        "name.",
        "",
    ],
    [
        "A flying raccoon!",
        "Now I've seen",
        "everything.",
        "",
        "Here is a letter",
        "from the Princess.",
    ],
    [
        "Nice tail!",
        "Is that a raccoon",
        "thing or a plumber",
        "thing?",
        "",
        "",
    ],
    [
        "What kind of car",
        "does a raccoon",
        "drive?",
        "",
        "A Furrari.",
        "",
    ],
];

/// Suit-specific quotes: shown when Mario visits the king in hammer suit.
const HAMMER_QUOTES: &[[&str; 6]] = &[
    [
        "Hey,you!",
        "How about lending me",
        "your clothes?",
        "No dice?!",
        "What a drag.",
        "",
    ],
    [
        "Nice outfit!",
        "Are those hammers?",
        "Can you fix my",
        "castle roof while",
        "you're here?",
        "",
    ],
    [
        "I used to be a",
        "plumber like",
        "you. Then I took",
        "a hammer in the",
        "knee...",
        "Now I'm King",
    ],
];

/// Fixed ROM offsets for the 3 suit-specific quote slots (120 bytes each).
/// Vanilla table at $A494 already points forms 4/5/6 here — we just replace content.
const FROG_QUOTE_OFFSET: usize = 0x3633C;
const RACCOON_QUOTE_OFFSET: usize = 0x363B4;
const HAMMER_QUOTE_OFFSET: usize = 0x3642C;

/// Free space in PRG027 for standard quote data + ASM hook.
const KING_QUOTE_BASE: usize = 0x379D9;

/// PRG027 file offset 0x36010 maps to CPU $A000.
fn cpu_addr(file_offset: usize) -> u16 {
    0xA000 + (file_offset - 0x36010) as u16
}

/// ROM offset of the vanilla quote selection code at CPU $A293.
/// Vanilla: `LDY Player_Form; LDA $A494,Y; ...` — indexes by powerup only.
/// We patch this to JMP to a hook that checks Player_Form first:
///   Form >= 4 (suit) → fall through to vanilla table lookup (unchanged)
///   Form < 4 (no suit) → index by World_Num for per-world quotes
const QUOTE_SELECT_PATCH: usize = 0x362A3;

/// Write randomized king quotes into the ROM.
pub fn randomize(rom: &mut Rom, rng: &mut ChaCha8Rng) {
    // --- 1. Write 7 unique standard quotes into free space ---
    let mut pool: Vec<usize> = (0..QUOTES.len()).collect();
    let mut chosen = Vec::with_capacity(7);
    for _ in 0..7 {
        let idx = pool.choose(rng).copied().unwrap();
        pool.retain(|&x| x != idx);
        chosen.push(idx);
    }

    let mut std_addrs = Vec::with_capacity(7);
    for (world, &quote_idx) in chosen.iter().enumerate() {
        let encoded = encode_quote(&QUOTES[quote_idx]);
        let file_offset = KING_QUOTE_BASE + world * 120;
        rom.write_range(file_offset, &encoded);
        std_addrs.push(cpu_addr(file_offset));
    }

    // --- 2. Write suit-specific quotes to vanilla slots ---
    // The vanilla pointer table at $A494/$A49B already maps forms 4/5/6
    // to these addresses, so we just replace the content.
    let frog_pick = FROG_QUOTES.choose(rng).unwrap();
    rom.write_range(FROG_QUOTE_OFFSET, &encode_quote(frog_pick));

    let raccoon_pick = RACCOON_QUOTES.choose(rng).unwrap();
    rom.write_range(RACCOON_QUOTE_OFFSET, &encode_quote(raccoon_pick));

    let hammer_pick = HAMMER_QUOTES.choose(rng).unwrap();
    rom.write_range(HAMMER_QUOTE_OFFSET, &encode_quote(hammer_pick));

    // --- 3. Write ASM hook for per-world standard quotes ---
    // Hook goes right after the 7 quote blocks in free space.
    let hook_file = KING_QUOTE_BASE + 7 * 120;
    let hook_cpu = cpu_addr(hook_file);
    let std_lo_cpu = hook_cpu + 40;
    let std_hi_cpu = hook_cpu + 47;

    //  0: LDA $ED          ; Player_Form
    //  2: CMP #$04
    //  4: BCS +18          ; suit → offset 24
    //  6: LDY $0727        ; World_Num (per-world path)
    //  9: LDA std_lo,Y
    // 12: STA $070D
    // 15: LDA std_hi,Y
    // 18: STA $7A04
    // 21: JMP $A2A1
    // 24: TAY              ; suit path — reuse vanilla table
    // 25: LDA $A494,Y
    // 28: STA $070D
    // 31: LDA $A49B,Y
    // 34: STA $7A04
    // 37: JMP $A2A1
    // 40: std_lo[7]        ; data
    // 47: std_hi[7]        ; data
    // Total: 54 bytes
    let mut hook: Vec<u8> = Vec::with_capacity(54);
    hook.extend_from_slice(&[0xA5, 0xED]);                          //  0: LDA $ED
    hook.extend_from_slice(&[0xC9, 0x04]);                          //  2: CMP #$04
    hook.extend_from_slice(&[0xB0, 18]);                            //  4: BCS +18 → offset 24
    hook.extend_from_slice(&[0xAC, 0x27, 0x07]);                   //  6: LDY $0727
    hook.extend_from_slice(&[0xB9, std_lo_cpu as u8, (std_lo_cpu >> 8) as u8]);
    hook.extend_from_slice(&[0x8D, 0x0D, 0x07]);                   // 12: STA $070D
    hook.extend_from_slice(&[0xB9, std_hi_cpu as u8, (std_hi_cpu >> 8) as u8]);
    hook.extend_from_slice(&[0x8D, 0x04, 0x7A]);                   // 18: STA $7A04
    hook.extend_from_slice(&[0x4C, 0xA1, 0xA2]);                   // 21: JMP $A2A1
    hook.push(0xA8);                                                // 24: TAY
    hook.extend_from_slice(&[0xB9, 0x94, 0xA4]);                   // 25: LDA $A494,Y
    hook.extend_from_slice(&[0x8D, 0x0D, 0x07]);                   // 28: STA $070D
    hook.extend_from_slice(&[0xB9, 0x9B, 0xA4]);                   // 31: LDA $A49B,Y
    hook.extend_from_slice(&[0x8D, 0x04, 0x7A]);                   // 34: STA $7A04
    hook.extend_from_slice(&[0x4C, 0xA1, 0xA2]);                   // 37: JMP $A2A1
    for addr in &std_addrs { hook.push(*addr as u8); }             // 40: std_lo[7]
    for addr in &std_addrs { hook.push((*addr >> 8) as u8); }     // 47: std_hi[7]

    rom.write_range(hook_file, &hook);

    // --- 4. Patch original site: JMP hook + NOP fill ---
    let mut patch = [0xEA_u8; 14];
    patch[0] = 0x4C;
    patch[1] = hook_cpu as u8;
    patch[2] = (hook_cpu >> 8) as u8;
    rom.write_range(QUOTE_SELECT_PATCH, &patch);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate_pool(name: &str, pool: &[[&str; 6]]) {
        for (i, quote) in pool.iter().enumerate() {
            for (j, line) in quote.iter().enumerate() {
                assert!(
                    line.len() <= 20,
                    "{name} quote {i} line {j} is {} chars (max 20): \"{line}\"",
                    line.len()
                );
                for c in line.chars() {
                    assert!(
                        matches!(c,
                            'A'..='Z' | 'a'..='z' | ' ' | ',' | '.' | '\'' | '!' | '?'
                        ),
                        "{name} quote {i} line {j} has invalid char '{c}'"
                    );
                }
            }
        }
    }

    #[test]
    fn all_quotes_fit_constraints() {
        validate_pool("QUOTES", QUOTES);
        validate_pool("FROG_QUOTES", FROG_QUOTES);
        validate_pool("RACCOON_QUOTES", RACCOON_QUOTES);
        validate_pool("HAMMER_QUOTES", HAMMER_QUOTES);
    }

    #[test]
    fn encode_round_trip() {
        let lines = [
            "Hello,world!",
            "Test line two.",
            "",
            "Line four here.",
            "",
            "The end.",
        ];
        let encoded = encode_quote(&lines);
        assert_eq!(encoded.len(), 120);
        // First char 'H' should be 0xB7
        assert_eq!(encoded[0], 0xB7);
        // Space padding at end of short lines
        assert_eq!(encoded[19], 0xFE);
    }

    #[test]
    fn pool_has_enough_quotes() {
        assert!(
            QUOTES.len() >= 7,
            "Need at least 7 quotes for 7 unique worlds, have {}",
            QUOTES.len()
        );
    }
}
