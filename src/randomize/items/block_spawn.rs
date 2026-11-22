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
//! A suit block gives a star, a Tanooki, a Frog or a Hammer Suit, picked per
//! block by the hash. Two kinds of block are suit blocks:
//!
//! - **? blocks on tile `$65`**, "? block with coin (again)", a duplicate of
//!   the coin block that only three vanilla blocks use. With the flag on,
//!   [`super::powerups`] moves those three to the coin-or-star block and
//!   places `$65` wherever it wants a suit. The plain ? star (`$62`) is left
//!   alone, so 7-7's stars stay stars.
//! - **Every brick, note and wood block star** ([`LATP_SUIT_TILES`]). These
//!   families have no spare tile to mark a suit with, so their star becomes
//!   the suit block instead — and because a suit block can still come out a
//!   star, they lose nothing.
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
//! Starman or one of three new objects on unused group-0 IDs ([`SUIT_IDS`]):
//! Tanooki, Frog and Hammer. They are Starmen in everything but the picture
//! and one stub: they start and move with the star's own handlers, colour
//! cycle included, draw the mushroom's tiles, and on touch jump into the
//! star's existing suit-granting code with that suit's index. The suit art
//! itself lives only in sprite page `$4C`, which ordinary levels never load —
//! hence a colour-cycling mushroom rather than the real picture. The three
//! look alike on purpose: which suit it is shows only when it is taken.
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
//! ## Under power-up gates
//!
//! With the maze's power-up gates ([`SpawnFlags::powerup_gates`]), a suit
//! block picks only from what the player has unlocked: `JSR pick`
//! ([`suit_pick`], [`FS_SUIT_PICK`]) replaces the `AND #3` table read. It walks
//! the pool, counting down `hash & 15` over the unlocked entries only, so the
//! pick is even across whatever is unlocked (within 1/16 when three are).
//! With nothing unlocked it lands on the pool's fifth entry, a 1-up.
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
use crate::randomize::maze::powerup_gate::{POWERUPS_FOUND, SUIT_POOL_BITS};
use crate::randomize::rom_data::{
    BLOCK_SPAWN_CPU, FS_BLOCK_SPAWN, FS_SUIT_HANDLER, FS_SUIT_PICK, prg_bank_file_to_cpu,
};
use crate::rom::Rom;

/// Salt for the one byte this module draws from the seed.
const BLOCK_SPAWN_SALT: u64 = 0x424C_4B5F_5350_4E31; // "BLK_SPN1"

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

/// The new objects, Tanooki / Frog / Hammer. Unused group-0 IDs whose PRG000
/// `Object_AttrFlags` already match the Starman's (bound box 1, fire-immune,
/// hit-not-kill), so only PRG001's tables need writing.
const SUIT_IDS: [u8; 3] = [0x01, 0x02, 0x1A];

/// `Level_ObjectID` + 5: the spawned power-up's ID, swapped after the store.
const SLOT5_OBJECT_ID: [u8; 2] = [0x76, 0x06];
const POWERUP_SLOT: u8 = 5;

/// `ObjInit_StarOrSuit`.
const STAR_INIT_CPU: u16 = 0xA7A9;
/// `ObjNorm_StarOrSuit`: colour-cycles while `PUp_StarManFlash` is negative,
/// which the suit handler sets, then raises, moves, bounces and draws.
const STAR_NORM_CPU: u16 = 0xA7D6;
/// `ObjHit_StarOrSuit`, and its suit half `PRG001_A818`: `TAY` / `LDA
/// PUp_StarOrSuitFrames,Y` — index 1/2/3 is Tanooki/Frog/Hammer — then the
/// "already wearing it?" test, the queue, the poof and the collect.
const STAR_HIT_CPU: u16 = 0xA7FE;
const STAR_SUIT_HIT_CPU: u16 = 0xA818;
/// `OBJ_POWERUP_MUSHROOM`, whose tiles the suit objects draw.
const MUSHROOM_ID: u8 = 0x0D;

/// What a suit block spawns, by hash (`AND #3`): the Starman, then the suits.
/// Under power-up gates a fifth entry follows: the 1-up, for nothing unlocked.
const SUIT_POOL: [u8; 4] = [STARMAN_ID, SUIT_IDS[0], SUIT_IDS[1], SUIT_IDS[2]];

/// Where [`suit_pick`] runs: PRG001, the spawn routine's own bank.
const SUIT_PICK_CPU: u16 = prg_bank_file_to_cpu(1, FS_SUIT_PICK);
const SUIT_PICK_LEN: usize = 38;

/// PRG008 `LATP_PowerUps` entries that become item code `B`, each with its
/// vanilla code: `$65` (coin, 4), and the brick, note and wood stars (3).
const LATP_SUIT_TILES: [(usize, u8); 4] = [
    (0x11692, 0x04), // TILEA_QBLOCKCOIN2      $65
    (0x11697, 0x03), // TILEA_BRICKSTAR        $6A
    (0x11688, 0x03), // TILEA_NOTESTAR         $31
    (0x1168C, 0x03), // TILEA_WOODBLOCKSTAR    $75
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
    pub(crate) suit_blocks: bool,
    /// The maze's power-up gates: a suit block picks only from the unlocked
    /// pool. Only meaningful with `suit_blocks`.
    pub(crate) powerup_gates: bool,
}

impl SpawnFlags {
    fn any(&self) -> bool {
        self.poison_mushrooms || self.suit_blocks
    }

    fn gated_suits(&self) -> bool {
        self.suit_blocks && self.powerup_gates
    }
}

/// The seed's salt byte, from its own substream so the main stream never sees
/// the draw.
pub(crate) fn salt(seed: u64) -> u8 {
    ChaCha8Rng::seed_from_u64(seed ^ BLOCK_SPAWN_SALT).random()
}

/// Install the spawn routine and, for suit blocks, the suit objects and the
/// suit tiles. Does nothing when no flag is on.
/// [`super::poison_mushroom::apply`] installs the poison object itself; this
/// only decides when to hand it out.
pub(crate) fn apply(rom: &mut Rom, flags: SpawnFlags, salt: u8) {
    if !flags.any() {
        return;
    }
    let routine = assemble(flags, salt);
    rom.write_range(FS_BLOCK_SPAWN, &routine.code);

    let jsr = [0x20, BLOCK_SPAWN_CPU as u8, (BLOCK_SPAWN_CPU >> 8) as u8];
    for &site in &SPAWN_SITES {
        debug_assert_eq!(rom.read_range(site, 3), SPAWN_STORE);
        rom.write_range(site, &jsr);
    }
    if flags.suit_blocks {
        install_suits(rom, &routine);
    }
    if flags.gated_suits() {
        rom.write_range(FS_SUIT_PICK, &suit_pick(&routine));
    }
}

/// Re-install the spawn routine with suit blocks gated, over a ROM the
/// pipeline built with Suit Blocks on. `testrom`'s way in until power-up
/// gates are an option. The routine already there must be exactly the
/// ungated one for `poison` and `salt`, which is what proves both right.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install_gate(rom: &mut Rom, poison: bool, salt: u8) {
    let plain = SpawnFlags { poison_mushrooms: poison, suit_blocks: true, powerup_gates: false };
    let was = assemble(plain, salt).code;
    assert_eq!(
        rom.read_range(FS_BLOCK_SPAWN, was.len()),
        &was[..],
        "the spawn routine is not the one Suit Blocks wrote for this seed"
    );
    let routine = assemble(SpawnFlags { powerup_gates: true, ..plain }, salt);
    rom.write_range(FS_BLOCK_SPAWN, &routine.code);
    // The Hit stubs moved with the code.
    for (i, &id) in SUIT_IDS.iter().enumerate() {
        let hit = routine.hit_cpu + 3 * i as u16;
        rom.write_range(HIT_TABLE + usize::from(id) * 2, &hit.to_le_bytes());
    }
    rom.write_range(FS_SUIT_PICK, &suit_pick(&routine));
}

/// The gated suit pick: `A` = the hash in, `A` = the object to spawn out.
/// `X` is untouched; `Y` is clobbered (the spawn routine reloads it).
/// `Temp_Var1` is scratch: both spawn sites rewrite it before reading it again.
///
/// ```text
/// pick:  AND #$0F / STA <T1      ; countdown: which unlocked entry
///        LDY #4                  ; the 1-up, if nothing is unlocked
///        LDA POWERUPS_FOUND / AND #pool bits / BEQ found
///        LDY #0
/// walk:  LDA bits,Y / AND POWERUPS_FOUND / BEQ next
///        DEC <T1 / BMI found     ; the (hash mod unlocked)-th unlocked one
/// next:  INY / TYA / AND #3 / TAY / BPL walk
/// found: LDA pool,Y / RTS
/// ```
fn suit_pick(routine: &Routine) -> [u8; SUIT_PICK_LEN] {
    let lo = |a: u16| a.to_le_bytes()[0];
    let hi = |a: u16| a.to_le_bytes()[1];
    let found = POWERUPS_FOUND;
    let mask = SUIT_POOL_BITS.iter().fold(0, |m, b| m | b);
    let (pool, bits) = (routine.pool_cpu, routine.bits_cpu);
    #[rustfmt::skip]
    let code = [
        0x29, 0x0F,                     //  0: AND #$0F
        0x85, 0x00,                     //  2: STA <T1
        0xA0, 0x04,                     //  4: LDY #4
        0xAD, lo(found), hi(found),     //  6: LDA POWERUPS_FOUND
        0x29, mask,                     //  9: AND #pool bits
        0xF0, 0x15,                     // 11: BEQ found (-> 34)
        0xA0, 0x00,                     // 13: LDY #0
        0xB9, lo(bits), hi(bits),       // 15: walk: LDA bits,Y
        0x2D, lo(found), hi(found),     // 18: AND POWERUPS_FOUND
        0xF0, 0x04,                     // 21: BEQ next (-> 27)
        0xC6, 0x00,                     // 23: DEC <T1
        0x30, 0x07,                     // 25: BMI found (-> 34)
        0xC8,                           // 27: next: INY
        0x98,                           // 28: TYA
        0x29, 0x03,                     // 29: AND #3
        0xA8,                           // 31: TAY
        0x10, 0xED,                     // 32: BPL walk (-> 15)
        0xB9, lo(pool), hi(pool),       // 34: found: LDA pool,Y
        0x60,                           // 37: RTS
    ];
    code
}

/// The three suit objects' table slots, and the suit tiles' item code.
fn install_suits(rom: &mut Rom, routine: &Routine) {
    let star = usize::from(STARMAN_ID);
    debug_assert_eq!(rom.read_byte(SUIT_BLOCK_ID_AT), SUIT_BLOCK_ID);
    debug_assert_eq!(rom.read_range(INIT_TABLE + star * 2, 2), STAR_INIT_CPU.to_le_bytes());
    debug_assert_eq!(rom.read_range(NORM_TABLE + star * 2, 2), STAR_NORM_CPU.to_le_bytes());
    debug_assert_eq!(rom.read_range(HIT_TABLE + star * 2, 2), STAR_HIT_CPU.to_le_bytes());

    for (i, &id) in SUIT_IDS.iter().enumerate() {
        let id = usize::from(id);
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
    }

    for (offset, vanilla) in LATP_SUIT_TILES {
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
    Poison,
    Done,
    Hit,
    Pool,
    Bits,
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
    /// The Tanooki Hit entry; Frog and Hammer are 3 and 6 bytes on.
    hit_cpu: u16,
    /// [`SUIT_POOL`] and, under power-up gates, the pool's unlock bits.
    pool_cpu: u16,
    bits_cpu: u16,
}

/// Build the spawn routine for `flags`. See the module doc for what it does;
/// the comments below are the instruction listing.
fn assemble(flags: SpawnFlags, salt: u8) -> Routine {
    assert!(flags.any(), "block_spawn: nothing to watch");
    let both = flags.poison_mushrooms && flags.suit_blocks;

    let mut a = Asm::default();
    a.op(&SPAWN_STORE); //                     STA Level_ObjectID,Y  (the store we displaced)

    // Only the watched IDs go on; anything else returns untouched.
    if both {
        a.op(&[0xC9, ONE_UP_ID]); //           CMP #$0B
        a.branch(0xF0, Label::Hash); //        BEQ hash
    }
    let last = if flags.suit_blocks { SUIT_BLOCK_ID } else { ONE_UP_ID };
    a.op(&[0xC9, last]); //                    CMP #id
    a.branch(0xD0, Label::Done); //            BNE done

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
    a.op(&[0x69, salt]); //                    ADC #salt

    if both {
        // Which kind: the object ID is still in Temp_Var1.
        a.op(&[0xA4, 0x00]); //                LDY Temp_Var1
        a.op(&[0xC0, ONE_UP_ID]); //           CPY #$0B
        a.branch(0xF0, Label::Poison); //      BEQ poison
    }
    if flags.suit_blocks {
        if flags.gated_suits() {
            let [lo, hi] = SUIT_PICK_CPU.to_le_bytes();
            a.op(&[0x20, lo, hi]); //          JSR pick   (from the unlocked pool)
        } else {
            a.op(&[0x29, 0x03]); //            AND #3
            a.op(&[0xA8]); //                  TAY
            a.absolute_y(0xB9, Label::Pool); // LDA SUIT_POOL,Y
        }
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

    if flags.suit_blocks {
        // Suit objects' Hit: three entries 3 bytes apart. Each `BIT abs`
        // swallows the next entry's `LDA #` as its operand, so every entry
        // falls through to the JMP holding its own suit index.
        let [hit_lo, hit_hi] = STAR_SUIT_HIT_CPU.to_le_bytes();
        a.label(Label::Hit);
        a.op(&[0xA9, 0x01]); //                LDA #1     (Tanooki)
        a.op(&[0x2C]); //                      BIT $02A9  (skip)
        a.op(&[0xA9, 0x02]); //                LDA #2     (Frog)
        a.op(&[0x2C]); //                      BIT $03A9  (skip)
        a.op(&[0xA9, 0x03]); //                LDA #3     (Hammer)
        a.op(&[0x4C, hit_lo, hit_hi]); //      JMP PRG001_A818 (the star's suit grant)

        a.label(Label::Pool);
        a.op(&SUIT_POOL);
        if flags.gated_suits() {
            a.op(&[ONE_UP_ID]); //             pool[4]: nothing unlocked
            a.label(Label::Bits);
            a.op(&SUIT_POOL_BITS);
        }
    }
    let at = |l| if flags.suit_blocks { a.cpu(BLOCK_SPAWN_CPU, l) } else { 0 };
    let (hit_cpu, pool_cpu) = (at(Label::Hit), at(Label::Pool));
    let bits_cpu = if flags.gated_suits() { a.cpu(BLOCK_SPAWN_CPU, Label::Bits) } else { 0 };
    Routine { code: a.finish(BLOCK_SPAWN_CPU), hit_cpu, pool_cpu, bits_cpu }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::rom_data::FREE_SPACE_ALLOCATIONS;

    /// Every flag combination that installs anything.
    pub(super) fn every_shape() -> Vec<SpawnFlags> {
        let mut out = Vec::new();
        for poison_mushrooms in [false, true] {
            for suit_blocks in [false, true] {
                let f = SpawnFlags { poison_mushrooms, suit_blocks, powerup_gates: false };
                if f.any() {
                    out.push(f);
                }
            }
        }
        out
    }

    /// The shapes with suit blocks gated: with and without poison.
    pub(super) fn gated_shapes() -> Vec<SpawnFlags> {
        [false, true]
            .map(|poison_mushrooms| SpawnFlags {
                poison_mushrooms,
                suit_blocks: true,
                powerup_gates: true,
            })
            .to_vec()
    }

    /// Where the code ends and [`SUIT_POOL`] begins, for the shapes that carry it.
    pub(super) fn code_len(flags: SpawnFlags, code: &[u8]) -> usize {
        let tables = if flags.gated_suits() {
            SUIT_POOL.len() + 1 + SUIT_POOL_BITS.len()
        } else if flags.suit_blocks {
            SUIT_POOL.len()
        } else {
            0
        };
        code.len() - tables
    }

    fn cpu() -> mos6502::cpu::CPU<mos6502::memory::Memory, mos6502::instruction::Ricoh2a03> {
        mos6502::cpu::CPU::new(mos6502::memory::Memory::new(), mos6502::instruction::Ricoh2a03)
    }

    /// Run the routine on an emulated CPU for one spawn: the ID it was asked
    /// about, the block's position and the player's map entry. Returns
    /// (`Level_ObjectID[5]`, `Y`, `X`).
    fn run(code: &[u8], id: u8, block: [u8; 4], entry: (u8, u8)) -> (u8, u8, u8) {
        run_with(code, None, id, block, entry)
    }

    /// [`run`], with the gated pick installed and `POWERUPS_FOUND` set.
    fn run_with(
        code: &[u8],
        pick: Option<(&[u8], u8)>,
        id: u8,
        block: [u8; 4],
        entry: (u8, u8),
    ) -> (u8, u8, u8) {
        use mos6502::memory::Bus;

        let mut cpu = cpu();
        let origin = BLOCK_SPAWN_CPU;
        cpu.memory.set_bytes(origin, code);
        if let Some((pick, found)) = pick {
            cpu.memory.set_bytes(SUIT_PICK_CPU, pick);
            cpu.memory.set_byte(POWERUPS_FOUND, found);
        }
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
        cpu.registers.program_counter = 0x8000;
        cpu.registers.stack_pointer = mos6502::registers::StackPointer(0xFF);
        cpu.registers.accumulator = id;
        cpu.registers.index_x = x;
        cpu.registers.index_y = POWERUP_SLOT;
        for _ in 0..2000 {
            cpu.single_step();
            if cpu.registers.program_counter == 0x8003 {
                let level_object = cpu.memory.get_byte(0x0671 + POWERUP_SLOT as u16);
                return (level_object, cpu.registers.index_y, cpu.registers.index_x);
            }
        }
        panic!("block_spawn: routine never returned");
    }

    fn reservation(offset: usize) -> usize {
        FREE_SPACE_ALLOCATIONS.iter().find(|a| a.offset == offset).unwrap().size
    }

    #[test]
    fn every_shape_fits_its_reservation() {
        let shapes = every_shape().into_iter().chain(gated_shapes());
        let biggest = shapes.map(|f| assemble(f, 0).code.len()).max().unwrap();
        let reserved = reservation(FS_BLOCK_SPAWN);
        assert!(biggest <= reserved, "{biggest} bytes > {reserved} reserved");
        assert!(SUIT_HANDLER.len() <= reservation(FS_SUIT_HANDLER));
        assert!(SUIT_PICK_LEN <= reservation(FS_SUIT_PICK));
    }

    /// Under power-up gates a suit block gives only what is unlocked, for
    /// every found byte: the star or a suit whose bit is set, a 1-up when none
    /// is, and every unlocked one turns up across a screen of blocks, evenly.
    #[test]
    fn gated_suit_blocks_pick_only_what_is_unlocked() {
        for flags in gated_shapes() {
            let routine = assemble(flags, 0x5A);
            let pick = suit_pick(&routine);
            for found in 0..=255u8 {
                let unlocked: Vec<u8> = SUIT_POOL
                    .iter()
                    .zip(SUIT_POOL_BITS)
                    .filter(|&(_, bit)| found & bit != 0)
                    .map(|(&id, _)| id)
                    .collect();
                let mut seen = std::collections::BTreeMap::<u8, usize>::new();
                for col in 0..16u8 {
                    for row in 0..4u8 {
                        let block = [col * 16, 3, row * 16 + 0x40, 0];
                        let (got, y, x) = run_with(
                            &routine.code,
                            Some((&pick, found)),
                            SUIT_BLOCK_ID,
                            block,
                            (0x20, 0x30),
                        );
                        assert_eq!((y, x), (POWERUP_SLOT, 2), "{flags:?} found {found:#04X}");
                        *seen.entry(got).or_default() += 1;
                    }
                }
                let ids: Vec<u8> = seen.keys().copied().collect();
                if unlocked.is_empty() {
                    assert_eq!(ids, [ONE_UP_ID], "found {found:#04X}: nothing unlocked");
                } else {
                    let mut want = unlocked.clone();
                    want.sort();
                    assert_eq!(ids, want, "found {found:#04X}");
                    let (lo, hi) = (seen.values().min().unwrap(), seen.values().max().unwrap());
                    assert!(hi - lo <= 64 / 4, "found {found:#04X}: lopsided {seen:?}");
                }
            }
        }
    }

    /// The other IDs still pass through the gated routine untouched, and the
    /// 1-up still splits into poison.
    #[test]
    fn gating_leaves_other_ids_alone() {
        for flags in gated_shapes() {
            let routine = assemble(flags, 0x11);
            let pick = suit_pick(&routine);
            for id in [0x00, STARMAN_ID, 0x19, 0x20] {
                let got = run_with(&routine.code, Some((&pick, 0)), id, [0x40, 1, 0x60, 0], (0, 0));
                assert_eq!(got.0, id, "{flags:?} id {id:02X}");
            }
            if flags.poison_mushrooms {
                let poisoned = (0..16u8)
                    .filter(|&col| {
                        let block = [col * 16, 2, 0x50, 0];
                        let got = run_with(
                            &routine.code,
                            Some((&pick, 0)),
                            ONE_UP_ID,
                            block,
                            (0x20, 0x30),
                        );
                        got.0 == POISON_ID
                    })
                    .count();
                assert!((4..=12).contains(&poisoned), "{poisoned}/16 poisoned");
            }
        }
    }

    #[test]
    fn every_shape_returns_with_x_and_y_intact_and_stores_the_id() {
        for flags in every_shape() {
            let code = assemble(flags, 0x5A).code;
            for id in [0x00, ONE_UP_ID, SUIT_BLOCK_ID, STARMAN_ID, 0x19, 0x20] {
                let (stored, y, x) = run(&code, id, [0x40, 1, 0x60, 0], (0x20, 0x30));
                assert_eq!(y, POWERUP_SLOT, "{flags:?} id {id:02X}: Y");
                assert_eq!(x, 2, "{flags:?} id {id:02X}: X");
                let poisoned = flags.poison_mushrooms && id == ONE_UP_ID && stored == POISON_ID;
                let suited =
                    flags.suit_blocks && id == SUIT_BLOCK_ID && SUIT_POOL.contains(&stored);
                assert!(
                    stored == id || poisoned || suited,
                    "{flags:?} id {id:02X}: stored {stored:02X}"
                );
                // The marker must never survive: it is not a real object.
                if flags.suit_blocks {
                    assert_ne!(stored, SUIT_BLOCK_ID, "{flags:?}: the $03 marker spawned");
                }
            }
        }
    }

    /// Blocks on one screen differ, and the star and every suit turn up.
    #[test]
    fn suits_vary_by_block() {
        for flags in every_shape().into_iter().filter(|f| f.suit_blocks) {
            let code = assemble(flags, 0x5A).code;
            let mut seen = std::collections::BTreeMap::<u8, usize>::new();
            for col in 0..16u8 {
                for row in 0..4u8 {
                    let block = [col * 16, 3, row * 16 + 0x40, 0];
                    let (suit, _, _) = run(&code, SUIT_BLOCK_ID, block, (0x20, 0x30));
                    *seen.entry(suit).or_default() += 1;
                }
            }
            let ids: Vec<u8> = seen.keys().copied().collect();
            assert_eq!(ids, [0x01, 0x02, STARMAN_ID, 0x1A], "{flags:?}: one screen, {seen:?}");
            assert!(seen.values().all(|&n| n >= 8), "{flags:?}: lopsided {seen:?}");
        }
    }

    /// Deja Vu: the same block, entered from another map tile, can differ.
    #[test]
    fn the_map_entry_moves_the_pick() {
        let flags = SpawnFlags { poison_mushrooms: false, suit_blocks: true, powerup_gates: false };
        let code = assemble(flags, 0).code;
        let picks: std::collections::BTreeSet<u8> = (0..8u8)
            .map(|tile| run(&code, SUIT_BLOCK_ID, [0x40, 1, 0x60, 0], (tile * 16, 0x30)).0)
            .collect();
        assert!(picks.len() > 1, "eight map tiles, one suit: {picks:?}");
    }

    #[test]
    fn poison_still_splits_one_up_blocks() {
        for flags in every_shape().into_iter().filter(|f| f.poison_mushrooms) {
            let code = assemble(flags, 0x11).code;
            let poisoned = (0..16u8)
                .filter(|&col| {
                    run(&code, ONE_UP_ID, [col * 16, 2, 0x50, 0], (0x20, 0x30)).0 == POISON_ID
                })
                .count();
            assert!((4..=12).contains(&poisoned), "{flags:?}: {poisoned}/16 poisoned");
        }
    }

    /// Each suit object's Hit entry reaches the star's suit grant holding its
    /// own index into `PUp_StarOrSuitFrames` (1 Tanooki, 2 Frog, 3 Hammer).
    #[test]
    fn each_hit_entry_carries_its_suit() {
        use mos6502::memory::Bus;
        for flags in [
            SpawnFlags { poison_mushrooms: true, suit_blocks: true, powerup_gates: false },
            SpawnFlags { poison_mushrooms: true, suit_blocks: true, powerup_gates: true },
        ] {
            let routine = assemble(flags, 0);
            for i in 0..3u16 {
                let mut cpu = cpu();
                cpu.memory.set_bytes(BLOCK_SPAWN_CPU, &routine.code);
                cpu.registers.program_counter = routine.hit_cpu + 3 * i;
                for _ in 0..8 {
                    if cpu.registers.program_counter == STAR_SUIT_HIT_CPU {
                        break;
                    }
                    cpu.single_step();
                }
                assert_eq!(cpu.registers.program_counter, STAR_SUIT_HIT_CPU, "entry {i}");
                assert_eq!(cpu.registers.accumulator, i as u8 + 1, "entry {i}");
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
        let off = SpawnFlags { poison_mushrooms: false, suit_blocks: false, powerup_gates: false };
        apply(&mut rom, off, 0x42);
        assert_eq!(rom.read_range(0, 393232), &before[..]);
    }

    /// On the real ROM: the suit objects' table slots, the suit tiles' new
    /// item code and its handler. Also checks every vanilla byte the install asserts.
    #[test]
    fn install_wires_the_objects_and_the_block() {
        let Ok(data) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            return;
        };
        let mut rom = Rom::from_bytes(&data).unwrap();
        let flags = SpawnFlags { poison_mushrooms: false, suit_blocks: true, powerup_gates: false };
        apply(&mut rom, flags, 0x42);
        let routine = assemble(flags, 0x42);
        for (i, &id) in SUIT_IDS.iter().enumerate() {
            let id = usize::from(id);
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
        }
        for (offset, _) in LATP_SUIT_TILES {
            assert_eq!(rom.read_byte(offset), LATP_SUIT, "LATP 0x{offset:05X}");
        }
        // The plain ? star keeps its code: 7-7's fixed stars depend on it.
        assert_eq!(rom.read_byte(0x1168F), 0x03, "TILEA_QBLOCKSTAR");
        assert_eq!(rom.read_range(FS_SUIT_HANDLER, SUIT_HANDLER.len()), SUIT_HANDLER);
        assert_eq!(cpu_to_file(STAR_SUIT_HIT_CPU), 0x02828);

        // Gating it afterwards, as testrom does, rewires the Hit entries.
        install_gate(&mut rom, false, 0x42);
        let gated = assemble(SpawnFlags { powerup_gates: true, ..flags }, 0x42);
        for (i, &id) in SUIT_IDS.iter().enumerate() {
            let at = HIT_TABLE + usize::from(id) * 2;
            let word = u16::from_le_bytes([rom.read_byte(at), rom.read_byte(at + 1)]);
            assert_eq!(word, gated.hit_cpu + 3 * i as u16);
        }
        assert_eq!(rom.read_range(FS_BLOCK_SPAWN, gated.code.len()), &gated.code[..]);
        assert_eq!(rom.read_range(FS_SUIT_PICK, SUIT_PICK_LEN), &suit_pick(&gated)[..]);
    }
}

#[cfg(test)]
mod asm_checks {
    use super::tests::{code_len, every_shape, gated_shapes};
    use super::*;
    use crate::randomize::rom_data::asm;

    const SPAWN_JSR: [u8; 3] = [0x20, BLOCK_SPAWN_CPU as u8, (BLOCK_SPAWN_CPU >> 8) as u8];

    #[test]
    fn every_spawn_routine_is_well_formed() {
        for flags in every_shape().into_iter().chain(gated_shapes()) {
            let code = assemble(flags, 0xA5).code;
            asm::check(&code)
                .allocation(FS_BLOCK_SPAWN)
                .origin(BLOCK_SPAWN_CPU)
                .data_from(code_len(flags, &code))
                .hook(&SPAWN_STORE, 0, &SPAWN_JSR)
                // Temp_Var1, and the engine's own object variables.
                .zero_page(0x02, &[0x76, 0x88, 0x91, 0xA3])
                .assert_ok();
        }
    }

    #[test]
    fn the_suit_pick_is_well_formed() {
        for flags in gated_shapes() {
            let routine = assemble(flags, 0);
            asm::check(&suit_pick(&routine))
                .allocation(FS_SUIT_PICK)
                .origin(SUIT_PICK_CPU)
                .zero_page(0x02, &[])
                .assert_ok();
        }
    }

    #[test]
    fn the_suit_handler_is_well_formed() {
        asm::check(&SUIT_HANDLER).allocation(FS_SUIT_HANDLER).assert_ok();
    }
}
