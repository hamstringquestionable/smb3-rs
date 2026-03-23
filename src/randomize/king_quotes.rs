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
        "Oh,thank heavens!",
        "I'm back to my old",
        "self again.",
        "Thank you so much.",
        "Here is a letter",
        "from the Princess.",
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
];

/// Free space in PRG027 for king quote data.
const KING_QUOTE_BASE: usize = 0x379D9;

/// Pointer table: 8 low bytes at 0x364A4, high bytes at 0x364AC.
const PTR_LO_OFFSET: usize = 0x364A4;
const PTR_HI_OFFSET: usize = 0x364AC;

/// CPU address for a message slot in PRG027 free space.
/// PRG027 file offset 0x36010 maps to CPU $A000.
fn cpu_addr(file_offset: usize) -> u16 {
    0xA000 + (file_offset - 0x36010) as u16
}

/// Write randomized king quotes into the ROM.
pub fn randomize(rom: &mut Rom, rng: &mut ChaCha8Rng) {
    // Pick 7 unique quotes (one per world 1-7)
    let mut pool: Vec<usize> = (0..QUOTES.len()).collect();
    let mut chosen = Vec::with_capacity(7);
    for _ in 0..7 {
        let idx = pool.choose(rng).copied().unwrap();
        pool.retain(|&x| x != idx);
        chosen.push(idx);
    }

    // Write 7 encoded quotes into free space and update pointer table
    for (world, &quote_idx) in chosen.iter().enumerate() {
        let encoded = encode_quote(&QUOTES[quote_idx]);
        let file_offset = KING_QUOTE_BASE + world * 120;
        rom.write_range(file_offset, &encoded);

        let addr = cpu_addr(file_offset);
        rom.write_byte(PTR_LO_OFFSET + world, addr as u8);
        rom.write_byte(PTR_HI_OFFSET + world, (addr >> 8) as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_quotes_fit_constraints() {
        for (i, quote) in QUOTES.iter().enumerate() {
            for (j, line) in quote.iter().enumerate() {
                assert!(
                    line.len() <= 20,
                    "Quote {i} line {j} is {} chars (max 20): \"{line}\"",
                    line.len()
                );
                for c in line.chars() {
                    assert!(
                        matches!(c,
                            'A'..='Z' | 'a'..='z' | ' ' | ',' | '.' | '\'' | '!' | '?'
                        ),
                        "Quote {i} line {j} has invalid char '{c}'"
                    );
                }
            }
        }
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
