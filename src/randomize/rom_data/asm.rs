//! Structural verification of hand-assembled 6502 patches.
//!
//! Every patch in this crate is a `&[u8]` of opcodes that no assembler ever
//! checked. The classic failure is a miscounted relative branch: it still
//! "assembles", lands mid-instruction, and the CPU executes an operand as an
//! opcode. Nothing about the byte array looks wrong, and the ROM boots.
//!
//! This module decodes those bytes with the `mos6502` crate's Ricoh 2A03
//! opcode table — the NES's CPU, so no decimal mode — and checks the
//! properties that hold for *any* routine, without knowing what it is for:
//!
//! * every byte decodes as part of a well-formed instruction, and the routine
//!   does not end mid-instruction
//! * it ends in `RTS`/`RTI`/`JMP` rather than running off its own end into the
//!   `$FF` filler that follows
//! * every relative branch lands on an instruction boundary, inside the routine
//! * the code fits its [`FREE_SPACE_ALLOCATIONS`] row and does not cross the
//!   end of its bank
//! * a hook displaces *whole* vanilla instructions, leaving no dangling operand
//!   behind for the CPU to run as an opcode
//!
//! What it cannot check is semantics — a well-formed routine can compute the
//! wrong answer. Executing one to settle that is a separate exercise; see the
//! arithmetic tests in `stomp_fairness` for the pattern.
//!
//! Test-only: `mos6502` is a dev-dependency and never reaches the CLI binary
//! or the WASM bundle.
//!
//! # Routines containing data
//!
//! The sweep is linear, so a routine carrying an inline data table would be
//! misdecoded — and would fail loudly here rather than pass quietly, which is
//! the intent. Every routine checked so far is pure code. The first one that
//! is not should grow a `.data(range)` opt-out rather than skip the check.

use std::collections::BTreeSet;

use mos6502::instruction::{AddressingMode, Instruction, Ricoh2a03};
use mos6502::Variant;

use super::free_space::{prg_bank_end, FREE_SPACE_ALLOCATIONS};

/// One instruction located by the linear sweep.
struct Decoded {
    start: usize,
    len: usize,
    instr: Instruction,
    /// `true` for the relative-addressed branches, whose targets get checked.
    relative: bool,
    /// `true` when the operand is a full 16-bit address, which is what makes a
    /// routine origin-locked. See [`Routine::origin`].
    absolute: bool,
}

/// Walk `code` as a straight-line instruction stream.
fn decode(code: &[u8]) -> Result<Vec<Decoded>, String> {
    let mut out = Vec::new();
    let mut pc = 0;
    while pc < code.len() {
        let byte = code[pc];
        let Some((instr, mode)) = Ricoh2a03::decode(byte) else {
            return Err(format!(
                "byte {pc} (0x{byte:02X}) is not a valid Ricoh 2A03 opcode"
            ));
        };
        let len = mode.extra_bytes() as usize + 1;
        if pc + len > code.len() {
            return Err(format!(
                "{instr:?} at byte {pc} needs {len} bytes but only {} remain — \
                 the routine ends mid-instruction",
                code.len() - pc
            ));
        }
        out.push(Decoded {
            start: pc,
            len,
            instr,
            relative: matches!(mode, AddressingMode::Relative),
            absolute: matches!(
                mode,
                AddressingMode::Absolute
                    | AddressingMode::AbsoluteX
                    | AddressingMode::AbsoluteY
                    | AddressingMode::Indirect
                    | AddressingMode::BuggyIndirect
            ),
        });
        pc += len;
    }
    Ok(out)
}

/// The vanilla bytes a hook overwrites, and the bytes written over them.
struct Hook<'a> {
    vanilla: &'a [u8],
    offset: usize,
    patch: &'a [u8],
}

/// A patch under verification. Build with [`check`], then [`Routine::assert_ok`].
pub struct Routine<'a> {
    code: &'a [u8],
    allocation: Option<usize>,
    hook: Option<Hook<'a>>,
    data_from: Option<usize>,
    fragment: bool,
    origin: Option<u16>,
}

/// Begin checking an assembled routine.
pub fn check(code: &[u8]) -> Routine<'_> {
    Routine {
        code,
        allocation: None,
        hook: None,
        data_from: None,
        fragment: false,
        origin: None,
    }
}

impl<'a> Routine<'a> {
    /// The [`FREE_SPACE_ALLOCATIONS`] row this routine is written to, named by
    /// its file offset so that owners holding several rows stay unambiguous.
    pub fn allocation(mut self, file_offset: usize) -> Self {
        self.allocation = Some(file_offset);
        self
    }

    /// The hook site: the vanilla ROM, the file offset the patch writes over,
    /// and the bytes written there.
    ///
    /// With [`Routine::origin`] also set, a hook that opens with `JSR`/`JMP`
    /// must target the routine's origin — the check that catches a relocated
    /// allocation whose hook was not moved with it.
    pub fn hook(mut self, vanilla: &'a [u8], offset: usize, patch: &'a [u8]) -> Self {
        self.hook = Some(Hook { vanilla, offset, patch });
        self
    }

    /// The CPU address this routine is assembled to run at.
    ///
    /// A relative branch says "12 bytes forward" and survives being moved. An
    /// absolute `JMP $DEB7` has the address baked into its bytes and does not:
    /// relocate the routine and it jumps to whatever now lives at the old
    /// address. A routine holding absolute references to itself — or to tables
    /// in its own tail — is *origin-locked*, and several here are
    /// (`FS_CANOE_SUMMON` says so in its own comment).
    ///
    /// Given the origin, every absolute operand pointing back inside the
    /// routine must land on an instruction boundary, or in the data region
    /// declared by [`Routine::data_from`]. One that lands anywhere else means
    /// the routine moved, or the address was typed wrong.
    pub fn origin(mut self, cpu: u16) -> Self {
        self.origin = Some(cpu);
        self
    }

    /// Bytes from here to the end are a data table, not code.
    ///
    /// A routine carrying its own lookup tables would otherwise be decoded as
    /// instructions past its last `RTS`/`JMP`, desynchronising everything after
    /// and reporting failures that say nothing about the code.
    pub fn data_from(mut self, offset: usize) -> Self {
        self.data_from = Some(offset);
        self
    }

    /// This array is one piece of a larger routine, not a self-contained one.
    ///
    /// Two shapes need it: a patch spliced *in place* over vanilla code, whose
    /// branches legitimately target the vanilla around it, and a fragment
    /// written next to further pieces in the same allocation, which it falls
    /// through into. Both drop the terminator requirement and allow branches to
    /// leave the array — branches landing *inside* it are still checked.
    pub fn fragment(mut self) -> Self {
        self.fragment = true;
        self
    }

    /// Run every configured check, panicking with all failures at once.
    pub fn assert_ok(self) {
        let mut problems: Vec<String> = Vec::new();

        let code = &self.code[..self.data_from.unwrap_or(self.code.len())];
        match decode(code) {
            Err(e) => problems.push(e),
            Ok(instrs) => {
                // Trailing NOPs are padding — several routines are NOP-filled out
                // to the length of the vanilla bytes they replace — so the
                // terminator is the last instruction that actually does something.
                let last = instrs
                    .iter()
                    .rfind(|i| !matches!(i.instr, Instruction::NOP));
                match last {
                    None => problems.push("routine is empty or all NOPs".to_string()),
                    Some(last)
                        if !self.fragment
                            && !matches!(
                                last.instr,
                                Instruction::RTS | Instruction::RTI | Instruction::JMP
                            ) =>
                    {
                        problems.push(format!(
                            "routine ends in {:?} at byte {}; execution would run past the \
                             end of the routine into whatever follows it",
                            last.instr, last.start
                        ));
                    }
                    Some(_) => {}
                }

                let starts: BTreeSet<usize> = instrs.iter().map(|i| i.start).collect();
                for i in instrs.iter().filter(|i| i.relative) {
                    let rel = code[i.start + 1] as i8 as isize;
                    let target = i.start as isize + i.len as isize + rel;
                    if target < 0 || target as usize >= code.len() {
                        if !self.fragment {
                            problems.push(format!(
                                "{:?} at byte {} branches to {target}, outside the routine \
                                 (0..{})",
                                i.instr,
                                i.start,
                                code.len()
                            ));
                        }
                    } else if !starts.contains(&(target as usize)) {
                        problems.push(format!(
                            "{:?} at byte {} branches to byte {target}, which is \
                             mid-instruction",
                            i.instr, i.start
                        ));
                    }
                }

                // Absolute references back into the routine — the origin lock.
                if let Some(origin) = self.origin {
                    let end = origin as usize + self.code.len();
                    let data = self.data_from.unwrap_or(self.code.len());
                    for i in instrs.iter().filter(|i| i.absolute) {
                        let addr =
                            u16::from_le_bytes([code[i.start + 1], code[i.start + 2]]) as usize;
                        if addr < origin as usize || addr >= end {
                            continue; // points outside; nothing here can judge it
                        }
                        let off = addr - origin as usize;
                        if off >= data || starts.contains(&off) {
                            continue; // a table read, or a real instruction
                        }
                        problems.push(format!(
                            "{:?} at byte {} targets ${addr:04X}, which is byte {off} of this \
                             routine — mid-instruction. The routine is origin-locked to \
                             ${origin:04X}; moving it invalidates absolute references like \
                             this one.",
                            i.instr, i.start
                        ));
                    }
                }
            }
        }

        if let Some(offset) = self.allocation {
            match FREE_SPACE_ALLOCATIONS.iter().find(|a| a.offset == offset) {
                None => problems.push(format!(
                    "no FREE_SPACE_ALLOCATIONS row starts at 0x{offset:05X}"
                )),
                Some(a) => {
                    if self.code.len() > a.size {
                        problems.push(format!(
                            "routine is {} bytes but `{}` reserves only {}",
                            self.code.len(),
                            a.label,
                            a.size
                        ));
                    }
                    let bank_end = prg_bank_end(a.offset);
                    if a.offset + self.code.len() > bank_end {
                        problems.push(format!(
                            "routine runs to 0x{:05X}, past the end of its bank \
                             (0x{bank_end:05X}) — the tail would execute whatever is \
                             paged in next",
                            a.offset + self.code.len()
                        ));
                    }
                }
            }
        }

        if let Some(h) = &self.hook {
            if let Err(e) = check_displacement(h.vanilla, h.offset, h.patch.len()) {
                problems.push(e);
            }
            // A hook that calls or jumps into the routine must name its origin.
            if let Some(origin) = self.origin
                && h.patch.len() >= 3
                && matches!(h.patch[0], 0x20 | 0x4C)
            {
                let target = u16::from_le_bytes([h.patch[1], h.patch[2]]);
                if target != origin {
                    problems.push(format!(
                        "the hook at 0x{:05X} targets ${target:04X} but the routine is at \
                         ${origin:04X} — the allocation moved and the hook did not follow",
                        h.offset
                    ));
                }
            }
        }

        assert!(
            problems.is_empty(),
            "assembled routine is malformed:\n  - {}",
            problems.join("\n  - ")
        );
    }
}

/// A hook must overwrite a whole number of vanilla instructions.
///
/// Take one byte too few and the tail of the last displaced instruction is
/// left behind, where the CPU will run its operand as an opcode.
fn check_displacement(vanilla: &[u8], offset: usize, patch_len: usize) -> Result<(), String> {
    let mut covered = 0;
    let mut displaced = Vec::new();
    while covered < patch_len {
        let byte = vanilla[offset + covered];
        let Some((instr, mode)) = Ricoh2a03::decode(byte) else {
            return Err(format!(
                "vanilla byte at 0x{:05X} (0x{byte:02X}) is not a valid opcode, so the \
                 hook site cannot be checked",
                offset + covered
            ));
        };
        displaced.push(instr);
        covered += mode.extra_bytes() as usize + 1;
    }
    if covered != patch_len {
        return Err(format!(
            "the {patch_len}-byte hook at 0x{offset:05X} displaces {displaced:?}, which is \
             {covered} bytes — {} operand byte(s) would be left dangling for the CPU to \
             execute as an opcode",
            covered - patch_len
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The decoder is the foundation every other check stands on, so pin it
    /// against instructions whose lengths are not in dispute.
    #[test]
    fn decode_lengths_match_the_addressing_modes() {
        //     LDA $D0,X   BPL +2   EOR #$FF  LSR A  JMP $1234  RTS
        let code = [0xB5, 0xD0, 0x10, 0x02, 0x49, 0xFF, 0x4A, 0x4C, 0x34, 0x12, 0x60];
        let got: Vec<(usize, usize)> =
            decode(&code).unwrap().iter().map(|i| (i.start, i.len)).collect();
        assert_eq!(got, [(0, 2), (2, 2), (4, 2), (6, 1), (7, 3), (10, 1)]);
    }

    #[test]
    fn rejects_a_branch_into_the_middle_of_an_instruction() {
        // BPL +1 lands on the $FF operand of EOR #$FF rather than on the EOR.
        let code = [0x10, 0x01, 0x49, 0xFF, 0x60];
        let problems = std::panic::catch_unwind(|| check(&code).assert_ok());
        assert!(problems.is_err());
    }

    #[test]
    fn rejects_a_routine_that_falls_off_its_end() {
        let code = [0xA9, 0x00]; // LDA #$00, no terminator
        assert!(std::panic::catch_unwind(|| check(&code).assert_ok()).is_err());
    }

    /// `.fragment()` allows branches to *leave* the array, but a branch landing
    /// inside it must still be on a boundary — otherwise the opt-out would turn
    /// the check off rather than narrow it.
    #[test]
    fn fragment_still_rejects_an_internal_mid_instruction_branch() {
        // BPL +1 lands on the $FF operand of EOR #$FF, inside the array.
        let code = [0x10, 0x01, 0x49, 0xFF, 0xCA];
        assert!(std::panic::catch_unwind(|| check(&code).fragment().assert_ok()).is_err());
        // ...while a branch past the end is what `.fragment()` is for.
        let out = [0x10, 0x20, 0x49, 0xFF, 0xCA];
        check(&out).fragment().assert_ok();
    }

    /// `.data_from` must exclude the table from decoding, not merely tolerate it.
    #[test]
    fn data_from_excludes_the_table_but_still_checks_the_code() {
        // LDA #$00, RTS, then bytes that do not decode as valid instructions.
        let code = [0xA9, 0x00, 0x60, 0xFF, 0xFF, 0xFF];
        check(&code).data_from(3).assert_ok();
        // Without the opt-out the trailing table is read as code and fails.
        assert!(std::panic::catch_unwind(|| check(&code).assert_ok()).is_err());
    }

    /// Relocating an origin-locked routine must be caught: the same bytes that
    /// pass at their real origin fail one byte away, because the absolute
    /// reference then resolves into the middle of an instruction.
    #[test]
    fn origin_check_catches_a_relocated_routine() {
        //   byte 0: LDA #$00   byte 2: JMP $8005   byte 5: RTS
        let code = [0xA9, 0x00, 0x4C, 0x05, 0x80, 0x60];
        check(&code).origin(0x8000).assert_ok();
        assert!(std::panic::catch_unwind(|| check(&code).origin(0x8001).assert_ok()).is_err());
    }

    /// An absolute operand pointing *outside* the routine is somebody else's
    /// address and cannot be judged from here.
    #[test]
    fn origin_check_ignores_addresses_outside_the_routine() {
        // JMP $C123 — a vanilla routine elsewhere in the bank.
        let code = [0x4C, 0x23, 0xC1];
        check(&code).origin(0x8000).assert_ok();
    }

    #[test]
    fn hook_must_target_the_routine_origin() {
        let vanilla = [0x84, 0x01, 0xB5, 0xA3];
        let code = [0x60];
        // JSR $9FB6 + NOP, against a routine that really is at $9FB6.
        let good = [0x20, 0xB6, 0x9F, 0xEA];
        check(&code).origin(0x9FB6).hook(&vanilla, 0, &good).assert_ok();
        // The same hook after the allocation moved.
        assert!(std::panic::catch_unwind(|| {
            check(&code).origin(0x9FC0).hook(&vanilla, 0, &good).assert_ok()
        })
        .is_err());
    }

    #[test]
    fn rejects_a_hook_that_leaves_a_dangling_operand() {
        // STY $01 (2 bytes) then LDA $A3,X (2 bytes); a 3-byte hook splits the
        // second one and strands its $A3 operand.
        let vanilla = [0x84, 0x01, 0xB5, 0xA3];
        assert!(check_displacement(&vanilla, 0, 3).is_err());
        assert!(check_displacement(&vanilla, 0, 4).is_ok());
    }
}
