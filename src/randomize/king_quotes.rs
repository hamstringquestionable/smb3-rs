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
///
/// Generic over the line type because the joke pools are `&'static str` picked
/// off a shelf while the oracle composes his with `format!` — see [`Lines`].
fn encode_quote<S: AsRef<str>>(lines: &[S; 6]) -> [u8; 120] {
    let mut buf = [0xFE; 120]; // fill with spaces
    for (i, line) in lines.iter().enumerate() {
        for (j, c) in line.as_ref().chars().enumerate() {
            if j < 20 {
                buf[i * 20 + j] = encode_char(c);
            }
        }
    }
    buf
}

/// Pool of quotes the king can say. Each is 6 lines x 20 chars max.
/// Characters available: A-Z, a-z, space, comma, period, apostrophe, !, ?
// One array element per on-screen line, which is the point: each string is a
// line the NES draws, with its own width limit. rustfmt would pack all six onto
// one row and hide the shape of the quote. The `/king-quote` skill writes here.
#[rustfmt::skip]
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
        "One does not simply",
        "walk into Bowser's",
        "castle.",
        "",
        "And yet,here we",
        "are. Good luck.",
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
        "Have you tried",
        "sudo rescue king?",
        "",
        "Permission denied.",
        "",
        "Here is your letter.",
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
        "Yeah,if you could",
        "go ahead and defeat",
        "Bowser one more",
        "time,that'd be",
        "great. Thanks.",
        "",
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
        "Pop quiz,plumber.",
        "There's a bomb on",
        "the airship.",
        "",
        "Just kidding. It's",
        "a koopaling.",
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
        "It's a hundred six",
        "miles to Bowser,",
        "it's dark, and",
        "we're wearing",
        "overalls...",
        "Hit it!",
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
    [
        "Oh, DannyP,",
        "the pipes, the pipes",
        "are calling.",
        "",
        "",
        "",
    ],
    [
        "And who could",
        "forget dear",
        "Rat Boy?",
        "",
        "",
        "",
    ],
    [
        "Every plumber",
        "reaches a warp zone",
        "he can't come back",
        "from.",
        "",
        "",
    ],
    [
        "Nobody wants to win?",
        "Nobody wants to win",
        "this race?",
        "Everybody's",
        "playing to",
        "not lose...",
    ],
    [
        "I bet you wish this",
        "was a Varia Suit",
        "...",
        "oops wrong",
        "randomizer!",
        "",
    ],
    [
        "You may not be",
        "the best but at",
        "least you can",
        "beat Dr. Torstol",
        "",
        "",
    ],
    [
        "HAXOR",
        "TO D",
        "MAXOR",
        "",
        "",
        "",
    ],
    [
        "It looks like you're",
        "trying to clip",
        "through this wall.",
        "Would you like help?",
        "",
        "",
    ],
    [
        "Foul Tarnished,in",
        "search of the",
        "Elden Ring?",
        "Wendy borrowed it.",
        "She keeps throwing",
        "it at people.",
    ],
    [
        "Lemmy rolled his",
        "ball right off",
        "his airship.",
        "",
        "",
        "FALL OUT!",
    ],
    [
        "FINISH HIM!",
        "",
        "Oh.You already",
        "did.Nevermind.",
        "",
        "",
    ],
    [
        "A wild plumber",
        "appeared!",
        "",
        "You used RESCUE.",
        "It was super",
        "effective!",
    ],
    [
        "The wizard seemed",
        "kind of sus.",
        "",
        "I saw him vent",
        "into the chimney.",
        "",
    ],
    [
        "FUS RO DAH!",
        "",
        "Sorry.The wizard",
        "also taught me a",
        "few words.",
        "",
    ],
    [
        "I'm the king of",
        "the world!",
        "",
        "Well.One eighth",
        "of it,anyway.",
        "",
    ],
    [
        "The wand was stolen",
        "and we asked,who",
        "you gonna call?",
        "",
        "A plumber,it turns",
        "out.",
    ],
    [
        "If you can dodge a",
        "Rocky Wrench you can",
        "dodge a Cheep Cheep.",
        "",
        "",
        "",
    ],
    [
        "Don't have negative",
        "thoughts.",
        "",
        "Remember your",
        "mantra.",
        "",
    ],
    [
        "If you listen",
        "closely, you can",
        "hear DemonBattler",
        "screaming in the",
        "distance.",
        "",
    ],
    [
        "The wand? Forget the",
        "wand. This kingdom",
        "needs decorating.",
        "",
        "Put a Birdo on it!",
        "",
    ],
    [
        "They're taking the",
        "Toads to Dark Land!",
        "They're taking the",
        "Toads to Dark Land!",
        "They're taking the",
        "Toads to Dark Land!",
    ],
];

/// Suit-specific quotes: shown when Mario visits the king wearing frog suit.
#[rustfmt::skip]
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
    [
        "It isn't easy",
        "being green,",
        "is it?",
        "",
        "Here is your letter.",
        "",
    ],
    [
        "You swam all the",
        "way here in THAT?",
        "",
        "But that's none of",
        "my business.",
        "",
    ],
    [
        "You crossed six",
        "lanes of traffic",
        "to get here?",
        "",
        "Frogger could never.",
        "",
    ],
    [
        "Whatever you do,",
        "do NOT kiss me.",
        "I know how that",
        "story ends.",
        "",
        "",
    ],
    [
        "Thank you,hero!",
        "This rescue was",
        "simply",
        "unfroggettable.",
        "",
        "",
    ],
    [
        "What does a frog",
        "eat with his",
        "burger?",
        "",
        "French flies.",
        "",
    ],
    [
        "Why are frogs",
        "always so happy?",
        "",
        "They eat whatever",
        "bugs them.",
        "",
    ],
    [
        "I'm so hoppy",
        "you came!",
        "",
        "Sorry.Frog puns.",
        "Won't hoppen again.",
        "",
    ],
    [
        "A frog hero! Just",
        "like Slippy.",
        "",
        "Wait.Bad example.",
        "",
        "",
    ],
    [
        "Don't forget to",
        "start your timer,",
        "HumanMustard.",
        "",
        "Oh, too late.",
        "Sorry, viewers!",
    ],
    [
        "You have my sword.",
        "",
        "And my hammer.",
        "",
        "And your frog suit.",
        "",
    ],
];

/// Suit-specific quotes: shown when Mario visits the king as raccoon/tanooki.
#[rustfmt::skip]
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
    [
        "Ain't no thing like",
        "me,except me!",
        "",
        "Well.Except you,",
        "apparently.",
        "",
    ],
    [
        "The Princess calls",
        "you a trash panda.",
        "",
        "I'm sure it's a",
        "compliment.",
        "",
    ],
    [
        "A raccoon offered",
        "me a loan once.",
        "I'm still paying",
        "off this castle.",
        "",
        "",
    ],
    [
        "With a tail like",
        "that you should",
        "work in retail.",
        "",
        "Ha! Retail.",
        "",
    ],
    [
        "You can FLY?",
        "",
        "Quick,do a barrel",
        "roll!",
        "",
        "",
    ],
    [
        "It's a bird!",
        "It's a plane!",
        "No,it's a flying",
        "raccoon plumber?",
        "",
        "",
    ],
    [
        "You didn't knock",
        "over my bins on",
        "the way in,did you?",
        "",
        "",
        "",
    ],
];

/// Suit-specific quotes: shown when Mario visits the king in hammer suit.
#[rustfmt::skip]
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
    [
        "Oh wait,before",
        "you go...",
        "",
        "STOP!",
        "",
        "Hammer time.",
    ],
    [
        "Love the outfit.",
        "I bet nobody can",
        "touch this.",
        "",
        "",
        "",
    ],
    [
        "Whosoever holds",
        "this hammer,if he",
        "be worthy...",
        "",
        "Eh,you get the idea.",
        "",
    ],
    [
        "When all you have",
        "is a hammer,every",
        "problem looks like",
        "a Koopa.",
        "",
        "",
    ],
    [
        "You really NAILED",
        "that rescue.",
        "",
        "Get it? Because",
        "hammers? Anyone?",
        "",
    ],
    [
        "GUARDS! A Hammer",
        "Bro in the castle!",
        "",
        "Oh.It's just you.",
        "My apologies.",
        "",
    ],
    [
        "You look positively",
        "smashing,my friend.",
        "",
        "",
        "",
        "",
    ],
    [
        "A hammer,eh?",
        "You know what this",
        "castle needs?",
        "",
        "MORE POWER.",
        "",
    ],
    [
        "If I had a hammer,",
        "I'd hammer in the",
        "morning...",
        "",
        "Sorry.Old song.",
        "",
    ],
];

/// Fixed ROM offsets for the 3 suit-specific quote slots (120 bytes each).
/// Vanilla table at $A494 already points forms 4/5/6 here — we just replace content.
const FROG_QUOTE_OFFSET: usize = 0x3633C;
const RACCOON_QUOTE_OFFSET: usize = 0x363B4;
const HAMMER_QUOTE_OFFSET: usize = 0x3642C;

/// Free space in PRG027 for standard quote data + ASM hook.
const KING_QUOTE_BASE: usize = super::rom_data::FS_KING_QUOTES;

/// PRG027 (file 0x36010) is an $A000-window bank.
fn cpu_addr(file_offset: usize) -> u16 {
    super::rom_data::prg_bank_file_to_cpu(27, file_offset)
}

/// ROM offset of the vanilla quote selection code at CPU $A293.
/// Vanilla: `LDY Player_Form; LDA $A494,Y; ...` — indexes by powerup only.
/// We patch this to JMP to a hook that checks Player_Form first:
///   Form >= 4 (suit) → fall through to vanilla table lookup (unchanged)
///   Form < 4 (no suit) → index by World_Num for per-world quotes
const QUOTE_SELECT_PATCH: usize = 0x362A3;

/// The king who speaks for the oracle: the king of the world the player
/// **starts** in — `world_order`'s first entry.
///
/// That is what makes him worth hearing. His "where the airship takes you
/// next" is a prediction the player can act on from the first airship; heard
/// on the fifth world it would predict the sixth, with one world left to spend
/// it on. With world order off the first world *is* Grass Land, so the
/// fallback is not arbitrary.
///
/// The sprite follows with no ROM-side work. `TAndK_DrawKingAndToad` picks the
/// king's CHR page, pattern index and both attribute bytes with
/// `LDX World_Num` (`prg027.asm:777`), the quote-select hook indexes with
/// `LDY $0727`, and the stomp threshold table is read the same way — three
/// lookups off one byte, resolved in the same frame. So the line always
/// arrives in the mouth of the king whose world it is, whichever king that is
/// this seed.
///
/// Falls back to Grass Land when the progression starts in Dark Land
/// (`world_count` 0), which has no king.
fn oracle_world(facts: &OracleFacts) -> usize {
    facts
        .world_progression
        .and_then(<[u8]>::first)
        .map(|&w| usize::from(w))
        .filter(|&w| w < 7)
        .unwrap_or(0)
}

/// Vanilla's stomp threshold: every Koopaling takes three hits. This is the
/// table to pass when `koopaling_hits` is off, and it is what makes the
/// "is this even randomized?" line fire exactly when it is not.
pub const VANILLA_KOOPALING_HITS: [u8; 7] = [3; 7];

/// One king per seed reacts to the Koopaling stomp thresholds instead of
/// telling a joke. The table is `FS_KOOPA_HITS_TABLE`, which the generated
/// stomp code reads with `LDY $0727` — the same `World_Num` the quote-select
/// hook indexes by. Same index, so a per-world line baked here is still
/// correct however the player reaches that world, the world maze included.
///
/// Indexed by stomp count minus one.
#[rustfmt::skip]
const KOOPA_BUCKETS: [[&str; 6]; 5] = [
    [
        "One stomp.",
        "",
        "I did not even have",
        "time to finish my",
        "tea.",
        "",
    ],
    [
        "Two stomps.",
        "",
        "Adequate. I have",
        "seen better, but I",
        "have also been a",
        "dog for a week.",
    ],
    [
        "Three stomps.",
        "",
        "The number it has",
        "always been.",
        "",
        "Is this randomized?",
    ],
    [
        "Four stomps.",
        "",
        "That one had been",
        "practising. I could",
        "hear it through the",
        "wall.",
    ],
    [
        "Five stomps!",
        "",
        "I watched the last",
        "two through my",
        "fingers.",
        "",
    ],
];

#[rustfmt::skip]
const PAT_ALL_EQUAL: [&str; 6] = [
    "Every one of them",
    "takes the same",
    "number of stomps.",
    "",
    "Is this even",
    "randomized?",
];

#[rustfmt::skip]
const PAT_ALL_PUSHOVER: [&str; 6] = [
    "Not one of them",
    "takes more than two",
    "stomps.",
    "",
    "This is embarrassing",
    "for all of us.",
];

#[rustfmt::skip]
const PAT_ALL_BRUTAL: [&str; 6] = [
    "Every last one of",
    "them takes four",
    "stomps or more.",
    "",
    "I would apologise,",
    "but I did not do it.",
];

#[rustfmt::skip]
const PAT_MEAN_LOW: [&str; 6] = [
    "I have reviewed the",
    "reports. This is a",
    "kingdom of",
    "pushovers.",
    "",
    "Enjoy it.",
];

#[rustfmt::skip]
const PAT_MEAN_HIGH: [&str; 6] = [
    "I have reviewed the",
    "reports.",
    "",
    "Every one of them",
    "is a brute. I am",
    "sorry.",
];

#[rustfmt::skip]
const PAT_THREE_ONES: [&str; 6] = [
    "Three of my",
    "brothers are guarded",
    "by creatures you can",
    "stomp once.",
    "",
    "Do not gloat.",
];

#[rustfmt::skip]
const PAT_THREE_FIVES: [&str; 6] = [
    "Three of them will",
    "take five stomps",
    "each.",
    "",
    "I shall not say",
    "which. Sleep well.",
];

/// Everything the oracle is allowed to know.
///
/// One field per fact, every one of them already decided by the time the quotes
/// are written — `randomizer::randomize_inner` fills this in immediately before
/// the call. A topic that cannot be answered out of these fields is a topic the
/// king may not raise: he is believed, so a line naming the wrong world is
/// worse than no line at all.
pub struct OracleFacts<'a> {
    /// Per-world Koopaling stomp thresholds; [`VANILLA_KOOPALING_HITS`] when
    /// the option is off, which is a real fact about the ROM and not a
    /// placeholder.
    pub koopaling_hits: [u8; 7],
    /// The world progression in play order, or `None` when the worlds run in
    /// their vanilla sequence and "what comes next" is not a prediction.
    ///
    /// The world maze supplies it too: clearing an airship there follows this
    /// same table (the spine), so the remark stays true in both modes.
    pub world_progression: Option<&'a [u8]>,
    /// 1-F's treasure chest, or `None` when the fort deal left that level off
    /// every map this seed.
    pub one_f_chest: Option<OneFChest>,
}

/// The chest in 1-F's sub-area — what is in it, and where the level landed.
///
/// That chest is the one the player has to make a real decision about: the room
/// holding it *is* the level's secret exit, so it can be taken without fighting
/// Boom-Boom, and whether the detour pays depends entirely on the item.
#[derive(Clone, Copy, Debug)]
pub struct OneFChest {
    /// Internal world number of the map the level was dealt to.
    pub world: usize,
    /// Global item ID, read back from `items::ONE_F_CHEST_ITEM`.
    pub item: u8,
}

/// Six composed lines. The oracle names worlds and items, so unlike the joke
/// pools his text cannot be a `&'static` array picked off a shelf.
type Lines = [String; 6];

fn say(lines: [&str; 6]) -> Lines {
    lines.map(String::from)
}

/// What the oracle says this seed: one of the topics he can speak to, drawn at
/// random.
///
/// The stomp thresholds never decline, so there is always at least one.
fn oracle_remark(facts: &OracleFacts, rng: &mut ChaCha8Rng) -> Lines {
    let mut repertoire = vec![topic_stomp_thresholds(facts)];
    repertoire.extend(topic_next_world(facts));
    repertoire.extend(topic_one_f_chest(facts));
    repertoire.choose(rng).expect("the stomp topic never declines").clone()
}

/// The eight worlds by name, indexed by internal world number.
///
/// Names, not display numbers, and that is a correctness point rather than a
/// stylistic one: `world_order` renumbers the maps, so "World 6" means one
/// thing on the map screen and another in `World_Num`, while "Ice Land" means
/// the same thing in every mode.
const WORLD_NAMES: [&str; 8] = [
    "Grass Land",
    "Desert Land",
    "Water Land",
    "Giant Land",
    "Sky Land",
    "Ice Land",
    "Pipe Land",
    "Dark Land",
];

/// One closing line per world for the next-world remark, indexed the same way.
const NEXT_WORLD_FLAVOUR: [&str; 8] = [
    "You have been there.",
    "Take water with you.",
    "Hold your breath.",
    "Nothing there fits.",
    "Do not look down.",
    "Nothing holds still.",
    "I got lost there.",
    "I am sorry.",
];

/// The thresholds, either as a remark on the table's shape or as his own
/// world's count. The one topic that never declines, which is what keeps the
/// repertoire non-empty.
fn topic_stomp_thresholds(facts: &OracleFacts) -> Lines {
    let table = &facts.koopaling_hits;
    match table_pattern(table) {
        Some(quote) => say(*quote),
        None => {
            // The generator only ever writes 1-5; clamp so a future widening of
            // that range cannot index off the end of the bucket array.
            let count = usize::from(table[oracle_world(facts)].clamp(1, 5));
            say(KOOPA_BUCKETS[count - 1])
        }
    }
}

/// Where the airship is about to take the player.
///
/// Silent without a shuffled progression: in vanilla order the answer is always
/// the next world along, and a prediction nobody could have got wrong is not a
/// prediction. The oracle is the progression's first world (see
/// [`oracle_world`]), so the airship after his is `order[1]`; a progression
/// with no second entry (`world_count` 0) has nothing to predict.
fn topic_next_world(facts: &OracleFacts) -> Option<Lines> {
    let next = usize::from(*facts.world_progression?.get(1)?);
    Some([
        "I have seen where".into(),
        "the airship takes".into(),
        "you next.".into(),
        String::new(),
        format!("{}.", WORLD_NAMES[next]),
        NEXT_WORLD_FLAVOUR[next].into(),
    ])
}

/// Whether 1-F's chest is worth the detour.
///
/// **Only the strong verdicts.** Five of the twelve possible items are a matter
/// of taste — a leaf is welcome and never urgent — and the king declines those
/// rather than shrugging on the record.
fn topic_one_f_chest(facts: &OracleFacts) -> Option<Lines> {
    let chest = facts.one_f_chest?;
    let (item, verdict) = one_f_verdict(chest.item)?;
    Some([
        "A fortress hides a".into(),
        "chest. Look for it".into(),
        format!("in {}.", WORLD_NAMES[chest.world]),
        String::new(),
        item.into(),
        verdict.into(),
    ])
}

/// The item line and the verdict line, or `None` where the king has no strong
/// opinion. Global item IDs, as written into the chest's `D6` object.
fn one_f_verdict(item: u8) -> Option<(&'static str, &'static str)> {
    Some(match item {
        // Worth the walk: two map items that save whole levels, the box that
        // walks a world for free, and the whistle that skips several.
        0x07 => ("A cloud is inside.", "Go. Go now."),
        0x0B => ("A hammer is inside.", "You will want that."),
        0x0C => ("A whistle is inside.", "Yes. That whistle."),
        0x0D => ("A music box.", "Sleep is a weapon."),
        // Not worth it: a powerup you will find in the next block, a suit for
        // water the fortress does not have, and a star that expires on the way.
        0x01 => ("One mushroom.", "Do not walk for it."),
        0x04 => ("A frog suit.", "Not worth the walk."),
        0x09 => ("A starman.", "It will not wait."),
        _ => return None,
    })
}

/// A remark about the *shape* of the whole threshold table, or `None` when the
/// table is unremarkable and the king should fall back to his own world.
///
/// Every predicate here is order-invariant — a fact about the seven values,
/// never about the order the player meets them in. That is deliberate: the
/// world maze lets a player reach airships in an order chosen at run time, so
/// "the last one took five" is unknowable when these bytes are written, while
/// "two of them take five" stays true however the run goes.
///
/// Most specific first; the first predicate that holds wins.
fn table_pattern(table: &[u8; 7]) -> Option<&'static [&'static str; 6]> {
    let sum: u32 = table.iter().map(|&v| u32::from(v)).sum();
    let ones = table.iter().filter(|&&v| v == 1).count();
    let fives = table.iter().filter(|&&v| v == 5).count();
    let lo = *table.iter().min().expect("table is non-empty");
    let hi = *table.iter().max().expect("table is non-empty");

    if lo == hi {
        return Some(&PAT_ALL_EQUAL);
    }
    if hi <= 2 {
        return Some(&PAT_ALL_PUSHOVER);
    }
    if lo >= 4 {
        return Some(&PAT_ALL_BRUTAL);
    }
    // 7 worlds, so mean <= 2.0 is sum <= 14 and mean >= 4.0 is sum >= 28.
    if sum <= 14 {
        return Some(&PAT_MEAN_LOW);
    }
    if sum >= 28 {
        return Some(&PAT_MEAN_HIGH);
    }
    if ones >= 3 {
        return Some(&PAT_THREE_ONES);
    }
    if fives >= 3 {
        return Some(&PAT_THREE_FIVES);
    }
    None
}

/// Write randomized king quotes into the ROM.
///
/// `enabled` gates the ROM writes only. Every RNG draw this module makes
/// happens above the early return, so a run with quotes off consumes exactly
/// the same seed stream as one with them on and nothing downstream shifts.
/// Keep it that way: never move a `choose` call below the return.
pub fn randomize(rom: &mut Rom, rng: &mut ChaCha8Rng, enabled: bool, facts: &OracleFacts) {
    // --- 1. Draw every quote, whether or not we are going to write one ---
    // choose_multiple samples without replacement, so the 7 quotes are unique.
    // It draws inside the call, not lazily as the returned iterator is walked —
    // so what keeps the stream stable is calling it, not collecting it.
    let std_picks: Vec<&[&str; 6]> = QUOTES.choose_multiple(rng, 7).collect();
    let frog_pick = FROG_QUOTES.choose(rng).unwrap();
    let raccoon_pick = RACCOON_QUOTES.choose(rng).unwrap();
    let hammer_pick = HAMMER_QUOTES.choose(rng).unwrap();
    // The oracle picks his topic here too, for the same reason: it is a draw.
    let oracle = oracle_remark(facts, rng);

    // Off leaves vanilla's own king text in place — the kings still speak, they
    // just say what they say in the original game. Nothing reads the free-space
    // block or the hook below unless the select-site patch lands, so skipping
    // all of it is enough; there is nothing to blank.
    if !enabled {
        return;
    }

    // --- 2. Write 7 unique standard quotes into free space ---
    let oracle_at = oracle_world(facts);
    let mut std_addrs = Vec::with_capacity(7);
    for (world, quote) in std_picks.iter().enumerate() {
        // The oracle's drawn quote is replaced rather than skipped, so the pick
        // above still consumes exactly seven and the stream stays put.
        let encoded = if world == oracle_at { encode_quote(&oracle) } else { encode_quote(quote) };
        let file_offset = KING_QUOTE_BASE + world * 120;
        rom.write_range(file_offset, &encoded);
        std_addrs.push(cpu_addr(file_offset));
    }

    // --- 3. Write suit-specific quotes to vanilla slots ---
    // The vanilla pointer table at $A494/$A49B already maps forms 4/5/6
    // to these addresses, so we just replace the content.
    rom.write_range(FROG_QUOTE_OFFSET, &encode_quote(frog_pick));
    rom.write_range(RACCOON_QUOTE_OFFSET, &encode_quote(raccoon_pick));
    rom.write_range(HAMMER_QUOTE_OFFSET, &encode_quote(hammer_pick));

    // --- 4. Write ASM hook for per-world standard quotes ---
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
    hook.extend_from_slice(&[0xA5, 0xED]); //  0: LDA $ED
    hook.extend_from_slice(&[0xC9, 0x04]); //  2: CMP #$04
    hook.extend_from_slice(&[0xB0, 18]); //  4: BCS +18 → offset 24
    hook.extend_from_slice(&[0xAC, 0x27, 0x07]); //  6: LDY $0727
    hook.extend_from_slice(&[0xB9, std_lo_cpu as u8, (std_lo_cpu >> 8) as u8]);
    hook.extend_from_slice(&[0x8D, 0x0D, 0x07]); // 12: STA $070D
    hook.extend_from_slice(&[0xB9, std_hi_cpu as u8, (std_hi_cpu >> 8) as u8]);
    hook.extend_from_slice(&[0x8D, 0x04, 0x7A]); // 18: STA $7A04
    hook.extend_from_slice(&[0x4C, 0xA1, 0xA2]); // 21: JMP $A2A1
    hook.push(0xA8); // 24: TAY
    hook.extend_from_slice(&[0xB9, 0x94, 0xA4]); // 25: LDA $A494,Y
    hook.extend_from_slice(&[0x8D, 0x0D, 0x07]); // 28: STA $070D
    hook.extend_from_slice(&[0xB9, 0x9B, 0xA4]); // 31: LDA $A49B,Y
    hook.extend_from_slice(&[0x8D, 0x04, 0x7A]); // 34: STA $7A04
    hook.extend_from_slice(&[0x4C, 0xA1, 0xA2]); // 37: JMP $A2A1
    for addr in &std_addrs {
        hook.push(*addr as u8);
    } // 40: std_lo[7]
    for addr in &std_addrs {
        hook.push((*addr >> 8) as u8);
    } // 47: std_hi[7]

    rom.write_range(hook_file, &hook);

    // --- 5. Patch original site: JMP hook + NOP fill ---
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

    /// The box the NES draws: six lines, 20 columns, and a charset with no
    /// digits in it. Composed lines have to clear the same bar as picked ones.
    fn validate_lines(name: &str, lines: &[String; 6]) {
        let borrowed: [&str; 6] = std::array::from_fn(|i| lines[i].as_str());
        validate_pool(name, &[borrowed]);
    }

    /// Facts with nothing in them but a stomp table: the shape most tests want.
    fn facts(koopaling_hits: [u8; 7]) -> OracleFacts<'static> {
        OracleFacts { koopaling_hits, world_progression: None, one_f_chest: None }
    }

    #[test]
    fn all_quotes_fit_constraints() {
        validate_pool("QUOTES", QUOTES);
        validate_pool("FROG_QUOTES", FROG_QUOTES);
        validate_pool("RACCOON_QUOTES", RACCOON_QUOTES);
        validate_pool("HAMMER_QUOTES", HAMMER_QUOTES);
        validate_pool("KOOPA_BUCKETS", &KOOPA_BUCKETS);
    }

    /// Every quote the Koopaling king can reach fits the box.
    ///
    /// The pattern quotes are separate consts rather than one pool, so walking
    /// the selector over every possible table is what proves none of them was
    /// added without being measured. 5^7 tables is 78,125 — cheap to exhaust,
    /// and exhausting it is the point: a pattern that only fires on a rare
    /// shape would otherwise ship unchecked.
    #[test]
    fn koopaling_quotes_fit_constraints() {
        let mut table = [1u8; 7];
        loop {
            validate_lines("stomp thresholds", &topic_stomp_thresholds(&facts(table)));
            // Odometer over 1..=5 in seven digits.
            let mut i = 0;
            while i < 7 {
                table[i] += 1;
                if table[i] <= 5 {
                    break;
                }
                table[i] = 1;
                i += 1;
            }
            if i == 7 {
                break;
            }
        }
    }

    /// `koopaling_hits` off means vanilla, and vanilla is a flat three — so the
    /// king who notices flatness is exactly the king who says the flag is off.
    #[test]
    fn flag_off_table_reads_as_unrandomized() {
        assert_eq!(
            table_pattern(&VANILLA_KOOPALING_HITS),
            Some(&PAT_ALL_EQUAL),
            "a flat table must trip the all-equal remark"
        );
    }

    /// The bucket fallback names the world's own count, not a neighbour's.
    #[test]
    fn bucket_fallback_reports_its_own_world() {
        // Deliberately unremarkable: mean 3, no three-of-a-kind, not flat.
        let mut table = [1u8, 2, 3, 4, 5, 3, 3];
        // No progression, so the oracle falls back to Grass Land's slot.
        for count in 1..=5u8 {
            table[0] = count;
            assert!(table_pattern(&table).is_none(), "test table must reach the fallback");
            assert_eq!(
                topic_stomp_thresholds(&facts(table)),
                say(KOOPA_BUCKETS[usize::from(count) - 1]),
                "count {count} got the wrong bucket"
            );
        }
    }

    /// Every line the oracle can compose fits the box — worlds and items are
    /// substituted into his text, so the width check has to walk the products,
    /// not the templates.
    #[test]
    fn every_composed_oracle_line_fits() {
        // The line depends only on the successor, so the first world is moot.
        for next in 0..8u8 {
            let order = [0, next];
            let f = OracleFacts {
                koopaling_hits: VANILLA_KOOPALING_HITS,
                world_progression: Some(&order),
                one_f_chest: None,
            };
            let lines = topic_next_world(&f).expect("the first world has a successor here");
            validate_lines("next world", &lines);
        }

        for world in 0..8 {
            for item in 0..=0xFFu8 {
                let f = OracleFacts {
                    koopaling_hits: VANILLA_KOOPALING_HITS,
                    world_progression: None,
                    one_f_chest: Some(OneFChest { world, item }),
                };
                if let Some(lines) = topic_one_f_chest(&f) {
                    validate_lines("1-F chest", &lines);
                }
            }
        }
    }

    /// The oracle declines what he cannot know, rather than guessing.
    #[test]
    fn the_oracle_declines_what_it_cannot_know() {
        // Vanilla world order: "next" is not a prediction.
        assert!(topic_next_world(&facts(VANILLA_KOOPALING_HITS)).is_none());

        // `world_count` 0: the player starts in Dark Land, which has no king,
        // so the oracle falls back to Grass Land — which is not on the
        // progression. And a first world with nothing after it.
        for order in [vec![7u8], vec![3]] {
            let f = OracleFacts {
                koopaling_hits: VANILLA_KOOPALING_HITS,
                world_progression: Some(&order),
                one_f_chest: None,
            };
            assert!(topic_next_world(&f).is_none(), "order {order:?} has no successor to name");
        }

        // 1-F off every map this seed.
        assert!(topic_one_f_chest(&facts(VANILLA_KOOPALING_HITS)).is_none());

        // The five middling items: no strong opinion, so no remark.
        for item in [0x02, 0x03, 0x05, 0x06, 0x08] {
            let f = OracleFacts {
                koopaling_hits: VANILLA_KOOPALING_HITS,
                world_progression: None,
                one_f_chest: Some(OneFChest { world: 0, item }),
            };
            assert!(topic_one_f_chest(&f).is_none(), "item {item:#04X} must not draw a verdict");
        }
    }

    /// **The world he names is the world the level is in.**
    ///
    /// The 1-F remark is the one line in the game that sends a player somewhere
    /// specific, and the world it names comes from the writer's report rather
    /// than from anything the console reads. This checks that report against
    /// the finished ROM the other way round: find the fortress level in the
    /// pointer tables, then read what the king actually said about it.
    ///
    /// A wrong answer here is silent everywhere else — the ROM boots, the king
    /// speaks, and the player walks to the wrong map.
    ///
    /// Which king is speaking is read back out of the ROM as well: the oracle
    /// is the king of the starting world, so the slot is the world the game
    /// boots into (`WORLD_INIT_OPERAND`), not anything this module computed.
    /// Run with world order on too, since that is what moves him off Grass Land.
    #[test]
    fn the_king_names_the_world_1f_is_really_in() {
        use crate::randomize::rom_data;
        use crate::randomize::world_order::WORLD_INIT_OPERAND;
        use crate::randomizer::Options;

        let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };

        // One encoded 20-column line, which is how the quote is stored.
        let line = |s: &str| encode_quote(&[s, "", "", "", "", ""])[..20].to_vec();
        let opener = line("A fortress hides a");

        let shuffled = Options { world_order: true, ..Options::default() };
        let (mut spoke, mut spoke_elsewhere) = (0, 0);
        for options in [Options::default(), shuffled] {
            for seed in 0..24u64 {
                let out = crate::generate_patched_rom(&bytes, seed, &options, None)
                    .expect("a randomize succeeds");
                let rom = Rom::from_bytes_lax(&out, true).expect("the output parses");

                let oracle_at = usize::from(rom.read_byte(WORLD_INIT_OPERAND));
                let quote = rom.read_range(KING_QUOTE_BASE + oracle_at * 120, 120).to_vec();
                if quote[..20] != opener[..] {
                    continue; // he talked about something else this seed
                }
                spoke += 1;
                if oracle_at != 0 {
                    spoke_elsewhere += 1;
                }

                // Where 1-F actually ended up, read back out of the pointer tables.
                let world = (0..8)
                    .find(|&wi| {
                        let wt = &rom_data::WORLDS[wi];
                        (0..wt.entry_count).any(|idx| {
                            let e = rom_data::read_entry(&rom, wt, idx);
                            u16::from_le_bytes([e.obj_lo, e.obj_hi])
                                == rom_data::FORTRESS_1F_OBJ_PTR
                        })
                    })
                    .unwrap_or_else(|| {
                        panic!("seed {seed}: he named a fortress that is not placed")
                    });

                assert_eq!(
                    quote[40..60],
                    line(&format!("in {}.", WORLD_NAMES[world]))[..],
                    "seed {seed}: the king sent the player to the wrong world (1-F is in {})",
                    WORLD_NAMES[world]
                );
            }
        }
        assert!(spoke > 0, "no seed reached the 1-F remark, so this test proved nothing");
        assert!(
            spoke_elsewhere > 0,
            "the oracle never spoke from a king other than Grass Land's, so world order's \
             starting world was never exercised"
        );
    }

    #[test]
    fn encode_round_trip() {
        let lines = ["Hello,world!", "Test line two.", "", "Line four here.", "", "The end."];
        let encoded = encode_quote(&lines);
        assert_eq!(encoded.len(), 120);
        // First char 'H' should be 0xB7
        assert_eq!(encoded[0], 0xB7);
        // Space padding at end of short lines
        assert_eq!(encoded[19], 0xFE);
    }

    /// The `enabled` gate must not change how much RNG the module consumes.
    ///
    /// This is the same property `king_quotes_off_only_skips_its_own_writes`
    /// checks end-to-end, asserted at the module boundary instead. The two are
    /// not redundant: the ROM-level test can only observe a divergence where
    /// something downstream actually *draws*, so on a configuration that
    /// happens to draw nothing after this point it would pass while the module
    /// was quietly consuming a different number of values. This one compares
    /// the generator state itself and holds regardless.
    #[test]
    fn enabled_gate_does_not_change_rng_consumption() {
        use rand::{RngCore, SeedableRng};

        let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };

        // Several seeds: choose_multiple picks its sampling strategy from the
        // pool sizes, not the seed, but the draw counts are worth checking on
        // more than one stream.
        for seed in 0..16u64 {
            let mut rom_on = Rom::from_bytes(&bytes).expect("test ROM parses");
            let mut rng_on = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_on, &mut rng_on, true, &facts(VANILLA_KOOPALING_HITS));

            let mut rom_off = Rom::from_bytes(&bytes).expect("test ROM parses");
            let mut rng_off = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_off, &mut rng_off, false, &facts(VANILLA_KOOPALING_HITS));

            // Identical position in the stream => identical continuations.
            let tail_on: Vec<u64> = (0..8).map(|_| rng_on.next_u64()).collect();
            let tail_off: Vec<u64> = (0..8).map(|_| rng_off.next_u64()).collect();
            assert_eq!(
                tail_on, tail_off,
                "seed {seed}: the rng is at a different position with quotes off — a draw \
                 moved below the early return, so every later module shifts"
            );
        }
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
