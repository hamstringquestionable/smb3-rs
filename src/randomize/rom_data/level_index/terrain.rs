//! An area's terrain, by running the engine's own level loader.
//!
//! `LevelLoad_ByTileset` (PRG030 `$9A1D`) runs on the `mos6502` dev-dep CPU
//! with the four PRG windows SMB3 maps during a level load (`$8000` = PRG030,
//! `$A000` = the tileset's bank, `$C000` = PRG014, `$E000` = PRG031), and the
//! result is read out of `Tile_Mem` (`$6000-$794F`). Test-only; see
//! `docs/level_terrain.md` for what it is good for and its limits.

use mos6502::cpu::CPU;
use mos6502::instruction::Ricoh2a03;
use mos6502::memory::Bus;
use mos6502::registers::Status;

use super::Area;
use crate::randomize::rom_data::PAGE_A000_BY_TILESET;
use crate::rom::Rom;

const LEVEL_LOAD_BY_TILESET: u16 = 0x9A1D;
const FILL_TILE_ATTR_TABLE: u16 = 0x951B;
const LEVEL_LAYPTR: u16 = 0x0061;
const LEVEL_JCT_CTL: u16 = 0x03DE;
const LEVEL_TILESET: u16 = 0x070A;
const TILE_MEM: usize = 0x6000;
const TILE_ATTR_TABLE: usize = 0x7E94;
/// Where the harness's fake caller "returns" to. Never executed.
const SENTINEL: u16 = 0x0300;
pub(crate) const SCREEN_BYTES: usize = 0x1B0;
pub(crate) const ROWS: usize = 27;

struct Nes<'a> {
    prg: &'a [u8],
    ram: [u8; 0x800],
    sram: Vec<u8>,
    a000: usize,
    c000: usize,
    mmc3_cmd: u8,
    /// Accesses the model does not back (PPU, APU, unmapped).
    stray: Vec<(u16, bool)>,
}

impl Nes<'_> {
    fn bank_byte(&self, bank: usize, addr: u16) -> u8 {
        self.prg[bank * 0x2000 + (addr as usize & 0x1FFF)]
    }
}

impl Bus for Nes<'_> {
    fn get_byte(&mut self, a: u16) -> u8 {
        match a {
            0x0000..=0x1FFF => self.ram[a as usize & 0x7FF],
            0x6000..=0x7FFF => self.sram[a as usize - 0x6000],
            0x8000..=0x9FFF => self.bank_byte(30, a),
            0xA000..=0xBFFF => self.bank_byte(self.a000, a),
            0xC000..=0xDFFF => self.bank_byte(self.c000, a),
            0xE000..=0xFFFF => self.bank_byte(31, a),
            _ => {
                self.stray.push((a, false));
                (a >> 8) as u8 // open bus: the address high byte was last on the bus
            }
        }
    }

    fn set_byte(&mut self, a: u16, v: u8) {
        match a {
            0x0000..=0x1FFF => self.ram[a as usize & 0x7FF] = v,
            0x6000..=0x7FFF => self.sram[a as usize - 0x6000] = v,
            // MMC3 bank select / bank data. SMB3 runs PRG mode 1 ($8000 fixed
            // to the second-last bank), so R6 is $C000 and R7 is $A000.
            0x8000..=0x9FFF if a & 1 == 0 => self.mmc3_cmd = v,
            0x8000..=0x9FFF => match self.mmc3_cmd & 7 {
                6 => self.c000 = v as usize & 0x1F,
                7 => self.a000 = v as usize & 0x1F,
                _ => {}
            },
            0xA000..=0xFFFF => {} // mirroring / IRQ registers
            _ => self.stray.push((a, true)),
        }
    }
}

/// What a load produced.
pub(crate) struct Terrain {
    /// `Tile_Mem`, 15 screens of 27 x 16 tile bytes.
    pub(crate) tiles: Vec<u8>,
    /// `Tile_AttrTable` after the tileset fill.
    pub(crate) attr: [u8; 8],
    pub(crate) steps: usize,
    pub(crate) sbc_executed: usize,
    pub(crate) stray: Vec<(u16, bool)>,
}

impl Terrain {
    pub(crate) fn tile(&self, screen: usize, row: usize, col: usize) -> u8 {
        self.tiles[screen * SCREEN_BYTES + row * 16 + col]
    }
}

fn call(
    cpu: &mut CPU<Nes, Ricoh2a03>,
    target: u16,
    budget: usize,
) -> Result<(usize, usize), String> {
    let ret = SENTINEL.wrapping_sub(1).to_le_bytes();
    cpu.memory.ram[0x1FF] = ret[1];
    cpu.memory.ram[0x1FE] = ret[0];
    cpu.registers.stack_pointer = mos6502::registers::StackPointer(0xFD);
    cpu.registers.program_counter = target;
    let (mut steps, mut sbc, mut fixed) = (0, 0, 0);
    let mut trail = std::collections::VecDeque::new();
    while cpu.registers.program_counter != SENTINEL {
        if steps == budget || cpu.registers.program_counter == 0xFF40 {
            if std::env::var_os("TERRAIN_TRACE").is_some() {
                eprintln!("  last PCs: {:04X?}", trail);
            }
            let r = &cpu.memory.ram;
            return Err(format!(
                "no return after {steps} steps, PC ${:04X}; cmd {:02X} {:02X} {:02X}, next ptr ${:02X}{:02X}",
                cpu.registers.program_counter, r[0x0E], r[0x0F], r[0x706], r[0x62], r[0x61]
            ));
        }
        let pc = cpu.registers.program_counter;
        let is_sbc = matches!(
            cpu.memory.get_byte(pc),
            0xE1 | 0xE5 | 0xE9 | 0xED | 0xF1 | 0xF5 | 0xF9 | 0xFD
        );
        let a0 = cpu.registers.accumulator;
        let c0 = cpu.registers.status.contains(Status::PS_CARRY);
        let strays = cpu.memory.stray.len();
        trail.push_back((pc, cpu.memory.a000));
        if trail.len() > 48 {
            trail.pop_front();
        }
        cpu.single_step();
        if cpu.memory.stray.len() > strays && std::env::var_os("TERRAIN_TRACE").is_some() {
            eprintln!(
                "  stray {:04X?} at PC ${pc:04X} (A000 bank {})",
                cpu.memory.stray.last().unwrap(),
                cpu.memory.a000
            );
        }
        steps += 1;
        if is_sbc {
            // The mos6502 crate sets carry wrongly after SBC (see
            // `sbc_probe` below), so recover the operand from the result
            // (a1 = a0 - m - borrow, mod 256) and set carry as the 6502 does.
            sbc += 1;
            let a1 = cpu.registers.accumulator;
            let borrow = u8::from(!c0);
            let m = a0.wrapping_sub(a1).wrapping_sub(borrow);
            let carry = i16::from(a0) - i16::from(m) - i16::from(borrow) >= 0;
            if std::env::var_os("NO_SBC_FIX").is_none()
                && cpu.registers.status.contains(Status::PS_CARRY) != carry
            {
                fixed += 1;
                cpu.registers.status.set(Status::PS_CARRY, carry);
            }
        }
    }
    if std::env::var_os("TERRAIN_VERBOSE").is_some() {
        eprintln!("  ${target:04X}: {sbc} SBC, {fixed} carries corrected");
    }
    Ok((steps, sbc))
}

/// Load `area` the way a level entry does (no junction) and return its tiles.
pub(crate) fn load(rom: &Rom, area: &Area) -> Result<Terrain, String> {
    let prg = &rom.data[0x10..0x10 + 0x40000];
    let mut nes = Nes {
        prg,
        ram: [0; 0x800],
        sram: vec![0; 0x2000],
        a000: PAGE_A000_BY_TILESET[area.tileset as usize],
        c000: 14,
        mmc3_cmd: 0,
        stray: Vec::new(),
    };
    let [lo, hi] = area.layout_ptr.to_le_bytes();
    nes.ram[LEVEL_LAYPTR as usize] = lo;
    nes.ram[LEVEL_LAYPTR as usize + 1] = hi;
    nes.ram[LEVEL_JCT_CTL as usize] = 0;
    nes.ram[LEVEL_TILESET as usize] = area.tileset;
    // The plains table's generator 45 (LittleCloudSolidRun) has a vanilla
    // BEQ-for-BNE bug that executes a BRK. On hardware the IRQ handler sees
    // Reset_Latch = $5A (set at boot) and returns with RTI; without it the
    // handler jumps to reset. Raster_Effect $80 is its "do nothing" mode.
    nes.sram[0x7964 - 0x6000] = 0x5A; // Reset_Latch
    nes.ram[0x0101] = 0x80; // Raster_Effect

    let mut cpu = CPU::new(nes, Ricoh2a03);
    let (s1, b1) = call(&mut cpu, LEVEL_LOAD_BY_TILESET, 5_000_000)?;
    let (s2, b2) = call(&mut cpu, FILL_TILE_ATTR_TABLE, 10_000)?;
    let m = &cpu.memory;
    let mut attr = [0; 8];
    attr.copy_from_slice(&m.sram[TILE_ATTR_TABLE - 0x6000..][..8]);
    Ok(Terrain {
        tiles: m.sram[TILE_MEM - 0x6000..][..15 * SCREEN_BYTES].to_vec(),
        attr,
        steps: s1 + s2,
        sbc_executed: b1 + b2,
        stray: m.stray.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::overworld::node_catalog::{NodeCatalog, level_name_key};
    use crate::randomize::rom_data::level_index::trace;

    /// `#` solid, `-` solid on top only, `o` anything else that isn't the
    /// clear tile, `.` the clear tile (whatever row 0 col 0 holds).
    fn render(t: &Terrain, screens: usize) -> String {
        let clear = t.tile(0, 0, 0);
        let mut out = String::new();
        for row in 0..ROWS {
            for s in 0..screens {
                for col in 0..16 {
                    let tile = t.tile(s, row, col);
                    let q = (tile >> 6) as usize;
                    out.push(if tile == clear {
                        '.'
                    } else if tile >= t.attr[q + 4] {
                        '#'
                    } else if tile >= t.attr[q] {
                        '-'
                    } else {
                        'o'
                    });
                }
            }
            out.push('\n');
        }
        out
    }

    /// Run with `cargo test --release --lib terrain_render -- --ignored --nocapture`;
    /// `TERRAIN_LEVEL` picks the level (default 1-1).
    #[test]
    #[ignore = "instrument"]
    fn terrain_render() {
        let Ok(bytes) = std::fs::read(
            std::env::var("TERRAIN_ROM")
                .unwrap_or_else(|_| "roms/Super Mario Bros. 3 (USA) (Rev 1).nes".into()),
        ) else {
            return;
        };
        let rom = Rom::from_bytes_lax(&bytes, true).unwrap();
        let name = std::env::var("TERRAIN_LEVEL").unwrap_or_else(|_| "1-1".into());
        let views = NodeCatalog::build(&rom, true).entry_views();
        let view = views.iter().find(|v| level_name_key(&v.name) == level_name_key(&name)).unwrap();
        let (areas, _) = trace(&rom, view.level_entry.as_ref().unwrap()).unwrap();
        for (i, area) in areas.iter().enumerate() {
            let t = load(&rom, area).unwrap();
            let screens = area.header.screens() as usize;
            println!(
                "{name} area {i}: tileset {} layout ${:04X}, {screens} screens, vertical {}, \
                 {} steps, {} SBC, attr {:02X?}, stray {:04X?}",
                area.tileset,
                area.layout_ptr,
                area.header.vertical(),
                t.steps,
                t.sbc_executed,
                t.attr,
                t.stray.iter().take(8).collect::<Vec<_>>()
            );
            print!("{}", render(&t, screens.min(15)));
            // TERRAIN_COMPARE=<dump>: 6480 bytes of Tile_Mem + 8 of
            // Tile_AttrTable from a real emulator, for area 0.
            if let (0, Some(path)) = (i, std::env::var_os("TERRAIN_COMPARE")) {
                let real = std::fs::read(path).unwrap();
                let ours: Vec<u8> = t.tiles.iter().chain(&t.attr).copied().collect();
                assert_eq!(real.len(), ours.len());
                let diffs: Vec<usize> = (0..ours.len()).filter(|&k| ours[k] != real[k]).collect();
                // LoadLevel_RandomPUpClouds (PRG014) scatters 60 $10-$12
                // clouds by RNG; the harness's zeroed RNG stacks them all at
                // $6000. Those are expected; anything else is a real miss.
                let cloud =
                    |k: usize| k == 0 || (ours[k] == 0x80 && (0x10..=0x12).contains(&real[k]));
                let other: Vec<usize> = diffs.iter().copied().filter(|&k| !cloud(k)).collect();
                println!(
                    "COMPARE: {} of {} bytes differ; {} are RNG clouds, {} are not",
                    diffs.len(),
                    ours.len(),
                    diffs.len() - other.len(),
                    other.len()
                );
                for &k in &other {
                    let (s, r, c) = (k / SCREEN_BYTES, k % SCREEN_BYTES / 16, k % 16);
                    println!(
                        "  ${:04X} screen {s} row {r} col {c}: ours {:02X} real {:02X}",
                        0x6000 + k,
                        ours[k],
                        real[k]
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod sweep {
    use super::*;
    use crate::randomize::rom_data::level_index::every_level;

    #[test]
    #[ignore = "instrument"]
    fn terrain_sweep() {
        let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else { return };
        let rom = Rom::from_bytes_lax(&bytes, true).unwrap();
        let (mut ok, mut bad, mut max_steps) = (0, 0, 0);
        let mut seen = std::collections::BTreeSet::new();
        let mut by_ts = std::collections::BTreeMap::<u8, usize>::new();
        for level in every_level(&rom) {
            for area in level {
                if !seen.insert((area.layout_ptr, area.tileset)) {
                    continue;
                }
                match load(&rom, &area) {
                    Ok(t) if t.stray.is_empty() => {
                        ok += 1;
                        max_steps = max_steps.max(t.steps);
                        *by_ts.entry(area.tileset).or_default() += 1;
                    }
                    Ok(t) => {
                        bad += 1;
                        let n = t.stray.len().min(6);
                        eprintln!(
                            "ts {} ${:04X}: stray {:04X?}",
                            area.tileset,
                            area.layout_ptr,
                            &t.stray[..n]
                        );
                    }
                    Err(e) => {
                        bad += 1;
                        eprintln!("ts {} ${:04X}: {e}", area.tileset, area.layout_ptr);
                    }
                }
            }
        }
        eprintln!("{ok} areas clean, {bad} failed, max {max_steps} steps; by tileset {by_ts:?}");
    }
}

#[cfg(test)]
mod sbc_probe {
    use mos6502::cpu::CPU;
    use mos6502::instruction::Ricoh2a03;
    use mos6502::memory::{Bus, Memory};
    use mos6502::registers::Status;

    #[test]
    #[ignore = "instrument"]
    fn sbc_probe() {
        for (a, m) in [(5u8, 1u8), (1, 2), (0x80, 0x10), (0x10, 0x10)] {
            let mut mem = Memory::new();
            mem.set_bytes(0x0200, &[0x38, 0xA9, a, 0xE9, m]);
            let mut cpu = CPU::new(mem, Ricoh2a03);
            cpu.registers.program_counter = 0x0200;
            for _ in 0..3 {
                cpu.single_step();
            }
            eprintln!(
                "SEC; LDA #${a:02X}; SBC #${m:02X} -> A=${:02X} C={} (real C={})",
                cpu.registers.accumulator,
                cpu.registers.status.contains(Status::PS_CARRY),
                a >= m
            );
        }
    }
}
