//! What a hit block hands out: Suit Blocks and Poison Mushroom, through one
//! shared hook on the block-spawn sites.
//!
//! Both features answer the same question — *which* thing does this particular
//! block give — and they answer it the same way: a hash of where the block is,
//! salted per seed. So they share one routine, generated from the flags that
//! are on, instead of each hooking the spawn sites itself.
//!
//! ## Suit blocks
//!
//! A suit block hands out a colour-cycling mushroom whose contents show only
//! when it is taken. Suit Blocks has two modes, and they are **not** a ladder:
//! On's suit block can be a star and Wild's cannot.
//!
//! | | On | Wild |
//! |---|---|---|
//! | the suit block gives | star, Tanooki, Frog, Hammer (`AND #3`) | Fire ×2, Leaf ×2, Tanooki, Frog, Hammer, and one suit again — picked per seed (`AND #7`) |
//! | small Mario gets | the suit block's pick | a plain mushroom (unless Modern Power-ups) |
//! | ? blocks | flower, leaf or suit block (`$65`) | suit block or star, 5 to 1 |
//! | bricks, notes, wood | their *star* tile is the suit block | their *flower* tile is the suit block; star tiles stay stars |
//! | the star | inside the suit block, looks like a star | its own tile, looks like a star |
//!
//! Wild takes the star out of the suit block because a star never looked like
//! a mystery anyway, and because it makes every no-star rule (the blocks that
//! must make small Mario big, the airship rule) a choice [`super::powerups`]
//! makes per block: never roll the star tile there.
//!
//! The suit block's tiles:
//!
//! - **? blocks on tile `$65`**, "? block with coin (again)", a duplicate of
//!   the coin block that only three vanilla blocks use. With the flag on,
//!   [`super::powerups`] moves those three to the coin-or-star block and
//!   places `$65` wherever it wants a suit. The plain ? star (`$62`) is left
//!   alone, so 7-7's stars stay stars.
//! - **One brick, note and wood tile each** ([`LATP_SUIT_TILES`] in On,
//!   [`LATP_WILD_TILES`] in Wild). These families have no spare tile to mark a
//!   suit with, so one of their power-up tiles becomes the suit block: the
//!   star in On (a suit block can still be a star, so they lose nothing), the
//!   flower in Wild (every block is a suit block, so the flower is not
//!   missed, and the star stays a real star).
//!
//! **Block → spawn.** A hit tile's item code comes from `LATP_PowerUps`
//! (PRG008), and a jump table turns the code into the index the bouncing block
//! carries into `Bouncer_PUp` (PRG001). The suit tiles' code becomes `B`, whose
//! vanilla handler only served the unused desert pipeworks tile; its bytes are
//! rewritten as [`SUIT_HANDLER`], which returns index 8. That is one past the
//! end of `Bouncer_PUp`, so it reads the first byte of the table after it — a
//! `$03` no vanilla block can produce. Seeing `$03` at the spawn site is the
//! whole signal; nothing global (like the star's colour-flash byte) is involved,
//! so two blocks hit back to back cannot confuse each other.
//!
//! **Spawn → suit.** The hook hashes the block and swaps the `$03` for the
//! pool's pick: the Starman, or one of the mystery objects on unused group-0
//! IDs ([`SUIT_IDS`], and in Wild [`WILD_IDS`]). They are Starmen in
//! everything but the picture and one stub: they start and move with the
//! star's own handlers, colour cycle included, draw the mushroom's tiles, and
//! on touch jump into the star's existing suit-granting code with their
//! `Player_Suit` value. That code queues any suit, so Fire and Leaf go through
//! it too (with the suit "poof", not the flower's flicker). The suit art itself
//! lives only in sprite page `$4C`, which ordinary levels never load — hence a
//! colour-cycling mushroom rather than the real picture. They look alike on
//! purpose: what it is shows only when it is taken.
//!
//! **Small Mario, Wild.** Before hashing, a suit block hit by small Mario
//! spawns a plain mushroom, as vanilla's flower and leaf blocks do. The suit
//! handler has set `PUp_StarManFlash` to `$80` for the colour cycle, and the
//! mushroom reads that byte as "came from a Big ? box" (it would pop out
//! sideways, unmasked), so the check clears it first — `ASL` turns `$80` into
//! `$00` in three bytes. With Modern Power-ups on, small Mario gets the pick
//! itself, the way that option gives him flowers.
//!
//! ## Poison mushrooms
//!
//! A 1-Up block keeps the real 1-Up or swaps in the poison object `$0A`
//! (installed by [`super::poison_mushroom`]) on bit 0 of the same hash: about
//! half of them, fixed per seed.
//!
//! ## The hash
//!
//! Both PRG001 sites that commit a hit block's object (`$A587` / `$AADD`,
//! `STA Level_ObjectID,Y`) are patched to `JSR` the routine. On entry `A` and
//! `Temp_Var1` hold the object ID, `X` is the bouncing block (its world
//! position is in `Objects_X/XHi/Y/YHi,X`) and `Y` is 5, the slot every block
//! power-up goes into. The routine replays the store, and for a watched ID
//! hashes:
//!
//! ```text
//! (Map_Entered_X + Map_Entered_Y + block X + block Y) >> 4
//!     + block XHi + block YHi + salt
//! ```
//!
//! The map entry is the overworld tile the level was entered from, per player,
//! so two Deja Vu copies of a level hand out different things. Block and map
//! coordinates are multiples of 16 — their low nibbles are zero — so they are
//! summed first and shifted down before the screen numbers and salt join.
//! `salt` is a seed-derived byte baked in as an immediate.
//!
//! ## Scope
//!
//! Only blocks are affected. Big `?` boxes spawn their suits from their own
//! code and are untouched. 7-7's four fixed stars are ? stars, which keep the
//! plain star, and its chain stars come from the coin-or-star blocks (item
//! code 5), which this never touches.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use super::poison_mushroom::{
    ATTR1_TABLE, ATTR2_TABLE, ATTR3_TABLE, HIT_TABLE, INIT_TABLE, KILL_TABLE, NORM_TABLE,
    PATSTART_TABLE, PATTABLE_SEL_TABLE,
};
use crate::SuitBlocks;
use crate::randomize::rom_data::{BLOCK_SPAWN_CPU, FS_BLOCK_SPAWN, FS_SUIT_HANDLER};
use crate::rom::Rom;

/// Salt for the one byte this module draws from the seed.
const BLOCK_SPAWN_SALT: u64 = 0x424C_4B5F_5350_4E31; // "BLK_SPN1"
/// Salt for Wild's extra suit, on its own substream so the hash salt above is
/// the same byte with or without Wild.
const BONUS_SUIT_SALT: u64 = 0x424C_4B5F_424F_4E31; // "BLK_BON1"

/// The vanilla `STA Level_ObjectID,Y` at both block-spawn sites.
const SPAWN_STORE: [u8; 3] = [0x99, 0x71, 0x06];
/// File offsets of the two block-spawn sites (PRG001 `$A587` / `$AADD`). Both
/// resolve the block's object through `Bouncer_PUp`.
const SPAWN_SITES: [usize; 2] = [0x02597, 0x02AED];

const ONE_UP_ID: u8 = 0x0B;
const POISON_ID: u8 = 0x0A;
const STARMAN_ID: u8 = 0x0C;

/// What a suit block spawns: `Bouncer_PUp[8]`, one past the table, which is
/// the first byte of `Bounce_TileReplacements` (`CHNGTILE_TOGNOTEBLOCK`).
const SUIT_BLOCK_ID: u8 = 0x03;
/// `Bouncer_PUp` + 8, file offset; pinned so a change to it is noticed.
const SUIT_BLOCK_ID_AT: usize = 0x02504;

/// A mystery object: an unused group-0 ID and the `Player_Suit` value its
/// touch grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mystery {
    id: u8,
    suit: u8,
}

/// The suits, Tanooki / Frog / Hammer, in both modes. Unused group-0 IDs whose
/// PRG000 `Object_AttrFlags` already match the Starman's (bound box 1,
/// fire-immune, hit-not-kill).
const SUIT_IDS: [Mystery; 3] =
    [Mystery { id: 0x01, suit: 5 }, Mystery { id: 0x02, suit: 4 }, Mystery { id: 0x1A, suit: 6 }];
/// Wild's Fire Flower and Super Leaf. `$0F` and `$10` are `ObjInit_DoNothing`
/// slots no level places and no engine code spawns; their PRG000 attribute
/// byte is `$00`, so the install copies the Starman's over it.
const WILD_IDS: [Mystery; 2] = [Mystery { id: 0x0F, suit: 2 }, Mystery { id: 0x10, suit: 3 }];

/// PRG000 `Object_AttrFlags` (`$82F4`), a byte per object ID.
const ATTR_FLAGS_TABLE: usize = 0x00304;

/// `Level_ObjectID` + 5: the spawned power-up's ID, swapped after the store.
const SLOT5_OBJECT_ID: [u8; 2] = [0x76, 0x06];
const POWERUP_SLOT: u8 = 5;
/// `Player_Suit` (zero page): 0 is small.
const PLAYER_SUIT: u8 = 0xED;
/// `PUp_StarManFlash`, which [`SUIT_HANDLER`] sets to `$80`.
const STAR_FLASH: [u8; 2] = [0x86, 0x05];

/// `ObjInit_StarOrSuit`.
const STAR_INIT_CPU: u16 = 0xA7A9;
/// `ObjNorm_StarOrSuit`: colour-cycles while `PUp_StarManFlash` is negative,
/// which the suit handler sets, then raises, moves, bounces and draws.
const STAR_NORM_CPU: u16 = 0xA7D6;
/// `ObjHit_StarOrSuit`.
const STAR_HIT_CPU: u16 = 0xA7FE;
/// Inside the star's suit half, `PRG001_A818` + 4: past its `TAY` / `LDA
/// PUp_StarOrSuitFrames,Y`, at `CMP <Player_Suit` with the suit value in `A`.
/// From here on it is the "already wearing it?" test, the queue, the poof and
/// the collect — for any `Player_Suit` value, not just the table's three.
const STAR_SUIT_GRANT_CPU: u16 = 0xA81C;
/// The two bytes there, `CMP <Player_Suit`.
const STAR_SUIT_GRANT: [u8; 2] = [0xC5, PLAYER_SUIT];
/// `OBJ_POWERUP_MUSHROOM`, whose tiles the mystery objects draw, and what
/// small Mario gets from a Wild suit block.
const MUSHROOM_ID: u8 = 0x0D;

/// On's suit block, by hash (`AND #3`): the Starman, then the suits.
const SUIT_POOL: [u8; 4] = [STARMAN_ID, SUIT_IDS[0].id, SUIT_IDS[1].id, SUIT_IDS[2].id];

/// Wild's suit block, by hash (`AND #7`): Fire and Leaf twice, each suit once,
/// and the last slot a suit again — which one is the seed's pick.
fn wild_pool(bonus: u8) -> [u8; 8] {
    let [fire, leaf] = WILD_IDS.map(|m| m.id);
    let [tanooki, frog, hammer] = SUIT_IDS.map(|m| m.id);
    [fire, fire, leaf, leaf, tanooki, frog, hammer, bonus]
}

/// PRG008 `LATP_PowerUps` entries that become item code `B` in On, each with
/// its vanilla code: `$65` (coin, 4), and the brick, note and wood stars (3).
const LATP_SUIT_TILES: [(usize, u8); 4] = [
    (0x11692, 0x04), // TILEA_QBLOCKCOIN2      $65
    (0x11697, 0x03), // TILEA_BRICKSTAR        $6A
    (0x11688, 0x03), // TILEA_NOTESTAR         $31
    (0x1168C, 0x03), // TILEA_WOODBLOCKSTAR    $75
];
/// The same for Wild: `$65`, and the brick, note and wood *flowers* (1).
const LATP_WILD_TILES: [(usize, u8); 4] = [
    (0x11692, 0x04), // TILEA_QBLOCKCOIN2      $65
    (0x11695, 0x01), // TILEA_BRICKFLOWER      $68
    (0x11686, 0x01), // TILEA_NOTEFLOWER       $2F
    (0x1168A, 0x01), // TILEA_WOODBLOCKFLOWER  $73
];
const LATP_SUIT: u8 = 0x0B;

/// `LATP_BrickAltClear` ($B90D), code `B`'s handler, as vanilla has it. Only
/// the unused desert pipeworks junction tile (`TILE9_PIPEWORKS_JCT`) reaches
/// it.
const BRICK_ALT_CLEAR: [u8; 10] = [0x20, 0x40, 0xB8, 0xF0, 0x04, 0xA9, 0x0C, 0x85, 0x0B, 0x60];
/// Code `B`'s handler for a suit block: `LATP_Star` with the marker index. The
/// flash is set to `$80` the way `LATP_Star` sets it, so whatever the block
/// comes out as colour-cycles like any star, and `$80 & 3` is frame 0, so a
/// Big `?` box's leftover value can never reach `ObjInit_StarOrSuit`'s frame
/// pick.
#[rustfmt::skip]
const SUIT_HANDLER: [u8; 8] = [
    0xA9, 0x80,             // LDA #$80
    0x8D, 0x86, 0x05,       // STA PUp_StarManFlash
    0xA0, 0x08,             // LDY #8   (Bouncer_PUp index: the $03 marker)
    0x60,                   // RTS
];

/// Which block contents the spawn routine has to handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SpawnFlags {
    pub(crate) poison_mushrooms: bool,
    pub(crate) suits: SuitBlocks,
    /// Modern Power-ups: small Mario skips the mushroom. Only Wild asks.
    pub(crate) modern_powerups: bool,
}

impl SpawnFlags {
    fn any(&self) -> bool {
        self.poison_mushrooms || self.suits.any()
    }

    fn wild(&self) -> bool {
        self.suits == SuitBlocks::Wild
    }

    /// Whether small Mario gets a plain mushroom before the hash.
    fn small_mushroom(&self) -> bool {
        self.wild() && !self.modern_powerups
    }

    /// The mystery objects this mode installs, in Hit-entry order.
    fn mysteries(&self) -> Vec<Mystery> {
        let mut out = Vec::new();
        if self.suits.any() {
            out.extend(SUIT_IDS);
        }
        if self.wild() {
            out.extend(WILD_IDS);
        }
        out
    }
}

/// What the seed decides for the routine: the hash salt, and which suit fills
/// Wild's eighth slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Seeded {
    pub(crate) salt: u8,
    /// An ID from [`SUIT_IDS`].
    pub(crate) bonus_suit: u8,
}

impl Seeded {
    /// Each from its own substream so the main stream never sees the draws.
    pub(crate) fn from_seed(seed: u64) -> Self {
        let salt = ChaCha8Rng::seed_from_u64(seed ^ BLOCK_SPAWN_SALT).random();
        let pick: usize = ChaCha8Rng::seed_from_u64(seed ^ BONUS_SUIT_SALT).random_range(..3);
        Self { salt, bonus_suit: SUIT_IDS[pick].id }
    }
}

/// Install the spawn routine and, for suit blocks, the mystery objects and the
/// suit tiles. Does nothing when no flag is on.
/// [`super::poison_mushroom::apply`] installs the poison object itself; this
/// only decides when to hand it out.
pub(crate) fn apply(rom: &mut Rom, flags: SpawnFlags, seeded: Seeded) {
    if !flags.any() {
        return;
    }
    let routine = assemble(flags, seeded);
    rom.write_range(FS_BLOCK_SPAWN, &routine.code);

    let jsr = [0x20, BLOCK_SPAWN_CPU as u8, (BLOCK_SPAWN_CPU >> 8) as u8];
    for &site in &SPAWN_SITES {
        debug_assert_eq!(rom.read_range(site, 3), SPAWN_STORE);
        rom.write_range(site, &jsr);
    }
    if flags.suits.any() {
        install_suits(rom, flags, &routine);
    }
}

/// The mystery objects' table slots, and the suit tiles' item code.
fn install_suits(rom: &mut Rom, flags: SpawnFlags, routine: &Routine) {
    let star = usize::from(STARMAN_ID);
    debug_assert_eq!(rom.read_byte(SUIT_BLOCK_ID_AT), SUIT_BLOCK_ID);
    debug_assert_eq!(rom.read_range(INIT_TABLE + star * 2, 2), STAR_INIT_CPU.to_le_bytes());
    debug_assert_eq!(rom.read_range(NORM_TABLE + star * 2, 2), STAR_NORM_CPU.to_le_bytes());
    debug_assert_eq!(rom.read_range(HIT_TABLE + star * 2, 2), STAR_HIT_CPU.to_le_bytes());
    debug_assert_eq!(rom.read_range(0x02010 + 0x081C, 2), STAR_SUIT_GRANT);

    for (i, m) in flags.mysteries().into_iter().enumerate() {
        let id = usize::from(m.id);
        let hit = routine.hit_cpu + 3 * i as u16;
        rom.write_range(INIT_TABLE + id * 2, &STAR_INIT_CPU.to_le_bytes());
        rom.write_range(NORM_TABLE + id * 2, &STAR_NORM_CPU.to_le_bytes());
        rom.write_range(HIT_TABLE + id * 2, &hit.to_le_bytes());
        for table in [ATTR1_TABLE, ATTR2_TABLE, ATTR3_TABLE, PATTABLE_SEL_TABLE, KILL_TABLE] {
            rom.write_byte(table + id, rom.read_byte(table + star));
        }
        // The picture: the mushroom's tiles, from the same always-loaded page.
        let mushroom = usize::from(MUSHROOM_ID);
        rom.write_byte(PATSTART_TABLE + id, rom.read_byte(PATSTART_TABLE + mushroom));
        // PRG000's attribute byte: already the Starman's for the suits, `$00`
        // for Wild's two.
        let attr = rom.read_byte(ATTR_FLAGS_TABLE + star);
        debug_assert!([0x00, attr].contains(&rom.read_byte(ATTR_FLAGS_TABLE + id)));
        rom.write_byte(ATTR_FLAGS_TABLE + id, attr);
    }

    let tiles = if flags.wild() { LATP_WILD_TILES } else { LATP_SUIT_TILES };
    for (offset, vanilla) in tiles {
        debug_assert_eq!(rom.read_byte(offset), vanilla, "LATP 0x{offset:05X}");
        rom.write_byte(offset, LATP_SUIT);
    }
    debug_assert_eq!(rom.read_range(FS_SUIT_HANDLER, BRICK_ALT_CLEAR.len()), BRICK_ALT_CLEAR);
    rom.write_range(FS_SUIT_HANDLER, &SUIT_HANDLER);
}

/// PRG001 CPU address to file offset.
#[cfg(test)]
fn cpu_to_file(cpu: u16) -> usize {
    0x02010 + usize::from(cpu - 0xA000)
}

/// Branch targets inside the routine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Label {
    Hash,
    Store,
    Poison,
    Done,
    Hit,
    Pool,
}

/// A two-pass assembler just big enough for [`assemble`]: bytes, labels,
/// relative branches and one absolute operand. The routine has three shapes
/// depending on the flags, and hand-counting branch offsets for each is exactly
/// the error `asm::check` exists to catch — better not to make it.
#[derive(Default)]
struct Asm {
    code: Vec<u8>,
    labels: Vec<(Label, usize)>,
    branches: Vec<(usize, Label)>,
    absolutes: Vec<(usize, Label)>,
}

impl Asm {
    fn op(&mut self, bytes: &[u8]) {
        self.code.extend_from_slice(bytes);
    }

    fn label(&mut self, label: Label) {
        self.labels.push((label, self.code.len()));
    }

    fn branch(&mut self, opcode: u8, to: Label) {
        self.code.push(opcode);
        self.branches.push((self.code.len(), to));
        self.code.push(0);
    }

    fn absolute_y(&mut self, opcode: u8, to: Label) {
        self.code.push(opcode);
        self.absolutes.push((self.code.len(), to));
        self.code.extend_from_slice(&[0, 0]);
    }

    fn at(&self, label: Label) -> usize {
        self.labels.iter().find(|(l, _)| *l == label).map(|&(_, at)| at).unwrap_or_else(|| {
            panic!("block_spawn: branch to {label:?}, which this shape does not emit")
        })
    }

    fn cpu(&self, origin: u16, label: Label) -> u16 {
        origin + self.at(label) as u16
    }

    fn finish(mut self, origin: u16) -> Vec<u8> {
        for &(operand, to) in &self.branches {
            let offset = self.at(to) as isize - (operand as isize + 1);
            let offset = i8::try_from(offset).expect("block_spawn: branch out of range");
            self.code[operand] = offset as u8;
        }
        for &(operand, to) in &self.absolutes {
            let cpu = origin + self.at(to) as u16;
            self.code[operand..operand + 2].copy_from_slice(&cpu.to_le_bytes());
        }
        self.code
    }
}

/// The assembled routine and the entry points the object tables need.
struct Routine {
    code: Vec<u8>,
    /// The first mystery object's Hit entry; each next one is 3 bytes on, in
    /// [`SpawnFlags::mysteries`] order.
    hit_cpu: u16,
}

/// Build the spawn routine for `flags`. See the module doc for what it does;
/// the comments below are the instruction listing.
fn assemble(flags: SpawnFlags, seeded: Seeded) -> Routine {
    assert!(flags.any(), "block_spawn: nothing to watch");
    let suits = flags.suits.any();
    let both = flags.poison_mushrooms && suits;

    let mut a = Asm::default();
    a.op(&SPAWN_STORE); //                     STA Level_ObjectID,Y  (the store we displaced)

    // Only the watched IDs go on; anything else returns untouched.
    if both {
        a.op(&[0xC9, ONE_UP_ID]); //           CMP #$0B
        a.branch(0xF0, Label::Hash); //        BEQ hash
    }
    let last = if suits { SUIT_BLOCK_ID } else { ONE_UP_ID };
    a.op(&[0xC9, last]); //                    CMP #id
    a.branch(0xD0, Label::Done); //            BNE done

    if flags.small_mushroom() {
        // A suit block, and small Mario: a plain mushroom, flash cleared.
        a.op(&[0xA5, PLAYER_SUIT]); //         LDA <Player_Suit
        a.branch(0xD0, Label::Hash); //        BNE hash   (big: the pool)
        a.op(&[0x0E, STAR_FLASH[0], STAR_FLASH[1]]); // ASL PUp_StarManFlash ($80 -> 0)
        a.op(&[0xA9, MUSHROOM_ID]); //         LDA #$0D
        a.branch(0xD0, Label::Store); //       BNE store  (always: $0D)
    }

    a.label(Label::Hash);
    a.op(&[0xAC, 0x26, 0x07]); //              LDY Player_Current
    a.op(&[0xB9, 0x7A, 0x79]); //              LDA Map_Entered_X,Y
    a.op(&[0x18]); //                          CLC
    a.op(&[0x79, 0x76, 0x79]); //              ADC Map_Entered_Y,Y
    a.op(&[0x75, 0x91]); //                    ADC Objects_X,X      (block)
    a.op(&[0x75, 0xA3]); //                    ADC Objects_Y,X
    a.op(&[0x4A, 0x4A, 0x4A, 0x4A]); //        LSR x4  (fold the 16-px grid down)
    a.op(&[0x75, 0x76]); //                    ADC Objects_XHi,X
    a.op(&[0x75, 0x88]); //                    ADC Objects_YHi,X
    a.op(&[0x69, seeded.salt]); //             ADC #salt

    if both {
        // Which kind: the object ID is still in Temp_Var1.
        a.op(&[0xA4, 0x00]); //                LDY Temp_Var1
        a.op(&[0xC0, ONE_UP_ID]); //           CPY #$0B
        a.branch(0xF0, Label::Poison); //      BEQ poison
    }
    if suits {
        let mask = if flags.wild() { 0x07 } else { 0x03 };
        a.op(&[0x29, mask]); //                AND #3 (On) / #7 (Wild)
        a.op(&[0xA8]); //                      TAY
        a.absolute_y(0xB9, Label::Pool); //    LDA pool,Y
        a.label(Label::Store);
        a.op(&[0x8D, SLOT5_OBJECT_ID[0], SLOT5_OBJECT_ID[1]]); // STA Level_ObjectID+5
        if flags.poison_mushrooms {
            a.branch(0xD0, Label::Done); //    BNE done   (always: no ID is 0)
        }
    }
    if flags.poison_mushrooms {
        a.label(Label::Poison);
        a.op(&[0x29, 0x01]); //                AND #1
        a.branch(0xF0, Label::Done); //        BEQ done   (even: keep the 1-Up)
        a.op(&[0xA9, POISON_ID]); //           LDA #$0A
        a.op(&[0x8D, SLOT5_OBJECT_ID[0], SLOT5_OBJECT_ID[1]]); // STA Level_ObjectID+5
    }
    a.label(Label::Done);
    a.op(&[0xA0, POWERUP_SLOT]); //            LDY #5     (the caller still needs it)
    a.op(&[0x60]); //                          RTS

    if suits {
        // Mystery objects' Hit: one entry per object, 3 bytes apart. Each
        // `BIT abs` swallows the next entry's `LDA #` as its operand, so every
        // entry falls through to the JMP holding its own Player_Suit value.
        let mysteries = flags.mysteries();
        a.label(Label::Hit);
        for (i, m) in mysteries.iter().enumerate() {
            a.op(&[0xA9, m.suit]); //          LDA #suit
            if i + 1 < mysteries.len() {
                a.op(&[0x2C]); //              BIT $..A9  (skip the next LDA)
            }
        }
        let [lo, hi] = STAR_SUIT_GRANT_CPU.to_le_bytes();
        a.op(&[0x4C, lo, hi]); //              JMP PRG001_A818+4 (the star's suit grant)

        a.label(Label::Pool);
        if flags.wild() {
            a.op(&wild_pool(seeded.bonus_suit));
        } else {
            a.op(&SUIT_POOL);
        }
    }
    let hit_cpu = if suits { a.cpu(BLOCK_SPAWN_CPU, Label::Hit) } else { 0 };
    Routine { code: a.finish(BLOCK_SPAWN_CPU), hit_cpu }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::rom_data::FREE_SPACE_ALLOCATIONS;

    const SEEDED: Seeded = Seeded { salt: 0x5A, bonus_suit: 0x02 };

    /// Every flag combination that installs anything.
    pub(super) fn every_shape() -> Vec<SpawnFlags> {
        let mut out = Vec::new();
        for poison_mushrooms in [false, true] {
            for suits in [SuitBlocks::Off, SuitBlocks::On, SuitBlocks::Wild] {
                for modern_powerups in [false, true] {
                    let f = SpawnFlags { poison_mushrooms, suits, modern_powerups };
                    if f.any() {
                        out.push(f);
                    }
                }
            }
        }
        out
    }

    fn cpu() -> mos6502::cpu::CPU<mos6502::memory::Memory, mos6502::instruction::Ricoh2a03> {
        mos6502::cpu::CPU::new(mos6502::memory::Memory::new(), mos6502::instruction::Ricoh2a03)
    }

    /// One spawn's result.
    #[derive(Debug)]
    struct Spawned {
        /// `Level_ObjectID[5]`.
        object: u8,
        y: u8,
        x: u8,
        /// `PUp_StarManFlash` afterwards; it went in as `$80`.
        flash: u8,
    }

    /// Run the routine on an emulated CPU for one spawn: the ID it was asked
    /// about, the block's position, the player's map entry and `Player_Suit`.
    fn run(code: &[u8], id: u8, block: [u8; 4], entry: (u8, u8), suit: u8) -> Spawned {
        use mos6502::memory::Bus;

        let mut cpu = cpu();
        let origin = BLOCK_SPAWN_CPU;
        cpu.memory.set_bytes(origin, code);
        // JSR origin at $8000; we stop at the return.
        cpu.memory.set_bytes(0x8000, &[0x20, origin as u8, (origin >> 8) as u8]);
        let x = 2u8; // the bouncing block's slot
        let [bx, bxhi, by, byhi] = block;
        cpu.memory.set_byte(0x91 + x as u16, bx);
        cpu.memory.set_byte(0x76 + x as u16, bxhi);
        cpu.memory.set_byte(0xA3 + x as u16, by);
        cpu.memory.set_byte(0x88 + x as u16, byhi);
        cpu.memory.set_byte(0x0726, 0); // Player_Current
        cpu.memory.set_byte(0x797A, entry.0);
        cpu.memory.set_byte(0x7976, entry.1);
        cpu.memory.set_byte(0x0000, id); // Temp_Var1
        cpu.memory.set_byte(u16::from(PLAYER_SUIT), suit);
        cpu.memory.set_byte(0x0586, 0x80); // PUp_StarManFlash, as the handler leaves it
        cpu.registers.program_counter = 0x8000;
        cpu.registers.stack_pointer = mos6502::registers::StackPointer(0xFF);
        cpu.registers.accumulator = id;
        cpu.registers.index_x = x;
        cpu.registers.index_y = POWERUP_SLOT;
        for _ in 0..2000 {
            cpu.single_step();
            if cpu.registers.program_counter == 0x8003 {
                return Spawned {
                    object: cpu.memory.get_byte(0x0671 + POWERUP_SLOT as u16),
                    y: cpu.registers.index_y,
                    x: cpu.registers.index_x,
                    flash: cpu.memory.get_byte(0x0586),
                };
            }
        }
        panic!("block_spawn: routine never returned");
    }

    /// Big Mario (`Player_Suit` 1).
    const BIG: u8 = 1;

    /// Where the routine's pool table starts: it is the tail of the code.
    pub(super) fn pool_at(flags: SpawnFlags, code: &[u8]) -> usize {
        code.len()
            - if flags.wild() {
                8
            } else if flags.suits.any() {
                SUIT_POOL.len()
            } else {
                0
            }
    }

    fn reservation(offset: usize) -> usize {
        FREE_SPACE_ALLOCATIONS.iter().find(|a| a.offset == offset).unwrap().size
    }

    #[test]
    fn every_shape_fits_its_reservation() {
        let biggest = every_shape().iter().map(|&f| assemble(f, SEEDED).code.len()).max().unwrap();
        let reserved = reservation(FS_BLOCK_SPAWN);
        assert!(biggest <= reserved, "{biggest} bytes > {reserved} reserved");
        assert!(SUIT_HANDLER.len() <= reservation(FS_SUIT_HANDLER));
    }

    /// What a suit block may spawn in `flags`' mode.
    fn suit_pool(flags: SpawnFlags) -> Vec<u8> {
        if flags.wild() { wild_pool(SEEDED.bonus_suit).to_vec() } else { SUIT_POOL.to_vec() }
    }

    #[test]
    fn every_shape_returns_with_x_and_y_intact_and_stores_the_id() {
        for flags in every_shape() {
            let code = assemble(flags, SEEDED).code;
            for suit in [0, BIG] {
                for id in [0x00, ONE_UP_ID, SUIT_BLOCK_ID, STARMAN_ID, 0x19, 0x20] {
                    let s = run(&code, id, [0x40, 1, 0x60, 0], (0x20, 0x30), suit);
                    let at = format!("{flags:?} id {id:02X} suit {suit}");
                    assert_eq!(s.y, POWERUP_SLOT, "{at}: Y");
                    assert_eq!(s.x, 2, "{at}: X");
                    let poisoned =
                        flags.poison_mushrooms && id == ONE_UP_ID && s.object == POISON_ID;
                    let suited = flags.suits.any()
                        && id == SUIT_BLOCK_ID
                        && (suit_pool(flags).contains(&s.object) || s.object == MUSHROOM_ID);
                    assert!(s.object == id || poisoned || suited, "{at}: stored {:02X}", s.object);
                    // The marker must never survive: it is not a real object.
                    if flags.suits.any() {
                        assert_ne!(s.object, SUIT_BLOCK_ID, "{at}: the $03 marker spawned");
                    }
                }
            }
        }
    }

    /// Wild, small Mario: a plain mushroom from every suit block, with the
    /// flash cleared so it rises out of the block like a vanilla mushroom.
    /// Big Mario, or Modern Power-ups, or On: never a mushroom, flash kept.
    #[test]
    fn small_mario_gets_a_plain_mushroom_in_wild_only() {
        for flags in every_shape().into_iter().filter(|f| f.suits.any()) {
            let code = assemble(flags, SEEDED).code;
            for col in 0..16u8 {
                let block = [col * 16, 3, 0x50, 0];
                let small = run(&code, SUIT_BLOCK_ID, block, (0x20, 0x30), 0);
                let big = run(&code, SUIT_BLOCK_ID, block, (0x20, 0x30), BIG);
                assert_ne!(big.object, MUSHROOM_ID, "{flags:?}: big Mario got a mushroom");
                assert_eq!(big.flash, 0x80, "{flags:?}: big Mario lost the colour cycle");
                if flags.small_mushroom() {
                    assert_eq!(small.object, MUSHROOM_ID, "{flags:?}");
                    assert_eq!(small.flash, 0x00, "{flags:?}: mushroom left flashing");
                } else {
                    assert_eq!(small.object, big.object, "{flags:?}: small differs from big");
                    assert_eq!(small.flash, 0x80, "{flags:?}");
                }
            }
        }
    }

    /// The poison path never touches the flash, whatever Mario's size.
    #[test]
    fn one_up_blocks_ignore_mario_size() {
        for flags in every_shape().into_iter().filter(|f| f.poison_mushrooms) {
            let code = assemble(flags, SEEDED).code;
            for col in 0..16u8 {
                let block = [col * 16, 2, 0x50, 0];
                let small = run(&code, ONE_UP_ID, block, (0x20, 0x30), 0);
                let big = run(&code, ONE_UP_ID, block, (0x20, 0x30), BIG);
                assert_eq!(small.object, big.object, "{flags:?}");
                assert_eq!(small.flash, 0x80, "{flags:?}");
            }
        }
    }

    /// Blocks on one screen differ, and every outcome of the mode turns up.
    #[test]
    fn suits_vary_by_block() {
        for flags in every_shape().into_iter().filter(|f| f.suits.any()) {
            let code = assemble(flags, SEEDED).code;
            let mut seen = std::collections::BTreeMap::<u8, usize>::new();
            for col in 0..16u8 {
                for row in 0..4u8 {
                    let block = [col * 16, 3, row * 16 + 0x40, 0];
                    let s = run(&code, SUIT_BLOCK_ID, block, (0x20, 0x30), BIG);
                    *seen.entry(s.object).or_default() += 1;
                }
            }
            let ids: Vec<u8> = seen.keys().copied().collect();
            let (expect, floor): (&[u8], usize) = if flags.wild() {
                (&[0x01, 0x02, 0x0F, 0x10, 0x1A], 4)
            } else {
                (&[0x01, 0x02, STARMAN_ID, 0x1A], 8)
            };
            assert_eq!(ids, expect, "{flags:?}: one screen, {seen:?}");
            assert!(seen.values().all(|&n| n >= floor), "{flags:?}: lopsided {seen:?}");
        }
    }

    /// Wild's pool: Fire and Leaf twice, each suit once, and the seed's extra
    /// suit last — and over seeds, each of the three is that extra.
    #[test]
    fn wild_pool_doubles_fire_and_leaf_and_one_suit_per_seed() {
        let flags =
            SpawnFlags { poison_mushrooms: false, suits: SuitBlocks::Wild, modern_powerups: false };
        let mut extras = std::collections::BTreeSet::new();
        for seed in 0..64 {
            let seeded = Seeded::from_seed(seed);
            let r = assemble(flags, seeded);
            let pool = &r.code[pool_at(flags, &r.code)..];
            assert_eq!(pool, wild_pool(seeded.bonus_suit), "seed {seed}");
            assert_eq!(pool[..7], [0x0F, 0x0F, 0x10, 0x10, 0x01, 0x02, 0x1A]);
            assert!(SUIT_IDS.iter().any(|m| m.id == pool[7]), "seed {seed}: extra {:02X}", pool[7]);
            extras.insert(pool[7]);
        }
        assert_eq!(extras.len(), 3, "the extra suit never varies: {extras:?}");
    }

    /// The hash salt is the same byte with or without Wild.
    #[test]
    fn the_salt_does_not_depend_on_the_mode() {
        let salt = |seed| ChaCha8Rng::seed_from_u64(seed ^ BLOCK_SPAWN_SALT).random::<u8>();
        for seed in 0..16 {
            assert_eq!(Seeded::from_seed(seed).salt, salt(seed));
        }
    }

    /// Deja Vu: the same block, entered from another map tile, can differ.
    #[test]
    fn the_map_entry_moves_the_pick() {
        for suits in [SuitBlocks::On, SuitBlocks::Wild] {
            let flags = SpawnFlags { poison_mushrooms: false, suits, modern_powerups: false };
            let code = assemble(flags, Seeded { salt: 0, bonus_suit: 0x01 }).code;
            let picks: std::collections::BTreeSet<u8> = (0..8u8)
                .map(|tile| {
                    run(&code, SUIT_BLOCK_ID, [0x40, 1, 0x60, 0], (tile * 16, 0x30), BIG).object
                })
                .collect();
            assert!(picks.len() > 1, "{suits:?}: eight map tiles, one suit: {picks:?}");
        }
    }

    #[test]
    fn poison_still_splits_one_up_blocks() {
        for flags in every_shape().into_iter().filter(|f| f.poison_mushrooms) {
            let code = assemble(flags, Seeded { salt: 0x11, bonus_suit: 0x01 }).code;
            let poisoned = (0..16u8)
                .filter(|&col| {
                    let s = run(&code, ONE_UP_ID, [col * 16, 2, 0x50, 0], (0x20, 0x30), BIG);
                    s.object == POISON_ID
                })
                .count();
            assert!((4..=12).contains(&poisoned), "{flags:?}: {poisoned}/16 poisoned");
        }
    }

    /// Each mystery object's Hit entry reaches the star's suit grant holding
    /// its own `Player_Suit` value: Tanooki 5, Frog 4, Hammer 6, then Wild's
    /// Fire 2 and Leaf 3.
    #[test]
    fn each_hit_entry_carries_its_suit() {
        use mos6502::memory::Bus;
        for flags in every_shape().into_iter().filter(|f| f.suits.any()) {
            let routine = assemble(flags, SEEDED);
            let mysteries = flags.mysteries();
            assert_eq!(mysteries.len(), if flags.wild() { 5 } else { 3 });
            for (i, m) in mysteries.iter().enumerate() {
                let mut cpu = cpu();
                cpu.memory.set_bytes(BLOCK_SPAWN_CPU, &routine.code);
                cpu.registers.program_counter = routine.hit_cpu + 3 * i as u16;
                for _ in 0..8 {
                    if cpu.registers.program_counter == STAR_SUIT_GRANT_CPU {
                        break;
                    }
                    cpu.single_step();
                }
                assert_eq!(cpu.registers.program_counter, STAR_SUIT_GRANT_CPU, "entry {i}");
                assert_eq!(cpu.registers.accumulator, m.suit, "{flags:?} entry {i}");
            }
        }
    }

    #[test]
    fn off_writes_nothing() {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        let mut rom = Rom::from_bytes_lax(&data, true).unwrap();
        let before = rom.read_range(0, 393232).to_vec();
        let off =
            SpawnFlags { poison_mushrooms: false, suits: SuitBlocks::Off, modern_powerups: false };
        apply(&mut rom, off, SEEDED);
        assert_eq!(rom.read_range(0, 393232), &before[..]);
    }

    /// On the real ROM: the mystery objects' table slots, the suit tiles' new
    /// item code and its handler. Also checks every vanilla byte the install
    /// asserts.
    #[test]
    fn install_wires_the_objects_and_the_block() {
        let Ok(data) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            return;
        };
        let vanilla = Rom::from_bytes(&data).unwrap();
        for suits in [SuitBlocks::On, SuitBlocks::Wild] {
            let mut rom = vanilla.clone();
            let flags = SpawnFlags { poison_mushrooms: false, suits, modern_powerups: false };
            apply(&mut rom, flags, SEEDED);
            let routine = assemble(flags, SEEDED);
            let star_attr = rom.read_byte(ATTR_FLAGS_TABLE + usize::from(STARMAN_ID));
            assert_eq!(star_attr, 0xC1, "Starman: bound box 1, fire-immune, hit-not-kill");
            for (i, m) in flags.mysteries().into_iter().enumerate() {
                let id = usize::from(m.id);
                let word = |t: usize| {
                    u16::from_le_bytes([rom.read_byte(t + id * 2), rom.read_byte(t + id * 2 + 1)])
                };
                assert_eq!(word(INIT_TABLE), STAR_INIT_CPU);
                assert_eq!(word(NORM_TABLE), STAR_NORM_CPU);
                assert_eq!(word(HIT_TABLE), routine.hit_cpu + 3 * i as u16);
                assert_eq!(rom.read_byte(PATTABLE_SEL_TABLE + id), 0x00, "no page request");
                let mushroom = usize::from(MUSHROOM_ID);
                let patstart = rom.read_byte(PATSTART_TABLE + mushroom);
                assert_eq!(rom.read_byte(PATSTART_TABLE + id), patstart, "mushroom tiles");
                assert_eq!(rom.read_byte(ATTR_FLAGS_TABLE + id), star_attr, "{id:02X} attr");
            }
            // Wild's two IDs are unused in vanilla: no init, no attributes.
            for m in WILD_IDS {
                assert_eq!(vanilla.read_byte(ATTR_FLAGS_TABLE + usize::from(m.id)), 0x00);
            }
            let (mine, other) = match suits {
                SuitBlocks::Wild => (LATP_WILD_TILES, LATP_SUIT_TILES),
                _ => (LATP_SUIT_TILES, LATP_WILD_TILES),
            };
            for (offset, _) in mine {
                assert_eq!(rom.read_byte(offset), LATP_SUIT, "{suits:?} LATP 0x{offset:05X}");
            }
            // The other mode's tiles keep their vanilla codes (the shared $65 aside).
            for (offset, code) in other.into_iter().filter(|t| !mine.contains(t)) {
                assert_eq!(rom.read_byte(offset), code, "{suits:?} LATP 0x{offset:05X}");
            }
            // The plain ? star keeps its code: 7-7's fixed stars depend on it.
            assert_eq!(rom.read_byte(0x1168F), 0x03, "TILEA_QBLOCKSTAR");
            assert_eq!(rom.read_range(FS_SUIT_HANDLER, SUIT_HANDLER.len()), SUIT_HANDLER);
            let grant = cpu_to_file(STAR_SUIT_GRANT_CPU);
            assert_eq!(rom.read_range(grant, 2), STAR_SUIT_GRANT);
        }
    }
}

#[cfg(test)]
mod asm_checks {
    use super::tests::{every_shape, pool_at};
    use super::*;
    use crate::randomize::rom_data::asm;

    const SPAWN_JSR: [u8; 3] = [0x20, BLOCK_SPAWN_CPU as u8, (BLOCK_SPAWN_CPU >> 8) as u8];

    #[test]
    fn every_spawn_routine_is_well_formed() {
        for flags in every_shape() {
            let routine = assemble(flags, Seeded { salt: 0xA5, bonus_suit: 0x1A });
            asm::check(&routine.code)
                .allocation(FS_BLOCK_SPAWN)
                .origin(BLOCK_SPAWN_CPU)
                .data_from(pool_at(flags, &routine.code))
                .hook(&SPAWN_STORE, 0, &SPAWN_JSR)
                // Temp_Var1, the engine's own object variables, Player_Suit.
                .zero_page(0x02, &[0x76, 0x88, 0x91, 0xA3, PLAYER_SUIT])
                .assert_ok();
        }
    }

    #[test]
    fn the_suit_handler_is_well_formed() {
        asm::check(&SUIT_HANDLER).allocation(FS_SUIT_HANDLER).assert_ok();
    }
}
