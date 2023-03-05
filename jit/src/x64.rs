use dynasmrt::{
    dynasm, mmap::MutableBuffer, x64::X64Relocation, DynasmApi, DynasmLabelApi, VecAssembler,
};

use gameroy::{
    consts::{CB_CLOCK, CLOCK, LEN},
    gameboy::{
        cpu::{Cpu, ImeState},
        GameBoy,
    },
    interpreter::{Condition, Interpreter, Reg, Reg16},
};

use crate::{trace_a_block, Block};

macro_rules! offset {
    (@ $parent:path, $field:tt) => {
        memoffset::offset_of!($parent, $field)
    };
    (@ $parent:path, $field:tt : $next:path, $($tail:tt)*) => {
        {
            #[allow(dead_code)] fn is_eq(x: $parent) -> $next { x.$field }
            memoffset::offset_of!($parent, $field)
        }
        + offset!(@ $next, $($tail)*)
    };
    ($parent:path, $field:tt : $next:path, $($tail:tt)*) => {
        offset!(@ $parent, $field: $next, $($tail)*)
    };
    ($parent:path, $field:tt) => {
        memoffset::offset_of!($parent, $field)
    };
}

pub struct BlockCompiler<'gb> {
    gb: &'gb GameBoy,
    /// The value of PC for the current instruction
    pc: u16,
    length: u16,
    /// the accumulated clock count since the last write to GameBoy.clock_count
    accum_clock_count: u32,
    max_clock_cycles: u32,
}

impl<'a> BlockCompiler<'a> {
    pub fn new(gb: &'a GameBoy) -> Self {
        let (start, length, max_clock_cycles) = trace_a_block(gb, gb.cpu.pc);
        Self {
            gb,
            pc: start,
            length,
            accum_clock_count: 0,
            max_clock_cycles,
        }
    }

    /// Update self.pc to the next instruction
    pub fn compile_block(mut self) -> Block {
        println!(
            "compiling {:02x}_{:04x} (len: {}, cycles: {})",
            self.gb.cartridge.curr_bank(),
            self.pc,
            self.length,
            self.max_clock_cycles,
        );
        let mut ops: dynasmrt::VecAssembler<X64Relocation> = dynasmrt::VecAssembler::new(0);

        let push_rbp_offset;
        let push_rbx_offset;
        let push_rax_offset;
        let prolog_len;

        dynasm!(ops
            ; .arch x64
            ;; push_rbp_offset = ops.offset().0 as u8
            ; push rbp
            ;; push_rbx_offset = ops.offset().0 as u8
            ; push rbx
            ;; push_rax_offset = ops.offset().0 as u8
            ; push rax
            ;; prolog_len = ops.offset().0 as u8
            ; mov rbp, rsp
            ; mov rbx, rdi
        );

        let start = self.pc;
        let end = start + self.length;
        while self.pc < end {
            let op = self.gb.read(self.pc);

            // if STOP or HALT, fallback to interpreter
            if op == 0x10 || op == 0x76 {
                break;
            }

            // if true, the opcode was compiled without handling clock_count
            if self.compile_opcode(&mut ops, op) {
                // TODO: remember to include branching time when implemented.
                self.accum_clock_count += CLOCK[op as usize] as u32;
                if op == 0xcb {
                    self.accum_clock_count += CB_CLOCK[op as usize] as u32;
                }
            }

            self.pc = self.pc.wrapping_add(LEN[op as usize] as u16);
        }

        self.update_clock_count(&mut ops);

        // NOTE: this is current unecessary because all blocks end up in a interpreter call.
        // self.update_pc(&mut ops);

        dynasm!(ops
            ; .arch x64
            ; ->exit:
            ; pop rax
            ; pop rbx
            ; pop rbp
            ; ret
        );

        // See: https://pmeerw.net/blog/programming/RtlAddFunctionTable.html

        let code = ops.finalize().unwrap();

        cfg_if::cfg_if! {
            if #[cfg(target_os = "windows")] {
                let buffer = crate::windows::to_mutable_buffer_with_unwin_info(
                    code,
                    prolog_len,
                    push_rax_offset,
                    push_rbx_offset,
                    push_rbp_offset,
                );
            } else {
                let _ = (prolog_len, push_rax_offset, push_rbx_offset, push_rbp_offset);
                let buffer = to_mutable_buffer(code);
            }
        }

        let compiled_code = buffer.make_exec().unwrap();

        Block {
            _start_address: start,
            _length: self.length,
            max_clock_cycles: self.max_clock_cycles,
            fn_ptr: unsafe { std::mem::transmute(compiled_code.as_ptr()) },
            _compiled_code: compiled_code,
        }
    }

    fn update_clock_count(&mut self, ops: &mut VecAssembler<X64Relocation>) {
        // add the accumulated clock_count
        if self.accum_clock_count != 0 {
            assert!(self.accum_clock_count <= i32::MAX as u32);
            let c = offset!(GameBoy, clock_count);
            dynasm!(ops
                ; .arch x64
                ; add DWORD [rbx + c as i32], self.accum_clock_count as i32
            );
            self.accum_clock_count = 0;
        }
    }

    fn update_pc(&mut self, ops: &mut VecAssembler<X64Relocation>) {
        let pc = offset!(GameBoy, cpu: Cpu, pc);
        dynasm!(ops
            ; .arch x64
            ; mov WORD [rbx + pc as i32], self.pc as i16
        );
    }

    /// Compile a Opcode. Return false if the compiled fallbacks to the interpreter (which means
    /// that clock_count were already updated).
    fn compile_opcode(&mut self, ops: &mut VecAssembler<X64Relocation>, op: u8) -> bool {
        match op {
            // LD (BC),A 1:8 - - - -
            // 0x02 => self.load(ops, Reg::BC, Reg::A),
            // INC BC 1:8 - - - -
            0x03 => self.inc16(ops, Reg::BC),
            // INC B 1:4 Z 0 H -
            0x04 => self.inc(ops, Reg::B),
            // LD B,d8 2:8 - - - -
            0x06 => self.load(ops, Reg::B, Reg::Im8),
            // LD A,(BC) 1:8 - - - -
            // 0x0a => self.load(ops, Reg::A, Reg::BC),
            // INC C 1:4 Z 0 H -
            0x0c => self.inc(ops, Reg::C),
            // LD C,d8 2:8 - - - -
            0x0e => self.load(ops, Reg::C, Reg::Im8),
            // LD (DE),A 1:8 - - - -
            // 0x12 => self.load(ops, Reg::DE, Reg::A),
            // INC DE 1:8 - - - -
            0x13 => self.inc16(ops, Reg::DE),
            // INC D 1:4 Z 0 H -
            0x14 => self.inc(ops, Reg::D),
            // LD D,d8 2:8 - - - -
            0x16 => self.load(ops, Reg::D, Reg::Im8),
            // LD A,(DE) 1:8 - - - -
            // 0x1a => self.load(ops, Reg::A, Reg::DE),
            // INC E 1:4 Z 0 H -
            0x1c => self.inc(ops, Reg::E),
            // LD E,d8 2:8 - - - -
            0x1e => self.load(ops, Reg::E, Reg::Im8),
            // LD (HL+),A 1:8 - - - -
            // 0x22 => self.load(ops, Reg::HLI, Reg::A),
            // INC HL 1:8 - - - -
            0x23 => self.inc16(ops, Reg::HL),
            // INC H 1:4 Z 0 H -
            0x24 => self.inc(ops, Reg::H),
            // LD H,d8 2:8 - - - -
            0x26 => self.load(ops, Reg::H, Reg::Im8),
            // LD A,(HL+) 1:8 - - - -
            // 0x2a => self.load(ops, Reg::A, Reg::HLI),
            // INC L 1:4 Z 0 H -
            0x2c => self.inc(ops, Reg::L),
            // LD L,d8 2:8 - - - -
            0x2e => self.load(ops, Reg::L, Reg::Im8),
            // LD (HL-),A 1:8 - - - -
            // 0x32 => self.load(ops, Reg::HLD, Reg::A),
            // INC SP 1:8 - - - -
            0x33 => self.inc16(ops, Reg::SP),
            // LD (HL),d8 2:12 - - - -
            // 0x36 => self.load(ops, Reg::HL, Reg::Im8),
            // LD A,(HL-) 1:8 - - - -
            // 0x3a => self.load(ops, Reg::A, Reg::HLD),
            // INC A 1:4 Z 0 H -
            0x3c => self.inc(ops, Reg::A),
            // LD A,d8 2:8 - - - -
            0x3e => self.load(ops, Reg::A, Reg::Im8),
            // LD B,B 1:4 - - - -
            0x40 => self.load(ops, Reg::B, Reg::B),
            // LD B,C 1:4 - - - -
            0x41 => self.load(ops, Reg::B, Reg::C),
            // LD B,D 1:4 - - - -
            0x42 => self.load(ops, Reg::B, Reg::D),
            // LD B,E 1:4 - - - -
            0x43 => self.load(ops, Reg::B, Reg::E),
            // LD B,H 1:4 - - - -
            0x44 => self.load(ops, Reg::B, Reg::H),
            // LD B,L 1:4 - - - -
            0x45 => self.load(ops, Reg::B, Reg::L),
            // LD B,(HL) 1:8 - - - -
            // 0x46 => self.load(ops, Reg::B, Reg::HL),
            // LD B,A 1:4 - - - -
            0x47 => self.load(ops, Reg::B, Reg::A),
            // LD C,B 1:4 - - - -
            0x48 => self.load(ops, Reg::C, Reg::B),
            // LD C,C 1:4 - - - -
            0x49 => self.load(ops, Reg::C, Reg::C),
            // LD C,D 1:4 - - - -
            0x4a => self.load(ops, Reg::C, Reg::D),
            // LD C,E 1:4 - - - -
            0x4b => self.load(ops, Reg::C, Reg::E),
            // LD C,H 1:4 - - - -
            0x4c => self.load(ops, Reg::C, Reg::H),
            // LD C,L 1:4 - - - -
            0x4d => self.load(ops, Reg::C, Reg::L),
            // LD C,(HL) 1:8 - - - -
            // 0x4e => self.load(ops, Reg::C, Reg::HL),
            // LD C,A 1:4 - - - -
            0x4f => self.load(ops, Reg::C, Reg::A),
            // LD D,B 1:4 - - - -
            0x50 => self.load(ops, Reg::D, Reg::B),
            // LD D,C 1:4 - - - -
            0x51 => self.load(ops, Reg::D, Reg::C),
            // LD D,D 1:4 - - - -
            0x52 => self.load(ops, Reg::D, Reg::D),
            // LD D,E 1:4 - - - -
            0x53 => self.load(ops, Reg::D, Reg::E),
            // LD D,H 1:4 - - - -
            0x54 => self.load(ops, Reg::D, Reg::H),
            // LD D,L 1:4 - - - -
            0x55 => self.load(ops, Reg::D, Reg::L),
            // LD D,(HL) 1:8 - - - -
            // 0x56 => self.load(ops, Reg::D, Reg::HL),
            // LD D,A 1:4 - - - -
            0x57 => self.load(ops, Reg::D, Reg::A),
            // LD E,B 1:4 - - - -
            0x58 => self.load(ops, Reg::E, Reg::B),
            // LD E,C 1:4 - - - -
            0x59 => self.load(ops, Reg::E, Reg::C),
            // LD E,D 1:4 - - - -
            0x5a => self.load(ops, Reg::E, Reg::D),
            // LD E,E 1:4 - - - -
            0x5b => self.load(ops, Reg::E, Reg::E),
            // LD E,H 1:4 - - - -
            0x5c => self.load(ops, Reg::E, Reg::H),
            // LD E,L 1:4 - - - -
            0x5d => self.load(ops, Reg::E, Reg::L),
            // LD E,(HL) 1:8 - - - -
            // 0x5e => self.load(ops, Reg::E, Reg::HL),
            // LD E,A 1:4 - - - -
            0x5f => self.load(ops, Reg::E, Reg::A),
            // LD H,B 1:4 - - - -
            0x60 => self.load(ops, Reg::H, Reg::B),
            // LD H,C 1:4 - - - -
            0x61 => self.load(ops, Reg::H, Reg::C),
            // LD H,D 1:4 - - - -
            0x62 => self.load(ops, Reg::H, Reg::D),
            // LD H,E 1:4 - - - -
            0x63 => self.load(ops, Reg::H, Reg::E),
            // LD H,H 1:4 - - - -
            0x64 => self.load(ops, Reg::H, Reg::H),
            // LD H,L 1:4 - - - -
            0x65 => self.load(ops, Reg::H, Reg::L),
            // LD H,(HL) 1:8 - - - -
            // 0x66 => self.load(ops, Reg::H, Reg::HL),
            // LD H,A 1:4 - - - -
            0x67 => self.load(ops, Reg::H, Reg::A),
            // LD L,B 1:4 - - - -
            0x68 => self.load(ops, Reg::L, Reg::B),
            // LD L,C 1:4 - - - -
            0x69 => self.load(ops, Reg::L, Reg::C),
            // LD L,D 1:4 - - - -
            0x6a => self.load(ops, Reg::L, Reg::D),
            // LD L,E 1:4 - - - -
            0x6b => self.load(ops, Reg::L, Reg::E),
            // LD L,H 1:4 - - - -
            0x6c => self.load(ops, Reg::L, Reg::H),
            // LD L,L 1:4 - - - -
            0x6d => self.load(ops, Reg::L, Reg::L),
            // LD L,(HL) 1:8 - - - -
            // 0x6e => self.load(ops, Reg::L, Reg::HL),
            // LD L,A 1:4 - - - -
            0x6f => self.load(ops, Reg::L, Reg::A),
            // LD (HL),B 1:8 - - - -
            // 0x70 => self.load(ops, Reg::HL, Reg::B),
            // LD (HL),C 1:8 - - - -
            // 0x71 => self.load(ops, Reg::HL, Reg::C),
            // LD (HL),D 1:8 - - - -
            // 0x72 => self.load(ops, Reg::HL, Reg::D),
            // LD (HL),E 1:8 - - - -
            // 0x73 => self.load(ops, Reg::HL, Reg::E),
            // LD (HL),H 1:8 - - - -
            // 0x74 => self.load(ops, Reg::HL, Reg::H),
            // LD (HL),L 1:8 - - - -
            // 0x75 => self.load(ops, Reg::HL, Reg::L),
            // LD (HL),A 1:8 - - - -
            // 0x77 => self.load(ops, Reg::HL, Reg::A),
            // LD A,B 1:4 - - - -
            0x78 => self.load(ops, Reg::A, Reg::B),
            // LD A,C 1:4 - - - -
            0x79 => self.load(ops, Reg::A, Reg::C),
            // LD A,D 1:4 - - - -
            0x7a => self.load(ops, Reg::A, Reg::D),
            // LD A,E 1:4 - - - -
            0x7b => self.load(ops, Reg::A, Reg::E),
            // LD A,H 1:4 - - - -
            0x7c => self.load(ops, Reg::A, Reg::H),
            // LD A,L 1:4 - - - -
            0x7d => self.load(ops, Reg::A, Reg::L),
            // LD A,(HL) 1:8 - - - -
            // 0x7e => self.load(ops, Reg::A, Reg::HL),
            // LD A,A 1:4 - - - -
            0x7f => self.load(ops, Reg::A, Reg::A),
            // LD (a16),A 3:16 - - - -
            // 0xea => self.load(ops, Reg::Im16, Reg::A),
            // LD A,(a16) 3:16 - - - -
            // 0xfa => self.load(ops, Reg::A, Reg::Im16),
            _ => {
                self.update_clock_count(ops);
                self.update_pc(ops);

                let call = interpreter_call(op);
                dynasm!(ops
                    ; .arch x64
                    ; mov rax, QWORD call as usize as i64
                    ; mov rdi, rbx
                    ; call rax
                    ; test rax, rax
                    ; jnz ->exit
                );
                return false;
            }
        }
        true
    }

    pub fn load(&mut self, ops: &mut VecAssembler<X64Relocation>, dst: Reg, src: Reg) {
        let dst = match dst {
            Reg::A => offset!(GameBoy, cpu: Cpu, a),
            Reg::B => offset!(GameBoy, cpu: Cpu, b),
            Reg::C => offset!(GameBoy, cpu: Cpu, c),
            Reg::D => offset!(GameBoy, cpu: Cpu, d),
            Reg::E => offset!(GameBoy, cpu: Cpu, e),
            Reg::H => offset!(GameBoy, cpu: Cpu, h),
            Reg::L => offset!(GameBoy, cpu: Cpu, l),
            Reg::Im8 => todo!(),
            Reg::Im16 => todo!(),
            Reg::BC => todo!(),
            Reg::DE => todo!(),
            Reg::HL => todo!(),
            Reg::SP => todo!(),
            Reg::HLI => todo!(),
            Reg::HLD => todo!(),
        };
        let src = match src {
            Reg::A => offset!(GameBoy, cpu: Cpu, a),
            Reg::B => offset!(GameBoy, cpu: Cpu, b),
            Reg::C => offset!(GameBoy, cpu: Cpu, c),
            Reg::D => offset!(GameBoy, cpu: Cpu, d),
            Reg::E => offset!(GameBoy, cpu: Cpu, e),
            Reg::H => offset!(GameBoy, cpu: Cpu, h),
            Reg::L => offset!(GameBoy, cpu: Cpu, l),
            Reg::Im8 => {
                return dynasm!(ops
                    ; mov BYTE [rbx + dst as i32], self.gb.read(self.pc.wrapping_add(1)) as i8
                );
            }
            Reg::Im16 => todo!(),
            Reg::BC => todo!(),
            Reg::DE => todo!(),
            Reg::HL => todo!(),
            Reg::SP => todo!(),
            Reg::HLI => todo!(),
            Reg::HLD => todo!(),
        };

        dynasm!(ops
            ; movzx eax, BYTE [rbx + src as i32]
            ; mov BYTE [rbx + dst as i32], al
        );
    }

    pub fn inc(&mut self, ops: &mut VecAssembler<X64Relocation>, reg: Reg) {
        let reg = match reg {
            Reg::A => offset!(GameBoy, cpu: Cpu, a),
            Reg::B => offset!(GameBoy, cpu: Cpu, b),
            Reg::C => offset!(GameBoy, cpu: Cpu, c),
            Reg::D => offset!(GameBoy, cpu: Cpu, d),
            Reg::E => offset!(GameBoy, cpu: Cpu, e),
            Reg::H => offset!(GameBoy, cpu: Cpu, h),
            Reg::L => offset!(GameBoy, cpu: Cpu, l),
            _ => unreachable!(),
        };
        let f = offset!(GameBoy, cpu: Cpu, f);

        // uses rax, rcx, rdx
        dynasm!(ops
            ; movzx	eax, BYTE [rbx + reg as i32] // load reg
            ; movzx	ecx, BYTE [rbx + f as i32]   // load f
            ; inc	al                           // increase reg
            ; sete	dl // Z flag
            ; mov	[rbx + reg as i32], al       // save reg
            ; and	cl, 0x1F                     // clear Z, N, H
            ; shl	dl, 7
            ; or	dl, cl                       // set Z
            ; test	al, 0xF
            ; sete	al // H flag
            ; shl	al, 5
            ; or	al, dl                       // set H
            ; mov	[rbx + f as i32], al         // save f
        );
    }

    pub fn inc16(&mut self, ops: &mut VecAssembler<X64Relocation>, reg: Reg) {
        let reg = match reg {
            Reg::BC => {
                debug_assert!(offset!(GameBoy, cpu: Cpu, c) + 1 == offset!(GameBoy, cpu: Cpu, b));
                offset!(GameBoy, cpu: Cpu, c)
            }
            Reg::DE => {
                debug_assert!(offset!(GameBoy, cpu: Cpu, e) + 1 == offset!(GameBoy, cpu: Cpu, d));
                offset!(GameBoy, cpu: Cpu, e)
            }
            Reg::HL => {
                debug_assert!(offset!(GameBoy, cpu: Cpu, l) + 1 == offset!(GameBoy, cpu: Cpu, h));
                offset!(GameBoy, cpu: Cpu, l)
            }
            Reg::SP => {
                offset!(GameBoy, cpu: Cpu, sp)
            }
            _ => unreachable!(),
        };
        dynasm!(ops
            ; inc WORD [rbx + reg as i32]
        );
    }
}

#[allow(dead_code)]
fn to_mutable_buffer(code: Vec<u8>) -> MutableBuffer {
    let mut buffer = MutableBuffer::new(code.len()).unwrap();
    buffer.set_len(code.len());
    buffer[..].copy_from_slice(code.as_slice());
    buffer
}

macro_rules! call {
    ($($call:tt)*) => {
        {
            extern "sysv64" fn function(gb: &mut GameBoy) -> bool {
                // println!("running {}", stringify!($($call)*));
                let mut interpreter = Interpreter(gb);
                if interpreter.handle_interrupt().is_break() {
                    return true;
                }

                if interpreter.0.cpu.ime == ImeState::ToBeEnable {
                    interpreter.0.cpu.ime = ImeState::Enabled;
                }

                //call to instructions relies on pc being already read.
                interpreter.read_next_pc();
                interpreter.$($call)*;
                false
            }
            function
        }
    };
}

fn interpreter_call(op: u8) -> extern "sysv64" fn(&mut GameBoy) -> bool {
    use Condition::*;
    match op {
        // NOP 1:4 - - - -
        0x00 => call!(nop()),
        // LD BC,d16 3:12 - - - -
        0x01 => call!(load16(Reg16::BC, Reg16::Im16)),
        // LD (BC),A 1:8 - - - -
        0x02 => call!(load(Reg::BC, Reg::A)),
        // INC BC 1:8 - - - -
        0x03 => call!(inc(Reg::BC)),
        // INC B 1:4 Z 0 H -
        0x04 => call!(inc(Reg::B)),
        // DEC B 1:4 Z 1 H -
        0x05 => call!(dec(Reg::B)),
        // LD B,d8 2:8 - - - -
        0x06 => call!(load(Reg::B, Reg::Im8)),
        // RLCA 1:4 0 0 0 C
        0x07 => call!(rlca()),
        // LD (a16),SP 3:20 - - - -
        0x08 => call!(load16(Reg16::Im16, Reg16::SP)),
        // ADD HL,BC 1:8 - 0 H C
        0x09 => call!(add16(Reg16::BC)),
        // LD A,(BC) 1:8 - - - -
        0x0a => call!(load(Reg::A, Reg::BC)),
        // DEC BC 1:8 - - - -
        0x0b => call!(dec(Reg::BC)),
        // INC C 1:4 Z 0 H -
        0x0c => call!(inc(Reg::C)),
        // DEC C 1:4 Z 1 H -
        0x0d => call!(dec(Reg::C)),
        // LD C,d8 2:8 - - - -
        0x0e => call!(load(Reg::C, Reg::Im8)),
        // RRCA 1:4 0 0 0 C
        0x0f => call!(rrca()),
        // STOP 0 2:4 - - - -
        0x10 => call!(stop()),
        // LD DE,d16 3:12 - - - -
        0x11 => call!(load16(Reg16::DE, Reg16::Im16)),
        // LD (DE),A 1:8 - - - -
        0x12 => call!(load(Reg::DE, Reg::A)),
        // INC DE 1:8 - - - -
        0x13 => call!(inc(Reg::DE)),
        // INC D 1:4 Z 0 H -
        0x14 => call!(inc(Reg::D)),
        // DEC D 1:4 Z 1 H -
        0x15 => call!(dec(Reg::D)),
        // LD D,d8 2:8 - - - -
        0x16 => call!(load(Reg::D, Reg::Im8)),
        // RLA 1:4 0 0 0 C
        0x17 => call!(rla()),
        // JR r8 2:12 - - - -
        0x18 => call!(jump_rel(None)),
        // ADD HL,DE 1:8 - 0 H C
        0x19 => call!(add16(Reg16::DE)),
        // LD A,(DE) 1:8 - - - -
        0x1a => call!(load(Reg::A, Reg::DE)),
        // DEC DE 1:8 - - - -
        0x1b => call!(dec(Reg::DE)),
        // INC E 1:4 Z 0 H -
        0x1c => call!(inc(Reg::E)),
        // DEC E 1:4 Z 1 H -
        0x1d => call!(dec(Reg::E)),
        // LD E,d8 2:8 - - - -
        0x1e => call!(load(Reg::E, Reg::Im8)),
        // RRA 1:4 0 0 0 C
        0x1f => call!(rra()),
        // JR NZ,r8 2:12/8 - - - -
        0x20 => call!(jump_rel(NZ)),
        // LD HL,d16 3:12 - - - -
        0x21 => call!(load16(Reg16::HL, Reg16::Im16)),
        // LD (HL+),A 1:8 - - - -
        0x22 => call!(load(Reg::HLI, Reg::A)),
        // INC HL 1:8 - - - -
        0x23 => call!(inc(Reg::HL)),
        // INC H 1:4 Z 0 H -
        0x24 => call!(inc(Reg::H)),
        // DEC H 1:4 Z 1 H -
        0x25 => call!(dec(Reg::H)),
        // LD H,d8 2:8 - - - -
        0x26 => call!(load(Reg::H, Reg::Im8)),
        // DAA 1:4 Z - 0 C
        0x27 => call!(daa()),
        // JR Z,r8 2:12/8 - - - -
        0x28 => call!(jump_rel(Z)),
        // ADD HL,HL 1:8 - 0 H C
        0x29 => call!(add16(Reg16::HL)),
        // LD A,(HL+) 1:8 - - - -
        0x2a => call!(load(Reg::A, Reg::HLI)),
        // DEC HL 1:8 - - - -
        0x2b => call!(dec(Reg::HL)),
        // INC L 1:4 Z 0 H -
        0x2c => call!(inc(Reg::L)),
        // DEC L 1:4 Z 1 H -
        0x2d => call!(dec(Reg::L)),
        // LD L,d8 2:8 - - - -
        0x2e => call!(load(Reg::L, Reg::Im8)),
        // CPL 1:4 - 1 1 -
        0x2f => call!(cpl()),
        // JR NC,r8 2:12/8 - - - -
        0x30 => call!(jump_rel(NC)),
        // LD SP,d16 3:12 - - - -
        0x31 => call!(load16(Reg16::SP, Reg16::Im16)),
        // LD (HL-),A 1:8 - - - -
        0x32 => call!(load(Reg::HLD, Reg::A)),
        // INC SP 1:8 - - - -
        0x33 => call!(inc(Reg::SP)),
        // INC (HL) 1:12 Z 0 H -
        0x34 => call!(inc16(Reg::HL)),
        // DEC (HL) 1:12 Z 1 H -
        0x35 => call!(dec16(Reg::HL)),
        // LD (HL),d8 2:12 - - - -
        0x36 => call!(load(Reg::HL, Reg::Im8)),
        // SCF 1:4 - 0 0 1
        0x37 => call!(scf()),
        // JR C,r8 2:12/8 - - - -
        0x38 => call!(jump_rel(C)),
        // ADD HL,SP 1:8 - 0 H C
        0x39 => call!(add16(Reg16::SP)),
        // LD A,(HL-) 1:8 - - - -
        0x3a => call!(load(Reg::A, Reg::HLD)),
        // DEC SP 1:8 - - - -
        0x3b => call!(dec(Reg::SP)),
        // INC A 1:4 Z 0 H -
        0x3c => call!(inc(Reg::A)),
        // DEC A 1:4 Z 1 H -
        0x3d => call!(dec(Reg::A)),
        // LD A,d8 2:8 - - - -
        0x3e => call!(load(Reg::A, Reg::Im8)),
        // CCF 1:4 - 0 0 C
        0x3f => call!(ccf()),
        // LD B,B 1:4 - - - -
        0x40 => call!(load(Reg::B, Reg::B)),
        // LD B,C 1:4 - - - -
        0x41 => call!(load(Reg::B, Reg::C)),
        // LD B,D 1:4 - - - -
        0x42 => call!(load(Reg::B, Reg::D)),
        // LD B,E 1:4 - - - -
        0x43 => call!(load(Reg::B, Reg::E)),
        // LD B,H 1:4 - - - -
        0x44 => call!(load(Reg::B, Reg::H)),
        // LD B,L 1:4 - - - -
        0x45 => call!(load(Reg::B, Reg::L)),
        // LD B,(HL) 1:8 - - - -
        0x46 => call!(load(Reg::B, Reg::HL)),
        // LD B,A 1:4 - - - -
        0x47 => call!(load(Reg::B, Reg::A)),
        // LD C,B 1:4 - - - -
        0x48 => call!(load(Reg::C, Reg::B)),
        // LD C,C 1:4 - - - -
        0x49 => call!(load(Reg::C, Reg::C)),
        // LD C,D 1:4 - - - -
        0x4a => call!(load(Reg::C, Reg::D)),
        // LD C,E 1:4 - - - -
        0x4b => call!(load(Reg::C, Reg::E)),
        // LD C,H 1:4 - - - -
        0x4c => call!(load(Reg::C, Reg::H)),
        // LD C,L 1:4 - - - -
        0x4d => call!(load(Reg::C, Reg::L)),
        // LD C,(HL) 1:8 - - - -
        0x4e => call!(load(Reg::C, Reg::HL)),
        // LD C,A 1:4 - - - -
        0x4f => call!(load(Reg::C, Reg::A)),
        // LD D,B 1:4 - - - -
        0x50 => call!(load(Reg::D, Reg::B)),
        // LD D,C 1:4 - - - -
        0x51 => call!(load(Reg::D, Reg::C)),
        // LD D,D 1:4 - - - -
        0x52 => call!(load(Reg::D, Reg::D)),
        // LD D,E 1:4 - - - -
        0x53 => call!(load(Reg::D, Reg::E)),
        // LD D,H 1:4 - - - -
        0x54 => call!(load(Reg::D, Reg::H)),
        // LD D,L 1:4 - - - -
        0x55 => call!(load(Reg::D, Reg::L)),
        // LD D,(HL) 1:8 - - - -
        0x56 => call!(load(Reg::D, Reg::HL)),
        // LD D,A 1:4 - - - -
        0x57 => call!(load(Reg::D, Reg::A)),
        // LD E,B 1:4 - - - -
        0x58 => call!(load(Reg::E, Reg::B)),
        // LD E,C 1:4 - - - -
        0x59 => call!(load(Reg::E, Reg::C)),
        // LD E,D 1:4 - - - -
        0x5a => call!(load(Reg::E, Reg::D)),
        // LD E,E 1:4 - - - -
        0x5b => call!(load(Reg::E, Reg::E)),
        // LD E,H 1:4 - - - -
        0x5c => call!(load(Reg::E, Reg::H)),
        // LD E,L 1:4 - - - -
        0x5d => call!(load(Reg::E, Reg::L)),
        // LD E,(HL) 1:8 - - - -
        0x5e => call!(load(Reg::E, Reg::HL)),
        // LD E,A 1:4 - - - -
        0x5f => call!(load(Reg::E, Reg::A)),
        // LD H,B 1:4 - - - -
        0x60 => call!(load(Reg::H, Reg::B)),
        // LD H,C 1:4 - - - -
        0x61 => call!(load(Reg::H, Reg::C)),
        // LD H,D 1:4 - - - -
        0x62 => call!(load(Reg::H, Reg::D)),
        // LD H,E 1:4 - - - -
        0x63 => call!(load(Reg::H, Reg::E)),
        // LD H,H 1:4 - - - -
        0x64 => call!(load(Reg::H, Reg::H)),
        // LD H,L 1:4 - - - -
        0x65 => call!(load(Reg::H, Reg::L)),
        // LD H,(HL) 1:8 - - - -
        0x66 => call!(load(Reg::H, Reg::HL)),
        // LD H,A 1:4 - - - -
        0x67 => call!(load(Reg::H, Reg::A)),
        // LD L,B 1:4 - - - -
        0x68 => call!(load(Reg::L, Reg::B)),
        // LD L,C 1:4 - - - -
        0x69 => call!(load(Reg::L, Reg::C)),
        // LD L,D 1:4 - - - -
        0x6a => call!(load(Reg::L, Reg::D)),
        // LD L,E 1:4 - - - -
        0x6b => call!(load(Reg::L, Reg::E)),
        // LD L,H 1:4 - - - -
        0x6c => call!(load(Reg::L, Reg::H)),
        // LD L,L 1:4 - - - -
        0x6d => call!(load(Reg::L, Reg::L)),
        // LD L,(HL) 1:8 - - - -
        0x6e => call!(load(Reg::L, Reg::HL)),
        // LD L,A 1:4 - - - -
        0x6f => call!(load(Reg::L, Reg::A)),
        // LD (HL),B 1:8 - - - -
        0x70 => call!(load(Reg::HL, Reg::B)),
        // LD (HL),C 1:8 - - - -
        0x71 => call!(load(Reg::HL, Reg::C)),
        // LD (HL),D 1:8 - - - -
        0x72 => call!(load(Reg::HL, Reg::D)),
        // LD (HL),E 1:8 - - - -
        0x73 => call!(load(Reg::HL, Reg::E)),
        // LD (HL),H 1:8 - - - -
        0x74 => call!(load(Reg::HL, Reg::H)),
        // LD (HL),L 1:8 - - - -
        0x75 => call!(load(Reg::HL, Reg::L)),
        // HALT 1:4 - - - -
        0x76 => call!(halt()),
        // LD (HL),A 1:8 - - - -
        0x77 => call!(load(Reg::HL, Reg::A)),
        // LD A,B 1:4 - - - -
        0x78 => call!(load(Reg::A, Reg::B)),
        // LD A,C 1:4 - - - -
        0x79 => call!(load(Reg::A, Reg::C)),
        // LD A,D 1:4 - - - -
        0x7a => call!(load(Reg::A, Reg::D)),
        // LD A,E 1:4 - - - -
        0x7b => call!(load(Reg::A, Reg::E)),
        // LD A,H 1:4 - - - -
        0x7c => call!(load(Reg::A, Reg::H)),
        // LD A,L 1:4 - - - -
        0x7d => call!(load(Reg::A, Reg::L)),
        // LD A,(HL) 1:8 - - - -
        0x7e => call!(load(Reg::A, Reg::HL)),
        // LD A,A 1:4 - - - -
        0x7f => call!(load(Reg::A, Reg::A)),
        // ADD A,B 1:4 Z 0 H C
        0x80 => call!(add(Reg::B)),
        // ADD A,C 1:4 Z 0 H C
        0x81 => call!(add(Reg::C)),
        // ADD A,D 1:4 Z 0 H C
        0x82 => call!(add(Reg::D)),
        // ADD A,E 1:4 Z 0 H C
        0x83 => call!(add(Reg::E)),
        // ADD A,H 1:4 Z 0 H C
        0x84 => call!(add(Reg::H)),
        // ADD A,L 1:4 Z 0 H C
        0x85 => call!(add(Reg::L)),
        // ADD A,(HL) 1:8 Z 0 H C
        0x86 => call!(add(Reg::HL)),
        // ADD A,A 1:4 Z 0 H C
        0x87 => call!(add(Reg::A)),
        // ADC A,B 1:4 Z 0 H C
        0x88 => call!(adc(Reg::B)),
        // ADC A,C 1:4 Z 0 H C
        0x89 => call!(adc(Reg::C)),
        // ADC A,D 1:4 Z 0 H C
        0x8a => call!(adc(Reg::D)),
        // ADC A,E 1:4 Z 0 H C
        0x8b => call!(adc(Reg::E)),
        // ADC A,H 1:4 Z 0 H C
        0x8c => call!(adc(Reg::H)),
        // ADC A,L 1:4 Z 0 H C
        0x8d => call!(adc(Reg::L)),
        // ADC A,(HL) 1:8 Z 0 H C
        0x8e => call!(adc(Reg::HL)),
        // ADC A,A 1:4 Z 0 H C
        0x8f => call!(adc(Reg::A)),
        // SUB B 1:4 Z 1 H C
        0x90 => call!(sub(Reg::B)),
        // SUB C 1:4 Z 1 H C
        0x91 => call!(sub(Reg::C)),
        // SUB D 1:4 Z 1 H C
        0x92 => call!(sub(Reg::D)),
        // SUB E 1:4 Z 1 H C
        0x93 => call!(sub(Reg::E)),
        // SUB H 1:4 Z 1 H C
        0x94 => call!(sub(Reg::H)),
        // SUB L 1:4 Z 1 H C
        0x95 => call!(sub(Reg::L)),
        // SUB (HL) 1:8 Z 1 H C
        0x96 => call!(sub(Reg::HL)),
        // SUB A 1:4 Z 1 H C
        0x97 => call!(sub(Reg::A)),
        // SBC A,B 1:4 Z 1 H C
        0x98 => call!(sbc(Reg::B)),
        // SBC A,C 1:4 Z 1 H C
        0x99 => call!(sbc(Reg::C)),
        // SBC A,D 1:4 Z 1 H C
        0x9a => call!(sbc(Reg::D)),
        // SBC A,E 1:4 Z 1 H C
        0x9b => call!(sbc(Reg::E)),
        // SBC A,H 1:4 Z 1 H C
        0x9c => call!(sbc(Reg::H)),
        // SBC A,L 1:4 Z 1 H C
        0x9d => call!(sbc(Reg::L)),
        // SBC A,(HL) 1:8 Z 1 H C
        0x9e => call!(sbc(Reg::HL)),
        // SBC A,A 1:4 Z 1 H C
        0x9f => call!(sbc(Reg::A)),
        // AND B 1:4 Z 0 1 0
        0xa0 => call!(and(Reg::B)),
        // AND C 1:4 Z 0 1 0
        0xa1 => call!(and(Reg::C)),
        // AND D 1:4 Z 0 1 0
        0xa2 => call!(and(Reg::D)),
        // AND E 1:4 Z 0 1 0
        0xa3 => call!(and(Reg::E)),
        // AND H 1:4 Z 0 1 0
        0xa4 => call!(and(Reg::H)),
        // AND L 1:4 Z 0 1 0
        0xa5 => call!(and(Reg::L)),
        // AND (HL) 1:8 Z 0 1 0
        0xa6 => call!(and(Reg::HL)),
        // AND A 1:4 Z 0 1 0
        0xa7 => call!(and(Reg::A)),
        // XOR B 1:4 Z 0 0 0
        0xa8 => call!(xor(Reg::B)),
        // XOR C 1:4 Z 0 0 0
        0xa9 => call!(xor(Reg::C)),
        // XOR D 1:4 Z 0 0 0
        0xaa => call!(xor(Reg::D)),
        // XOR E 1:4 Z 0 0 0
        0xab => call!(xor(Reg::E)),
        // XOR H 1:4 Z 0 0 0
        0xac => call!(xor(Reg::H)),
        // XOR L 1:4 Z 0 0 0
        0xad => call!(xor(Reg::L)),
        // XOR (HL) 1:8 Z 0 0 0
        0xae => call!(xor(Reg::HL)),
        // XOR A 1:4 Z 0 0 0
        0xaf => call!(xor(Reg::A)),
        // OR B 1:4 Z 0 0 0
        0xb0 => call!(or(Reg::B)),
        // OR C 1:4 Z 0 0 0
        0xb1 => call!(or(Reg::C)),
        // OR D 1:4 Z 0 0 0
        0xb2 => call!(or(Reg::D)),
        // OR E 1:4 Z 0 0 0
        0xb3 => call!(or(Reg::E)),
        // OR H 1:4 Z 0 0 0
        0xb4 => call!(or(Reg::H)),
        // OR L 1:4 Z 0 0 0
        0xb5 => call!(or(Reg::L)),
        // OR (HL) 1:8 Z 0 0 0
        0xb6 => call!(or(Reg::HL)),
        // OR A 1:4 Z 0 0 0
        0xb7 => call!(or(Reg::A)),
        // CP B 1:4 Z 1 H C
        0xb8 => call!(cp(Reg::B)),
        // CP C 1:4 Z 1 H C
        0xb9 => call!(cp(Reg::C)),
        // CP D 1:4 Z 1 H C
        0xba => call!(cp(Reg::D)),
        // CP E 1:4 Z 1 H C
        0xbb => call!(cp(Reg::E)),
        // CP H 1:4 Z 1 H C
        0xbc => call!(cp(Reg::H)),
        // CP L 1:4 Z 1 H C
        0xbd => call!(cp(Reg::L)),
        // CP (HL) 1:8 Z 1 H C
        0xbe => call!(cp(Reg::HL)),
        // CP A 1:4 Z 1 H C
        0xbf => call!(cp(Reg::A)),
        // RET NZ 1:20/8 - - - -
        0xc0 => call!(ret(NZ)),
        // POP BC 1:12 - - - -
        0xc1 => call!(pop(Reg16::BC)),
        // JP NZ,a16 3:16/12 - - - -
        0xc2 => call!(jump(NZ)),
        // JP a16 3:16 - - - -
        0xc3 => call!(jump(None)),
        // CALL NZ,a16 3:24/12 - - - -
        0xc4 => call!(call(NZ)),
        // PUSH BC 1:16 - - - -
        0xc5 => call!(push(Reg16::BC)),
        // ADD A,d8 2:8 Z 0 H C
        0xc6 => call!(add(Reg::Im8)),
        // RST 00H 1:16 - - - -
        0xc7 => call!(rst(0x00)),
        // RET Z 1:20/8 - - - -
        0xc8 => call!(ret(Z)),
        // RET 1:16 - - - -
        0xc9 => call!(ret(None)),
        // JP Z,a16 3:16/12 - - - -
        0xca => call!(jump(Z)),
        // PREFIX CB 1:4 - - - -
        0xcb => call!(interpret_op_cb()),
        // CALL Z,a16 3:24/12 - - - -
        0xcc => call!(call(Z)),
        // CALL a16 3:24 - - - -
        0xcd => call!(call(None)),
        // ADC A,d8 2:8 Z 0 H C
        0xce => call!(adc(Reg::Im8)),
        // RST 08H 1:16 - - - -
        0xcf => call!(rst(0x08)),
        // RET NC 1:20/8 - - - -
        0xd0 => call!(ret(NC)),
        // POP DE 1:12 - - - -
        0xd1 => call!(pop(Reg16::DE)),
        // JP NC,a16 3:16/12 - - - -
        0xd2 => call!(jump(NC)),
        //
        0xd3 => call!(invalid_opcode(0xd3)),
        // CALL NC,a16 3:24/12 - - - -
        0xd4 => call!(call(NC)),
        // PUSH DE 1:16 - - - -
        0xd5 => call!(push(Reg16::DE)),
        // SUB d8 2:8 Z 1 H C
        0xd6 => call!(sub(Reg::Im8)),
        // RST 10H 1:16 - - - -
        0xd7 => call!(rst(0x10)),
        // RET C 1:20/8 - - - -
        0xd8 => call!(ret(C)),
        // RETI 1:16 - - - -
        0xd9 => call!(reti()),
        // JP C,a16 3:16/12 - - - -
        0xda => call!(jump(C)),
        //
        0xdb => call!(invalid_opcode(0xdb)),
        // CALL C,a16 3:24/12 - - - -
        0xdc => call!(call(C)),
        //
        0xdd => call!(invalid_opcode(0xdd)),
        // SBC A,d8 2:8 Z 1 H C
        0xde => call!(sbc(Reg::Im8)),
        // RST 18H 1:16 - - - -
        0xdf => call!(rst(0x18)),
        // LDH (a8),A 2:12 - - - -
        0xe0 => call!(loadh(Reg::Im8, Reg::A)),
        // POP HL 1:12 - - - -
        0xe1 => call!(pop(Reg16::HL)),
        // LD (C),A 2:8 - - - -
        0xe2 => call!(loadh(Reg::C, Reg::A)),
        //
        0xe3 => call!(invalid_opcode(0xe3)),
        //
        0xe4 => call!(invalid_opcode(0xe4)),
        // PUSH HL 1:16 - - - -
        0xe5 => call!(push(Reg16::HL)),
        // AND d8 2:8 Z 0 1 0
        0xe6 => call!(and(Reg::Im8)),
        // RST 20H 1:16 - - - -
        0xe7 => call!(rst(0x20)),
        // ADD SP,r8 2:16 0 0 H C
        0xe8 => call!(add_sp()),
        // JP HL 1:4 - - - -
        0xe9 => call!(jump_hl()),
        // LD (a16),A 3:16 - - - -
        0xea => call!(load(Reg::Im16, Reg::A)),
        //
        0xeb => call!(invalid_opcode(0xeb)),
        //
        0xec => call!(invalid_opcode(0xec)),
        //
        0xed => call!(invalid_opcode(0xed)),
        // XOR d8 2:8 Z 0 0 0
        0xee => call!(xor(Reg::Im8)),
        // RST 28H 1:16 - - - -
        0xef => call!(rst(0x28)),
        // LDH A,(a8) 2:12 - - - -
        0xf0 => call!(loadh(Reg::A, Reg::Im8)),
        // POP AF 1:12 Z N H C
        0xf1 => call!(pop(Reg16::AF)),
        // LD A,(C) 2:8 - - - -
        0xf2 => call!(loadh(Reg::A, Reg::C)),
        // DI 1:4 - - - -
        0xf3 => call!(di()),
        //
        0xf4 => call!(invalid_opcode(0xf4)),
        // PUSH AF 1:16 - - - -
        0xf5 => call!(push(Reg16::AF)),
        // OR d8 2:8 Z 0 0 0
        0xf6 => call!(or(Reg::Im8)),
        // RST 30H 1:16 - - - -
        0xf7 => call!(rst(0x30)),
        // LD HL,SP+r8 2:12 0 0 H C
        0xf8 => call!(ldhl_sp()),
        // LD SP,HL 1:8 - - - -
        0xf9 => call!(load16(Reg16::SP, Reg16::HL)),
        // LD A,(a16) 3:16 - - - -
        0xfa => call!(load(Reg::A, Reg::Im16)),
        // EI 1:4 - - - -
        0xfb => call!(ei()),
        //
        0xfc => call!(invalid_opcode(0xfc)),
        //
        0xfd => call!(invalid_opcode(0xfd)),
        // CP d8 2:8 Z 1 H C
        0xfe => call!(cp(Reg::Im8)),
        // RST 38H 1:16 - - - -
        0xff => call!(rst(0x38)),
    }
}
