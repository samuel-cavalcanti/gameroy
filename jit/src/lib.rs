use dynasmrt::ExecutableBuffer;
use gameroy::{
    consts::{self, CB_CLOCK, CLOCK, LEN},
    disassembler::{Address, Cursor},
    gameboy::{cpu::CpuState, GameBoy},
    interpreter::Interpreter,
};
use std::{
    collections::HashMap,
    hash::{BuildHasher, Hasher},
};

use self::x64::BlockCompiler;

#[cfg(target_os = "windows")]
mod windows;

mod x64;

pub struct Block {
    _start_address: u16,
    _length: u16,
    initial_block_clock_cycles: u32,
    max_clock_cycles: u32,
    fn_ptr: unsafe extern "sysv64" fn(&mut GameBoy),
    pub _compiled_code: ExecutableBuffer,
}

impl Block {
    #[inline(never)]
    fn call(&self, gb: &mut GameBoy) {
        // SAFETY: As long as `Block`s are only generated from BlockCompiler::compile, and
        // Self::_compiled_code is not mutated, self.fn_ptr should be pointing to a valid x64
        // function.
        unsafe { (self.fn_ptr)(gb) }
    }
}

/// A block of instruction to be compiled.
struct BlockTrace {
    instrs: Vec<Instr>,
    length: u16,
    // Pairs of (instr index, cycles count) of points where the next_interrupt is checked. It is
    // often after a write.
    interrupt_checks: Vec<(u16, u32)>,
}

struct Instr {
    op: [u8; 3],
    pc: u16,
    bank: u16,
}

fn trace_a_block(gb: &GameBoy) -> BlockTrace {
    let bank = gb.cartridge.curr_bank();

    let cursor = Cursor {
        bank0: bank.0,
        bank: Some(bank.1),
        pc: gb.cpu.pc,
        reg_a: None,
    };

    let mut cursors = vec![cursor];

    let mut interrupt_checks = Vec::new();

    let mut mark_check = |instrs: &Vec<Instr>, max_clock_cycles: &mut u32| {
        // +12 in case the instructions branches.
        interrupt_checks.push((instrs.len() as u16 - 1, *max_clock_cycles + 12));
    };

    let mut curr_clock_count = 0;
    let mut length = 0;

    let mut instrs = Vec::new();

    while let Some(cursor) = cursors.pop() {
        let (op, len) = cursor.get_op(gb);
        length += len as u16;
        curr_clock_count += if op[0] == 0xcb {
            CB_CLOCK[op[1] as usize] as u32
        } else {
            CLOCK[op[0] as usize] as u32
        };

        instrs.push(Instr {
            op,
            pc: cursor.pc,
            bank: if cursor.pc <= 0x3FFF {
                cursor.bank0
            } else {
                cursor.bank.unwrap()
            },
        });

        // after writing to RAM, a next_interrupt check is emmited.
        if consts::WRITE_RAM[op[0] as usize] {
            mark_check(&instrs, &mut curr_clock_count);
        }

        if [
            0x18, 0xc3, 0xc7, 0xc9, 0xcd, 0xcf, 0xd7, 0xd7, 0xe7, 0xe9, 0xef, 0xff, 0xff,
        ]
        .contains(&op[0])
        {
            break;
        }

        let (step, _jump) = gameroy::disassembler::compute_step(len, cursor, &op, &gb.cartridge);

        let step = match step {
            Some(step) if step.pc < 0x4000 && step.bank0 == bank.0 || step.bank == Some(bank.1) => {
                step
            }
            _ => break,
        };

        let (op, len) = step.get_op(gb);
        if step.pc + len as u16 > 0x8000 {
            break;
        }

        if [0x10, 0x76].contains(&op[0]) {
            break;
        }

        cursors.push(step);
    }

    mark_check(&instrs, &mut curr_clock_count);

    BlockTrace {
        instrs,
        length,
        interrupt_checks,
    }
}

pub struct NoHashHasher(u64);
impl Hasher for NoHashHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, _: &[u8]) {
        panic!("should only be hashing u16")
    }

    fn write_u16(&mut self, i: u16) {
        self.0 = (self.0 << 16) | i as u64;
    }
}
impl BuildHasher for NoHashHasher {
    type Hasher = Self;

    fn build_hasher(&self) -> Self::Hasher {
        Self(0)
    }
}

pub struct JitCompiler {
    pub blocks: HashMap<Address, Block, NoHashHasher>,
}

impl Default for JitCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl JitCompiler {
    pub fn new() -> Self {
        Self {
            blocks: HashMap::with_hasher(NoHashHasher(0)),
        }
    }

    pub fn get_block(&mut self, gb: &GameBoy) -> Option<&Block> {
        let pc = gb.cpu.pc;
        let bank = gb.cartridge.curr_bank();

        if pc >= 0x8000 {
            // don't compile code outside ROM
            return None;
        }

        let op = gb.cartridge.read(pc);

        // if STOP or HALT, fallback to interpreter
        if op == 0x10 || op == 0x76 {
            return None;
        }

        let len = LEN[op as usize];

        if pc < 0x4000 && pc + len as u16 >= 0x4000 {
            return None;
        }
        if pc + len as u16 >= 0x8000 {
            return None;
        }

        let address = Address::from_pc(bank, pc)?;
        Some(
            self.blocks
                .entry(address)
                .or_insert_with(|| BlockCompiler::new(gb).compile_block()),
        )
    }

    pub fn interpret_block(&mut self, gb: &mut GameBoy) {
        let block = self.get_block(gb);
        let next_interrupt = gb.next_interrupt.get();
        let start_clock = gb.clock_count;
        match block {
            Some(block)
                if gb.cpu.state == CpuState::Running
                    && (gb.clock_count + block.initial_block_clock_cycles as u64 + 4)
                        < next_interrupt =>
            {
                // let bank = gb.cartridge.curr_bank();
                // println!(
                //     "running {:02x} {:04x} ({})",
                //     if block._start_address <= 0x3FFF {
                //         bank.0
                //     } else {
                //         bank.1
                //     },
                //     block._start_address,
                //     gb.clock_count,
                // );
                block.call(gb);
                debug_assert!(gb.clock_count - start_clock <= block.max_clock_cycles as u64);
                debug_assert!(gb.clock_count != start_clock);

                // assert that no interrupt happened inside the block (unless it happend in a write
                // of the last instruction).
                debug_assert!(
                    gb.clock_count - 24 < gb.next_interrupt.get(),
                    "{} < {}",
                    gb.clock_count,
                    next_interrupt
                );
            }
            _ => {
                // println!("interpr {:04x} ({})", gb.cpu.pc, gb.clock_count);
                let mut inter = Interpreter(gb);
                loop {
                    let op = inter.0.read(inter.0.cpu.pc);

                    inter.interpret_op();

                    let is_jump = [
                        0xc2, 0xc3, 0xca, 0xd2, 0xda, 0xe9, 0x18, 0x20, 0x28, 0x30, 0x38, 0xc4,
                        0xcc, 0xcd, 0xd4, 0xdc, 0xc0, 0xc8, 0xc9, 0xd0, 0xd8, 0xd9, 0xc7, 0xcf,
                        0xd7, 0xdf, 0xe7, 0xef, 0xf7, 0xff,
                    ]
                    .contains(&op);

                    let is_interrupt = [0x40, 0x48, 0x50, 0x58, 0x60].contains(&inter.0.cpu.pc);

                    if is_jump || is_interrupt || inter.0.cpu.state != CpuState::Running {
                        break;
                    }
                }
            }
        }
    }
}
